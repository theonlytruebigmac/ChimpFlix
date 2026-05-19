//! /api/v1/libraries handlers.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chimpflix_library::queries;
use chimpflix_library::{Library, LibraryUpdate, NewLibrary, ScanEmitter, ScanEvent, ScanJob};
use tracing::{info, warn};

use crate::api::error::ApiError;
use crate::auth::{AuthUser, OwnerAuth};
use crate::events::Event;
use crate::state::AppState;

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<ListResponse>, ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let libraries = queries::list_libraries(&state.pool, acc.as_deref()).await?;
    Ok(Json(ListResponse { libraries }))
}

pub async fn create(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Json(input): Json<NewLibrary>,
) -> Result<(StatusCode, Json<Library>), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::validation("name is required"));
    }
    if input.paths.is_empty() {
        return Err(ApiError::validation(
            "paths must contain at least one entry",
        ));
    }
    let lib = queries::create_library(&state.pool, input).await?;
    Ok((StatusCode::CREATED, Json(lib)))
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Library>, ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    if let Some(ref allowed) = acc {
        if !allowed.contains(&id) {
            return Err(ApiError::NotFound);
        }
    }
    let lib = queries::get_library(&state.pool, id)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(lib))
}

pub async fn update(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(id): Path<i64>,
    Json(update): Json<LibraryUpdate>,
) -> Result<Json<Library>, ApiError> {
    let lib = queries::update_library(&state.pool, id, update)
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(lib))
}

