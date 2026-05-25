//! Cast / AirPlay support endpoints.
//!
//! The browser SDKs (Google Cast Web Sender, Apple AirPlay via
//! `webkitShowPlaybackTargetPicker`) hand the receiver device an
//! absolute URL. The receiver fetches that URL directly with no
//! cookies, no Origin, no CSRF token — every browser-side credential
//! mechanism we lean on disappears.
//!
//! For AirPlay this is a non-issue: Safari/iOS reuses the local
//! `<video>` element's cookies when handshaking with the AirPlay
//! target, so the existing cookie auth still works.
//!
//! For Cast we need a token-in-URL fallback. This endpoint mints a
//! short-lived HMAC token bound to the calling user; the player
//! appends it as `?ct=<token>` to the manifest + segment URLs before
//! handing them to the Cast sender. The stream extractor
//! ([`crate::auth::StreamAuthUser`]) recognizes the token and treats
//! it as the user's cookie equivalent. Library access still flows
//! through `ensure_*_accessible`, so the token can't reach anything
//! the user couldn't already see.

use axum::Json;
use axum::extract::State;
use chimpflix_common::now_ms;
use serde::Serialize;

use crate::api::error::ApiError;
use crate::auth::{AuthUser, cast_token};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct CastSignResponse {
    pub token: String,
    pub expires_at_ms: i64,
}

/// Mint a cast token for the calling user. Always returns 200 — there
/// is no per-content access check here because the token only proxies
/// the user's existing rights; the actual library-access gate runs at
/// stream-fetch time. Operators don't need to whitelist files for
/// casting.
pub async fn sign(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<CastSignResponse>, ApiError> {
    let expires_at_ms = now_ms() + cast_token::DEFAULT_TTL_MS;
    let token = cast_token::mint(_user.id, expires_at_ms, &state.auth.session_secret);
    Ok(Json(CastSignResponse {
        token,
        expires_at_ms,
    }))
}
