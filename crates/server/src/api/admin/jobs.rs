//! `/admin/jobs` — operator-side queue introspection.
//!
//! Three endpoints back the Maintenance → Job queue admin page:
//!
//!   * `GET /admin/jobs/summary` — aggregate counts by status.
//!   * `GET /admin/jobs` — paged listing with kind + status filters.
//!   * `POST /admin/jobs/{id}/requeue` — flip a `failed`/`dead` row
//!     back to `queued` with a fresh attempt counter.
//!
//! Owner-gated. The queue moves CPU- and DB-intensive work; a typo
//! by a lower-tier admin (re-queueing thousands of dead rows in
//! one click) is the kind of foot-gun we keep behind the top role.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chimpflix_library::queries;
use chimpflix_library::{JobRow, JobStatus, JobSummary, NewAuditEntry};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SummaryResponse {
    #[serde(flatten)]
    pub summary: JobSummary,
}

pub async fn summary(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<SummaryResponse>, ApiError> {
    let summary = queries::job_summary(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(SummaryResponse { summary }))
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    /// 0-based row offset for paged reads. The admin UI sends
    /// `offset = (page - 1) * limit`; old callers that only sent
    /// `limit` continue to work (no offset = first page).
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub jobs: Vec<JobRow>,
    /// Total rows matching the kind/status filter — drives the
    /// pagination footer's "X–Y of Z" summary and last-page button.
    pub total: i64,
    /// Live per-job progress snapshot for whichever jobs in the
    /// `jobs` vec are currently executing. Keyed by `job.id`.
    /// Missing entries mean "not currently running" (queued,
    /// succeeded, dead, etc.) — the UI shows progress only for
    /// rows present here. Sourced from the in-memory
    /// `JobProgressStore`; resets on server restart.
    pub progress: std::collections::HashMap<i64, crate::jobs::progress::JobProgress>,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(q): Query<ListQuery>,
) -> Result<Json<ListResponse>, ApiError> {
    let status = if let Some(s) = q.status.as_deref() {
        Some(JobStatus::from_db_str(s).map_err(|e| ApiError::validation(format!("{e}")))?)
    } else {
        None
    };
    let limit = q.limit.unwrap_or(100);
    let offset = q.offset.unwrap_or(0);
    let (jobs, total) = tokio::try_join!(
        queries::list_jobs(&state.pool, q.kind.as_deref(), status, limit, offset),
        queries::count_jobs(&state.pool, q.kind.as_deref(), status),
    )
    .map_err(ApiError::Internal)?;
    // Filter the live progress snapshot to just the ids in the
    // returned page. A 50k-job library could otherwise serialize
    // tens of thousands of "Starting" entries the UI doesn't render.
    let page_ids: std::collections::HashSet<i64> = jobs.iter().map(|j| j.id).collect();
    let progress: std::collections::HashMap<i64, crate::jobs::progress::JobProgress> = state
        .job_progress
        .snapshot()
        .into_iter()
        .filter(|(id, _)| page_ids.contains(id))
        .collect();
    Ok(Json(ListResponse {
        jobs,
        total,
        progress,
    }))
}

#[derive(Debug, Serialize)]
pub struct RequeueResponse {
    pub requeued: bool,
}

pub async fn requeue(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(job_id): Path<i64>,
) -> Result<(StatusCode, Json<RequeueResponse>), ApiError> {
    let requeued = queries::requeue_job(&state.pool, job_id)
        .await
        .map_err(ApiError::Internal)?;
    if !requeued {
        return Err(ApiError::NotFound);
    }
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "job.requeue".into(),
            target_kind: Some("job".into()),
            target_id: Some(job_id.to_string()),
            payload_json: None,
            ip: None,
            user_agent: None,
        },
    )
    .await;
    Ok((StatusCode::OK, Json(RequeueResponse { requeued: true })))
}

/// Sweep every existing file that lacks any pipeline artifact and
/// enqueue the corresponding jobs. One-shot backfill — useful after
/// upgrading to the discovery pipeline or restoring from a backup
/// without artifact tables. Owner-only because a careless click on a
/// large library can enqueue tens of thousands of rows.
pub async fn process_all_pending(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
) -> Result<(StatusCode, Json<crate::jobs::pipeline::SweepCounts>), ApiError> {
    // Acquire the bulk-write lock so two concurrent operator clicks
    // (or this + a library-delete cascade) don't race for the SQLite
    // writer slot. The retry layer would catch the contention either
    // way; holding the lock means we don't even create it. The await
    // is brief — the lock is released as soon as `enqueue_full_sweep`
    // returns, before this handler responds to the client.
    let _permit = state
        .bulk_write_lock
        .acquire()
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("bulk_write_lock acquire: {e}")))?;
    let counts = crate::jobs::pipeline::enqueue_full_sweep(&state)
        .await
        .map_err(ApiError::Internal)?;
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "job.process_all_pending".into(),
            target_kind: None,
            target_id: None,
            // Record the enqueued counts so the audit trail shows how
            // much work was kicked off by the operator action.
            payload_json: serde_json::to_string(&counts).ok(),
            ip: None,
            user_agent: None,
        },
    )
    .await;
    Ok((StatusCode::OK, Json(counts)))
}

#[derive(Debug, Deserialize)]
pub struct WipeQuery {
    pub kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WipeResponse {
    pub removed: u64,
}

/// Delete all currently-queued rows (optionally scoped to one
/// kind). Use this when "Process all pending" was clicked by
/// mistake on a large library and the operator wants to bail out
/// before the worker has burned through too much CPU. Running
/// jobs are NOT killed — they finish their current file and then
/// stop because no more queued rows are eligible.
pub async fn wipe_queued(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Query(q): Query<WipeQuery>,
) -> Result<Json<WipeResponse>, ApiError> {
    let removed = queries::wipe_queued_jobs(&state.pool, q.kind.as_deref())
        .await
        .map_err(ApiError::Internal)?;
    // Log the kind filter (null = all kinds) and the deletion count so
    // the audit trail captures both scope and impact of the wipe.
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "job.wipe_queued".into(),
            target_kind: None,
            target_id: None,
            payload_json: Some(
                serde_json::to_string(&serde_json::json!({
                    "kind": q.kind,
                    "removed": removed,
                }))
                .unwrap_or_default(),
            ),
            ip: None,
            user_agent: None,
        },
    )
    .await;
    Ok(Json(WipeResponse { removed }))
}

/// Delete every `dead` row regardless of finished_at. Backs the admin
/// "Clear dead" button — used after a renamed/removed job kind leaves
/// orphaned rows that no handler will ever process (cleanup_old_jobs
/// still respects the TTL, so those rows would otherwise linger).
pub async fn clear_dead(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
) -> Result<Json<WipeResponse>, ApiError> {
    let removed = queries::clear_dead_jobs(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "job.clear_dead".into(),
            target_kind: None,
            target_id: None,
            payload_json: Some(format!(r#"{{"removed":{removed}}}"#)),
            ip: None,
            user_agent: None,
        },
    )
    .await;
    Ok(Json(WipeResponse { removed }))
}
