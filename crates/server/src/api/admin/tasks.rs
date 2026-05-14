//! `/admin/tasks*` — CRUD + run-now + history for scheduled tasks.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::http::header::USER_AGENT;
use chimpflix_common::now_ms;
use chimpflix_library::{
    NewAuditEntry, NewScheduledTask, ScheduledTask, ScheduledTaskUpdate, TaskRun, queries,
};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::scheduler;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct TasksListResponse {
    pub tasks: Vec<ScheduledTask>,
    pub kinds: Vec<scheduler::TaskKindInfo>,
}

#[derive(Debug, Serialize)]
pub struct TaskResponse {
    pub task: ScheduledTask,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<TasksListResponse>, ApiError> {
    let tasks = queries::list_scheduled_tasks(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(TasksListResponse {
        tasks,
        kinds: scheduler::registry(),
    }))
}

pub async fn create(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<NewScheduledTask>,
) -> Result<(StatusCode, Json<TaskResponse>), ApiError> {
    validate_kind(&input.kind)?;
    let next = scheduler::next_after(&input.cron_expr, now_ms())
        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    validate_params(&input.kind, &input.params_json)?;

    let task = queries::create_scheduled_task(&state.pool, input.clone(), next)
        .await
        .map_err(ApiError::Internal)?;

    audit(&state, actor.id, &headers, "task.create", task.id, &input).await;

    Ok((StatusCode::CREATED, Json(TaskResponse { task })))
}

pub async fn update(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ScheduledTaskUpdate>,
) -> Result<Json<TaskResponse>, ApiError> {
    if queries::get_scheduled_task(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    let recomputed_next = if let Some(ref expr) = input.cron_expr {
        Some(
            scheduler::next_after(expr, now_ms())
                .map_err(|e| ApiError::validation(format!("{e:#}")))?,
        )
    } else {
        None
    };
    if let Some(ref params) = input.params_json {
        let existing = queries::get_scheduled_task(&state.pool, id)
            .await
            .map_err(ApiError::Internal)?
            .ok_or(ApiError::NotFound)?;
        validate_params(&existing.kind, params)?;
    }

    let task = queries::update_scheduled_task(&state.pool, id, input.clone(), recomputed_next)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    audit(&state, actor.id, &headers, "task.update", id, &input).await;

    Ok(Json(TaskResponse { task }))
}

pub async fn delete(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let removed = queries::delete_scheduled_task(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    audit(&state, actor.id, &headers, "task.delete", id, &()).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn run_now(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    scheduler::run_now(state.clone(), id)
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    audit(&state, actor.id, &headers, "task.run_now", id, &()).await;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Deserialize)]
pub struct RunsParams {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunsResponse {
    pub runs: Vec<TaskRun>,
}

pub async fn list_runs(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(id): Path<i64>,
    Query(params): Query<RunsParams>,
) -> Result<Json<RunsResponse>, ApiError> {
    let runs = queries::list_task_runs(&state.pool, id, params.limit.unwrap_or(50))
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(RunsResponse { runs }))
}

fn validate_kind(kind: &str) -> Result<(), ApiError> {
    let known: Vec<&str> = scheduler::registry().into_iter().map(|t| t.kind).collect();
    if !known.iter().any(|k| *k == kind) {
        return Err(ApiError::validation(format!(
            "unknown task kind `{kind}` — valid kinds: {}",
            known.join(", ")
        )));
    }
    Ok(())
}

fn validate_params(kind: &str, params_json: &str) -> Result<(), ApiError> {
    let parsed: serde_json::Value = serde_json::from_str(params_json)
        .map_err(|e| ApiError::validation(format!("params_json must be JSON: {e}")))?;
    if !parsed.is_object() {
        return Err(ApiError::validation("params_json must be a JSON object"));
    }
    // Kind-specific shape checks.
    match kind {
        "scan_library" | "detect_markers" => {
            if !parsed.get("library_id").and_then(|v| v.as_i64()).is_some() {
                return Err(ApiError::validation(format!(
                    "{kind} requires params.library_id (integer)"
                )));
            }
        }
        _ => {}
    }
    Ok(())
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
            target_kind: Some("task".into()),
            target_id: Some(target_id.to_string()),
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}
