//! /api/v1/people handlers — person detail + filmography for the
//! cast click-through route.

use axum::Json;
use axum::extract::{Path, State};
use chimpflix_library::ListedItem;
use chimpflix_library::models::Person;
use chimpflix_library::queries;
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
pub struct PersonDetail {
    #[serde(flatten)]
    pub person: Person,
    pub items: Vec<ListedItem>,
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<PersonDetail>, ApiError> {
    let person = queries::get_person(&state.pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let acc = access(&state, &user).await?;
    let items =
        queries::list_items_for_person(&state.pool, id, user.id, acc.as_deref()).await?;
    Ok(Json(PersonDetail { person, items }))
}
