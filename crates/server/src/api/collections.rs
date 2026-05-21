//! /api/v1/collections handlers — movie franchises (TMDB collections).

use axum::Json;
use axum::extract::{Path, Query, State};
use chimpflix_library::ListedItem;
use chimpflix_library::queries::{self, CollectionRow};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Default, Deserialize)]
pub struct ListQuery {
    /// Include auto (TMDB-discovered franchise) collections alongside
    /// the user-curated manual + smart ones. Defaults to false — the
    /// home rail caller wants a clean slate until the operator sets up
    /// collections of their own, while the admin panel passes
    /// `?include_auto=true` to manage everything.
    #[serde(default)]
    pub include_auto: bool,
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<CollectionsResponse>, ApiError> {
    let acc = access(&state, &user).await?;
    let collections =
        queries::list_collections(&state.pool, acc.as_deref(), q.include_auto).await?;
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
    let items = queries::list_items_in_collection(&state.pool, id, user.id, acc.as_deref()).await?;
    Ok(Json(CollectionDetail { collection, items }))
}
