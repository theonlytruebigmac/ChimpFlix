//! /api/v1/play-state handlers.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chimpflix_library::queries;
use chimpflix_library::{OnDeckResponse, PlayStateBatch, ScrobbleRequest};

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Json(batch): Json<PlayStateBatch>,
) -> Result<StatusCode, ApiError> {
    if batch.updates.is_empty() {
        return Err(ApiError::validation("updates must not be empty"));
    }
    for (i, u) in batch.updates.iter().enumerate() {
        match (u.item_id, u.episode_id) {
            (Some(_), Some(_)) => {
                return Err(ApiError::validation(format!(
                    "update #{i}: only one of item_id or episode_id may be set",
                )));
            }
            (None, None) => {
                return Err(ApiError::validation(format!(
                    "update #{i}: one of item_id or episode_id is required",
                )));
            }
            _ => {}
        }
    }
    queries::apply_play_state_batch(&state.pool, user.id, batch).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn scrobble(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<ScrobbleRequest>,
) -> Result<StatusCode, ApiError> {
    if req.item_id.is_none() && req.episode_id.is_none() {
        return Err(ApiError::validation(
            "scrobble requires item_id or episode_id",
        ));
    }
    if req.item_id.is_some() && req.episode_id.is_some() {
        return Err(ApiError::validation(
            "scrobble must not have both item_id and episode_id",
        ));
    }
    queries::scrobble(&state.pool, user.id, req.item_id, req.episode_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn on_deck(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<OnDeckResponse>, ApiError> {
    let resp = queries::on_deck(&state.pool, user.id).await?;
    Ok(Json(resp))
}
