//! Filesystem watcher that auto-triggers library scans when media files
//! appear, disappear, or change on disk.
//!
//! Replaces the cron-only model: instead of waiting for the scheduled
//! `scan_library` task, an mkv landing in a watched path queues a scan
//! within seconds. Debounced (5s of silence) so a `cp -r` doesn't fan
//! into dozens of scan jobs.
//!
//! Per-library reconfiguration is supported via a 30s periodic re-sync:
//! the watcher polls `library_paths` and arms newly-added roots / drops
//! removed ones without needing a restart. The `scan_automatically`
//! setting is re-read every loop iteration so the operator can pause
//! event processing live.
//!
//! Backend selection:
//!   * Default — `notify::RecommendedWatcher` (inotify on Linux). Cheap,
//!     low-latency, but **does not see events on NFS / SMB mounts** and
//!     can miss events from bind-mounts that don't propagate inotify
//!     into the container namespace.
//!   * `file_watcher_use_polling=true` — `notify::PollWatcher`. Stat-
//!     walks every watched root every N seconds. Higher CPU + I/O, but
//!     works on remote filesystems. Required for Docker-on-NAS setups
//!     where the media drive is NFS on the host and bind-mounted in.
//!
//! Backend is chosen at startup from settings; toggling requires a
//! restart to re-arm with the new backend.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use chimpflix_library::queries;
use notify::{
    Config as NotifyConfig, EventKind, PollWatcher, RecommendedWatcher, RecursiveMode, Watcher,
    event::{CreateKind, ModifyKind, RemoveKind},
};
use sqlx::Row;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::state::AppState;

/// How long we wait after the last filesystem event before firing a
/// scan. Long enough that a `cp` of a multi-file release coalesces;
/// short enough that the user sees the library refresh on the order of
/// seconds, not minutes.
const DEBOUNCE: Duration = Duration::from_secs(5);

/// Backend-agnostic wrapper. `notify::Watcher` is object-safe but the
/// concrete watcher types differ enough (`RecommendedWatcher` vs
/// `PollWatcher`) that an enum is clearer than a `Box<dyn Watcher>` —
/// no virtual dispatch tax and the dispatch matches at the call site.
enum WatcherBackend {
    Recommended(RecommendedWatcher),
    Polling(PollWatcher),
}

