//! Request extractors that resolve a session cookie into the current
//! user (or 401 the request).

use std::net::IpAddr;
use std::str::FromStr;

use axum::extract::FromRequestParts;
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use chimpflix_common::now_ms;
use chimpflix_library::UserRole;
use chimpflix_library::queries;

use crate::api::error::ApiError;
use crate::api::rate_limit;
use crate::auth::{COOKIE_NAME, cookie};
use crate::net;
use crate::state::AppState;

/// Sentinel session id used by the network-bypass path. The bypass
/// fakes an AuthUser without a real DB-backed session row; this
/// sentinel keeps `session_id` typed as `i64` (no Option) while still
/// being recognisable to any code path that wants to opt out of
/// session-scoped operations (e.g. "sign out other devices" can't
/// sensibly target a sessionless bypass caller). Negative so it can't
/// collide with a real autoincrement id.
pub const BYPASS_SESSION_ID: i64 = -1;

/// The authenticated user. Reject the request with 401 if no valid session.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    /// Available for handler-side logging; not yet consumed by callers.
    #[allow(dead_code)]
    pub username: String,
    pub role: UserRole,
    /// Row id of the session that authenticated this request, or
    /// `BYPASS_SESSION_ID` when the request was approved via a
    /// network bypass CIDR. Used by "sign out of all OTHER devices"
    /// and by session-rotation paths that want to spare the current
    /// session.
    pub session_id: i64,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        // Network-level bypass: when the request's client IP matches
        // an operator-configured CIDR, we treat it as the server
        // owner without requiring a session cookie. Used for LAN
        // automation (Home Assistant, monitoring scripts, cron jobs)
        // against an internet-exposed server. Empty list (default)
        // disables this entirely.
        let bypass_raw = state.settings.read().await.auth_bypass_cidrs.clone();
        if !bypass_raw.trim().is_empty() {
            if let Some(ip) = client_ip(parts) {
                let nets = net::parse_cidr_list(&bypass_raw);
                if net::ip_in_list(ip, &nets) {
                    if let Some(owner) = queries::find_first_owner(&state.pool)
                        .await
                        .map_err(ApiError::Internal)?
                    {
                        return Ok(AuthUser {
                            id: owner.id,
                            username: owner.username,
                            role: owner.role,
                            session_id: BYPASS_SESSION_ID,
                        });
                    }
                    // No owner user on this deployment — fall through
                    // to normal cookie auth so the request still
                    // produces a coherent 401 rather than silently
                    // succeeding with no user identity.
                }
            }
        }

        let header = parts
            .headers
            .get(COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let raw = cookie::find_cookie(header, COOKIE_NAME).ok_or(ApiError::Unauthorized)?;
        let (session_id, nonce) =
            cookie::parse_value(raw, &state.auth.session_secret).ok_or(ApiError::Unauthorized)?;

        let session = queries::find_session(&state.pool, session_id)
            .await
            .map_err(ApiError::Internal)?
            .ok_or(ApiError::Unauthorized)?;

        if session.nonce != nonce || session.expires_at < now_ms() {
            return Err(ApiError::Unauthorized);
        }

        let user = queries::find_user_by_id(&state.pool, session.user_id)
            .await
            .map_err(ApiError::Internal)?
            .ok_or(ApiError::Unauthorized)?;

        // Best-effort: update last_seen_at without blocking the request.
        let pool = state.pool.clone();
        let sid = session.id;
        tokio::spawn(async move {
            let _ = queries::touch_session(&pool, sid).await;
        });

        Ok(AuthUser {
            id: user.id,
            username: user.username,
            role: user.role,
            session_id: session.id,
        })
    }
}

/// Pull the client IP from headers (X-Forwarded-For / X-Real-IP) and
/// parse it into an IpAddr. Returns None when the headers are absent
/// or the value is malformed — bypass logic treats "no IP" as "no
/// match", which keeps the safe default.
fn client_ip(parts: &Parts) -> Option<IpAddr> {
    let raw = rate_limit::header_client_ip(&parts.headers)?;
    IpAddr::from_str(&raw).ok()
}

/// Wraps `AuthUser` and additionally enforces `role = owner`. The
/// strongest gate — use for truly sensitive operations (credentials
/// vault, library mounts, server URLs, owner-role mutations,
/// destructive maintenance / backup ops).
#[derive(Debug, Clone)]
pub struct OwnerAuth(pub AuthUser);

impl FromRequestParts<AppState> for OwnerAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if user.role != UserRole::Owner {
            return Err(ApiError::Forbidden);
        }
        Ok(OwnerAuth(user))
    }
}

/// Wraps `AuthUser` and enforces `role IN (owner, admin)`. Use this
/// for the bulk of admin-surface routes (user CRUD, library settings,
/// invites, scheduled tasks, etc.). Endpoints that need to act on
/// another user account must additionally enforce the role hierarchy
/// via [`crate::auth::can_act_on`] — admins must never modify owner
/// accounts even when reaching an admin-gated handler.
#[derive(Debug, Clone)]
pub struct AdminAuth(pub AuthUser);

impl FromRequestParts<AppState> for AdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let user = AuthUser::from_request_parts(parts, state).await?;
        if !user.role.is_admin_or_owner() {
            return Err(ApiError::Forbidden);
        }
        Ok(AdminAuth(user))
    }
}
