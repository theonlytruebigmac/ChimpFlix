//! /api/v1/people handlers — person detail + filmography for the
//! cast click-through route.

use axum::Json;
use axum::extract::{Path, Query, State};
use chimpflix_library::ListedItem;
use chimpflix_library::models::Person;
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

async fn access(state: &AppState, user: &AuthUser) -> Result<Option<Vec<i64>>, ApiError> {
    queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)
}

#[derive(Debug, Deserialize, Default)]
pub struct FilmographyQuery {
    /// Defaults to 50; clamped to 200 server-side.
    #[serde(default)]
    pub page_size: Option<i64>,
    #[serde(default)]
    pub page: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct PersonDetail {
    #[serde(flatten)]
    pub person: Person,
    pub items: Vec<ListedItem>,
    /// Total item count for this person visible to the requesting
    /// user. Paired with `page_size` to drive client-side
    /// pagination UI. See MONTH 1 in
    /// `docs/PUBLIC_RELEASE_HARDENING.md`.
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(q): Query<FilmographyQuery>,
) -> Result<Json<PersonDetail>, ApiError> {
    let person = queries::get_person(&state.pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let acc = access(&state, &user).await?;

    let page_size = q.page_size.unwrap_or(50).clamp(1, 200);
    let page = q.page.unwrap_or(1).max(1);
    let offset = (page - 1).saturating_mul(page_size);

    let total =
        queries::count_items_for_person(&state.pool, id, acc.as_deref()).await?;
    let items = queries::list_items_for_person(
        &state.pool,
        id,
        user.id,
        acc.as_deref(),
        page_size,
        offset,
    )
    .await?;
    Ok(Json(PersonDetail {
        person,
        items,
        total,
        page,
        page_size,
    }))
}