impl WatcherBackend {
    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        match self {
            Self::Recommended(w) => w.watch(path, mode),
            Self::Polling(w) => w.watch(path, mode),
        }
    }
    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        match self {
            Self::Recommended(w) => w.unwatch(path),
            Self::Polling(w) => w.unwatch(path),
        }
    }
}

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
    let (tx, mut rx) = mpsc::channel::<notify::Result<notify::Event>>(16384);
    let dispatch = move |res: notify::Result<notify::Event>| {
        if tx.try_send(res).is_err() {
            // Channel full — record the drop so the main loop can
            // trigger a full library rescan when it next wakes.
            overflow_flag_cb.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    };
    // Decide backend once at startup. The settings struct mirrors the
    // DB row reloaded on PATCH /admin/settings, but the running watcher
    // can't be hot-swapped without re-arming every watch and risking a
    // window where new files land in neither backend — so we treat the
    // backend choice as restart-required, like scan_automatically.
    let (use_polling, poll_interval_secs) = {
        let s = state.settings.read().await;
        (
            s.file_watcher_use_polling,
            s.file_watcher_poll_interval_secs,
        )
    };
    let mut watcher: WatcherBackend = if use_polling {
        let interval = Duration::from_secs(poll_interval_secs.clamp(5, 3600) as u64);
        info!(
            poll_interval_secs = poll_interval_secs,
            "file watcher: using PollWatcher backend (set for NFS/SMB compat)"
        );
        let cfg = NotifyConfig::default().with_poll_interval(interval);
        WatcherBackend::Polling(PollWatcher::new(dispatch, cfg)?)
    } else {
        info!("file watcher: using RecommendedWatcher backend (inotify on Linux)");
        WatcherBackend::Recommended(notify::recommended_watcher(dispatch)?)
    };

    // Track currently-watched paths so the periodic re-poll only arms
    // newcomers and unwatches removed roots. The companion `sorted`
    // Vec is kept in deepest-first order so event paths get matched
    // against the most-specific root first (fixes nested roots like
    // `/media` + `/media/movies` ascribing events to the wrong library).
    let mut watched: HashMap<PathBuf, i64> = HashMap::new();
    let mut sorted: Vec<(PathBuf, i64)> = Vec::new();
    // Paths we've already warned about being inaccessible. Throttle so
    // a partially-mounted/permission-denied root doesn't spam WARN
    // every 30s for the lifetime of the process. Cleared when the path
    // recovers; if it goes missing again later, a fresh WARN fires.
    let mut warned_missing: HashSet<PathBuf> = HashSet::new();
    sync_watched(
        &state,
        &mut watcher,
        &mut watched,
        &mut sorted,
        &mut warned_missing,
    )
    .await;
    let mut last_resync = Instant::now();
    const RESYNC_INTERVAL: Duration = Duration::from_secs(30);

    // Per-library debounce buckets. When a library's bucket has been
    // quiet for DEBOUNCE we queue a scan for it.
    let mut pending: HashMap<i64, Instant> = HashMap::new();

    loop {
        // Either drain a batch of events or wake every second to check
        // pending buckets + whether it's time to re-poll the libraries
        // table for new/removed paths.
        // Live-read `scan_automatically` once per iteration. When
        // the operator flips it off in admin, the watcher keeps
        // running but stops queueing scans — `pending` gets cleared
        // and incoming events are dropped on the floor. inotify
        // still delivers events into the channel (zero idle cost
        // at the kernel layer), they just don't go anywhere.
        // Toggling back on resumes within the next select iteration.
        let auto_scan = state.settings.read().await.scan_automatically;

        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(Ok(ev)) => {
                        if !auto_scan {
                            continue;
                        }
                        if !is_interesting(&ev.kind) {
                            continue;
                        }
                        for p in &ev.paths {
                            if let Some(lib_id) = match_library(&sorted, p) {
                                let first = !pending.contains_key(&lib_id);
                                pending.insert(lib_id, Instant::now());
                                if first {
                                    // Per-library debounce-window-opened log
                                    // so operators can confirm the watcher
                                    // is seeing their writes without having
                                    // to enable DEBUG for the whole crate.
                                    info!(
                                        library_id = lib_id,
                                        path = %p.display(),
                                        "file watcher: event matched, debouncing"
                                    );
                                } else {
                                    debug!(
                                        library_id = lib_id,
                                        path = %p.display(),
                                        "file watcher: event matched (window still open)"
                                    );
                                }
                            }
                        }
                    }
                    Some(Err(e)) => warn!(error = %e, "file watcher event error"),
                    None => return Ok(()), // sender dropped
                }
            }
            _ = sleep(Duration::from_secs(1)) => {
                let now = Instant::now();
                // While paused, drain pending so we don't queue a
                // burst when the operator re-enables. The next
                // events after toggle-on rebuild the pending set
                // from scratch — same semantic as a cold start.
                if !auto_scan {
                    pending.clear();
                    if now.duration_since(last_resync) >= RESYNC_INTERVAL {
                        last_resync = now;
                        sync_watched(
                            &state,
                            &mut watcher,
                            &mut watched,
                            &mut sorted,
                            &mut warned_missing,
                        )
                        .await;
                    }
                    continue;
                }
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
                    if spawn_scan(state.clone(), lib_id).await {
                        pending.remove(&lib_id);
                    } else {
                        // A scan is already running for this library —
                        // we couldn't acquire the per-library lock.
                        // Keep the pending entry alive so we re-check
                        // after another DEBOUNCE window. Reset the
                        // timestamp to `now` so the retry cadence is
                        // bounded and we don't tight-loop. Without
                        // this, the events that landed during the
                        // in-flight scan are silently dropped — and
                        // a long scan that's already iterated past
                        // the new file's directory won't pick them
                        // up either. (This was the most likely root
                        // cause of operators reporting "I have to
                        // run manual scans to find new files.")
                        pending.insert(lib_id, now);
                    }
                }
                if now.duration_since(last_resync) >= RESYNC_INTERVAL {
                    last_resync = now;
                    sync_watched(
                        &state,
                        &mut watcher,
                        &mut watched,
                        &mut sorted,
                        &mut warned_missing,
                    )
                    .await;
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
    watcher: &mut WatcherBackend,
    watched: &mut HashMap<PathBuf, i64>,
    sorted: &mut Vec<(PathBuf, i64)>,
    warned_missing: &mut HashSet<PathBuf>,
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
    let any_changes = !removed.is_empty();
    for path in removed {
        let _ = watcher.unwatch(&path);
        watched.remove(&path);
        warned_missing.remove(&path);
        info!(path = %path.display(), "file watcher: unwatched");
    }
    // Drop stale entries for paths that are no longer configured at all
    // (operator deleted the library) so we don't carry orphan warnings.
    warned_missing.retain(|p| current_set.contains_key(p));

    // Arm new paths. `std::fs::metadata` over `Path::exists()` so we can
    // surface WHY the check failed (ENOENT vs EACCES vs broken symlink
    // vs other I/O) — exists() collapses every error to false, which
    // makes "skipping non-existent" misleading for permission and
    // mount-namespace problems. Throttle the WARN: once per missing
    // transition, then DEBUG on subsequent retries until the path
    // recovers (at which point a fresh "watching for changes" INFO
    // fires from the success branch below).
    let mut added_any = false;
    for (path, id) in current_set {
        if watched.contains_key(&path) {
            continue;
        }
        match std::fs::metadata(&path) {
            Ok(_) => match watcher.watch(&path, RecursiveMode::Recursive) {
                Ok(()) => {
                    if warned_missing.remove(&path) {
                        info!(
                            path = %path.display(),
                            library_id = id,
                            "watching for changes (path recovered)"
                        );
                    } else {
                        info!(
                            path = %path.display(),
                            library_id = id,
                            "watching for changes"
                        );
                    }
                    watched.insert(path, id);
                    added_any = true;
                }
                Err(e) => warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to watch library path"
                ),
            },
            Err(e) => {
                let kind = e.kind();
                if warned_missing.insert(path.clone()) {
                    // First time we've seen this path miss — loud WARN
                    // with the actual error so the operator can fix
                    // (perm denied? mount race? typo?).
                    warn!(
                        path = %path.display(),
                        error = %e,
                        error_kind = ?kind,
                        "library path unavailable; will keep retrying every 30s"
                    );
                } else {
                    // Already warned once; downgrade to DEBUG so we
                    // don't spam the log every 30s for the lifetime
                    // of an unmounted root.
                    debug!(
                        path = %path.display(),
                        error_kind = ?kind,
                        "library path still unavailable"
                    );
                }
            }
        }
    }
    // Rebuild the sorted vec only when the set changed. Deepest path
    // first so `match_library`'s linear scan picks the most-specific
    // root for nested setups (e.g. `/media` + `/media/movies` both
    // configured — events under the subpath must go to the deeper id).
    if any_changes || added_any {
        let mut next: Vec<(PathBuf, i64)> = watched
            .iter()
            .map(|(p, id)| (p.clone(), *id))
            .collect();
        next.sort_by(|a, b| b.0.components().count().cmp(&a.0.components().count()));
        *sorted = next;
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

fn match_library(sorted: &[(PathBuf, i64)], event_path: &std::path::Path) -> Option<i64> {
    sorted
        .iter()
        .find(|(root, _)| event_path.starts_with(root))
        .map(|(_, id)| *id)
}

async fn library_paths(state: &AppState) -> Result<Vec<(PathBuf, i64)>> {
    let rows = sqlx::query("SELECT library_id, path FROM library_paths")
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

/// Try to queue a scan for `library_id`. Returns true if a scan was
/// queued (or the create-job step hit a recoverable DB error and the
/// caller shouldn't retry), false if another scan is already running
/// for this library — in which case the caller should keep the pending
/// entry alive and retry after the next DEBOUNCE window. (See the
/// retry comment at the call site for why dropping these events was
/// the most likely root cause of "files I added aren't showing up
/// without a manual scan.")
async fn spawn_scan(state: AppState, library_id: i64) -> bool {
    if !state.try_acquire_library_scan(library_id).await {
        debug!(
            library_id,
            "file watcher: deferring (another scan for this library is in progress)"
        );
        return false;
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
            // DB-level failure: don't retry-loop; operator needs to
            // intervene. Returning true drops the pending entry so we
            // don't pile retries on a structurally broken pool.
            return true;
        }
    };
    info!(library_id, job_id = job.id, "file watcher: queued scan");

    let pool = state.pool.clone();
    let ffmpeg = state.ffmpeg.clone();
    let tmdb = state.tmdb_snapshot().await;
    let tvdb = state.tvdb_snapshot().await;
    let anilist = state.anilist_snapshot().await;
    let tvmaze = state.tvmaze.clone();
    let omdb = state.omdb_snapshot().await;
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
            state_for_release.clone(),
            inner_emitter,
        );
        let scan_ok = match chimpflix_library::run_scan(
            pool.clone(),
            ffmpeg,
            tmdb,
            tvdb,
            anilist,
            tvmaze,
            omdb,
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
        let _ = scan_ok;
        // Release the lock as the final act so a concurrent scheduled
        // scan or manual trigger can proceed cleanly.
        state_for_release.release_library_scan(library_id).await;
    });
    true
}
