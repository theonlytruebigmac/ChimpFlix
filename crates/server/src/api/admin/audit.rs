//! `GET /admin/audit` — paginated admin action log.

use axum::Json;
use axum::extract::{Query, State};
use chimpflix_library::{AuditLogEntry, queries};
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ListParams {
    /// Cursor: return entries with id strictly less than this value.
    #[serde(default)]
    pub before: Option<i64>,
    /// Page size; clamped server-side to 1..=200.
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub entries: Vec<AuditLogEntry>,
    pub next_before: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(params): Query<ListParams>,
) -> Result<Json<ListResponse>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let entries = queries::list_audit(&state.pool, params.before, limit)
        .await
        .map_err(ApiError::Internal)?;
    let next_before = entries.last().map(|e| e.id);
    Ok(Json(ListResponse {
        entries,
        next_before,
    }))
}
