//! `/media-files/{id}/preview/{manifest|sprite}` — scrub-preview API.
//!
//! Two endpoints per media file:
//!   - `manifest` returns the JSON the player needs to compute tile
//!     offsets (interval, dimensions, column count, total tile count).
//!   - `sprite` streams the actual JPEG.
//!
//! Both return 404 until the `generate_previews` scheduled task runs for
//! the file. The player is expected to feature-detect (treat a 404 as
//! "no scrub preview available" and skip the hover overlay).

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use chimpflix_library::queries;
use serde::Serialize;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct PreviewManifest {
    pub sprite_url: String,
    pub interval_ms: i64,
    pub tile_width: i64,
    pub tile_height: i64,
    pub tile_cols: i64,
    pub tile_count: i64,
}

pub async fn manifest(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<PreviewManifest>, ApiError> {
    let record = queries::get_preview_sprite(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(PreviewManifest {
        sprite_url: format!("/api/v1/media-files/{id}/preview/sprite"),
        interval_ms: record.interval_ms,
        tile_width: record.tile_width,
        tile_height: record.tile_height,
        tile_cols: record.tile_cols,
        tile_count: record.tile_count,
    }))
}

pub async fn sprite(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let record = queries::get_preview_sprite(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    let bytes = tokio::fs::read(&record.path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/jpeg"),
            (header::CACHE_CONTROL, "public, max-age=31536000"),
        ],
        Body::from(bytes),
    )
        .into_response())
}
