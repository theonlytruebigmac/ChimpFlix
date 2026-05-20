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
use std::path::Path as StdPath;
use tracing::{info, warn};

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
// `detect_markers_file` queue handler so crash recovery and
// per-file retry semantics are uniform across manual, scheduled,
// and file-watcher pathways. `maybe_auto_capture_fingerprint` and
// `override_intro_via_fingerprint` (below) are still used directly
// by the per-file handler.

/// Auto-capture: when a Chapter-source intro is detected and the
/// show has no existing fingerprint, extract the signature range's
/// audio and store it with `captured_by='auto'`. Lets the
/// fingerprinting feature work end-to-end without any operator
/// input on shows whose containers ship with chapter metadata
/// (Bluray rips, well-mastered MKVs).
///
/// Best-effort and idempotent — if a fingerprint already exists for
/// the show (manual or auto), we skip silently. The upsert query
/// also refuses to overwrite a manual row with an auto write, so
/// even if this races with an operator save, manual wins.
// `seen_with_fp` is a caller-owned cache of `show_id`s already known
// to have a fingerprint. Without it, every file in a large library
// triggers its own DB SELECT against `show_intro_fingerprints` —
// 10k episodes = 10k indexed queries per scheduled detection pass,
// all returning "yes, still there". The cache cuts it to one query
// per show. Callers should reuse the same set across the whole
// detect-markers loop.
pub(crate) async fn maybe_auto_capture_fingerprint(
    pool: &sqlx::SqlitePool,
    ffmpeg: &chimpflix_transcoder::FfmpegConfig,
    media_file_id: i64,
    path: &StdPath,
    detected: &[chimpflix_transcoder::DetectedMarker],
    seen_with_fp: &mut std::collections::HashSet<i64>,
) {
    // Find a chapter-derived intro with a signature range. Blackdetect
    // intros are deliberately skipped — their range is the fade
    // envelope, not the theme audio.
    let Some(intro) = detected.iter().find(|m| {
        m.kind == chimpflix_transcoder::MarkerKind::Intro
            && m.source == chimpflix_transcoder::MarkerSource::Chapter
            && m.signature_range.is_some()
    }) else {
        return;
    };
    let Some((sig_start, sig_end)) = intro.signature_range else {
        return;
    };

    let show_id = match queries::show_id_for_media_file(pool, media_file_id).await {
        Ok(Some(id)) => id,
        _ => return, // movie / lookup failure
    };

    if seen_with_fp.contains(&show_id) {
        return;
    }

    // Skip if any fingerprint already exists for the show. The
    // upsert below would refuse to overwrite a manual row, but
    // checking first also stops us from re-capturing on every
    // detect_markers pass — without this every scheduled run
    // would extract + overwrite the existing auto fingerprint.
    match queries::get_show_intro_fingerprint(pool, show_id, None).await {
        Ok(Some(_)) => {
            // Record so subsequent episodes don't repeat the query.
            seen_with_fp.insert(show_id);
            return;
        }
        Ok(None) => {} // fall through to capture
        Err(e) => {
            warn!(
                media_file_id,
                show_id,
                error = %format!("{e:#}"),
                "auto-capture: existing-fingerprint check failed",
            );
            return;
        }
    }

    let duration_ms = sig_end - sig_start;
    if duration_ms <= 0 {
        return;
    }
    let fp = match chimpflix_transcoder::fingerprint::extract_fingerprint(
        ffmpeg,
        path,
        sig_start,
        duration_ms,
    )
    .await
    {
        Ok(fp) => fp,
        Err(e) => {
            warn!(
                media_file_id,
                show_id,
                error = %format!("{e:#}"),
                "auto-capture: extract failed",
            );
            return;
        }
    };
    if fp.is_empty() {
        return;
    }
    let blob = chimpflix_transcoder::fingerprint::encode_blob(&fp);
    if let Err(e) = queries::upsert_show_intro_fingerprint(
        pool,
        show_id,
        None,
        &blob,
        duration_ms,
        Some(media_file_id),
        "auto",
    )
    .await
    {
        warn!(
            media_file_id,
            show_id,
            error = %format!("{e:#}"),
            "auto-capture: persist failed",
        );
        return;
    }
    seen_with_fp.insert(show_id);
    info!(
        media_file_id,
        show_id,
        frames = fp.len(),
        duration_ms,
        "show intro fingerprint auto-captured from chapter metadata",
    );
}

