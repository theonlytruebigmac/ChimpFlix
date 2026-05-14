//! /api/v1/collections handlers — movie franchises (TMDB collections).

use axum::Json;
use axum::extract::{Path, State};
use chimpflix_library::queries::{self, CollectionRow};
use chimpflix_library::ListedItem;
use serde::Serialize;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

async fn access(state: &AppState, user: &AuthUser) -> Result<Option<Vec<i64>>, ApiError> {
    queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)
}

#[derive(Debug, Serialize)]
pub struct CollectionsResponse {
    pub collections: Vec<CollectionRow>,
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<CollectionsResponse>, ApiError> {
    let acc = access(&state, &user).await?;
    let collections = queries::list_collections(&state.pool, acc.as_deref()).await?;
    Ok(Json(CollectionsResponse { collections }))
}

#[derive(Debug, Serialize)]
pub struct CollectionDetail {
    #[serde(flatten)]
    pub collection: CollectionRow,
    pub items: Vec<ListedItem>,
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<CollectionDetail>, ApiError> {
    let acc = access(&state, &user).await?;
    let collection = queries::get_collection(&state.pool, id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    let items =
        queries::list_items_in_collection(&state.pool, id, user.id, acc.as_deref()).await?;
    Ok(Json(CollectionDetail { collection, items }))
}
