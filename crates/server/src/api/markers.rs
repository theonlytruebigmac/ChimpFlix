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
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::jobs::handlers::detect_markers_file;
use crate::state::AppState;

/// Bounds the editor — markers wider than this are almost certainly a
/// mis-clicked drag and unhelpful as skip targets. Backend enforces
/// independently of the UI so a hand-rolled PUT can't sneak past.
const MAX_MARKER_WIDTH_MS: i64 = 30 * 60 * 1000; // 30 minutes

#[derive(Debug, Serialize)]
pub struct DetectResponse {
    /// Number of media files the queued job is expected to process.
    /// Computed at enqueue time for the user-facing toast; the actual
    /// detection runs inside the worker, so this is an estimate that
    /// can drift if files are added or removed before the worker
    /// picks the job up.
    pub queued: usize,
    /// Row id of the enqueued job, when this endpoint enqueues
    /// through the durable queue. None when the work was dispatched
    /// to the legacy in-memory spawn path (library-level detection,
    /// which is on the migration backlog).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<i64>,
}

/// Detect markers for every file under a single item. For movies this is
/// 1 file; for shows it walks every episode. Work runs through the
/// durable job queue (`jobs` table): one `detect_markers_file` job per
/// media_file_id, so a server restart mid-batch resumes any in-flight
/// files individually and crashes don't take down the whole sweep.
pub async fn detect_for_item(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(item_id): Path<i64>,
) -> Result<(StatusCode, Json<DetectResponse>), ApiError> {
    // Resolve the item up front so we 404 cleanly when it doesn't
    // exist (instead of silently enqueueing zero jobs).
    let detail = queries::get_item_detail(&state.pool, item_id, _owner.0.id, None)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    let file_ids: Vec<i64> = match detail.item.kind {
        ItemKind::Movie => detail.files.iter().map(|f| f.id).collect(),
        ItemKind::Show => sqlx::query_scalar::<_, i64>(
            "SELECT mf.id
             FROM media_files mf
             JOIN episodes e ON e.id = mf.episode_id
             JOIN seasons s ON s.id = e.season_id
             WHERE s.show_id = ? AND mf.removed_at IS NULL",
        )
        .bind(item_id)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?,
    };

    let queued = detect_markers_file::enqueue_for_files(&state.pool, &file_ids)
        .await
        .map_err(ApiError::Internal)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(DetectResponse {
            queued,
            // No single job_id when we fan out — admin queue page is
            // the source of truth for progress across multiple jobs.
            job_id: None,
        }),
    ))
}

/// Detect markers for every file in a library. Enqueues one
/// `detect_markers_file` job per media_file_id — crash-safe via the
/// queue, deduped on file_id so a re-trigger while previous jobs are
/// in flight doesn't double-process.
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
    let file_ids: Vec<i64> = files.iter().map(|(id, _, _)| *id).collect();
    let queued = detect_markers_file::enqueue_for_files(&state.pool, &file_ids)
        .await
        .map_err(ApiError::Internal)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(DetectResponse {
            queued,
            job_id: None,
        }),
    ))
}

// Legacy in-memory `spawn_detection` removed in the
// discovery-pipeline migration — all detection now runs through the
// `detect_markers_file` queue handler so crash recovery and per-file
// retry semantics are uniform across manual, scheduled, and
// file-watcher pathways.
//
// The chromaprint auto-capture + override helpers that used to live
// here were retired when tacet (audio-fingerprint matching against
// per-season references) replaced the chapter-metadata-derived
// fingerprinting path. The `show_intro_fingerprints` table, the
// admin "Intro fingerprints" page, the `chimpflix_transcoder::
// fingerprint` chromaprint module, and the per-media-file fingerprint
// endpoints (`GET/DELETE /media-files/{id}/intro-fingerprint`) all
// went with it — see phase-69 drop migration.

#[derive(Debug, Serialize)]
pub struct MarkerListResponse {
    pub media_file_id: i64,
    pub duration_ms: Option<i64>,
    pub markers: Vec<queries::MarkerRow>,
}

/// List every marker (auto + manual) attached to a single media file.
/// Owner-only — the operator-side editor consumes this, and we don't
/// surface the manual/auto distinction on player paths.
pub async fn list_for_media_file(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(media_file_id): Path<i64>,
) -> Result<Json<MarkerListResponse>, ApiError> {
    let duration_ms = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT duration_ms FROM media_files WHERE id = ?",
    )
    .bind(media_file_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .ok_or(ApiError::NotFound)?;
    let markers = queries::list_markers_full(&state.pool, media_file_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(MarkerListResponse {
        media_file_id,
        duration_ms,
        markers,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ManualMarkerInput {
    /// "intro" | "credits" | "commercial". The player special-cases the
    /// first two for its skip-button copy; anything else renders with a
    /// generic "Skip" label.
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// Optional human label shown on the editor row, e.g. "S01 OP
    /// (long version)". Not surfaced to the player today.
    pub label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReplaceManualMarkersInput {
    pub markers: Vec<ManualMarkerInput>,
}

/// Replace every manual marker on `media_file_id` with the supplied
/// set. Auto-detected rows are preserved — they're regenerated by the
/// detection task. Validates that each marker has a positive width
/// and a known kind; rejects the whole batch on any violation.
pub async fn replace_manual(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(media_file_id): Path<i64>,
    Json(input): Json<ReplaceManualMarkersInput>,
) -> Result<Json<MarkerListResponse>, ApiError> {
    // Confirm the file exists + grab duration_ms for the response and
    // for the validation clamp below.
    let duration_ms = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT duration_ms FROM media_files WHERE id = ?",
    )
    .bind(media_file_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .ok_or(ApiError::NotFound)?;

    // Server-side validation. The editor enforces these too but a
    // hand-rolled PUT can sneak past, and a busted marker (negative
    // width, kind="banana") makes the player look broken.
    let mut rows: Vec<(String, i64, i64, Option<String>)> =
        Vec::with_capacity(input.markers.len());
    for m in &input.markers {
        let kind = m.kind.trim().to_ascii_lowercase();
        if !matches!(kind.as_str(), "intro" | "credits" | "commercial") {
            return Err(ApiError::validation(format!(
                "unknown marker kind \"{}\" (expected intro / credits / commercial)",
                m.kind,
            )));
        }
        if m.start_ms < 0 || m.end_ms <= m.start_ms {
            return Err(ApiError::validation(
                "each marker must have start_ms >= 0 and end_ms > start_ms",
            ));
        }
        if let Some(dur) = duration_ms
            && m.end_ms > dur
        {
            return Err(ApiError::validation(
                "marker end_ms exceeds the file's duration",
            ));
        }
        if m.end_ms - m.start_ms > MAX_MARKER_WIDTH_MS {
            return Err(ApiError::validation(
                "marker is unreasonably wide (>30 min) — split into smaller markers",
            ));
        }
        rows.push((kind, m.start_ms, m.end_ms, m.label.clone()));
    }

    queries::replace_manual_markers(&state.pool, media_file_id, &rows)
        .await
        .map_err(ApiError::Internal)?;

    let markers = queries::list_markers_full(&state.pool, media_file_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(MarkerListResponse {
        media_file_id,
        duration_ms,
        markers,
    }))
}

