//! Request extractors that resolve a session cookie into the current
//! user (or 401 the request).

use axum::extract::FromRequestParts;
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use chimpflix_common::now_ms;
use chimpflix_library::UserRole;
use chimpflix_library::queries;

use crate::api::error::ApiError;
use crate::auth::{COOKIE_NAME, cookie};
use crate::state::AppState;

/// The authenticated user. Reject the request with 401 if no valid session.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i64,
    /// Available for handler-side logging; not yet consumed by callers.
    #[allow(dead_code)]
    pub username: String,
    pub role: UserRole,
    /// Row id of the session that authenticated this request. Used by
    /// "sign out of all OTHER devices" and by session-rotation paths
    /// that want to spare the current session.
    pub session_id: i64,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
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

/// Wraps `AuthUser` and additionally enforces `role = owner`.
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
