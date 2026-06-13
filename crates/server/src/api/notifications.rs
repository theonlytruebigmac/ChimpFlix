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
use sqlx::Row;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct NotificationsListResponse {
    pub notifications: Vec<Notification>,
    pub unread: i64,
    /// Total rows for the user — drives the bell-page paginator and
    /// lets the UI surface "showing N of M" instead of silently
    /// truncating at the previous fixed `limit=200` ceiling.
    pub total: i64,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    /// 1-200; defaults to 50. The bell UI shows ~10 at a time so the
    /// default leaves room for "load more" without a follow-up call.
    #[serde(default)]
    pub limit: Option<i64>,
    /// Row offset for paging. 0-based.
    #[serde(default)]
    pub offset: Option<i64>,
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<NotificationsListResponse>, ApiError> {
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let offset = q.offset.unwrap_or(0).max(0);
    // Single query so the page rows, unread count, and total are all from
    // the same SQLite snapshot — no risk of the badge count drifting from
    // the list if a notification arrives or is marked-read between calls.
    let rows = sqlx::query(
        "SELECT n.*,
                (SELECT COUNT(*) FROM notifications WHERE user_id = ?) AS total,
                (SELECT COUNT(*) FROM notifications WHERE user_id = ? AND read_at IS NULL) AS unread
           FROM notifications n
          WHERE n.user_id = ?
          ORDER BY n.created_at DESC
          LIMIT ? OFFSET ?",
    )
    .bind(user.id)
    .bind(user.id)
    .bind(user.id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let (total, unread) = rows
        .first()
        .map(|r| {
            let t: i64 = r.try_get("total").unwrap_or(0);
            let u: i64 = r.try_get("unread").unwrap_or(0);
            (t, u)
        })
        .unwrap_or((0, 0));
    let notifications = rows
        .iter()
        .map(|r| -> anyhow::Result<Notification> {
            Ok(Notification {
                id: r.try_get("id")?,
                user_id: r.try_get("user_id")?,
                kind: r.try_get("kind")?,
                payload_json: r.try_get("payload_json")?,
                read_at: r.try_get::<Option<i64>, _>("read_at").ok().flatten(),
                created_at: r.try_get("created_at")?,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(ApiError::Internal)?;
    Ok(Json(NotificationsListResponse {
        notifications,
        unread,
        total,
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

#[derive(Debug, Serialize)]
pub struct ClearResponse {
    pub cleared: u64,
}

/// Delete all of the caller's notifications (the bell's "Clear all").
pub async fn clear_all(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ClearResponse>, ApiError> {
    let cleared = queries::clear_notifications(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(ClearResponse { cleared }))
}
