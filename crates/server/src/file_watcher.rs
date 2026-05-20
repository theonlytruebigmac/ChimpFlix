//! Filesystem watcher that auto-triggers library scans when media files
//! appear, disappear, or change on disk.
//!
//! Replaces the cron-only model: instead of waiting for the scheduled
//! `scan_library` task, an mkv landing in a watched path queues a scan
//! within seconds. Debounced (5s of silence) so a `cp -r` doesn't fan
//! into dozens of scan jobs.
//!
//! Per-library reconfiguration is intentionally out of scope for v1 —
//! the watcher is built once at startup from current library paths.
//! Adding/removing a library requires a server restart to re-arm.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chimpflix_library::queries;
use notify::{
    EventKind, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RemoveKind},
};
use sqlx::Row;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::state::AppState;

/// How long we wait after the last filesystem event before firing a
/// scan. Long enough that a `cp` of a multi-file release coalesces;
/// short enough that the user sees the library refresh on the order of
/// seconds, not minutes.
const DEBOUNCE: Duration = Duration::from_secs(5);

pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        if let Err(e) = run(state).await {
            warn!(error = %format!("{e:#}"), "file watcher exited");
        }
    });
}

async fn run(state: AppState) -> Result<()> {
    // Bounded so a sudden burst of filesystem events (mass rename, full
    // library restore, rsync sweep) can't grow the channel without limit
    // and OOM the server. 16384 events covers a season pack rsync
    // (typical: 1k-5k events) plus headroom. When the channel IS full,
    // the notify callback flips `overflow_flag` so the main loop knows
    // events were dropped and force-rescans every library to catch the
    // missed files, rather than waiting on the 30s periodic re-sync
    // which only re-checks library paths (not their contents).
    let overflow_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let overflow_flag_cb = overflow_flag.clone();
    let (tx, mut rx) =
        mpsc::channel::<notify::Result<notify::Event>>(16384);
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        if tx.try_send(res).is_err() {
            // Channel full — record the drop so the main loop can
            // trigger a full library rescan when it next wakes.
            overflow_flag_cb.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    })?;

    // Track currently-watched paths so the periodic re-poll only arms
    // newcomers and unwatches removed roots.
    let mut watched: HashMap<PathBuf, i64> = HashMap::new();
    sync_watched(&state, &mut watcher, &mut watched).await;
    let mut last_resync = Instant::now();
    const RESYNC_INTERVAL: Duration = Duration::from_secs(30);

    // Per-library debounce buckets. When a library's bucket has been
    // quiet for DEBOUNCE we queue a scan for it.
    let mut pending: HashMap<i64, Instant> = HashMap::new();

    loop {
        // Either drain a batch of events or wake every second to check
        // pending buckets + whether it's time to re-poll the libraries
        // table for new/removed paths.
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(Ok(ev)) => {
                        if !is_interesting(&ev.kind) {
                            continue;
                        }
                        let paths_vec: Vec<(PathBuf, i64)> =
                            watched.iter().map(|(p, id)| (p.clone(), *id)).collect();
                        for p in &ev.paths {
                            if let Some(lib_id) = match_library(&paths_vec, p) {
                                pending.insert(lib_id, Instant::now());
                            }
                        }
                    }
                    Some(Err(e)) => warn!(error = %e, "file watcher event error"),
                    None => return Ok(()), // sender dropped
                }
            }
            _ = sleep(Duration::from_secs(1)) => {
                let now = Instant::now();
                // Channel-overflow recovery: notify dropped at least
                // one event since we last looked, so the incremental
                // pending-buckets set is incomplete. Force-queue a
                // scan for every currently-watched library so missed
                // additions get picked up.
                if overflow_flag.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    warn!(
                        watched_count = watched.len(),
                        "file watcher: event channel overflowed — forcing a rescan of every library"
                    );
                    for lib_id in watched.values() {
                        pending.insert(*lib_id, now);
                    }
                }
                let due: Vec<i64> = pending
                    .iter()
                    .filter(|(_, t)| now.duration_since(**t) >= DEBOUNCE)
                    .map(|(id, _)| *id)
                    .collect();
                for lib_id in due {
                    pending.remove(&lib_id);
                    spawn_scan(state.clone(), lib_id).await;
                }
                if now.duration_since(last_resync) >= RESYNC_INTERVAL {
                    last_resync = now;
                    sync_watched(&state, &mut watcher, &mut watched).await;
                }
            }
        }
    }
}

/// Reconcile the watcher's active path set against `library_paths` in
/// SQLite. Arms newly-added paths and disarms removed ones — keeps the
/// watcher in sync with admin library changes without a server restart.
async fn sync_watched(
    state: &AppState,
    watcher: &mut RecommendedWatcher,
    watched: &mut HashMap<PathBuf, i64>,
) {
    let current = match library_paths(state).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %format!("{e:#}"), "file watcher: re-poll failed");
            return;
        }
    };
    let current_set: HashMap<PathBuf, i64> = current.into_iter().collect();

    // Disarm paths no longer in the libraries table.
    let removed: Vec<PathBuf> = watched
        .keys()
        .filter(|p| !current_set.contains_key(*p))
        .cloned()
        .collect();
    for path in removed {
        let _ = watcher.unwatch(&path);
        watched.remove(&path);
        info!(path = %path.display(), "file watcher: unwatched");
    }

    // Arm new paths.
    for (path, id) in current_set {
        if watched.contains_key(&path) {
            continue;
        }
        if !path.exists() {
            warn!(path = %path.display(), "skipping non-existent library path");
            continue;
        }
        match watcher.watch(&path, RecursiveMode::Recursive) {
            Ok(()) => {
                info!(path = %path.display(), library_id = id, "watching for changes");
                watched.insert(path, id);
            }
            Err(e) => warn!(path = %path.display(), error = %e, "failed to watch library path"),
        }
    }
}

