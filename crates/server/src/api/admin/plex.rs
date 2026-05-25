//! `/admin/plex/*` — Plex OAuth identity management.
//!
//! The server stores a per-install Plex client identifier
//! (`server_settings.plex_client_identifier`) that's used to identify
//! ChimpFlix to Plex's API during PIN-based OAuth flows. The
//! identifier is generated lazily on first use and reused across
//! restarts so in-flight authorizations survive a redeploy.
//!
//! Operators occasionally want to rotate it — e.g. they suspect the
//! identifier has been logged somewhere it shouldn't have been, or
//! they're moving the install to a new operator and want the new
//! owner to look like a clean Plex client. Rotating doesn't
//! invalidate existing per-user Plex tokens (those are stored
//! separately under each user's auth-provider row); it only changes
//! what *future* PIN flows announce to Plex.

use axum::Json;
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, queries};
use serde::Serialize;
use tracing::info;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;
use axum::extract::State;

#[derive(Debug, Serialize)]
pub struct RotateIdentifierResponse {
    /// True when the rotation completed — the server will mint a
    /// fresh identifier on the next `/auth/plex/start` call. False
    /// is reserved for future "rotation refused" cases (e.g. an
    /// in-flight auth flow blocking) but currently always true.
    pub rotated: bool,
}

/// `POST /admin/plex/rotate-identifier` — drop the persisted Plex
/// client identifier so the next OAuth flow mints a fresh one.
///
/// Two-step: clear the DB column, then clear the cached
/// `PlexOAuthHandle`. The handle clear is what makes the rotation
/// take effect *immediately* — without it, the in-process cache
/// would keep serving the old identifier until restart.
///
/// Owner-only. Audit-logged so the operator's intent is on record.
pub async fn rotate_identifier(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
) -> Result<Json<RotateIdentifierResponse>, ApiError> {
    queries::clear_plex_client_identifier(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    *state.plex_oauth.write().await = None;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "plex.identifier.rotate".into(),
            target_kind: Some("server".into()),
            target_id: None,
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;

    info!(
        actor_user_id = actor.id,
        "plex client identifier rotated; next /auth/plex/start will mint a fresh UUID"
    );

    Ok(Json(RotateIdentifierResponse { rotated: true }))
}
