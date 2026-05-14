//! /api/v1/play-state handlers.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use chimpflix_library::queries;
use chimpflix_library::{ListedItem, OnDeckResponse, PlayStateBatch, ScrobbleRequest};
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Deserialize)]
pub struct WatchedInput {
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub watched: bool,
}

/// Explicit toggle for the Plex-style "Mark as watched / unwatched"
/// modal action. Distinct from scrobble (which is the implicit threshold
/// crossing) and from update (which writes a specific position).
pub async fn set_watched(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<WatchedInput>,
) -> Result<StatusCode, ApiError> {
    match (req.item_id, req.episode_id) {
        (Some(_), Some(_)) => {
            return Err(ApiError::validation(
                "only one of item_id or episode_id may be set",
            ));
        }
        (None, None) => {
            return Err(ApiError::validation(
                "one of item_id or episode_id is required",
            ));
        }
        _ => {}
    }
    queries::set_watched(&state.pool, user.id, req.item_id, req.episode_id, req.watched)
        .await?;
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
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let resp = queries::on_deck(&state.pool, user.id, acc.as_deref()).await?;
    Ok(Json(resp))
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub items: Vec<ListedItem>,
}

pub async fn history(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, ApiError> {
    let limit = q.limit.unwrap_or(60).clamp(1, 200);
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let items = queries::list_watch_history(&state.pool, user.id, limit, acc.as_deref())
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(HistoryResponse { items }))
}