fn is_interesting(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(CreateKind::File)
            | EventKind::Create(CreateKind::Any)
            | EventKind::Remove(RemoveKind::File)
            | EventKind::Remove(RemoveKind::Any)
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Modify(ModifyKind::Data(_))
    )
}

fn match_library(paths: &[(PathBuf, i64)], event_path: &std::path::Path) -> Option<i64> {
    paths
        .iter()
        .find(|(root, _)| event_path.starts_with(root))
        .map(|(_, id)| *id)
}

async fn library_paths(state: &AppState) -> Result<Vec<(PathBuf, i64)>> {
    let rows = sqlx::query(
        "SELECT library_id, path FROM library_paths",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut out: Vec<(PathBuf, i64)> = rows
        .iter()
        .filter_map(|row| {
            let id: i64 = row.try_get("library_id").ok()?;
            let path: String = row.try_get("path").ok()?;
            Some((PathBuf::from(path), id))
        })
        .collect();
    // Sort by descending component count so the deepest matching root
    // wins in `match_library`'s `starts_with` scan. Without this, an
    // operator with nested roots like `/media` + `/media/movies` would
    // see file events under `/media/movies/...` ascribed to whichever
    // root happened to come first out of the DB — typically the
    // shorter one, which scans the wrong library.
    out.sort_by(|a, b| b.0.components().count().cmp(&a.0.components().count()));
    Ok(out)
}

async fn spawn_scan(state: AppState, library_id: i64) {
    // Coordinate with the scheduled scan + admin-triggered scan via
    // the shared per-library lock. A burst of file events during a
    // scheduled scan otherwise piles up parallel scanner runs that
    // hammer the same disk live transcodes are reading from. Bail
    // when the lock is held — whatever scan is already running will
    // sweep up the newly-landed file when it iterates the library.
    if !state.try_acquire_library_scan(library_id).await {
        info!(
            library_id,
            "file watcher: skipping (another scan for this library is in progress)"
        );
        return;
    }
    // Reuse the same orchestration as the manual /libraries/{id}/scan
    // route — create a scan_job row, then spawn the scanner with the
    // current TMDB/TVDB/AniList snapshots. Failure here logs and moves
    // on; the next event will retry.
    let job = match queries::create_scan_job(&state.pool, library_id).await {
        Ok(j) => j,
        Err(e) => {
            warn!(library_id, error = %format!("{e:#}"), "file watcher: create_scan_job failed");
            state.release_library_scan(library_id).await;
            return;
        }
    };
    info!(library_id, job_id = job.id, "file watcher: queued scan");

    let pool = state.pool.clone();
    let ffmpeg = state.ffmpeg.clone();
    let tmdb = state.tmdb_snapshot().await;
    let tvdb = state.tvdb_snapshot().await;
    let anilist = state.anilist_snapshot().await;
    let tvmaze = state.tvmaze.clone();
    let hub = state.hub.clone();
    let cache_root = state.transcoder.cache_root().to_path_buf();
    let state_for_release = state.clone();
    tokio::spawn(async move {
        let inner_emitter: chimpflix_library::ScanEmitter = Arc::new(move |evt| {
            hub.publish(crate::events::Event::Scan(evt));
        });
        // Pipeline wrapper: FileAdded fans out into discovery jobs.
        // Same wrapper used by the manual + scheduled scan paths so
        // file-watcher-discovered files get the same processing.
        let emitter = crate::jobs::pipeline::wrap_emitter_for_pipeline(
            pool.clone(),
            inner_emitter,
        );
        let scan_ok = match chimpflix_library::run_scan(
            pool.clone(),
            ffmpeg,
            tmdb,
            tvdb,
            anilist,
            tvmaze,
            library_id,
            job.id,
            Some(cache_root),
            emitter,
        )
        .await
        {
            Ok(()) => true,
            Err(e) => {
                warn!(library_id, job_id = job.id, error = %format!("{e:#}"), "file watcher scan failed");
                false
            }
        };
        // Optional post-scan trigger: detect markers for any file the
        // (Previously: a post-scan block enqueued marker detection
        // for new files, gated on the `detect_markers_on_add`
        // setting. That's now handled by the discovery pipeline
        // wrapper around the emitter — every FileAdded event fans
        // out into detect_markers_file / preview / loudness /
        // chapter-thumbs jobs automatically. The wrapper's
        // enqueue_job_unique dedup means re-triggers are no-ops.
        // Marker `detect_markers_on_add` setting is effectively
        // always-on with the new pipeline; the knob now affects
        // only the scheduled safety-net task.)
        let _ = scan_ok;
        // Release the lock as the final act so a concurrent scheduled
        // scan or manual trigger can proceed cleanly.
        state_for_release.release_library_scan(library_id).await;
    });
}
