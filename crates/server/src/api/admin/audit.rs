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
    /// Legacy callers (cursor-paginated). When `offset` is present
    /// it wins.
    #[serde(default)]
    pub before: Option<i64>,
    /// Page size; clamped server-side to 1..=200.
    #[serde(default)]
    pub limit: Option<i64>,
    /// 0-based row offset for the paginated admin UI. When set,
    /// the response includes `total` + `entries` for offset/limit
    /// navigation; `next_before` is still emitted so cursor
    /// consumers stay working.
    #[serde(default)]
    pub offset: Option<i64>,
    /// When set, filter to only entries authored by this user id.
    /// Drives the per-user Audit tab in the user-management drawer.
    #[serde(default)]
    pub actor_user_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ListResponse {
    pub entries: Vec<AuditLogEntry>,
    pub next_before: Option<i64>,
    /// Total rows in audit_log. Drives the paginated admin
    /// surface's "X–Y of Z" summary + jump-to-page. Present on
    /// every response so clients can opt in without a second call.
    pub total: i64,
}

pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(params): Query<ListParams>,
) -> Result<Json<ListResponse>, ApiError> {
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let offset = params.offset.unwrap_or(0);
    let (entries, total) = if let Some(actor) = params.actor_user_id {
        let entries = queries::list_audit_for_user(&state.pool, actor, limit, offset)
            .await
            .map_err(ApiError::Internal)?;
        let total = queries::count_audit_for_user(&state.pool, actor)
            .await
            .map_err(ApiError::Internal)?;
        (entries, total)
    } else if params.offset.is_some() {
        let entries = queries::list_audit_paged(&state.pool, limit, offset)
            .await
            .map_err(ApiError::Internal)?;
        let total = queries::count_audit(&state.pool)
            .await
            .map_err(ApiError::Internal)?;
        (entries, total)
    } else {
        let entries = queries::list_audit(&state.pool, params.before, limit)
            .await
            .map_err(ApiError::Internal)?;
        let total = queries::count_audit(&state.pool)
            .await
            .map_err(ApiError::Internal)?;
        (entries, total)
    };
    let next_before = entries.last().map(|e| e.id);
    Ok(Json(ListResponse {
        entries,
        next_before,
        total,
    }))
}
