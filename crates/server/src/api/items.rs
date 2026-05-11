//! /api/v1/items handlers.

use axum::Json;
use axum::extract::{Path, Query, State};
use chimpflix_library::queries;
use chimpflix_library::{ItemDetail, ItemFilter, ItemPage};

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(filter): Query<ItemFilter>,
) -> Result<Json<ItemPage>, ApiError> {
    let page = queries::list_items(&state.pool, filter, user.id).await?;
    Ok(Json(page))
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ItemDetail>, ApiError> {
    let detail = queries::get_item_detail(&state.pool, id, user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}
