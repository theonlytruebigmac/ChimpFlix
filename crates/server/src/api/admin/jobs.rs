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
use chimpflix_library::{JobRow, JobStatus, JobSummary};
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub jobs: Vec<JobRow>,
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
    let jobs = queries::list_jobs(&state.pool, q.kind.as_deref(), status, limit)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(ListResponse { jobs }))
}

#[derive(Debug, Serialize)]
pub struct RequeueResponse {
    pub requeued: bool,
}

pub async fn requeue(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(job_id): Path<i64>,
) -> Result<(StatusCode, Json<RequeueResponse>), ApiError> {
    let requeued = queries::requeue_job(&state.pool, job_id)
        .await
        .map_err(ApiError::Internal)?;
    if !requeued {
        return Err(ApiError::NotFound);
    }
    Ok((StatusCode::OK, Json(RequeueResponse { requeued: true })))
}

/// Sweep every existing file that lacks any pipeline artifact and
/// enqueue the corresponding jobs. One-shot backfill — useful after
/// upgrading to the discovery pipeline or restoring from a backup
/// without artifact tables. Owner-only because a careless click on a
/// large library can enqueue tens of thousands of rows.
pub async fn process_all_pending(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<(StatusCode, Json<crate::jobs::pipeline::SweepCounts>), ApiError> {
    let counts = crate::jobs::pipeline::enqueue_full_sweep(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok((StatusCode::ACCEPTED, Json(counts)))
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
    _owner: OwnerAuth,
    Query(q): Query<WipeQuery>,
) -> Result<Json<WipeResponse>, ApiError> {
    let removed = queries::wipe_queued_jobs(&state.pool, q.kind.as_deref())
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(WipeResponse { removed }))
}
