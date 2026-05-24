//! `/admin/tasks` — list endpoint for the Admin Home dashboard's
//! "Up next" + "Recently run" cards. CRUD + run-now + history were
//! retired when the advanced editor was folded into the registry-
//! driven surface; per-kind editing now lives at
//! `/admin/tasks/kind/{kind}`.

use axum::Json;
use axum::extract::State;
use chimpflix_library::{ScheduledTask, queries};
use serde::Serialize;

use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::scheduler;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct TasksListResponse {
    pub tasks: Vec<ScheduledTask>,
    pub kinds: Vec<scheduler::TaskKindInfo>,
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
