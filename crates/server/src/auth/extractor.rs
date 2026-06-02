//! Request extractors that resolve a session cookie into the current
//! user (or 401 the request).

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::LazyLock;

use axum::extract::FromRequestParts;
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use chimpflix_common::now_ms;
use chimpflix_library::UserRole;
use chimpflix_library::queries;
use tokio::sync::RwLock;

use crate::api::error::ApiError;
use crate::auth::{cookie, cookie_name};
use crate::client_ip::EffectiveClientIp;
use crate::net;
use crate::state::AppState;

/// Process-local set of client IPs that have already triggered an
/// info-level "network bypass granted" audit log. Keeps logging
/// quiet for chatty LAN clients while still surfacing the first
/// grant from each new source — usually enough for an operator to
/// notice if their bypass CIDR is wider than they intended.
static NETWORK_BYPASS_LOGGED: LazyLock<RwLock<HashSet<IpAddr>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

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
    /// The user's `kids_safe` preference, copied from the user row the
    /// extractor already loaded (no extra DB read). When true, listing
    /// handlers restrict results to kid-safe-certified titles. Defaults
    /// to `false` for the network-bypass owner + cast-token paths where
    /// preserving the historical (unfiltered) behaviour is correct — a
    /// LAN automation token or Cast session shouldn't inherit a viewer's
    /// kid-safe filter. The cookie auth path carries the real value.
    pub kids_safe: bool,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        // Network-level bypass: when the request's *effective* client
        // IP matches an operator-configured CIDR, we treat it as the
        // server owner without requiring a session cookie. Used for
        // LAN automation (Home Assistant, monitoring scripts, cron
        // jobs) against an internet-exposed server. Empty list
        // (default) disables this entirely.
        //
        // Security: `EffectiveClientIp` is set by the trusted-proxy
        // middleware (see [`crate::client_ip`]). The effective IP is
        // derived from proxy headers ONLY when the immediate peer
        // is in `TRUSTED_PROXIES`; otherwise it is the peer socket
        // address. This means a public attacker cannot spoof
        // `X-Forwarded-For: 192.168.x.x` to gain owner — without a
        // trusted proxy in front, headers are ignored, and the LAN
        // CIDR won't match a public peer IP.
        let bypass_raw = state.settings.read().await.auth_bypass_cidrs.clone();
        if !bypass_raw.trim().is_empty() {
            if let Some(ip) = client_ip(parts) {
                let nets = net::parse_cidr_list(&bypass_raw);
                if net::ip_in_list(ip, &nets) {
                    if let Some(owner) = queries::find_first_owner(&state.pool)
                        .await
                        .map_err(ApiError::Internal)?
                    {
                        // Audit-log the first bypass per (process, IP).
                        // Operators sometimes forget the CIDR is even
                        // configured; logging the grant once per IP
                        // makes "wait, why does that request have
                        // Owner with no cookie" debuggable. Subsequent
                        // bypasses from the same IP stay quiet so
                        // chatty LAN clients don't spam the log.
                        if NETWORK_BYPASS_LOGGED.write().await.insert(ip) {
                            tracing::info!(
                                client_ip = %ip,
                                owner_id = owner.id,
                                "auth: network-bypass CIDR matched — granting Owner without session cookie"
                            );
                        }
                        return Ok(AuthUser {
                            id: owner.id,
                            username: owner.username,
                            role: owner.role,
                            session_id: BYPASS_SESSION_ID,
                            // Network-bypass automation runs unfiltered —
                            // it stands in for the owner's full visibility.
                            kids_safe: false,
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
        let raw = cookie::find_cookie(header, cookie_name(state.auth.cookie_secure))
            .ok_or(ApiError::Unauthorized)?;
        let (session_id, nonce) =
            cookie::parse_value(raw, &state.auth.session_secret).ok_or(ApiError::Unauthorized)?;

        let session = queries::find_session(&state.pool, session_id)
            .await
            .map_err(ApiError::Internal)?
            .ok_or(ApiError::Unauthorized)?;

        // Compare hashes — DB stores SHA-256(nonce), cookie carries
        // the raw nonce. Equal-length-array comparison is fine: any
        // mismatch fails, and we have no timing-leak risk because the
        // attacker would need a valid (session_id, raw nonce) pair to
        // get this far in the first place.
        let cookie_nonce_hash = queries::hash_session_nonce(&nonce);
        let now = now_ms();
        if session.nonce_hash != cookie_nonce_hash || session.expires_at < now {
            return Err(ApiError::Unauthorized);
        }
        // Idle-session timeout: reject anything older than the idle
        // window since last_seen_at, even if the absolute cap hasn't
        // elapsed. Protects against "I left my laptop at the coffee
        // shop two weeks ago" — the session can no longer be silently
        // resumed. Note: BYPASS_SESSION_ID (network bypass) doesn't
        // come through here, so this only applies to real cookies.
        let idle_ms = now - session.last_seen_at;
        if idle_ms > crate::auth::SESSION_IDLE_TIMEOUT_S * 1000 {
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
            // Free read — `user` was already fetched above for auth.
            kids_safe: user.kids_safe,
        })
    }
}

/// Pull the effective client IP stashed by [`crate::client_ip::middleware`].
/// Returns None when the middleware hasn't run (e.g. in tests that build
/// an extractor invocation by hand) — bypass logic treats "no IP" as "no
/// match", which keeps the safe default.
fn client_ip(parts: &Parts) -> Option<IpAddr> {
    parts.extensions.get::<EffectiveClientIp>().map(|e| e.0)
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

/// Stream-endpoint extractor: accepts either the normal session cookie
/// or a short-lived `?ct=<token>` query parameter minted via
/// [`crate::auth::cast_token`]. Used on the manifest + segment routes
/// that a Chromecast receiver fetches directly (no cookie jar).
///
/// Token verification short-circuits the cookie path entirely; it
/// returns a synthetic `AuthUser` with `session_id = 0` (never a real
/// row id) so consumers that key off `session_id` can opt out of cast
/// requests if they care. Library access still flows through
/// `ensure_*_accessible`, so the token only ever grants what the user
/// could already see in their own browser.
#[derive(Debug, Clone)]
pub struct StreamAuthUser(pub AuthUser);

impl FromRequestParts<AppState> for StreamAuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        // Cast-token path first — if a `?ct=` query param is present
        // and verifies, skip the cookie machinery entirely. The token
        // is a transparent stand-in for the cookie; it grants the same
        // library access and nothing more.
        if let Some(query) = parts.uri.query() {
            if let Some(token) = extract_cast_token_param(query) {
                if let Some((user_id, _exp)) =
                    crate::auth::cast_token::verify(&token, &state.auth.session_secret)
                {
                    let user = queries::find_user_by_id(&state.pool, user_id)
                        .await
                        .map_err(ApiError::Internal)?
                        .ok_or(ApiError::Unauthorized)?;
                    return Ok(StreamAuthUser(AuthUser {
                        id: user.id,
                        username: user.username,
                        role: user.role,
                        // Sentinel: 0 ≠ any real session row id. Lets
                        // session-scoped operations notice "this isn't
                        // a real session" if they care, while keeping
                        // the field typed as `i64`.
                        session_id: 0,
                        // Streaming/cast playback isn't a listing surface;
                        // the kid-safe filter only gates browse/home lists.
                        kids_safe: false,
                    }));
                }
                // Token present but invalid — fall through to cookie
                // auth. A stale token shouldn't lock the user out if
                // their browser tab still has a working cookie.
            }
        }
        let inner = AuthUser::from_request_parts(parts, state).await?;
        Ok(StreamAuthUser(inner))
    }
}

/// Extract the value of the first `ct=...` parameter from a query
/// string. We hand-parse instead of pulling in a query-string crate
/// for one lookup — the only caller is the StreamAuthUser extractor
/// and the format is fixed.
fn extract_cast_token_param(query: &str) -> Option<String> {
    for pair in query.split('&') {
        if let Some(value) = pair.strip_prefix("ct=") {
            // URL-decode in case the receiver added padding/escapes.
            // We only really expect base64url charset (already URL-safe)
            // but the Cast SDK has been known to escape `=` defensively
            // on older receiver versions.
            return Some(value.replace("%3D", "=").replace("%2F", "/"));
        }
    }
    None
}

/// "Optional auth" wrapper for handlers that behave differently for
/// signed-in vs anonymous callers — instead of failing the whole
/// request with 401 when no session is present, missing/invalid
/// credentials yield `MaybeAuthUser(None)`.
///
/// Used by the Plex OAuth `start` / `poll` handlers: anonymous callers
/// drive Login or Signup; signed-in callers drive Link. A single
/// extractor lets one handler cover all three cases.
#[derive(Debug, Clone)]
pub struct MaybeAuthUser(pub Option<AuthUser>);

impl FromRequestParts<AppState> for MaybeAuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        match AuthUser::from_request_parts(parts, state).await {
            Ok(user) => Ok(MaybeAuthUser(Some(user))),
            // 401 / 403 → anonymous. Any other error (DB failure,
            // corrupt session row) should still surface so the caller
            // sees a real 500 instead of silently auth-less.
            Err(ApiError::Unauthorized) | Err(ApiError::Forbidden) => Ok(MaybeAuthUser(None)),
            Err(other) => Err(other),
        }
    }
}
