//! /api/v1/my-list handlers. Single per-user list (no named lists yet).

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chimpflix_library::{ListedItem, queries};
use serde::Serialize;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct MyListResponse {
    pub items: Vec<ListedItem>,
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<MyListResponse>, ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let items = queries::list_my_list(&state.pool, user.id, acc.as_deref())
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(MyListResponse { items }))
}

pub async fn add(
    State(state): State<AppState>,
    user: AuthUser,
    Path(item_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    // 404 if the item id doesn't exist or the user can't access its library
    // — same response in either case so we don't leak existence.
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    if queries::get_item(&state.pool, item_id, user.id, acc.as_deref())
        .await
        .map_err(ApiError::Internal)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    queries::add_to_my_list(&state.pool, user.id, item_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    user: AuthUser,
    Path(item_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    queries::remove_from_my_list(&state.pool, user.id, item_id)
        .await
        .map_err(ApiError::Internal)?;
    // Idempotent: deleting an already-absent row is fine.
    Ok(StatusCode::NO_CONTENT)
}
