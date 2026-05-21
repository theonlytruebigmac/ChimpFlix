//! `/admin/optimized*` — queue + completed list for Optimized Versions.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, NewOptimizedVersion, OptimizedVersion, queries};
use serde::Serialize;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct OptimizedListResponse {
    pub versions: Vec<OptimizedVersion>,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<OptimizedListResponse>, ApiError> {
    let versions = queries::list_optimized_versions(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(OptimizedListResponse { versions }))
}

pub async fn create(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<NewOptimizedVersion>,
) -> Result<(StatusCode, Json<OptimizedVersion>), ApiError> {
    // Ensure both source file and preset exist before queuing.
    if queries::get_transcoder_preset(&state.pool, input.preset_id)
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::validation(format!(
            "preset {} not found",
            input.preset_id
        )));
    }
    // Cheap source-file existence check.
    let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_files WHERE id = ?")
        .bind(input.source_file_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if exists == 0 {
        return Err(ApiError::validation(format!(
            "media_file {} not found",
            input.source_file_id
        )));
    }

    let row = queries::enqueue_optimized_version(&state.pool, input.clone())
        .await
        .map_err(ApiError::Internal)?;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "optimized.enqueue".into(),
            target_kind: Some("optimized_version".into()),
            target_id: Some(row.id.to_string()),
            payload_json: serde_json::to_string(&input).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok((StatusCode::CREATED, Json(row)))
}

pub async fn delete(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let path = queries::delete_optimized_version(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    // Best-effort file cleanup.
    if let Some(p) = path {
        if !p.is_empty() {
            let _ = tokio::fs::remove_file(&p).await;
        }
    }
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "optimized.delete".into(),
            target_kind: Some("optimized_version".into()),
            target_id: Some(id.to_string()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}