/// Replace any blackdetect-derived `intro` marker with a fingerprint
/// match against the show's canonical intro signature, when one
/// exists. Best-effort — any failure (lookup miss, extract error, no
/// match above threshold) leaves the existing intro row alone so
/// the fallback path still works. Exposed at `pub(crate)` so the
/// scheduled `detect_markers` task can call the same helper —
/// otherwise scheduled runs would silently skip the fingerprint
/// override and use blackdetect's guess on every episode.
pub(crate) async fn override_intro_via_fingerprint(
    pool: &sqlx::SqlitePool,
    ffmpeg: &chimpflix_transcoder::FfmpegConfig,
    file_id: i64,
    path: &StdPath,
    rows: &mut Vec<(String, i64, i64)>,
) {
    let show_id = match queries::show_id_for_media_file(pool, file_id).await {
        Ok(Some(id)) => id,
        _ => return, // movie / not a show file / lookup error
    };
    let stored = match queries::get_show_intro_fingerprint(pool, show_id, None).await {
        Ok(Some(fp)) => fp,
        _ => return, // no canonical fingerprint yet — operator hasn't marked one
    };
    let reference = match chimpflix_transcoder::fingerprint::decode_blob(&stored.fingerprint) {
        Ok(fp) => fp,
        Err(e) => {
            warn!(
                file_id,
                error = %format!("{e:#}"),
                "fingerprint decode failed; skipping match",
            );
            return;
        }
    };
    // Extract the first DEFAULT_MATCH_WINDOW_MS of the target file.
    // 10 minutes covers cold-opens + intro reliably; longer than
    // that and we're scanning the body of an episode for an intro
    // signature, which doesn't make sense.
    let target = match chimpflix_transcoder::fingerprint::extract_fingerprint(
        ffmpeg,
        path,
        0,
        chimpflix_transcoder::fingerprint::DEFAULT_MATCH_WINDOW_MS,
    )
    .await
    {
        Ok(fp) => fp,
        Err(e) => {
            warn!(
                file_id,
                error = %format!("{e:#}"),
                "fingerprint extract on target failed; falling back to blackdetect",
            );
            return;
        }
    };
    let m = match chimpflix_transcoder::fingerprint::match_fingerprint(
        &reference,
        &target,
        chimpflix_transcoder::fingerprint::DEFAULT_MATCH_THRESHOLD,
    ) {
        Some(m) => m,
        None => {
            // No confident match — most likely the canonical intro
            // wasn't actually present in this file's first 10 min
            // (cold open longer than expected, or a clip episode).
            // Leave the existing blackdetect intro in place.
            return;
        }
    };
    // Replace the existing intro row (if any) with the fingerprint
    // match. We anchor start_ms to 0 like the rest of the detector
    // does — Skip Intro jumping the user past the cold open is the
    // affordance, not jumping past just the theme song.
    let new_end = m.start_ms + stored.duration_ms;
    rows.retain(|(k, _, _)| k != "intro");
    rows.push(("intro".to_string(), 0, new_end));
    info!(
        file_id,
        match_start_ms = m.start_ms,
        match_score = m.score,
        intro_end_ms = new_end,
        "intro anchored via fingerprint",
    );
}

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

    // Capture the show's canonical intro fingerprint from the first
    // intro marker in the new set. Spawned async so the PUT returns
    // immediately — the fingerprint extraction takes 1-3s for a 60s
    // intro, and we don't want the editor save to block on it.
    // No-op for movies (show_id_for_media_file returns None).
    if let Some(intro) = rows.iter().find(|(k, _, _, _)| k == "intro") {
        let pool = state.pool.clone();
        let ffmpeg = state.ffmpeg.clone();
        let start_ms = intro.1;
        let end_ms = intro.2;
        tokio::spawn(async move {
            capture_show_intro_fingerprint(&pool, &ffmpeg, media_file_id, start_ms, end_ms).await;
        });
    }

    let markers = queries::list_markers_full(&state.pool, media_file_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(MarkerListResponse {
        media_file_id,
        duration_ms,
        markers,
    }))
}

