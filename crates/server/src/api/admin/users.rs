//! /admin/users*, /admin/sessions*, /admin/access — Phase 8 surface.
//!
//! User CRUD + invites are also reachable at /auth/users and /auth/invites
//! (existing). These mirrors live under /admin so the admin shell can
//! address everything from one namespace; the underlying handlers reuse
//! the same logic.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::http::header::USER_AGENT;
use chimpflix_library::{AccessMatrixEntry, NewAuditEntry, SessionSummary, queries};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SessionsListResponse {
    pub sessions: Vec<SessionSummary>,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<SessionsListResponse>, ApiError> {
    let sessions = queries::list_all_sessions(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(SessionsListResponse { sessions }))
}

pub async fn list_user_sessions(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(user_id): Path<i64>,
) -> Result<Json<SessionsListResponse>, ApiError> {
    let sessions = queries::list_user_sessions(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(SessionsListResponse { sessions }))
}

pub async fn revoke_session(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    queries::delete_session(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    audit(&state, actor.id, &headers, "session.revoke", id, &()).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn revoke_user_sessions(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(user_id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<RevokeResponse>, ApiError> {
    let count = queries::delete_user_sessions(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?;
    audit(
        &state,
        actor.id,
        &headers,
        "session.revoke_user",
        user_id,
        &serde_json::json!({ "count": count }),
    )
    .await;
    Ok(Json(RevokeResponse { revoked: count }))
}

#[derive(Debug, Serialize)]
pub struct RevokeResponse {
    pub revoked: u64,
}

/// Admin: wipe a user's TOTP enrollment + recovery codes. The user is
/// emailed nothing — admins typically only run this after a user has
/// directly asked because they lost their device. Login proceeds as
/// password-only until the user re-enrolls.
/// Admin: clear the in-memory login-attempt tracker for a user. Used
/// to unlock a user who got progressively backoff-locked out (e.g.
/// fat-fingered their password 6+ times). Doesn't change the user's
/// password — they just get to try again immediately. The matching
/// 2FA attempt key is also cleared for users with 2FA enabled.
pub async fn unlock_login_attempts(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let user = queries::find_user_by_id(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    // Same key shape the login handler uses (lowercase username).
    let pwd_key = user.username.trim().to_lowercase();
    state.login_attempts.clear(&pwd_key).await;
    // Plus the 2FA-specific bucket keyed by user id.
    let totp_key = format!("2fa:{user_id}");
    state.login_attempts.clear(&totp_key).await;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "user.login_attempts.unlock".into(),
            target_kind: Some("user".into()),
            target_id: Some(user_id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reset_user_totp(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Path(user_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let removed = queries::delete_user_totp(&state.pool, user_id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "user.2fa.reset".into(),
            target_kind: Some("user".into()),
            target_id: Some(user_id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    // Notify the OTHER admins so the action is visible across the team
    // (the actor doesn't need a notification of their own action).
    let actor_user = queries::find_user_by_id(&state.pool, actor.id)
        .await
        .ok()
        .flatten();
    let target_user = queries::find_user_by_id(&state.pool, user_id)
        .await
        .ok()
        .flatten();
    if let (Some(actor), Some(target)) = (actor_user, target_user) {
        crate::notifier::notify_two_factor_reset(&state, &actor, &target).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Serialize)]
pub struct AccessMatrixResponse {
    pub entries: Vec<AccessMatrixEntry>,
}

pub async fn get_access_matrix(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<AccessMatrixResponse>, ApiError> {
    let entries = queries::access_matrix(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AccessMatrixResponse { entries }))
}

#[derive(Debug, Deserialize)]
pub struct AccessUpdate {
    /// Bulk-replace shape: per library, the full list of allowed users.
    /// Omitted libraries are left as-is.
    pub libraries: Vec<LibraryAccessAssignment>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct LibraryAccessAssignment {
    pub library_id: i64,
    pub user_ids: Vec<i64>,
}

pub async fn put_access_matrix(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<AccessUpdate>,
) -> Result<Json<AccessMatrixResponse>, ApiError> {
    for assignment in &input.libraries {
        queries::set_library_user_ids(&state.pool, assignment.library_id, &assignment.user_ids)
            .await
            .map_err(ApiError::Internal)?;
    }
    audit(
        &state,
        actor.id,
        &headers,
        "access.matrix.update",
        0,
        &input.libraries,
    )
    .await;
    let entries = queries::access_matrix(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AccessMatrixResponse { entries }))
}

async fn audit<T: Serialize>(
    state: &AppState,
    actor_id: i64,
    headers: &HeaderMap,
    action: &str,
    target_id: i64,
    payload: &T,
) {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(actor_id),
            action: action.to_string(),
            target_kind: Some("user_admin".into()),
            target_id: if target_id == 0 {
                None
            } else {
                Some(target_id.to_string())
            },
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}
