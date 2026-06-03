//! `/admin/versions*` — queue + completed list for Optimized Versions.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, StatusCode};
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

    // Reject if an encode is actively running for this (source, preset) pair.
    // The upsert in enqueue_optimized_version has no status guard, so without
    // this check a re-submit would flip a running row back to 'queued' while
    // the worker is still encoding — leaving the row stuck 'queued' forever
    // once the worker's mark_optimized_finished no-ops on the missing 'running'
    // guard. Return 409 so the caller knows to cancel first if desired.
    let running: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM optimized_versions
         WHERE source_file_id = ? AND preset_id = ? AND status = 'running'",
    )
    .bind(input.source_file_id)
    .bind(input.preset_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    if running > 0 {
        return Err(ApiError::Conflict(
            "an encode for this (source_file, preset) pair is already running; cancel it first"
                .into(),
        ));
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

/// Cancel a queued or running optimized version.
///
/// * Queued → the DB row is flipped straight to `cancelled`; the
///   worker's claim query only picks up `queued` rows, so it never
///   touches this one.
/// * Running → the row is flipped to `cancelled` AND the id is inserted
///   into the in-memory cancel set the `optimize_versions` worker polls
///   between ffmpeg progress reads. The worker then kills its ffmpeg
///   child and removes the partial output file.
/// * Terminal (`success` / `failed` / already `cancelled`) → no-op; the
///   route still returns 200 so the UI's optimistic refresh is harmless.
///
/// Owner-gated (via `OwnerAuth`) and audited.
pub async fn cancel(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<OptimizedVersion>, ApiError> {
    let outcome = queries::cancel_optimized_version(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    let Some((prior_status, _output_path)) = outcome else {
        return Err(ApiError::NotFoundResource("optimized version"));
    };

    // Tell the worker to kill its ffmpeg child for ANY non-terminal row —
    // not just one we read as `running`. A row can flip queued→running in
    // the window between cancel_optimized_version's status read and its
    // guarded UPDATE; the UPDATE flips it either way, but only the
    // cancel-set entry makes a just-started worker stop. A stale id for a
    // row that never ran is harmless (ids are never reused; the worker
    // clears ids it handles).
    if prior_status == "queued" || prior_status == "running" {
        state.request_optimize_cancel(id).await;
    }

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "optimized.cancel".into(),
            target_kind: Some("optimized_version".into()),
            target_id: Some(id.to_string()),
            payload_json: Some(
                serde_json::json!({ "prior_status": prior_status }).to_string(),
            ),
            ip: None,
            user_agent,
        },
    )
    .await;

    // Hand back the row's current state so the client can reconcile
    // without a second round-trip.
    let versions = queries::list_optimized_versions(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    let row = versions
        .into_iter()
        .find(|v| v.id == id)
        .ok_or(ApiError::NotFoundResource("optimized version"))?;
    Ok(Json(row))
}

pub async fn delete(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    // Signal the worker to kill its ffmpeg child BEFORE removing the DB row.
    // If the row is currently 'running', the worker polls the cancel set on
    // each progress tick; without this the worker would run to completion
    // wasting CPU/GPU cycles (the DB delete makes its mark_optimized_finished
    // a no-op, but ffmpeg keeps encoding until it exits naturally). Sending
    // the cancel for a non-running or non-existent id is harmless — the
    // worker clears ids it handles, and ids are never reused.
    state.request_optimize_cancel(id).await;

    let output_path = queries::delete_optimized_version(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    // None means no row was deleted — the id never existed (or was already
    // cleaned up). Mirror delete_preset's NotFound behaviour.
    let Some(p) = output_path else {
        return Err(ApiError::NotFound);
    };
    // Best-effort file cleanup.
    if !p.is_empty() {
        let _ = tokio::fs::remove_file(&p).await;
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
