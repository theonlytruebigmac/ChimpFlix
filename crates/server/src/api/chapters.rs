//! `/media-files/{id}/chapters` — chapter list + thumbnail URLs.
//!
//! Chapters themselves are read on-demand via ffprobe rather than
//! persisted — they're baked into the source file and rarely change
//! without a rescan. The companion thumbnails are generated lazily
//! by the `generate_chapter_thumbs` scheduled task; their on-disk
//! presence is gated by `chapter_thumbs_generated_at` on `media_files`.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use chimpflix_library::queries;
use serde::Serialize;

use crate::api::access;
use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ChapterEntry {
    pub index: u32,
    pub start_ms: i64,
    pub end_ms: i64,
    pub title: Option<String>,
    /// Server-relative URL of the chapter thumbnail. `None` when the
    /// task hasn't run yet for this file, or when extraction failed.
    pub thumb_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChaptersResponse {
    pub chapters: Vec<ChapterEntry>,
    /// True when the `generate_chapter_thumbs` task has processed
    /// this file at least once. False means the chapter list may be
    /// populated (we probed inline) but thumbnails haven't been
    /// rendered yet.
    pub thumbs_ready: bool,
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ChaptersResponse>, ApiError> {
    access::ensure_file_accessible(&state, &user, id).await?;
    let path = queries::get_media_file_path(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    let chapters = chimpflix_transcoder::probe_chapters(&state.ffmpeg, std::path::Path::new(&path))
        .await
        .unwrap_or_default();
    let status = queries::get_chapter_thumbs_status(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    let thumbs_ready = matches!(status, Some((Some(_), _)));
    let entries = chapters
        .into_iter()
        .enumerate()
        .map(|(idx, ch)| {
            let thumb_url = if thumbs_ready {
                Some(format!("/api/v1/media-files/{id}/chapters/{idx}/thumb"))
            } else {
                None
            };
            ChapterEntry {
                index: idx as u32,
                start_ms: ch.start_ms,
                end_ms: ch.end_ms,
                title: ch.title,
                thumb_url,
            }
        })
        .collect();
    Ok(Json(ChaptersResponse {
        chapters: entries,
        thumbs_ready,
    }))
}

pub async fn thumb(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, index)): Path<(i64, u32)>,
) -> Result<Response, ApiError> {
    access::ensure_file_accessible(&state, &user, id).await?;
    let root = state.data_dir.join("chapter_thumbs");
    let path = chimpflix_transcoder::chapter_thumbs::thumb_path(&root, id, index);
    if !path.exists() {
        return Err(ApiError::NotFound);
    }
    let bytes = tokio::fs::read(&path)
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
