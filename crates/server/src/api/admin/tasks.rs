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
    validate_frequency(&input.frequency)?;
    validate_params(&input.kind, &input.params_json)?;
    let next = scheduler::compute_next_run_with_settings(
        &state,
        &input.frequency,
        &input.cron_expr,
        now_ms(),
        input.requires_maintenance_window,
    )
    .await
    .map_err(|e| ApiError::validation(format!("{e:#}")))?;

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
    let existing = queries::get_scheduled_task(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if let Some(ref freq) = input.frequency {
        validate_frequency(freq)?;
    }
    if let Some(ref params) = input.params_json {
        validate_params(&existing.kind, params)?;
    }

    // Recompute next_run_at if any schedule-affecting field is being
    // changed (frequency, cron, or window toggle). Merge the requested
    // patch with the existing row to feed the computer the *effective*
    // new state.
    let schedule_changed = input.frequency.is_some()
        || input.cron_expr.is_some()
        || input.requires_maintenance_window.is_some();
    let recomputed_next = if schedule_changed {
        let freq = input
            .frequency
            .as_deref()
            .unwrap_or(existing.frequency.as_str());
        let cron = input
            .cron_expr
            .as_deref()
            .unwrap_or(existing.cron_expr.as_str());
        let requires = input
            .requires_maintenance_window
            .unwrap_or(existing.requires_maintenance_window);
        Some(
            scheduler::compute_next_run_with_settings(&state, freq, cron, now_ms(), requires)
                .await
                .map_err(|e| ApiError::validation(format!("{e:#}")))?,
        )
    } else {
        None
    };

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

/// Accept any of the frequency enum values understood by the scheduler.
/// `custom` requires a valid cron_expr (validated later when the
/// computer parses it); other values use the fixed-interval table.
fn validate_frequency(frequency: &str) -> Result<(), ApiError> {
    const VALID: &[&str] = &[
        "manual",
        "hourly",
        "every_3_hours",
        "every_6_hours",
        "every_12_hours",
        "daily",
        "every_3_days",
        "weekly",
        "monthly",
        "on_change",
        "custom",
    ];
    if !VALID.contains(&frequency) {
        return Err(ApiError::validation(format!(
            "unknown frequency `{frequency}` — valid: {}",
            VALID.join(", ")
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
        "scan_library" => {
            if !parsed.get("library_id").and_then(|v| v.as_i64()).is_some() {
                return Err(ApiError::validation(format!(
                    "{kind} requires params.library_id (integer)"
                )));
            }
        }
        // `detect_markers` accepts an OPTIONAL library_id; omitted =
        // run for every library. The scheduler dispatch iterates
        // libraries when missing. The Plex-style simple-view toggle
        // relies on this — the row it creates has empty params.
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