pub async fn delete_one(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = queries::delete_library(&state.pool, id).await?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn trigger_scan(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<(StatusCode, Json<ScanJob>), ApiError> {
    let _library = queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Coordinate with the scheduled scan and file watcher via the
    // shared per-library lock. Operator hitting the scan button twice
    // (or hitting it while a scheduled scan is running) used to
    // spawn parallel scanner processes — same library, same disk,
    // same DB rows — which not only wasted IO but stalled live
    // transcodes sharing the cache disk. Reject the second trigger
    // with a 409 so the UI can show "already scanning" rather than
    // silently piling on.
    if !state.try_acquire_library_scan(library_id).await {
        return Err(ApiError::Conflict(
            "a scan is already in progress for this library".to_string(),
        ));
    }

    let job = queries::create_scan_job(&state.pool, library_id).await?;
    let job_id = job.id;

    let pool = state.pool.clone();
    let ffmpeg = state.ffmpeg.clone();
    let tmdb = state.tmdb_snapshot().await;
    let tvdb = state.tvdb_snapshot().await;
    let anilist = state.anilist_snapshot().await;
    let tvmaze = state.tvmaze.clone();
    let hub = state.hub.clone();

    let emitter: ScanEmitter = Arc::new(move |event: ScanEvent| {
        // Translate the scan completion / failure into a webhook event so
        // owners can subscribe to it. WebSocket clients still receive the
        // raw Event::Scan via the separate publish.
        if let ScanEvent::Completed {
            job_id,
            library_id,
            files_seen,
            files_added,
            files_updated,
            files_removed,
        } = &event
        {
            hub.publish(Event::Webhook(crate::events::WebhookEvent::new(
                "scan.completed",
                serde_json::json!({
                    "job_id": job_id,
                    "library_id": library_id,
                    "files_seen": files_seen,
                    "files_added": files_added,
                    "files_updated": files_updated,
                    "files_removed": files_removed,
                }),
            )));
        }
        hub.publish(Event::Scan(event));
    });

    let cache_root = state.transcoder.cache_root().to_path_buf();
    let state_for_release = state.clone();
    tokio::spawn(async move {
        if let Err(e) = chimpflix_library::run_scan(
            pool, ffmpeg, tmdb, tvdb, anilist, tvmaze, library_id, job_id,
            Some(cache_root),
            emitter,
        )
        .await
        {
            warn!(error = %format!("{e:#}"), library_id, job_id, "scan task ended with error");
        }
        state_for_release.release_library_scan(library_id).await;
    });

    Ok((StatusCode::ACCEPTED, Json(job)))
}

// ---------------------------------------------------------------------------
// Per-library stats (owner-only)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct LibraryStatsResponse {
    pub library_id: i64,
    pub items: i64,
    pub episodes: i64,
    pub files: i64,
    pub total_bytes: i64,
    pub orphan_files: i64,
    pub last_scanned_at: Option<i64>,
}

/// At-a-glance numbers for one library — items, episodes, files,
/// total size on disk, orphan-pending count, and last successful
/// scan timestamp. Used by the admin library card so the operator
/// can see the library's shape without drilling into individual
/// items.
pub async fn library_stats(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<Json<LibraryStatsResponse>, ApiError> {
    let _library = queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let s = queries::single_library_stats(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(LibraryStatsResponse {
        library_id: s.library_id,
        items: s.items,
        episodes: s.episodes,
        files: s.files,
        total_bytes: s.total_bytes,
        orphan_files: s.orphan_files,
        last_scanned_at: s.last_scanned_at,
    }))
}

// ---------------------------------------------------------------------------
// Per-library verify / purge (owner-only)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct VerifyResponse {
    pub library_id: i64,
    pub files_checked: usize,
    pub files_missing: usize,
    pub newly_marked_removed: u64,
    pub still_missing: usize,
    pub returned_files: usize,
    /// Total soft-deleted rows for this library after the verify run.
    /// Used by the admin UI to surface "N orphan(s) pending purge"
    /// even if this particular verify didn't change anything.
    pub orphan_count: i64,
}

/// Run the verify pass for one library synchronously and return the
/// report. Pairs with the scheduled `verify_libraries` task — that
/// one runs across the whole instance on a weekly cadence; this
/// one is the operator's "I want to check this library now" path.
pub async fn verify_library(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<Json<VerifyResponse>, ApiError> {
    let _library = queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let report = queries::verify_library(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?;
    let orphan_count = queries::count_removed_media_files(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(VerifyResponse {
        library_id: report.library_id,
        files_checked: report.files_checked,
        files_missing: report.files_missing,
        newly_marked_removed: report.newly_marked_removed,
        still_missing: report.still_missing,
        returned_files: report.returned_files,
        orphan_count,
    }))
}

#[derive(Debug, serde::Deserialize, Default)]
pub struct PurgeQuery {
    /// Override the default 7-day grace window for this run. Useful
    /// when the operator wants to immediately reap a library after
    /// confirming the files are genuinely gone (e.g., decommissioned
    /// drive). `Some(0)` purges everything currently marked removed.
    #[serde(default)]
    pub grace_days: Option<i64>,
}

#[derive(serde::Serialize)]
pub struct PurgeResponse {
    pub files_purged: u64,
    pub episodes_purged: u64,
    pub seasons_purged: u64,
    pub items_purged: u64,
}

/// Immediately purge soft-deleted files for this library. The
/// scheduled `purge_removed_files` task runs daily across the whole
/// instance; this endpoint lets the operator trigger it on demand
/// for a single library — e.g., after manually verifying their
/// removed files are gone for good.
///
/// NOTE: the underlying query is currently instance-wide, not per-
/// library — purge sweeps every expired row regardless of library.
/// Returning per-library counts here would require a more targeted
/// purge; for now the response is the global count, scoped only by
/// the cutoff the caller asked for.
pub async fn purge_library(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
    axum::extract::Query(q): axum::extract::Query<PurgeQuery>,
) -> Result<Json<PurgeResponse>, ApiError> {
    let _library = queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let grace_days = q.grace_days.unwrap_or(7).max(0);
    let cutoff_ms = chimpflix_common::now_ms() - grace_days * 86_400_000;
    let report = queries::purge_removed_media_files(&state.pool, cutoff_ms)
        .await
        .map_err(ApiError::Internal)?;
    evict_subtitle_caches(state.transcoder.cache_root(), &report.purged_paths).await;
    Ok(Json(PurgeResponse {
        files_purged: report.files_purged,
        episodes_purged: report.episodes_purged,
        seasons_purged: report.seasons_purged,
        items_purged: report.items_purged,
    }))
}

/// Best-effort cleanup of per-file WebVTT subtitle caches for files
/// that were just hard-deleted. Fires off the eviction asynchronously
/// since the cache lives on disk and could be 100s of entries for a
/// bulk-purge; we don't block the HTTP response on it.
async fn evict_subtitle_caches(cache_root: &std::path::Path, paths: &[String]) {
    if paths.is_empty() {
        return;
    }
    let cache_root = cache_root.to_path_buf();
    let paths: Vec<String> = paths.to_vec();
    tokio::spawn(async move {
        for p in paths {
            let _ = chimpflix_transcoder::evict_text_subs_cache(
                &cache_root,
                std::path::Path::new(&p),
            )
            .await;
        }
    });
}

// ---------------------------------------------------------------------------
// Per-library on-demand maintenance (owner-only)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct LibraryJobQueued {
    /// Number of items / files that will be processed in the
    /// background. Lets the UI show "Queued: 47 items" instead of
    /// a vague "queued" toast.
    pub queued: usize,
}

/// Re-run TMDB / TVDB metadata refresh for every item in this
/// library. Equivalent to the scheduled `refresh_metadata` task with
/// `library_id` set, but runs immediately and doesn't write a task
/// row. Spawns a background task; the response returns the queued
/// count before the work starts.
pub async fn refresh_metadata(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<(StatusCode, Json<LibraryJobQueued>), ApiError> {
    let _library = queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let Some(tmdb) = state.tmdb_snapshot().await else {
        // No TMDB credential — nothing to refresh.
        return Ok((StatusCode::ACCEPTED, Json(LibraryJobQueued { queued: 0 })));
    };
    let tvdb = state.tvdb_snapshot().await;
    let tvmaze = state.tvmaze.clone();
    let item_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM items WHERE library_id = ?")
            .bind(library_id)
            .fetch_all(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
    let queued = item_ids.len();
    let pool = state.pool.clone();
    tokio::spawn(async move {
        let mut ok = 0usize;
        let mut err = 0usize;
        for item_id in item_ids {
            match chimpflix_library::scanner::refresh_item_metadata(
                &pool,
                &tmdb,
                tvdb.as_ref(),
                tvmaze.as_ref(),
                item_id,
                None,
            )
            .await
            {
                Ok(()) => ok += 1,
                Err(_) => err += 1,
            }
        }
        info!(library_id, ok, err, "library refresh_metadata completed");
    });
    Ok((StatusCode::ACCEPTED, Json(LibraryJobQueued { queued })))
}

/// Generate scrub-preview sprite tiles for every media file in this
/// library that doesn't already have one. Sequential because each
/// sprite-gen pegs a CPU core. The standard `generate_previews`
/// scheduled task does the same thing on a batch-of-N cadence;
/// this endpoint just kicks the whole library at once for the
/// "I want previews on everything NOW" path.
pub async fn generate_previews(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<(StatusCode, Json<LibraryJobQueued>), ApiError> {
    let _library = queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    // Batch = 0 means "no per-call cap" inside the query.
    let candidates =
        queries::list_media_files_needing_previews(&state.pool, Some(library_id), 0)
            .await
            .map_err(ApiError::Internal)?;
    let queued = candidates.len();
    if queued == 0 {
        return Ok((StatusCode::ACCEPTED, Json(LibraryJobQueued { queued: 0 })));
    }

    let pool = state.pool.clone();
    let ffmpeg = state.ffmpeg.clone();
    let dir = state.data_dir.join("previews");
    tokio::spawn(async move {
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            warn!(error = %e, "could not create previews dir");
            return;
        }
        let mut ok = 0usize;
        let mut err = 0usize;
        for cand in &candidates {
            let duration = cand.duration_ms.unwrap_or(0);
            let output = dir.join(format!("{}.jpg", cand.id));
            match chimpflix_transcoder::generate_sprite(
                &ffmpeg,
                std::path::Path::new(&cand.path),
                &output,
                duration,
                chimpflix_transcoder::DEFAULT_INTERVAL_S,
                chimpflix_transcoder::DEFAULT_TILE_WIDTH,
            )
            .await
            {
                Ok(info) => {
                    if queries::record_preview_sprite(
                        &pool,
                        queries::PreviewSpriteRecord {
                            media_file_id: cand.id,
                            path: info.path.to_string_lossy().into_owned(),
                            interval_ms: info.interval_ms,
                            tile_width: i64::from(info.tile_width),
                            tile_height: i64::from(info.tile_height),
                            tile_cols: i64::from(info.tile_cols),
                            tile_count: i64::from(info.tile_count),
                        },
                    )
                    .await
                    .is_ok()
                    {
                        ok += 1;
                    } else {
                        err += 1;
                    }
                }
                Err(_) => err += 1,
            }
        }
        info!(library_id, ok, err, "library generate_previews completed");
    });

    Ok((StatusCode::ACCEPTED, Json(LibraryJobQueued { queued })))
}

// ---------------------------------------------------------------------------
// Per-library access (owner-only)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct AccessResponse {
    pub user_ids: Vec<i64>,
}

#[derive(serde::Deserialize)]
pub struct AccessInput {
    pub user_ids: Vec<i64>,
}

pub async fn get_access(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
) -> Result<Json<AccessResponse>, ApiError> {
    // Verify the library exists so we 404 cleanly instead of returning an
    // empty access list for a typo'd id.
    queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let user_ids = queries::list_library_user_ids(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AccessResponse { user_ids }))
}

pub async fn put_access(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Path(library_id): Path<i64>,
    Json(input): Json<AccessInput>,
) -> Result<StatusCode, ApiError> {
    queries::get_library(&state.pool, library_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    queries::set_library_user_ids(&state.pool, library_id, &input.user_ids)
        .await
        .map_err(ApiError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_scans(
    State(state): State<AppState>,
    user: AuthUser,
    Path(library_id): Path<i64>,
) -> Result<Json<ScanListResponse>, ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    if let Some(ref allowed) = acc {
        if !allowed.contains(&library_id) {
            return Err(ApiError::NotFound);
        }
    }
    let jobs = queries::list_scan_jobs(&state.pool, library_id, 50).await?;
    Ok(Json(ScanListResponse { scans: jobs }))
}

#[derive(serde::Serialize)]
pub struct ListResponse {
    libraries: Vec<Library>,
}

#[derive(serde::Serialize)]
pub struct ScanListResponse {
    scans: Vec<ScanJob>,
}
