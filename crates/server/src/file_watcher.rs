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
    // and OOM the server. 4096 events is plenty for normal use — typical
    // scan storms produce 100s of events, never 1000s. When full, the
    // notify callback drops the overflowing event; we recover via the
    // 30s periodic re-sync that re-scans library roots regardless.
    let (tx, mut rx) =
        mpsc::channel::<notify::Result<notify::Event>>(4096);
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        let _ = tx.try_send(res);
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
    Ok(rows
        .iter()
        .filter_map(|row| {
            let id: i64 = row.try_get("library_id").ok()?;
            let path: String = row.try_get("path").ok()?;
            Some((PathBuf::from(path), id))
        })
        .collect())
}

async fn spawn_scan(state: AppState, library_id: i64) {
    // Reuse the same orchestration as the manual /libraries/{id}/scan
    // route — create a scan_job row, then spawn the scanner with the
    // current TMDB/TVDB/AniList snapshots. Failure here logs and moves
    // on; the next event will retry.
    let job = match queries::create_scan_job(&state.pool, library_id).await {
        Ok(j) => j,
        Err(e) => {
            warn!(library_id, error = %format!("{e:#}"), "file watcher: create_scan_job failed");
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
    let state_for_markers = state.clone();
    tokio::spawn(async move {
        let emitter: chimpflix_library::ScanEmitter = Arc::new(move |evt| {
            hub.publish(crate::events::Event::Scan(evt));
        });
        let result = chimpflix_library::run_scan(
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
        .await;
        if let Err(e) = result {
            warn!(library_id, job_id = job.id, error = %format!("{e:#}"), "file watcher scan failed");
            return;
        }
        // Optional post-scan trigger: detect markers for any file the
        // scan introduced that doesn't have auto markers yet. Off by
        // default — gated on `detect_markers_on_add` because the
        // blackdetect pass costs ~30s/45min-episode and not every
        // operator wants that overhead on every drop.
        let on_add = state_for_markers.settings.read().await.detect_markers_on_add;
        if !on_add {
            return;
        }
        // 256 caps a single scan from queueing thousands of jobs at
        // once — typical drops are 1–20 files; an rsync of a season
        // pack still fits comfortably. Excess gets picked up by the
        // next scan or the scheduled `detect_markers` task.
        let files =
            match queries::list_media_files_needing_markers(&pool, library_id, 256).await {
                Ok(v) => v,
                Err(e) => {
                    warn!(library_id, error = %format!("{e:#}"), "file watcher: marker query failed");
                    return;
                }
            };
        if files.is_empty() {
            return;
        }
        info!(library_id, count = files.len(), "file watcher: queueing markers for new files");
        crate::api::markers::spawn_detection(&state_for_markers, files);
    });
}
