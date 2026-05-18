//! `/tags`, `/items/{id}/tags` — operator-managed tag CRUD.
//!
//! Read is open to any signed-in user. Write (add/remove on an item)
//! requires the owner role for now — keeps the tag namespace clean
//! while we figure out whether per-user tags are worth the complexity.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chimpflix_library::{Tag, queries};
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct TagListResponse {
    pub tags: Vec<Tag>,
}

pub async fn list_all(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<TagListResponse>, ApiError> {
    let tags = queries::list_tags(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(TagListResponse { tags }))
}

pub async fn list_for_item(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<TagListResponse>, ApiError> {
    let tags = queries::list_tags_for_item(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(TagListResponse { tags }))
}

#[derive(Debug, Deserialize)]
pub struct AddTagInput {
    pub name: String,
}

pub async fn add_to_item(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(input): Json<AddTagInput>,
) -> Result<Json<Tag>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let tag = queries::add_tag_to_item(&state.pool, id, &input.name)
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    Ok(Json(tag))
}

pub async fn remove_from_item(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, tag_id)): Path<(i64, i64)>,
) -> Result<StatusCode, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let removed = queries::remove_tag_from_item(&state.pool, id, tag_id)
        .await
        .map_err(ApiError::Internal)?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}