/// Best-effort: extract a chromaprint fingerprint from the manual
/// intro range and persist it as the show's canonical intro
/// signature. Failures (no audio, ffmpeg errors, show-id lookup
/// miss) are logged at `warn` and swallowed — the operator's manual
/// marker save isn't worth a 500 just because fingerprinting hit a
/// snag. Future calls retry on the next save.
async fn capture_show_intro_fingerprint(
    pool: &sqlx::SqlitePool,
    ffmpeg: &chimpflix_transcoder::FfmpegConfig,
    media_file_id: i64,
    start_ms: i64,
    end_ms: i64,
) {
    // We resolve season_id alongside show_id so per-season storage
    // is available when we toggle that on. For v1 we write to the
    // show-wide row (season_id = None) but keep the lookup so a
    // future code path that captures per-season has the data it
    // needs without another query.
    let (show_id, _season_id) = match queries::show_and_season_for_media_file(
        pool,
        media_file_id,
    )
    .await
    {
        Ok(Some(t)) => t,
        // Movies (no parent show) and lookup misses both skip — only
        // shows have a meaningful "canonical intro" to capture.
        Ok(None) => return,
        Err(e) => {
            warn!(
                media_file_id,
                error = %format!("{e:#}"),
                "fingerprint capture: show_id lookup failed",
            );
            return;
        }
    };
    let path: String = match sqlx::query_scalar("SELECT path FROM media_files WHERE id = ?")
        .bind(media_file_id)
        .fetch_optional(pool)
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => return,
        Err(e) => {
            warn!(
                media_file_id,
                error = %format!("{e:#}"),
                "fingerprint capture: path lookup failed",
            );
            return;
        }
    };
    let duration_ms = end_ms - start_ms;
    if duration_ms <= 0 {
        return;
    }
    let fp = match chimpflix_transcoder::fingerprint::extract_fingerprint(
        ffmpeg,
        StdPath::new(&path),
        start_ms,
        duration_ms,
    )
    .await
    {
        Ok(fp) => fp,
        Err(e) => {
            warn!(
                media_file_id,
                error = %format!("{e:#}"),
                "fingerprint capture: extract failed",
            );
            return;
        }
    };
    if fp.is_empty() {
        warn!(media_file_id, "fingerprint capture: empty result");
        return;
    }
    let blob = chimpflix_transcoder::fingerprint::encode_blob(&fp);
    if let Err(e) = queries::upsert_show_intro_fingerprint(
        pool,
        show_id,
        None, // show-wide for v1 — per-season is a future refinement
        &blob,
        duration_ms,
        Some(media_file_id),
        "manual",
    )
    .await
    {
        warn!(
            media_file_id,
            show_id,
            error = %format!("{e:#}"),
            "fingerprint capture: persist failed",
        );
        return;
    }
    info!(
        media_file_id,
        show_id,
        frames = fp.len(),
        duration_ms,
        "show intro fingerprint captured",
    );
}

// ─── Per-show intro fingerprint admin endpoints ───────────────────────

#[derive(Debug, Serialize)]
pub struct ShowFingerprintStatus {
    /// Resolved parent show id, or `None` for movie files.
    pub show_id: Option<i64>,
    /// True when a fingerprint exists for the show. The fingerprint
    /// blob itself is intentionally not returned — it's a packed u32
    /// array with no useful UI surface beyond "we have one".
    pub captured: bool,
    pub duration_ms: Option<i64>,
    pub captured_at: Option<i64>,
    pub captured_by: Option<String>,
}

/// Status endpoint backing the "Fingerprint captured" badge in the
/// marker editor. Keyed on the media file id (same as the rest of
/// the marker editor surface) and resolves the parent show id
/// server-side so the UI doesn't need to do that lookup itself.
pub async fn fingerprint_status(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(media_file_id): Path<i64>,
) -> Result<Json<ShowFingerprintStatus>, ApiError> {
    let show_id = queries::show_id_for_media_file(&state.pool, media_file_id)
        .await
        .map_err(ApiError::Internal)?;
    let Some(show_id) = show_id else {
        return Ok(Json(ShowFingerprintStatus {
            show_id: None,
            captured: false,
            duration_ms: None,
            captured_at: None,
            captured_by: None,
        }));
    };
    let fp = queries::get_show_intro_fingerprint(&state.pool, show_id, None)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(match fp {
        Some(fp) => ShowFingerprintStatus {
            show_id: Some(show_id),
            captured: true,
            duration_ms: Some(fp.duration_ms),
            captured_at: Some(fp.captured_at),
            captured_by: Some(fp.captured_by),
        },
        None => ShowFingerprintStatus {
            show_id: Some(show_id),
            captured: false,
            duration_ms: None,
            captured_at: None,
            captured_by: None,
        },
    }))
}

/// Delete every fingerprint row attached to the parent show of
/// `media_file_id`. Used by the operator's "Clear fingerprint"
/// affordance in the marker editor when they want to start over
/// (the previous capture was from a bad intro range, etc.).
pub async fn clear_fingerprint(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(media_file_id): Path<i64>,
) -> Result<Json<ShowFingerprintStatus>, ApiError> {
    let show_id = queries::show_id_for_media_file(&state.pool, media_file_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    let removed = queries::delete_all_show_intro_fingerprints(&state.pool, show_id)
        .await
        .map_err(ApiError::Internal)?;
    info!(media_file_id, show_id, removed, "show intro fingerprint cleared");
    Ok(Json(ShowFingerprintStatus {
        show_id: Some(show_id),
        captured: false,
        duration_ms: None,
        captured_at: None,
        captured_by: None,
    }))
}
