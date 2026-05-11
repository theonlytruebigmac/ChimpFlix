//! `GET /api/v1/seasons/{id}` — season detail with episodes.

use axum::Json;
use axum::extract::{Path, State};
use chimpflix_library::SeasonDetail;
use chimpflix_library::queries;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<SeasonDetail>, ApiError> {
    let detail = queries::get_season_detail(&state.pool, id, user.id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}
