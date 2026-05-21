//! `/api/v1/notifications/*` — list / mark-read / mark-all-read.
//!
//! Notifications belong to a single recipient (`user_id`). Every
//! endpoint here scopes by the authenticated user; there's no
//! "all notifications across the system" admin view — admins read
//! their own inbox like everyone else.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chimpflix_library::{Notification, queries};
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct NotificationsListResponse {
    pub notifications: Vec<Notification>,
    pub unread: i64,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// 1-200; defaults to 50. The bell UI shows ~10 at a time so the
    /// default leaves room for "load more" without a follow-up call.
    #[serde(default)]
    pub limit: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<NotificationsListResponse>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let notifications = queries::list_notifications(&state.pool, user.id, limit)
        .await
        .map_err(ApiError::Internal)?;
    let unread = queries::count_unread_notifications(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(NotificationsListResponse {
        notifications,
        unread,
    }))
}

#[derive(Debug, Serialize)]
pub struct UnreadCountResponse {
    pub unread: i64,
}

/// Lightweight polling endpoint for the bell badge. Avoids paying the
/// row-fetch cost when the UI just wants the count.
pub async fn unread_count(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<UnreadCountResponse>, ApiError> {
    let unread = queries::count_unread_notifications(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(UnreadCountResponse { unread }))
}

pub async fn mark_read(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let updated = queries::mark_notification_read(&state.pool, user.id, id)
        .await
        .map_err(ApiError::Internal)?;
    if updated {
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Already read OR not yours — either way a 404 is the correct
        // anti-enumeration response.
        Err(ApiError::NotFound)
    }
}

#[derive(Debug, Serialize)]
pub struct MarkAllResponse {
    pub marked: u64,
}

pub async fn mark_all_read(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<MarkAllResponse>, ApiError> {
    let marked = queries::mark_all_notifications_read(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(MarkAllResponse { marked }))
}
