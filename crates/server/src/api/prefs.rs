//! /api/v1/prefs handlers. Per-user preferences (per-device prefs like
//! trailerMuted stay in localStorage on the client).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct HiddenLibrariesResponse {
    pub library_ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
pub struct HiddenLibrariesInput {
    pub library_ids: Vec<i64>,
}

pub async fn get_hidden_libraries(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<HiddenLibrariesResponse>, ApiError> {
    let library_ids = queries::list_hidden_libraries(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(HiddenLibrariesResponse { library_ids }))
}

pub async fn put_hidden_libraries(
    State(state): State<AppState>,
    user: AuthUser,
    Json(input): Json<HiddenLibrariesInput>,
) -> Result<StatusCode, ApiError> {
    // Validate that every supplied ID refers to a real library; a FK
    // violation would otherwise surface as an opaque 500.
    if !input.library_ids.is_empty() {
        let unique: std::collections::HashSet<i64> =
            input.library_ids.iter().copied().collect();
        let found = queries::list_libraries(&state.pool, Some(&input.library_ids))
            .await
            .map_err(ApiError::Internal)?;
        if found.len() != unique.len() {
            return Err(ApiError::validation(
                "one or more library_ids do not exist",
            ));
        }
    }
    queries::set_hidden_libraries(&state.pool, user.id, &input.library_ids)
        .await
        .map_err(ApiError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}
