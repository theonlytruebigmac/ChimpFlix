//! Marker (intro / credits) detection endpoints.
//!
//! Detection runs ffmpeg's `blackdetect` filter, which is expensive — for
//! a 45-minute episode it scans the whole video pixel-by-pixel and can
//! take 30s+ on modest hardware. We always run it in a tokio task so the
//! HTTP response returns immediately and the caller can poll for results.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chimpflix_library::{ItemKind, queries};
use serde::Serialize;
use std::path::Path as StdPath;
use tracing::{info, warn};

use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct DetectResponse {
    /// Number of media files queued for analysis. The actual detection
    /// runs in the background; results are visible on the file's
    /// `markers` field once each task completes.
    pub queued: usize,
}

/// Detect markers for every file under a single item. For movies this is
/// 1 file; for shows it walks every episode.
pub async fn detect_for_item(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(item_id): Path<i64>,
) -> Result<(StatusCode, Json<DetectResponse>), ApiError> {
    // Skip the library access filter — owners bypass it. Item still has
    // to exist.
    let detail = queries::get_item_detail(&state.pool, item_id, _owner.0.id, None)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    let mut file_ids: Vec<(i64, String, Option<i64>)> = Vec::new();
    match detail.item.kind {
        ItemKind::Movie => {
            for f in &detail.files {
                let path = sqlx::query_scalar::<_, String>(
                    "SELECT path FROM media_files WHERE id = ?",
                )
                .bind(f.id)
                .fetch_one(&state.pool)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
                file_ids.push((f.id, path, f.duration_ms));
            }
        }
        ItemKind::Show => {
            // Every episode under every season.
            let rows = sqlx::query_as::<_, (i64, String, Option<i64>)>(
                "SELECT mf.id, mf.path, mf.duration_ms
                 FROM media_files mf
                 JOIN episodes e ON e.id = mf.episode_id
                 JOIN seasons s ON s.id = e.season_id
                 WHERE s.show_id = ?",
            )
            .bind(item_id)
            .fetch_all(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            file_ids.extend(rows);
        }
    }

    spawn_detection(&state, file_ids.clone());
    Ok((StatusCode::ACCEPTED, Json(DetectResponse { queued: file_ids.len() })))
}

/// Detect markers for every file in a library.
pub async fn detect_for_library(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<(StatusCode, Json<DetectResponse>), ApiError> {
    queries::get_library(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    let files = queries::list_media_files_in_library(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?;
    spawn_detection(&state, files.clone());
    Ok((StatusCode::ACCEPTED, Json(DetectResponse { queued: files.len() })))
}

/// Kicks off a sequential background task that runs detection on each file
/// in turn. Sequential rather than concurrent because each ffmpeg pass
/// pegs a CPU core; running them in parallel just thrashes.
fn spawn_detection(state: &AppState, files: Vec<(i64, String, Option<i64>)>) {
    let pool = state.pool.clone();
    let ffmpeg = state.ffmpeg.clone();
    tokio::spawn(async move {
        for (file_id, path, duration_ms) in files {
            let path_buf = StdPath::new(&path).to_path_buf();
            match chimpflix_transcoder::detect_markers(&ffmpeg, &path_buf, duration_ms)
                .await
            {
                Ok(detected) => {
                    let rows: Vec<(String, i64, i64)> = detected
                        .into_iter()
                        .map(|m| (m.kind.as_str().to_string(), m.start_ms, m.end_ms))
                        .collect();
                    if let Err(e) =
                        queries::replace_auto_markers(&pool, file_id, &rows).await
                    {
                        warn!(file_id, error = %format!("{e:#}"), "marker save failed");
                    } else {
                        info!(file_id, count = rows.len(), "markers detected");
                    }
                }
                Err(e) => {
                    warn!(file_id, error = %format!("{e:#}"), "marker detection failed");
                }
            }
        }
    });
}
