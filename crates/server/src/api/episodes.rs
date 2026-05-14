//! `GET /api/v1/episodes/{id}` — episode detail with files + play state.

use axum::Json;
use axum::extract::{Path, State};
use chimpflix_library::EpisodeDetail;
use chimpflix_library::queries;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<EpisodeDetail>, ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let detail = queries::get_episode_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}
