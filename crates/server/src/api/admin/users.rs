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
