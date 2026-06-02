//! Scanner orchestration: walk a library's root paths, classify each
//! video file, probe it, persist rows, optionally enrich via TMDB,
//! emit progress events along the way.
//!
//! Performance shape:
//!   * Per-file work (ffprobe + DB upserts + TMDB enrichment) runs
//!     in parallel via `buffer_unordered(SCAN_PARALLELISM)`. ffprobe
//!     and the metadata HTTP calls dominate latency, both are I/O
//!     bound, so concurrency gets near-linear speedup.
//!   * TMDB `fetch_season` calls are memoised per scan on
//!     `(show_tmdb_id, season_number)` — a 50-episode show used to
//!     hit the same endpoint 50 times.
//!   * `ffprobe` failures no longer drop the file. We persist the
//!     `media_files` row with NULL technical fields and log a warn;
//!     the operator can fix the source and re-scan or refresh.
//!
//! Remaining caveats:
//!   * Title-only matching for items (`UNIQUE (library_id, kind, sort_title)`);
//!     two distinct movies with the same title in the same library collide.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use chimpflix_metadata::{
    AniListClient, MetadataAgent, OmdbClient, TmdbClient, TmdbSeason, TvMazeClient, TvdbClient,
};
use chimpflix_transcoder::{FfmpegConfig, ProbeResult};
use futures::stream::{self, StreamExt};
use sqlx::SqlitePool;
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::events::{ScanEmitter, ScanEvent};
use crate::models::{ItemKind, LibraryKind};
use crate::parser::{self, Classification};
use crate::queries;

/// How many files the scanner processes concurrently. Each in-flight
/// task can hold one ffprobe subprocess + a handful of outstanding
/// HTTP requests against TMDB/TVDB/TVMaze/AniList.
///
/// Lowered 8 → 4 because the previous value was hitting
/// `SQLITE_BUSY_SNAPSHOT` under load — eight concurrent `process_file`
/// tasks each interleaving multiple writes with HTTP calls produced
/// constant writer-slot contention on the single SQLite writer. Four
/// is still plenty of parallelism to hide ffprobe latency (ffprobe
/// runs in its own subprocess, not on the tokio runtime, so the value
/// is really about HTTP/DB overlap) while halving the writer-queue
/// depth. The retry layer in `db::with_busy_retry` plus the move to
/// `BEGIN IMMEDIATE` on the hot job-enqueue path covers the residual
/// contention.
const SCAN_PARALLELISM: usize = 4;

/// Per-scan cache of TMDB season fetches. Keyed on
/// `(show_tmdb_id, season_number)`. Lives only for the duration of a
/// single scan — we deliberately don't persist this across scans so
/// freshly-aired episodes show up on the next run without manual
/// invalidation.
/// Per-scan TMDB season cache. Stores both successful fetches AND
/// confirmed-missing (HTTP 404) results so we don't re-hammer TMDB
/// for a season it definitively says doesn't exist. Common with
/// anime — file naming often uses release-group season numbering
/// that doesn't match TMDB's broadcast-season records, so the same
/// `(show_id, season=N)` 404 fires for every episode of that show.
type SeasonCache = Mutex<HashMap<(i64, i32), CachedSeason>>;

#[derive(Clone)]
enum CachedSeason {
    /// TMDB returned a season payload for this `(show_id, season)`.
    Found(Arc<TmdbSeason>),
    /// TMDB confirmed there's no such season for this `(show_id,
    /// season)` (HTTP 404). Cached so subsequent episodes of the
    /// same show don't re-trigger the lookup. The negative result
    /// is scan-scoped — a future scan re-checks in case TMDB has
    /// since added the season.
    Missing,
}

/// Per-scan set of show ids whose full episode list has already been
/// materialized into placeholder rows this scan. The scanner walks files
/// in parallel, so without this guard the first file of every episode of
/// a show would each re-fetch the season list and re-run the placeholder
/// upsert. We populate placeholders exactly once per show per scan: the
/// first file of a show that resolves a usable external id wins; later
/// files of the same show short-circuit on the `contains` check. Scoped
/// to the scan (not persisted) so the next scan re-checks and picks up
/// newly-announced episodes for ongoing shows.
type PlaceholderShows = Mutex<std::collections::HashSet<i64>>;

// AniList per-scan caches moved to `chimpflix_metadata::anilist_cache`
// so `AniListAgent` can hold them directly. Type aliases below keep
// the scanner's call sites readable.
type AniListCache = chimpflix_metadata::AniListShowCache;
type AniListEpisodeCache = chimpflix_metadata::AniListEpisodeListCache;
type AniListSeasonIdCache = chimpflix_metadata::AniListSeasonIdCache;

/// Process-wide semaphore that bounds how many WebVTT pre-warm
/// ffmpegs can run at once. Before this cap was added, a fresh scan
/// over a 1000-file library spawned one tokio task per text-sub
/// stream and each task launched ffmpeg — easily 50+ concurrent
/// ffmpeg processes contending for the CPU and stealing cycles from
/// any active live transcode. 4 is a conservative ceiling that still
/// makes meaningful progress while leaving the encoder reachable.
static SUBTITLE_PREWARM_LIMIT: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(4)));

const PROGRESS_INTERVAL: i64 = 25;

/// The enabled metadata agents for a library, **in owner-configured
/// priority order**. Loaded once per scan to avoid querying
/// `library_agents` on every file.
///
/// Order is load-bearing: the first agent in the chain is the canonical
/// source for fields it provides (its writes overwrite empty columns),
/// while later agents fill remaining nulls without clobbering earlier
/// writes. See `apply_show_metadata` / `apply_show_metadata_anilist` for
/// the SQL that implements the two modes via the `is_primary` parameter.
#[derive(Debug, Clone, Default)]
struct AgentChain {
    /// Enabled agent names, ordered by ascending priority value as
    /// returned from `list_library_agents` (which sorts by `priority
    /// ASC, agent_name ASC`). First element = highest priority = primary.
    order: Vec<String>,
    /// Operator-configured `metadata_language` (BCP-47), threaded into
    /// each agent so they can honor the locale preference. Defaults to
    /// "en-US" if the settings read fails.
    language: String,
}

/// Read the operator's metadata language preference, defaulting to
/// "en-US" if the settings row is unreadable. Used by the scan path to
/// thread the same language into every metadata agent — keeps the
/// AniList agent from writing romaji/native titles when the operator
/// has set en-US (and analogously for ja-JP).
async fn metadata_language_or_default(pool: &SqlitePool) -> String {
    queries::get_server_settings(pool)
        .await
        .map(|s| s.metadata_language)
        .unwrap_or_else(|_| "en-US".to_string())
}

/// Move `primary` to the front of `order` and put everything else
/// behind it in its existing relative order. When `primary` isn't in
/// the list (operator disabled it), the order is returned untouched —
/// the primary is treated as not-applicable for this library and the
/// first remaining agent becomes the de-facto primary.
///
/// Stable for non-primary entries: their relative order matches the
/// `library_agents.priority` they were read in with, so the operator's
/// fallback ordering still matters.
fn reorder_for_primary(order: Vec<String>, primary: &str) -> Vec<String> {
    if !order.iter().any(|n| n == primary) {
        return order;
    }
    let mut out = Vec::with_capacity(order.len());
    out.push(primary.to_string());
    out.extend(order.into_iter().filter(|n| n != primary));
    out
}

impl AgentChain {
    async fn load(pool: &SqlitePool, library_id: i64) -> Self {
        let language = metadata_language_or_default(pool).await;
        // Read the per-library primary metadata source. Defaults to
        // 'tmdb' if the row read fails — same fallback as the migration's
        // column default — so a missing-libraries-row pathology still
        // produces a runnable chain.
        let primary = sqlx::query_scalar::<_, String>(
            "SELECT primary_metadata_agent FROM libraries WHERE id = ?",
        )
        .bind(library_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "tmdb".to_string());
        match queries::list_library_agents(pool, library_id).await {
            Ok(agents) => Self {
                order: reorder_for_primary(
                    agents
                        .into_iter()
                        .filter(|a| a.enabled)
                        .map(|a| a.agent_name)
                        .collect(),
                    &primary,
                ),
                language,
            },
            Err(e) => {
                warn!(error = %format!("{e:#}"), library_id, "failed to load library agents — falling back to defaults");
                // Default fallback list (any agent that wasn't seeded
                // for this library still has a chance to run). Same
                // reorder rule honors the primary even on the fallback.
                Self {
                    order: reorder_for_primary(
                        vec![
                            "tmdb".into(),
                            "tvdb".into(),
                            "tvmaze".into(),
                            "anilist".into(),
                            "omdb".into(),
                        ],
                        &primary,
                    ),
                    language,
                }
            }
        }
    }

    /// Iterate the chain in priority order. First name = primary.
    fn ordered(&self) -> impl Iterator<Item = &str> {
        self.order.iter().map(String::as_str)
    }

    /// Position of `name` in the chain (0 = primary). `None` if not enabled.
    fn position(&self, name: &str) -> Option<usize> {
        self.order.iter().position(|s| s == name)
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run_scan(
    pool: SqlitePool,
    ffmpeg: FfmpegConfig,
    tmdb: Option<TmdbClient>,
    tvdb: Option<TvdbClient>,
    anilist: Option<AniListClient>,
    tvmaze: Option<TvMazeClient>,
    omdb: Option<OmdbClient>,
    library_id: i64,
    job_id: i64,
    // Transcoder cache root, used to pre-warm the WebVTT subtitle
    // cache as files are scanned. Pass the path the TranscodeManager
    // was constructed with. When None, scans skip subtitle
    // pre-warming and the cache populates lazily at session start
    // instead — used by paths that don't have a transcoder
    // reference handy (rare; mostly tests).
    cache_root: Option<std::path::PathBuf>,
    emitter: ScanEmitter,
) -> Result<()> {
    let library = queries::get_library(&pool, library_id)
        .await?
        .context("library not found")?;

    queries::mark_scan_running(&pool, job_id).await?;
    emitter(ScanEvent::Started { job_id, library_id });

    let result = scan_inner(
        &pool,
        &ffmpeg,
        tmdb.as_ref(),
        tvdb.as_ref(),
        anilist.as_ref(),
        tvmaze.as_ref(),
        omdb.as_ref(),
        &library.paths,
        library.kind,
        library_id,
        job_id,
        cache_root.as_deref(),
        &emitter,
    )
    .await;

    match result {
        Ok(counters) => {
            queries::mark_scan_completed(
                &pool,
                job_id,
                counters.files_seen,
                counters.files_added,
                counters.files_updated,
                counters.files_removed,
            )
            .await?;
            queries::touch_library_last_scan(&pool, library_id).await?;
            // "Empty trash automatically after every scan" (Plex parity).
            // When enabled, immediately hard-delete this library's
            // soft-removed files instead of waiting for the 7-day grace
            // window. Scoped to this library so a temporary unmount
            // elsewhere can't nuke an unrelated library's files. Best-
            // effort: a purge failure must not fail an otherwise-good
            // scan. (Per-file WebVTT cache for the purged paths is left
            // for lazy cleanup — orphaned entries are keyed by path+mtime
            // so they never serve stale content.)
            let empty_trash = queries::get_server_settings(&pool)
                .await
                .map(|s| s.empty_trash_after_scan)
                .unwrap_or(false);
            if empty_trash {
                match queries::purge_removed_media_files_for_library(
                    &pool,
                    library_id,
                    chimpflix_common::now_ms(),
                )
                .await
                {
                    Ok(report) if report.files_purged > 0 => info!(
                        library_id,
                        files_purged = report.files_purged,
                        items_purged = report.items_purged,
                        "empty-trash-after-scan purged removed files"
                    ),
                    Ok(_) => {}
                    Err(e) => warn!(
                        library_id,
                        error = %format!("{e:#}"),
                        "empty-trash-after-scan purge failed (non-fatal)"
                    ),
                }
            }
            emitter(ScanEvent::Completed {
                job_id,
                library_id,
                files_seen: counters.files_seen,
                files_added: counters.files_added,
                files_updated: counters.files_updated,
                files_removed: counters.files_removed,
            });
            info!(
                job_id,
                library_id,
                files_seen = counters.files_seen,
                files_added = counters.files_added,
                files_updated = counters.files_updated,
                "scan completed"
            );
            Ok(())
        }
        Err(e) => {
            let msg = format!("{e:#}");
            warn!(job_id, library_id, error = %msg, "scan failed");
            queries::mark_scan_failed(&pool, job_id, &msg).await?;
            emitter(ScanEvent::Failed {
                job_id,
                library_id,
                error: msg,
            });
            Err(e)
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Counters {
    files_seen: i64,
    files_added: i64,
    files_updated: i64,
    files_removed: i64,
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn scan_inner(
    pool: &SqlitePool,
    ffmpeg: &FfmpegConfig,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    anilist: Option<&AniListClient>,
    tvmaze: Option<&TvMazeClient>,
    omdb: Option<&OmdbClient>,
    roots: &[String],
    library_kind: LibraryKind,
    library_id: i64,
    job_id: i64,
    cache_root: Option<&Path>,
    emitter: &ScanEmitter,
) -> Result<Counters> {
    let existing = queries::existing_media_files(pool, library_id).await?;
    let scan = collect_candidates(roots).await?;
    let candidates = scan.files;
    let reachable_roots = scan.reachable_roots;
    let agents = AgentChain::load(pool, library_id).await;
    info!(
        library_id,
        count = candidates.len(),
        reachable_roots = reachable_roots.len(),
        enabled_agents = ?agents.order,
        "scan candidates collected"
    );

    // Snapshot the paths we're about to process — the reconciliation pass
    // at the end compares this set against the DB to discover what
    // disappeared from disk. Done before we hand `candidates` to the
    // streaming iterator because `into_iter()` consumes it.
    let seen_paths: std::collections::HashSet<String> = candidates
        .iter()
        .map(|(_, p)| p.to_string_lossy().to_string())
        .collect();

    let existing = Arc::new(existing);
    let agents = Arc::new(agents);
    let cache_root_owned: Option<PathBuf> = cache_root.map(Path::to_path_buf);
    let season_cache: Arc<SeasonCache> = Arc::new(Mutex::new(HashMap::new()));
    let anilist_cache: Arc<AniListCache> = Arc::new(Mutex::new(HashMap::new()));
    let anilist_episode_cache: Arc<AniListEpisodeCache> = Arc::new(Mutex::new(HashMap::new()));
    let anilist_season_id_cache: Arc<AniListSeasonIdCache> = Arc::new(Mutex::new(HashMap::new()));
    let placeholder_shows: Arc<PlaceholderShows> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));

    // Clone the network clients up front so each parallel worker owns
    // its own handle. reqwest::Client is internally Arc'd, so this is
    // a cheap refcount bump per worker — not duplicated state.
    let pool_owned = pool.clone();
    let ffmpeg_owned = ffmpeg.clone();
    let tmdb_owned = tmdb.cloned();
    let tvdb_owned = tvdb.cloned();
    let anilist_owned = anilist.cloned();
    let tvmaze_owned = tvmaze.cloned();
    let omdb_owned = omdb.cloned();

    let mut tasks = stream::iter(candidates.into_iter())
        .map(|(root, path)| {
            let pool = pool_owned.clone();
            let ffmpeg = ffmpeg_owned.clone();
            let tmdb = tmdb_owned.clone();
            let tvdb = tvdb_owned.clone();
            let anilist = anilist_owned.clone();
            let tvmaze = tvmaze_owned.clone();
            let omdb = omdb_owned.clone();
            let agents = agents.clone();
            let existing = existing.clone();
            let cache_root = cache_root_owned.clone();
            let season_cache = season_cache.clone();
            let anilist_cache = anilist_cache.clone();
            let anilist_episode_cache = anilist_episode_cache.clone();
            let anilist_season_id_cache = anilist_season_id_cache.clone();
            let placeholder_shows = placeholder_shows.clone();
            async move {
                let res = process_file(
                    &pool,
                    &ffmpeg,
                    tmdb.as_ref(),
                    tvdb.as_ref(),
                    anilist.as_ref(),
                    tvmaze.as_ref(),
                    omdb.as_ref(),
                    &agents,
                    &existing,
                    library_id,
                    library_kind,
                    &root,
                    &path,
                    cache_root.as_deref(),
                    &season_cache,
                    &anilist_cache,
                    &anilist_episode_cache,
                    &anilist_season_id_cache,
                    &placeholder_shows,
                )
                .await;
                (path, res)
            }
        })
        .buffer_unordered(SCAN_PARALLELISM);

    let mut counters = Counters::default();
    let mut since_progress = 0i64;

    while let Some((path, res)) = tasks.next().await {
        counters.files_seen += 1;
        match res {
            Ok((queries::FileOutcome::Added, Some(media_file_id))) => {
                counters.files_added += 1;
                // Hand the new file off to the discovery pipeline.
                // The consumer (server-side scan emitter) enqueues
                // detect_markers / preview / loudness / chapter
                // thumbs jobs against this id so processing starts
                // as soon as the row lands rather than waiting for
                // the next scheduled safety-net tick.
                emitter(ScanEvent::FileAdded {
                    job_id,
                    library_id,
                    media_file_id,
                });
            }
            Ok((queries::FileOutcome::Added, None)) => {
                // Outcome=Added implies file_id was created. This
                // arm should be unreachable; treat as Added without
                // an event rather than panicking on a future
                // refactor that returns Added with None.
                counters.files_added += 1;
            }
            Ok((queries::FileOutcome::Updated, _)) => counters.files_updated += 1,
            Ok((queries::FileOutcome::Unchanged, _)) => {}
            Err(e) => warn!(?path, error = %format!("{e:#}"), "failed to process file"),
        }
        since_progress += 1;
        if since_progress >= PROGRESS_INTERVAL {
            since_progress = 0;
            // Counter updates are a cosmetic side-effect — they power
            // the progress bar in the activity feed. A transient write
            // failure here (SQLITE_BUSY when 8 parallel `process_file`
            // tasks are contending for the writer slot, or the file
            // watcher racing on a sibling library) should NOT abort
            // the entire scan. Propagating the error with `?` was the
            // root cause of the "scan stops at 775/1090, status=failed,
            // database is locked" bug observed 2026-05-21 on a fresh
            // movies library: the periodic update tripped BUSY, the
            // scan bailed mid-walk, and the remaining 315 files were
            // never visited.
            //
            // Failure here logs at warn and continues; the next interval
            // will retry, and the terminal `mark_scan_completed` /
            // `mark_scan_failed` call writes the final tally regardless.
            // The activity feed just shows a slightly stale count for
            // a few seconds — acceptable trade vs. losing the rest of
            // the scan.
            if let Err(e) = queries::update_scan_counters(
                pool,
                job_id,
                counters.files_seen,
                counters.files_added,
                counters.files_updated,
                counters.files_removed,
            )
            .await
            {
                warn!(
                    job_id,
                    library_id,
                    files_seen = counters.files_seen,
                    error = %format!("{e:#}"),
                    "scan progress-counter update failed; continuing scan",
                );
            }
            emitter(ScanEvent::Progress {
                job_id,
                library_id,
                files_seen: counters.files_seen,
                files_added: counters.files_added,
                files_updated: counters.files_updated,
                files_removed: counters.files_removed,
            });
        }
    }

    // Reconciliation pass: soft-delete media_files whose on-disk path
    // disappeared. Scoped to the roots that were actually reachable this
    // scan — partial unmounts MUST NOT eat the offline files. Re-appeared
    // files are not touched here; the scanner's upsert already clears
    // `removed_at = NULL` whenever it processes a candidate, so a file
    // coming back online resurrects itself on the next scan automatically.
    if reachable_roots.is_empty() {
        warn!(
            library_id,
            "every library root is unreachable; skipping removal reconciliation"
        );
    } else {
        match queries::list_media_files_for_verify(pool, library_id).await {
            Ok(rows) => {
                let mut to_remove: Vec<i64> = Vec::new();
                for row in &rows {
                    if row.removed_at.is_some() {
                        continue; // already marked; nothing to do
                    }
                    if seen_paths.contains(&row.path) {
                        continue; // we just processed it (or upserted it)
                    }
                    // Only reconcile files whose path sits under a root we
                    // could actually walk this scan. Files under offline
                    // roots stay as-is until that mount returns and a
                    // subsequent scan reaches them.
                    let p = Path::new(&row.path);
                    if !reachable_roots.iter().any(|r| p.starts_with(r)) {
                        continue;
                    }
                    to_remove.push(row.id);
                }
                if !to_remove.is_empty() {
                    match queries::mark_media_files_removed(pool, &to_remove).await {
                        Ok(n) => {
                            counters.files_removed = n as i64;
                            info!(
                                library_id,
                                removed = n,
                                "scan reconciliation soft-deleted missing files"
                            );
                        }
                        Err(e) => warn!(
                            library_id,
                            error = %format!("{e:#}"),
                            "scan reconciliation: mark_media_files_removed failed",
                        ),
                    }
                }
            }
            Err(e) => warn!(
                library_id,
                error = %format!("{e:#}"),
                "scan reconciliation: list_media_files_for_verify failed; skipping removal pass",
            ),
        }
    }

    Ok(counters)
}

/// Output of [`collect_candidates`]. Carries the list of (root, file)
/// pairs plus the subset of roots that were actually reachable on disk.
/// The reachable list scopes the reconciliation pass so a partially
/// unmounted library doesn't soft-delete files under roots that are
/// just temporarily offline.
struct CandidateScan {
    files: Vec<(PathBuf, PathBuf)>,
    reachable_roots: Vec<PathBuf>,
}

async fn collect_candidates(roots: &[String]) -> Result<CandidateScan> {
    let roots: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
    tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        let mut reachable_roots = Vec::new();
        for root in &roots {
            if !root.exists() {
                warn!(root = %root.display(), "library root does not exist");
                continue;
            }
            reachable_roots.push(root.clone());
            // `follow_links(false)` keeps us from descending through
            // symlinks, which handles the common cycle case (Library
            // -> Show -> ../). The `max_depth` cap is a belt: a
            // pathological bind-mount loop (mount --bind A B where
            // B is under A) presents to walkdir as real directories
            // and would otherwise iterate forever. 32 levels is well
            // beyond any legitimate library tree
            // (Library/Show/Season/Episode-folder/Extras is 4-5).
            // We also skip "hidden" dotfiles to avoid descending into
            // .git or .DS_Store metadata trees from synced shares.
            for entry in WalkDir::new(root)
                .follow_links(false)
                .max_depth(32)
                .into_iter()
                .filter_entry(|e| {
                    e.file_name()
                        .to_str()
                        .map(|s| !s.starts_with('.') || s == ".")
                        .unwrap_or(true)
                })
            {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(error = %e, "walk error");
                        continue;
                    }
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.into_path();
                if !parser::is_video_file(&path) {
                    continue;
                }
                out.push((root.clone(), path));
            }
        }
        CandidateScan {
            files: out,
            reachable_roots,
        }
    })
    .await
    .context("walk join failed")
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn process_file(
    pool: &SqlitePool,
    ffmpeg: &FfmpegConfig,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    anilist: Option<&AniListClient>,
    tvmaze: Option<&TvMazeClient>,
    omdb: Option<&OmdbClient>,
    agents: &AgentChain,
    existing: &HashMap<String, i64>,
    library_id: i64,
    library_kind: LibraryKind,
    root: &Path,
    path: &Path,
    cache_root: Option<&Path>,
    season_cache: &SeasonCache,
    anilist_cache: &chimpflix_metadata::AniListShowCacheArc,
    anilist_episode_cache: &chimpflix_metadata::AniListEpisodeListCacheArc,
    anilist_season_id_cache: &chimpflix_metadata::AniListSeasonIdCacheArc,
    placeholder_shows: &PlaceholderShows,
) -> Result<(queries::FileOutcome, Option<i64>)> {
    // Non-UTF8 paths used to fail silently up the error chain with
    // only the generic "non-UTF8 path" message in the scan job log.
    // Operators reported files disappearing from the library without
    // an obvious cause; the lossy display string here lets them see
    // *which* file got rejected (typically a Latin-1 filename that
    // the filesystem driver didn't normalize) so they can rename it.
    let path_str = path.to_str().map(|s| s.to_string()).ok_or_else(|| {
        let lossy = path.to_string_lossy();
        warn!(
            path_lossy = %lossy,
            root = %root.display(),
            "scanner: skipping file with non-UTF8 path; rename via shell to recover"
        );
        anyhow::anyhow!("non-UTF8 path: {lossy}")
    })?;

    let meta = tokio::fs::metadata(path)
        .await
        .with_context(|| format!("stat {}", path.display()))?;
    let size = meta.len() as i64;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let existing_mtime = existing.get(&path_str).copied();

    // Fast-path: file is in the DB and mtime hasn't changed → skip the
    // expensive ffprobe + TMDB calls.
    if existing_mtime == Some(mtime_ms) {
        return Ok((queries::FileOutcome::Unchanged, None));
    }

    // `classify` is total now — it always returns a Classification.
    // When the regex pipeline couldn't extract clean metadata it
    // falls back to a cleaned-filename stub with `auto_matched =
    // false`. We persist that flag onto `items.auto_matched` so
    // the admin UI can surface a "fix this" affordance without
    // making the file invisible.
    let parser::ClassifyResult {
        class: classification,
        auto_matched,
    } = parser::classify(path, root, library_kind);
    if !auto_matched {
        // Keep the info log so operators can grep for surprising
        // names; the file is no longer dropped silently.
        info!(
            stem = %path.file_stem().and_then(|s| s.to_str()).unwrap_or("?"),
            path = %path.display(),
            library_kind = ?library_kind,
            "scanner: classifier couldn't extract season/episode/title — linking as unmatched (fix via the Unmatched files admin view)"
        );
    }

    // ffprobe can fail for legitimate reasons (truncated file, weird
    // container, sample/.nfo masquerading as .mkv, foreign-encoded
    // filename that ffprobe's quoting chokes on). Pre-this-change a
    // probe failure dropped the file from the catalog entirely; we now
    // log and persist the row with NULL technical fields so it's still
    // visible. Operators can fix the source and either rescan or use
    // the Refresh metadata path to re-probe.
    let probe = match chimpflix_transcoder::probe(ffmpeg, path).await {
        Ok(p) => p,
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %format!("{e:#}"),
                "scanner: ffprobe failed; linking file with empty technical metadata"
            );
            ProbeResult {
                duration_ms: None,
                bit_rate: None,
                size_bytes: None,
                container: None,
                streams: Vec::new(),
            }
        }
    };

    let mut movie_hint: Option<MovieHint> = None;
    let mut show_hint: Option<ShowHint> = None;
    let item_id: Option<i64>;
    let episode_id: Option<i64>;

    match classification {
        Classification::Movie {
            title,
            sort_title,
            year,
        } => {
            let id = queries::upsert_item_with_match(
                pool,
                library_id,
                ItemKind::Movie,
                &title,
                &sort_title,
                year,
                auto_matched,
            )
            .await?;
            // Movie hints drive TMDB enrichment. Skip enriching
            // unmatched stubs — the cleaned filename is unlikely to
            // resolve to a real TMDB match and we'd just burn quota
            // on garbage queries. Enrichment will re-run after the
            // operator fix-matches.
            if auto_matched {
                movie_hint = Some(MovieHint { title, year, id });
            }
            item_id = Some(id);
            episode_id = None;
        }
        Classification::Episode {
            show_title,
            show_sort_title,
            show_year,
            season,
            episode,
            title,
            absolute_number,
        } => {
            let show_id = queries::upsert_item_with_match(
                pool,
                library_id,
                ItemKind::Show,
                &show_title,
                &show_sort_title,
                show_year,
                auto_matched,
            )
            .await?;
            let season_id = queries::upsert_season(pool, show_id, season).await?;
            let fallback_title = title.unwrap_or_else(|| format!("Episode {episode}"));
            let ep_id = queries::upsert_episode(
                pool,
                season_id,
                episode,
                &fallback_title,
                absolute_number,
            )
            .await?;

            // Same enrichment-skip rationale as the Movie arm.
            if auto_matched {
                show_hint = Some(ShowHint {
                    show_title,
                    show_year,
                    show_id,
                    season_number: season,
                    episode_number: episode,
                    episode_id: ep_id,
                    absolute_number,
                });
            }
            item_id = None;
            episode_id = Some(ep_id);
        }
    }

    // Persist the media file row.
    let input = queries::MediaFileInput {
        item_id,
        episode_id,
        path: &path_str,
        size_bytes: probe.size_bytes.unwrap_or(size),
        mtime_ms,
        container: probe.container.as_deref(),
        duration_ms: probe.duration_ms,
        bit_rate: probe.bit_rate,
        width: probe.streams.iter().find_map(|s| s.width),
        height: probe.streams.iter().find_map(|s| s.height),
        hdr_format: probe.streams.iter().find_map(|s| s.hdr_format.as_deref()),
    };
    let (file_id, outcome) = queries::upsert_media_file(pool, input, existing_mtime).await?;
    queries::replace_media_streams(pool, file_id, &probe.streams).await?;

    // Pre-warm the WebVTT subtitle cache for every text subtitle in
    // the source. Without this, the user's first session-time pick
    // of a sub triggers a fresh ffmpeg extraction which on a Bluray
    // remux can take minutes — long enough to time out the /sessions
    // HTTP request and surface "Playback failed" before any video
    // arrives. With it, the cache is ready by the time anyone hits
    // play, and the session-start sidecar handler is a tokio::fs
    // read instead.
    //
    // Spawned per file so a slow extraction on one source doesn't
    // hold up the scanner; the scanner moves on, the cache fills in
    // the background. Cache-hit on already-extracted sub indices
    // turns these into cheap no-ops on subsequent scans.
    if let Some(cache_root) = cache_root {
        let text_indices: Vec<u32> = probe
            .streams
            .iter()
            .filter(|s| matches!(s.kind, chimpflix_transcoder::StreamKind::Subtitle))
            .scan(0u32, |idx, s| {
                let here = *idx;
                *idx += 1;
                let is_text = s
                    .codec
                    .as_deref()
                    .map(chimpflix_transcoder::is_text_subtitle_codec)
                    .unwrap_or(false);
                Some(is_text.then_some(here))
            })
            .flatten()
            .collect();
        if !text_indices.is_empty() {
            let ffmpeg_cfg = ffmpeg.clone();
            let cache_root_owned = cache_root.to_path_buf();
            let input_owned = path.to_path_buf();
            // Capture the semaphore Arc; acquire happens inside the
            // spawned task so the scanner doesn't block its own
            // sequential file loop on a saturated pre-warm queue.
            let limiter = SUBTITLE_PREWARM_LIMIT.clone();
            tokio::spawn(async move {
                let _permit = match limiter.acquire_owned().await {
                    Ok(p) => p,
                    // Semaphore closed: process shutdown. Drop the
                    // task quietly rather than running pre-warm
                    // against an outgoing FfmpegConfig.
                    Err(_) => return,
                };
                if let Err(e) = chimpflix_transcoder::scan_prewarm_text_subs(
                    &ffmpeg_cfg,
                    &cache_root_owned,
                    &input_owned,
                    &text_indices,
                )
                .await
                {
                    warn!(
                        error = %format!("{e:#}"),
                        path = %input_owned.display(),
                        "scan-time webvtt prewarm failed; first session play will fall back to on-demand extraction"
                    );
                }
            });
        }
    }

    if let Some(iid) = item_id {
        if let Some(d) = probe.duration_ms {
            queries::set_item_duration_if_null(pool, iid, d).await?;
        }
    }

    // Dispatch metadata enrichment in the operator-configured chain
    // order. Each enabled agent's `fetch_*` is called via the
    // [`MetadataAgent`] trait; results flow into the polymorphic
    // [`queries::apply_movie_data`] / [`queries::apply_show_data`]
    // writers which honor `WriteMode::Primary` for the first agent
    // (position 0 = overwrites null-or-stale columns) and
    // `WriteMode::FillNulls` for later agents.
    //
    // Provider-specific extras (TMDB collections + credits + extras,
    // AniList per-episode `streamingEpisodes` enrichment) still run via
    // legacy helpers below for the agents that have them. Subsequent
    // slices fold those into the trait too.
    if let Some(hint) = movie_hint {
        let lookup = chimpflix_metadata::MovieLookup {
            item_id: hint.id,
            title: hint.title.clone(),
            year: hint.year,
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
        };
        for agent_name in agents.ordered() {
            let primary = agents.position(agent_name) == Some(0);
            let mode = if primary {
                chimpflix_metadata::WriteMode::Primary
            } else {
                chimpflix_metadata::WriteMode::FillNulls
            };
            let data = match agent_name {
                "tmdb" => match tmdb {
                    Some(c) => match chimpflix_metadata::TmdbAgent::new(c.clone())
                        .fetch_movie(&lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), title = %hint.title, "TMDB movie lookup failed");
                            None
                        }
                    },
                    None => None,
                },
                "tvdb" => match tvdb {
                    Some(c) => match chimpflix_metadata::TvdbAgent::new(c.clone())
                        .fetch_movie(&lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), title = %hint.title, "TVDB movie lookup failed");
                            None
                        }
                    },
                    None => None,
                },
                "omdb" => match omdb {
                    Some(c) => match chimpflix_metadata::OmdbAgent::new(c.clone())
                        .fetch_movie(&lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), title = %hint.title, "OMDb movie lookup failed");
                            None
                        }
                    },
                    None => None,
                },
                // tvmaze + anilist not applicable to movies; ignore silently.
                _ => None,
            };
            let Some(data) = data else { continue };
            if let Err(e) =
                queries::apply_movie_data(pool, hint.id, &data, mode, agent_name).await
            {
                warn!(error = %format!("{e:#}"), agent = %agent_name, "apply movie data");
            }
            // Cast + crew, promotional videos, user reviews — all
            // flow through the common trait now. Source-scoped writes
            // so two agents contributing cast to the same item don't
            // clobber each other (item_credits.source / item_extras.source
            // / item_reviews.source).
            if !data.people.is_empty()
                && let Err(e) =
                    queries::apply_item_credits_for_source(pool, hint.id, &data.people, agent_name)
                        .await
            {
                warn!(error = %format!("{e:#}"), agent = %agent_name, "apply item credits");
            }
            if !data.videos.is_empty()
                && let Err(e) = queries::apply_item_extras(pool, hint.id, &data.videos).await
            {
                warn!(error = %format!("{e:#}"), agent = %agent_name, "apply item extras");
            }
            if !data.reviews.is_empty()
                && let Err(e) = queries::apply_item_reviews_for_source(
                    pool,
                    hint.id,
                    &data.reviews,
                    agent_name,
                )
                .await
            {
                warn!(error = %format!("{e:#}"), agent = %agent_name, "apply item reviews");
            }
            // TMDB collection (franchise) handling. MovieData.tmdb_collection
            // is the only agent that populates this; the apply layer
            // upserts the collection row and assigns the item to it,
            // then optionally fetches the full collection detail when
            // overview is still NULL (cached upstream so the extra call
            // is cheap).
            if let Some(coll) = data.tmdb_collection.as_ref()
                && let Some(c) = tmdb
            {
                apply_collection_ref_for_item(pool, c, hint.id, coll).await;
            }
        }
    }
    if let Some(mut hint) = show_hint {
        let mut show_lookup = chimpflix_metadata::ShowLookup {
            item_id: hint.show_id,
            title: hint.show_title.clone(),
            year: hint.show_year,
            imdb_id: None,
            tmdb_id: None,
            tvdb_id: None,
            anilist_id: None,
            tvmaze_id: None,
        };
        for agent_name in agents.ordered() {
            let primary = agents.position(agent_name) == Some(0);
            let mode = if primary {
                chimpflix_metadata::WriteMode::Primary
            } else {
                chimpflix_metadata::WriteMode::FillNulls
            };
            // anilist on non-anime libraries is a no-op (seed defaults only put
            // anilist in anime libraries, but operators can edit the chain).
            if agent_name == "anilist" && !matches!(library_kind, LibraryKind::Anime) {
                continue;
            }
            let data = match agent_name {
                "tmdb" => match tmdb {
                    Some(c) => match chimpflix_metadata::TmdbAgent::new(c.clone())
                        .fetch_show(&show_lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), title = %hint.show_title, "TMDB show lookup failed");
                            None
                        }
                    },
                    None => None,
                },
                "tvdb" => match tvdb {
                    Some(c) => match chimpflix_metadata::TvdbAgent::new(c.clone())
                        .fetch_show(&show_lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), title = %hint.show_title, "TVDB show lookup failed");
                            None
                        }
                    },
                    None => None,
                },
                "tvmaze" => match tvmaze {
                    Some(c) => match chimpflix_metadata::TvMazeAgent::new(c.clone())
                        .fetch_show(&show_lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), title = %hint.show_title, "TVMaze show lookup failed");
                            None
                        }
                    },
                    None => None,
                },
                "anilist" => match anilist {
                    Some(c) => {
                        let agent = chimpflix_metadata::AniListAgent::with_language(
                            c.clone(),
                            anilist_cache.clone(),
                            anilist_episode_cache.clone(),
                            anilist_season_id_cache.clone(),
                            agents.language.clone(),
                        );
                        match agent.fetch_show(&show_lookup).await {
                            Ok(d) => d,
                            Err(e) => {
                                warn!(error = %format!("{e:#}"), title = %hint.show_title, "AniList show lookup failed");
                                None
                            }
                        }
                    }
                    None => None,
                },
                "omdb" => match omdb {
                    Some(c) => match chimpflix_metadata::OmdbAgent::new(c.clone())
                        .fetch_show(&show_lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), title = %hint.show_title, "OMDb show lookup failed");
                            None
                        }
                    },
                    None => None,
                },
                _ => None,
            };
            if let Some(data) = data {
                if let Err(e) =
                    queries::apply_show_data(pool, hint.show_id, &data, mode, agent_name).await
                {
                    warn!(error = %format!("{e:#}"), agent = %agent_name, "apply show data");
                }
                // Carry the freshly discovered IDs forward so later
                // agents in the chain can address the same show by id
                // instead of re-running text search.
                if data.tmdb_id.is_some() {
                    show_lookup.tmdb_id = data.tmdb_id;
                }
                if data.tvdb_id.is_some() {
                    show_lookup.tvdb_id = data.tvdb_id;
                }
                if data.anilist_id.is_some() {
                    show_lookup.anilist_id = data.anilist_id;
                }
                if data.tvmaze_id.is_some() {
                    show_lookup.tvmaze_id = data.tvmaze_id;
                }
                if data.imdb_id.is_some() {
                    show_lookup.imdb_id = data.imdb_id.clone();
                }
                // Cast + crew, videos, reviews — same flow as the
                // movie path. Source-scoped writes; no TMDB-specific
                // branch here anymore (anything an agent populates in
                // `ShowData` flows through one of these helpers).
                if !data.people.is_empty()
                    && let Err(e) = queries::apply_item_credits_for_source(
                        pool,
                        hint.show_id,
                        &data.people,
                        agent_name,
                    )
                    .await
                {
                    warn!(error = %format!("{e:#}"), agent = %agent_name, "apply show credits");
                }
                if !data.videos.is_empty()
                    && let Err(e) =
                        queries::apply_item_extras(pool, hint.show_id, &data.videos).await
                {
                    warn!(error = %format!("{e:#}"), agent = %agent_name, "apply show extras");
                }
                if !data.reviews.is_empty()
                    && let Err(e) = queries::apply_item_reviews_for_source(
                        pool,
                        hint.show_id,
                        &data.reviews,
                        agent_name,
                    )
                    .await
                {
                    warn!(error = %format!("{e:#}"), agent = %agent_name, "apply show reviews");
                }
            }
        }

        // Absolute-episode resolver. Anime libraries with bare-number
        // filenames ("Show - 29.mkv") store the episode at
        // `(season=1, episode=29)` initially because the parser has
        // no S/E signal. If the operator has TMDB in this library's
        // chain, walk season counts to find the actual (season,
        // episode) and relocate the row.
        //
        // Gated on `agents.position("tmdb").is_some()` so removing
        // TMDB from a library's chain genuinely silences all TMDB
        // network activity — including this resolver. Without that
        // check the resolver would still fire TMDB calls whenever the
        // client was globally configured, defeating the chain config.
        let tmdb_in_chain = agents.position("tmdb").is_some();
        if tmdb_in_chain
            && let Some(absolute_number) = hint.absolute_number
            && let Some(tmdb_id) = show_lookup.tmdb_id
            && let Some(c) = tmdb
        {
            let stored_mode = queries::get_episode_numbering_mode(pool, hint.show_id)
                .await
                .unwrap_or_else(|_| "season_relative".to_string());
            // Skip detection-only work when the show is small enough
            // that the file number obviously fits in season 1.
            // Heuristic: if absolute_number > 12 it's worth checking.
            let already_absolute = stored_mode == "absolute";
            let worth_checking = already_absolute || absolute_number > 12;
            if worth_checking
                && let Some((target_season, target_ep)) = tmdb_resolve_absolute_episode(
                    season_cache,
                    c,
                    tmdb_id,
                    absolute_number,
                    50,
                )
                .await
                && (target_season, target_ep) != (hint.season_number, hint.episode_number)
            {
                match queries::move_episode_to_season(
                    pool,
                    hint.episode_id,
                    target_season,
                    target_ep,
                )
                .await
                {
                    Ok(true) => {
                        debug!(
                            show = %hint.show_title,
                            absolute_number,
                            from = ?(hint.season_number, hint.episode_number),
                            to = ?(target_season, target_ep),
                            "absolute-ep resolver remapped episode"
                        );
                        hint.season_number = target_season;
                        hint.episode_number = target_ep;
                        if !already_absolute
                            && let Err(e) = queries::set_episode_numbering_mode(
                                pool,
                                hint.show_id,
                                "absolute",
                            )
                            .await
                        {
                            warn!(error = %format!("{e:#}"), show_id = hint.show_id, "set numbering mode failed");
                        }
                    }
                    Ok(false) => {
                        debug!(
                            show = %hint.show_title,
                            target_season,
                            target_ep,
                            "absolute-ep resolver: target slot already occupied; leaving as-is"
                        );
                    }
                    Err(e) => warn!(
                        error = %format!("{e:#}"),
                        show = %hint.show_title,
                        "absolute-ep resolver: episode move failed"
                    ),
                }
            }
        }

        // Episode-fetch dispatch — runs after the show-fetch loop so
        // each agent's `EpisodeLookup` sees every id the chain has
        // accumulated. Mode follows chain position the same way
        // show-level dispatch does.
        let ep_lookup = chimpflix_metadata::EpisodeLookup {
            episode_id: hint.episode_id,
            show: show_lookup.clone(),
            season_number: hint.season_number,
            episode_number: hint.episode_number,
            absolute_number: hint.absolute_number,
        };
        for agent_name in agents.ordered() {
            let primary = agents.position(agent_name) == Some(0);
            let mode = if primary {
                chimpflix_metadata::WriteMode::Primary
            } else {
                chimpflix_metadata::WriteMode::FillNulls
            };
            if agent_name == "anilist" && !matches!(library_kind, LibraryKind::Anime) {
                continue;
            }
            let ep_data = match agent_name {
                "tmdb" => {
                    // TMDB episodes are still season-cached at this
                    // layer — use the existing cache via the legacy
                    // helper which writes through `apply_episode_metadata`.
                    // The trait method `TmdbAgent::fetch_episode` doesn't
                    // hit the cache, so we keep the cached path here
                    // and skip the trait call to avoid double-fetching.
                    if let Some(c) = tmdb
                        && let Some(tmdb_id) = show_lookup.tmdb_id
                    {
                        tmdb_apply_episodes_for_show(pool, c, season_cache, &hint, tmdb_id).await;
                    }
                    None
                }
                "tvdb" => match tvdb {
                    Some(c) => match chimpflix_metadata::TvdbAgent::new(c.clone())
                        .fetch_episode(&ep_lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), show = %hint.show_title, "TVDB episode fetch failed");
                            None
                        }
                    },
                    None => None,
                },
                "tvmaze" => match tvmaze {
                    Some(c) => match chimpflix_metadata::TvMazeAgent::new(c.clone())
                        .fetch_episode(&ep_lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), show = %hint.show_title, "TVMaze episode fetch failed");
                            None
                        }
                    },
                    None => None,
                },
                "anilist" => match anilist {
                    Some(c) => {
                        let agent = chimpflix_metadata::AniListAgent::with_language(
                            c.clone(),
                            anilist_cache.clone(),
                            anilist_episode_cache.clone(),
                            anilist_season_id_cache.clone(),
                            agents.language.clone(),
                        );
                        match agent.fetch_episode(&ep_lookup).await {
                            Ok(d) => d,
                            Err(e) => {
                                warn!(error = %format!("{e:#}"), show = %hint.show_title, "AniList episode fetch failed");
                                None
                            }
                        }
                    }
                    None => None,
                },
                "omdb" => match omdb {
                    Some(c) => match chimpflix_metadata::OmdbAgent::new(c.clone())
                        .fetch_episode(&ep_lookup)
                        .await
                    {
                        Ok(d) => d,
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), show = %hint.show_title, "OMDb episode fetch failed");
                            None
                        }
                    },
                    None => None,
                },
                _ => None,
            };
            if let Some(data) = ep_data {
                if let Err(e) =
                    queries::apply_episode_data(pool, hint.episode_id, &data, mode, agent_name)
                        .await
                {
                    warn!(error = %format!("{e:#}"), agent = %agent_name, "apply episode data");
                }
                // Multi-source episode cast: any agent that returned a
                // non-empty `people` Vec lands rows in `episode_credits`
                // attributed to its source. Today no agent populates
                // this yet — TVDB v4 has the data via `/episodes/extended`
                // but the trait impl doesn't fetch it. The wire is in
                // place so when that lands the writes already work.
                if !data.people.is_empty()
                    && let Err(e) = queries::apply_episode_credits_for_source(
                        pool,
                        hint.episode_id,
                        &data.people,
                        agent_name,
                    )
                    .await
                {
                    warn!(error = %format!("{e:#}"), agent = %agent_name, "apply episode credits");
                }
            }
        }

        // Placeholder population. The dispatch above only enriches the
        // ONE episode this file backs; the scanner never creates rows
        // for episodes that have no file. That leaves an in-progress
        // season incomplete — the highest file-backed episode looks
        // like the finale and freshly-announced episodes are missing
        // from the calendar. Materialize a placeholder `episodes` row
        // (no `media_files`) for every episode the chain's PRIMARY
        // agent lists, so the season is complete. Runs once per show
        // per scan (the `placeholder_shows` guard), reusing the episode
        // list the per-file dispatch already fetched (TMDB `season_cache`
        // / AniList episode cache) or one `fetch_episodes` per show for
        // TVDB / TVMaze — never a per-file fetch storm.
        populate_show_placeholders(
            pool,
            tmdb,
            tvdb,
            tvmaze,
            agents,
            library_kind,
            hint.show_id,
            &show_lookup,
            season_cache,
            placeholder_shows,
        )
        .await;
    }

    // Return the file_id so the caller can emit a FileAdded event
    // for the discovery pipeline. Only Added carries it; Updated /
    // Unchanged stay None (the pipeline only fires on first sight).
    let added_file_id = match outcome {
        queries::FileOutcome::Added => Some(file_id),
        _ => None,
    };
    Ok((outcome, added_file_id))
}

// ---------------------------------------------------------------------------
// Optional TMDB enrichment (best-effort; never fails the scan)
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct MovieHint {
    title: String,
    year: Option<i32>,
    id: i64,
}

#[derive(Debug, Clone)]
struct ShowHint {
    show_title: String,
    show_year: Option<i32>,
    show_id: i64,
    season_number: i32,
    episode_number: i32,
    episode_id: i64,
    /// When the parser saw an absolute-numbered anime file (no S/E
    /// tag), this is the raw on-disk number. The dispatch loop uses
    /// it to detect/resolve absolute numbering via the TMDB season
    /// counts and may rewrite `season_number` / `episode_number` to
    /// the season-relative equivalent before the episode-fetch loop.
    absolute_number: Option<i32>,
}

// Legacy `tmdb_apply_movie` / `apply_tvdb_for_movie` removed in Slice 3.
// Movie metadata now flows through the [`MetadataAgent`] trait
// (see `chimpflix_metadata::agents::{TmdbAgent, TvdbAgent}` + the new
// `queries::apply_movie_data` polymorphic writer). TMDB-only extras
// (credits / collections) are still invoked inline from the dispatch
// loop pending Slice 6's cast/crew unification.

/// Trait-friendly wrapper around [`apply_collection_for_item`]. Takes
/// the common `TmdbCollectionRef` from `MovieData` and forwards into
/// the existing TmdbCollectionStub-based apply path.
async fn apply_collection_ref_for_item(
    pool: &SqlitePool,
    client: &TmdbClient,
    item_id: i64,
    coll: &chimpflix_metadata::TmdbCollectionRef,
) {
    let stub = chimpflix_metadata::tmdb::TmdbCollectionStub {
        tmdb_id: coll.tmdb_id,
        name: coll.name.clone(),
        poster_path: coll.poster_path.clone(),
        backdrop_path: coll.backdrop_path.clone(),
    };
    apply_collection_for_item(pool, client, item_id, &stub).await;
}

/// Upsert the collection row, assign the item to it, and (once per
/// collection) enrich the overview by fetching the full /collection/{id}
/// detail. Best-effort: any failure logs and leaves the item un-grouped.
async fn apply_collection_for_item(
    pool: &SqlitePool,
    client: &TmdbClient,
    item_id: i64,
    stub: &chimpflix_metadata::TmdbCollectionStub,
) {
    let collection_id = match queries::upsert_collection_stub(pool, stub).await {
        Ok(id) => id,
        Err(e) => {
            warn!(error = %format!("{e:#}"), "upsert collection failed");
            return;
        }
    };
    if let Err(e) = queries::assign_item_collection(pool, item_id, collection_id).await {
        warn!(error = %format!("{e:#}"), "assign collection failed");
    }
    // Only fetch the full /collection detail when overview is still NULL.
    // Cheap check; saves a TMDB call on subsequent movies in the same
    // franchise.
    let row = sqlx::query("SELECT overview IS NULL AS needs FROM collections WHERE id = ?")
        .bind(collection_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let needs_enrichment = row
        .map(|r| sqlx::Row::try_get::<i64, _>(&r, "needs").unwrap_or(0) != 0)
        .unwrap_or(false);
    if !needs_enrichment {
        return;
    }
    match client.fetch_collection(stub.tmdb_id).await {
        Ok(full) => {
            if let Err(e) = queries::enrich_collection_overview(pool, collection_id, &full).await {
                warn!(error = %format!("{e:#}"), "enrich collection failed");
            }
        }
        Err(e) => {
            warn!(error = %format!("{e:#}"), tmdb_id = stub.tmdb_id, "collection fetch failed")
        }
    }
}

/// Public re-enrichment entry-point — used by:
///   - the scanner after a fresh identification (item_id has no metadata)
///   - the admin "Refresh metadata" endpoint (re-run with current tmdb_id)
///   - the Fix Match "apply" path (re-run with a user-chosen tmdb_id)
///
/// `override_tmdb_id` lets Fix Match force a specific identity. If None we
/// use whatever tmdb_id the item already carries, falling back to a
/// title-based search.
///
/// **Chain awareness:** when `override_tmdb_id` is None we honor the
/// item's library agent chain — if TMDB isn't in it, the TMDB path is
/// skipped entirely and we only run TVDB / TVMaze fallbacks. This
/// matches the chain semantics scan-time dispatch uses, so removing
/// TMDB from a library's chain truly silences all TMDB network
/// activity for that library. Fix Match bypasses this gate because the
/// operator explicitly chose a TMDB id (the override).
#[allow(clippy::too_many_arguments)]
pub async fn refresh_item_metadata(
    pool: &SqlitePool,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    tvmaze: Option<&TvMazeClient>,
    anilist: Option<&AniListClient>,
    omdb: Option<&OmdbClient>,
    item_id: i64,
    override_tmdb_id: Option<i64>,
) -> anyhow::Result<()> {
    use crate::models::ItemKind;
    let row = sqlx::query(
        "SELECT i.kind, i.title, i.year, i.tmdb_id, i.library_id, l.kind AS library_kind
         FROM items i
         JOIN libraries l ON l.id = i.library_id
         WHERE i.id = ?",
    )
    .bind(item_id)
    .fetch_one(pool)
    .await?;
    let kind = ItemKind::from_db(sqlx::Row::try_get::<&str, _>(&row, "kind")?)?;
    let title: String = sqlx::Row::try_get(&row, "title")?;
    let year: Option<i32> = sqlx::Row::try_get(&row, "year")?;
    let existing_tmdb: Option<i64> = sqlx::Row::try_get(&row, "tmdb_id")?;
    let library_id: i64 = sqlx::Row::try_get(&row, "library_id")?;
    let library_kind_str: String = sqlx::Row::try_get(&row, "library_kind")?;
    let library_kind = crate::models::LibraryKind::from_db(&library_kind_str)?;
    let target_tmdb = override_tmdb_id.or(existing_tmdb);

    // Operator removed TMDB from this library's agent chain → skip
    // the entire TMDB path. Fix Match (override_tmdb_id set) bypasses
    // this since the operator explicitly picked a TMDB id.
    let chain_has_tmdb = override_tmdb_id.is_some()
        || queries::list_library_agents(pool, library_id)
            .await
            .map(|agents| {
                agents
                    .iter()
                    .any(|a| a.enabled && a.agent_name == "tmdb")
            })
            .unwrap_or(true);

    // For shows we ALWAYS run the chain-aware refresh (regardless of
    // TMDB membership), because that's what populates per-episode
    // titles + summaries + stills + cast from whatever agents the
    // operator configured. Without it, refreshing an existing show
    // whose original scan pre-dated trait-based episode dispatch leaves
    // every episode row at the parser stub it was upserted with.
    // Chain-aware refresh — runs the full operator-configured agent
    // chain for shows AND movies. This is the path that picks up
    // TVDB stills / OMDb summaries / AniList episode titles into
    // existing rows the original scan upserted without that data.
    if matches!(kind, ItemKind::Show) {
        if let Err(e) = refresh_show_through_chain(
            pool, tmdb, tvdb, tvmaze, anilist, omdb, item_id, &title, year, library_id,
            library_kind,
        )
        .await
        {
            warn!(error = %format!("{e:#}"), item_id, "refresh: chain-aware show refresh failed");
        }
    } else if matches!(kind, ItemKind::Movie) {
        if let Err(e) = refresh_movie_through_chain(
            pool, tmdb, tvdb, tvmaze, omdb, item_id, &title, year, library_id,
        )
        .await
        {
            warn!(error = %format!("{e:#}"), item_id, "refresh: chain-aware movie refresh failed");
        }
    }

    // TMDB legacy path — runs only when TMDB is in the chain (or Fix
    // Match supplied an override) AND a TMDB client is configured.
    // The legacy path covers collection-detail backfill and the
    // movie-specific apply_movie_metadata flow; the chain pass above
    // covers everything the chain agents can supply directly.
    if !chain_has_tmdb || tmdb.is_none() {
        if !chain_has_tmdb {
            debug!(
                item_id,
                library_id,
                "refresh: TMDB not in library chain; skipping TMDB-specific extras"
            );
        }
        return refresh_item_metadata_non_tmdb(pool, tvdb, tvmaze, item_id, kind, &title, year)
            .await;
    }
    let client = tmdb.unwrap();

    let tmdb_id = match kind {
        ItemKind::Movie => {
            let meta = match target_tmdb {
                Some(id) => client.fetch_movie(id).await?,
                None => match client.lookup_movie(&title, year).await? {
                    Some(m) => m,
                    None => return Ok(()),
                },
            };
            let tid = meta.tmdb_id;
            let collection = meta.collection.clone();
            // Refresh is an operator-driven explicit override — TMDB
            // is treated as primary here regardless of the library's
            // agent-chain ordering, because the operator clicked
            // "refresh from TMDB" and expects TMDB's text to win.
            queries::apply_movie_metadata(pool, item_id, &meta, true).await?;
            if let Some(stub) = collection {
                apply_collection_for_item(pool, client, item_id, &stub).await;
            }
            tid
        }
        ItemKind::Show => {
            let meta = match target_tmdb {
                Some(id) => client.fetch_show(id).await?,
                None => match client.lookup_show(&title, year).await? {
                    Some(m) => m,
                    None => return Ok(()),
                },
            };
            let tid = meta.tmdb_id;
            queries::apply_show_metadata(pool, item_id, &meta, true).await?;
            refresh_show_episodes(pool, client, item_id, tid).await;
            tid
        }
    };

    enrich_credits_and_extras(
        pool,
        client,
        item_id,
        tmdb_id,
        matches!(kind, ItemKind::Show),
    )
    .await;

    refresh_item_metadata_non_tmdb(pool, tvdb, tvmaze, item_id, kind, &title, year).await?;
    Ok(())
}

/// Chain-aware single-movie refresh. Same pattern as
/// [`refresh_show_through_chain`] but for movies (no per-episode
/// loop). Walks the library's enabled agents in order, fetches
/// movie data through the trait, applies via the polymorphic
/// helpers. Independent of TMDB — works for libraries whose chain
/// doesn't include TMDB.
#[allow(clippy::too_many_arguments)]
async fn refresh_movie_through_chain(
    pool: &SqlitePool,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    tvmaze: Option<&TvMazeClient>,
    omdb: Option<&OmdbClient>,
    item_id: i64,
    title: &str,
    year: Option<i32>,
    library_id: i64,
) -> anyhow::Result<()> {
    let raw_agents = match queries::list_library_agents(pool, library_id).await {
        Ok(a) => a.into_iter().filter(|a| a.enabled).collect::<Vec<_>>(),
        Err(_) => return Ok(()),
    };
    let primary = sqlx::query_scalar::<_, String>(
        "SELECT primary_metadata_agent FROM libraries WHERE id = ?",
    )
    .bind(library_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "tmdb".to_string());
    let agents: Vec<crate::models::LibraryAgent> = {
        let (mut head, tail): (Vec<_>, Vec<_>) =
            raw_agents.into_iter().partition(|a| a.agent_name == primary);
        head.extend(tail);
        head
    };
    let row = sqlx::query(
        "SELECT tmdb_id, imdb_id, tvdb_id FROM items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    let lookup = chimpflix_metadata::MovieLookup {
        item_id,
        title: title.to_string(),
        year,
        imdb_id: row
            .as_ref()
            .and_then(|r| sqlx::Row::try_get::<Option<String>, _>(r, "imdb_id").ok().flatten()),
        tmdb_id: row
            .as_ref()
            .and_then(|r| sqlx::Row::try_get::<Option<i64>, _>(r, "tmdb_id").ok().flatten()),
        tvdb_id: row
            .as_ref()
            .and_then(|r| sqlx::Row::try_get::<Option<i64>, _>(r, "tvdb_id").ok().flatten()),
    };
    for (idx, agent) in agents.iter().enumerate() {
        let mode = if idx == 0 {
            chimpflix_metadata::WriteMode::Primary
        } else {
            chimpflix_metadata::WriteMode::FillNulls
        };
        let data: Option<chimpflix_metadata::MovieData> = match agent.agent_name.as_str() {
            "tmdb" => match tmdb {
                Some(c) => chimpflix_metadata::TmdbAgent::new(c.clone())
                    .fetch_movie(&lookup)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
            "tvdb" => match tvdb {
                Some(c) => chimpflix_metadata::TvdbAgent::new(c.clone())
                    .fetch_movie(&lookup)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
            "tvmaze" => match tvmaze {
                Some(c) => chimpflix_metadata::TvMazeAgent::new(c.clone())
                    .fetch_movie(&lookup)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
            "omdb" => match omdb {
                Some(c) => chimpflix_metadata::OmdbAgent::new(c.clone())
                    .fetch_movie(&lookup)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
            _ => None,
        };
        if let Some(data) = data {
            if let Err(e) =
                queries::apply_movie_data(pool, item_id, &data, mode, &agent.agent_name).await
            {
                warn!(error = %format!("{e:#}"), "refresh: apply_movie_data");
            }
            if !data.people.is_empty()
                && let Err(e) = queries::apply_item_credits_for_source(
                    pool,
                    item_id,
                    &data.people,
                    &agent.agent_name,
                )
                .await
            {
                warn!(error = %format!("{e:#}"), "refresh: apply movie credits");
            }
            if !data.videos.is_empty()
                && let Err(e) = queries::apply_item_extras(pool, item_id, &data.videos).await
            {
                warn!(error = %format!("{e:#}"), "refresh: apply movie extras");
            }
            if !data.reviews.is_empty()
                && let Err(e) = queries::apply_item_reviews_for_source(
                    pool,
                    item_id,
                    &data.reviews,
                    &agent.agent_name,
                )
                .await
            {
                warn!(error = %format!("{e:#}"), "refresh: apply movie reviews");
            }
            if let Some(coll) = data.tmdb_collection.as_ref()
                && let Some(c) = tmdb
            {
                apply_collection_ref_for_item(pool, c, item_id, coll).await;
            }
        }
    }
    Ok(())
}

/// Run the library's full agent chain over every episode of a show.
///
/// This is the "Refresh metadata" equivalent for episode-level
/// enrichment — without it, the Refresh button only re-runs TMDB
/// (legacy code path) and leaves TVDB/AniList/OMDb episode data
/// untouched. Operators who configured TheTVDB as primary then
/// clicked Refresh expecting TVDB stills + summaries to land got
/// nothing, because the legacy episode refresh was TMDB-only.
///
/// For each agent enabled in the library's chain we:
///   1. Re-fetch the show row (no-op if the agent already populated
///      its id; this is how AniList season-aware ids get resolved
///      for split-cour anime).
///   2. Carry the agent's freshly-known id forward into the
///      `ShowLookup` so subsequent agents can look up by id.
///   3. For each episode in the local DB, run the agent's
///      `fetch_episode` and apply via the polymorphic
///      `apply_episode_data` + `apply_episode_credits_for_source`.
///
/// Mode follows chain position the same way scan-time dispatch does:
/// position 0 = `Primary` (overwrites filename-derived titles), every
/// later agent = `FillNulls`.
#[allow(clippy::too_many_arguments)]
async fn refresh_show_through_chain(
    pool: &SqlitePool,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    tvmaze: Option<&TvMazeClient>,
    anilist: Option<&AniListClient>,
    omdb: Option<&OmdbClient>,
    show_id: i64,
    show_title: &str,
    show_year: Option<i32>,
    library_id: i64,
    library_kind: crate::models::LibraryKind,
) -> anyhow::Result<()> {
    let raw_agents = match queries::list_library_agents(pool, library_id).await {
        Ok(a) => a.into_iter().filter(|a| a.enabled).collect::<Vec<_>>(),
        Err(e) => {
            warn!(error = %format!("{e:#}"), show_id, "refresh: list library agents failed");
            return Ok(());
        }
    };
    // Read per-library primary and reorder the agent list so it runs
    // first. Mirrors what AgentChain::load does for the scan path.
    let primary = sqlx::query_scalar::<_, String>(
        "SELECT primary_metadata_agent FROM libraries WHERE id = ?",
    )
    .bind(library_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .unwrap_or_else(|| "tmdb".to_string());
    let agents: Vec<crate::models::LibraryAgent> = {
        let (mut head, tail): (Vec<_>, Vec<_>) =
            raw_agents.into_iter().partition(|a| a.agent_name == primary);
        head.extend(tail);
        head
    };
    let chain_names: Vec<&str> = agents.iter().map(|a| a.agent_name.as_str()).collect();
    let language = metadata_language_or_default(pool).await;
    info!(
        show_id,
        title = %show_title,
        chain = ?chain_names,
        language = %language,
        "refresh: chain start"
    );

    // Local per-refresh AniList caches. The pool is the same one scan
    // uses but we don't share its caches — refresh is operator-driven
    // and infrequent, so cold caches per invocation are fine and we
    // avoid coupling the two paths.
    let anilist_cache = chimpflix_metadata::anilist_cache::new_show_cache();
    let anilist_ep_cache = chimpflix_metadata::anilist_cache::new_episode_list_cache();
    let anilist_season_id_cache = chimpflix_metadata::anilist_cache::new_season_id_cache();
    let mut show_hits: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut ep_hits: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // Read every persisted external id off the show row so each
    // agent can do id-based lookups when their previous scan left
    // an id behind.
    let row = sqlx::query(
        "SELECT tmdb_id, imdb_id, tvdb_id, anilist_id, tvmaze_id FROM items WHERE id = ?",
    )
    .bind(show_id)
    .fetch_optional(pool)
    .await?;
    let mut show_lookup = chimpflix_metadata::ShowLookup {
        item_id: show_id,
        title: show_title.to_string(),
        year: show_year,
        imdb_id: row
            .as_ref()
            .and_then(|r| sqlx::Row::try_get::<Option<String>, _>(r, "imdb_id").ok().flatten()),
        tmdb_id: row
            .as_ref()
            .and_then(|r| sqlx::Row::try_get::<Option<i64>, _>(r, "tmdb_id").ok().flatten()),
        tvdb_id: row
            .as_ref()
            .and_then(|r| sqlx::Row::try_get::<Option<i64>, _>(r, "tvdb_id").ok().flatten()),
        anilist_id: row
            .as_ref()
            .and_then(|r| sqlx::Row::try_get::<Option<i64>, _>(r, "anilist_id").ok().flatten()),
        tvmaze_id: row
            .as_ref()
            .and_then(|r| sqlx::Row::try_get::<Option<i64>, _>(r, "tvmaze_id").ok().flatten()),
    };

    // Pass 1: show-level fetch per agent (carries forward IDs).
    for (idx, agent) in agents.iter().enumerate() {
        let primary = idx == 0;
        let mode = if primary {
            chimpflix_metadata::WriteMode::Primary
        } else {
            chimpflix_metadata::WriteMode::FillNulls
        };
        if agent.agent_name == "anilist"
            && !matches!(library_kind, crate::models::LibraryKind::Anime)
        {
            continue;
        }
        let show_data: Option<chimpflix_metadata::ShowData> = match agent.agent_name.as_str() {
            "tmdb" => match tmdb {
                Some(c) => chimpflix_metadata::TmdbAgent::new(c.clone())
                    .fetch_show(&show_lookup)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
            "tvdb" => match tvdb {
                Some(c) => chimpflix_metadata::TvdbAgent::new(c.clone())
                    .fetch_show(&show_lookup)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
            "tvmaze" => match tvmaze {
                Some(c) => chimpflix_metadata::TvMazeAgent::new(c.clone())
                    .fetch_show(&show_lookup)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
            "anilist" => match anilist {
                Some(c) => chimpflix_metadata::AniListAgent::with_language(
                    c.clone(),
                    anilist_cache.clone(),
                    anilist_ep_cache.clone(),
                    anilist_season_id_cache.clone(),
                    language.clone(),
                )
                .fetch_show(&show_lookup)
                .await
                .ok()
                .flatten(),
                None => None,
            },
            "omdb" => match omdb {
                Some(c) => chimpflix_metadata::OmdbAgent::new(c.clone())
                    .fetch_show(&show_lookup)
                    .await
                    .ok()
                    .flatten(),
                None => None,
            },
            _ => None,
        };
        show_hits.insert(agent.agent_name.clone(), show_data.is_some());
        if let Some(data) = show_data {
            if let Err(e) =
                queries::apply_show_data(pool, show_id, &data, mode, &agent.agent_name).await
            {
                warn!(error = %format!("{e:#}"), agent = %agent.agent_name, "refresh: apply_show_data");
            }
            if data.tmdb_id.is_some() {
                show_lookup.tmdb_id = data.tmdb_id;
            }
            if data.tvdb_id.is_some() {
                show_lookup.tvdb_id = data.tvdb_id;
            }
            if data.anilist_id.is_some() {
                show_lookup.anilist_id = data.anilist_id;
            }
            if data.tvmaze_id.is_some() {
                show_lookup.tvmaze_id = data.tvmaze_id;
            }
            if data.imdb_id.is_some() {
                show_lookup.imdb_id = data.imdb_id.clone();
            }
            if !data.people.is_empty()
                && let Err(e) = queries::apply_item_credits_for_source(
                    pool,
                    show_id,
                    &data.people,
                    &agent.agent_name,
                )
                .await
            {
                warn!(error = %format!("{e:#}"), "refresh: apply show credits");
            }
            if !data.videos.is_empty()
                && let Err(e) = queries::apply_item_extras(pool, show_id, &data.videos).await
            {
                warn!(error = %format!("{e:#}"), "refresh: apply show extras");
            }
            if !data.reviews.is_empty()
                && let Err(e) = queries::apply_item_reviews_for_source(
                    pool,
                    show_id,
                    &data.reviews,
                    &agent.agent_name,
                )
                .await
            {
                warn!(error = %format!("{e:#}"), "refresh: apply show reviews");
            }
        }
    }

    // Pass 2: per-episode fetch. Walk every persisted episode of
    // this show and ask each chain agent for its data. Cheap when
    // the agent already has nothing new (single SELECT + a cached
    // episode-list HTTP per show), comparable to a cold scan's
    // episode-level work but limited to one show.
    let episodes = sqlx::query(
        "SELECT e.id AS episode_id, s.season_number, e.episode_number, e.absolute_number
         FROM episodes e
         JOIN seasons s ON e.season_id = s.id
         WHERE s.show_id = ?
         ORDER BY s.season_number, e.episode_number",
    )
    .bind(show_id)
    .fetch_all(pool)
    .await?;

    for ep_row in episodes {
        let episode_id: i64 = sqlx::Row::try_get(&ep_row, "episode_id")?;
        let season_number: i32 = sqlx::Row::try_get(&ep_row, "season_number")?;
        let episode_number: i32 = sqlx::Row::try_get(&ep_row, "episode_number")?;
        let absolute_number: Option<i32> =
            sqlx::Row::try_get(&ep_row, "absolute_number").ok().flatten();
        let ep_lookup = chimpflix_metadata::EpisodeLookup {
            episode_id,
            show: show_lookup.clone(),
            season_number,
            episode_number,
            absolute_number,
        };
        for (idx, agent) in agents.iter().enumerate() {
            let primary = idx == 0;
            let mode = if primary {
                chimpflix_metadata::WriteMode::Primary
            } else {
                chimpflix_metadata::WriteMode::FillNulls
            };
            if agent.agent_name == "anilist"
                && !matches!(library_kind, crate::models::LibraryKind::Anime)
            {
                continue;
            }
            let ep_data: Option<chimpflix_metadata::EpisodeData> = match agent.agent_name.as_str()
            {
                "tmdb" => match tmdb {
                    Some(c) => chimpflix_metadata::TmdbAgent::new(c.clone())
                        .fetch_episode(&ep_lookup)
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                },
                "tvdb" => match tvdb {
                    Some(c) => chimpflix_metadata::TvdbAgent::new(c.clone())
                        .fetch_episode(&ep_lookup)
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                },
                "tvmaze" => match tvmaze {
                    Some(c) => chimpflix_metadata::TvMazeAgent::new(c.clone())
                        .fetch_episode(&ep_lookup)
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                },
                "anilist" => match anilist {
                    Some(c) => chimpflix_metadata::AniListAgent::with_language(
                        c.clone(),
                        anilist_cache.clone(),
                        anilist_ep_cache.clone(),
                        anilist_season_id_cache.clone(),
                        language.clone(),
                    )
                    .fetch_episode(&ep_lookup)
                    .await
                    .ok()
                    .flatten(),
                    None => None,
                },
                "omdb" => match omdb {
                    Some(c) => chimpflix_metadata::OmdbAgent::new(c.clone())
                        .fetch_episode(&ep_lookup)
                        .await
                        .ok()
                        .flatten(),
                    None => None,
                },
                _ => None,
            };
            if let Some(data) = ep_data {
                *ep_hits.entry(agent.agent_name.clone()).or_insert(0) += 1;
                if let Err(e) =
                    queries::apply_episode_data(pool, episode_id, &data, mode, &agent.agent_name)
                        .await
                {
                    warn!(error = %format!("{e:#}"), agent = %agent.agent_name, "refresh: apply_episode_data");
                }
                if !data.people.is_empty()
                    && let Err(e) = queries::apply_episode_credits_for_source(
                        pool,
                        episode_id,
                        &data.people,
                        &agent.agent_name,
                    )
                    .await
                {
                    warn!(error = %format!("{e:#}"), agent = %agent.agent_name, "refresh: apply episode credits");
                }
            }
        }
    }
    let show_hits_str = chain_names
        .iter()
        .map(|n| {
            format!(
                "{n}={}",
                if *show_hits.get(*n).unwrap_or(&false) {
                    "hit"
                } else {
                    "miss"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let ep_hits_str = chain_names
        .iter()
        .map(|n| format!("{n}={}", ep_hits.get(*n).copied().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(",");
    info!(
        show_id,
        title = %show_title,
        show = %show_hits_str,
        episodes = %ep_hits_str,
        "refresh: chain done"
    );

    // Complete the season(s) with placeholder rows for every episode the
    // primary agent knows about — the same step the scan path runs, so
    // the Refresh button also picks up newly-announced episodes for
    // ongoing shows (and fills in any season that only ever had
    // file-backed rows). `show_lookup` now carries every id Pass 1
    // resolved. Build the same kind of `AgentChain` the scan uses so the
    // primary-agent selection matches. Fresh per-refresh caches: refresh
    // is operator-driven and infrequent, so a cold `season_cache` is
    // fine, and the single-element `placeholder_shows` just satisfies the
    // helper's once-per-show contract.
    let chain = AgentChain {
        order: agents.iter().map(|a| a.agent_name.clone()).collect(),
        language,
    };
    let season_cache: Arc<SeasonCache> = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let placeholder_shows: Arc<PlaceholderShows> =
        Arc::new(Mutex::new(std::collections::HashSet::new()));
    populate_show_placeholders(
        pool,
        tmdb,
        tvdb,
        tvmaze,
        &chain,
        library_kind,
        show_id,
        &show_lookup,
        &season_cache,
        &placeholder_shows,
    )
    .await;

    Ok(())
}

/// Run the TVDB + TVMaze fallback portion of refresh. Extracted so the
/// chain-gated `refresh_item_metadata` can call it both as the
/// post-TMDB fill-nulls pass AND as the TMDB-skipped early-return
/// path (when TMDB isn't in the library's chain).
async fn refresh_item_metadata_non_tmdb(
    pool: &SqlitePool,
    tvdb: Option<&TvdbClient>,
    tvmaze: Option<&TvMazeClient>,
    item_id: i64,
    kind: crate::models::ItemKind,
    title: &str,
    year: Option<i32>,
) -> anyhow::Result<()> {
    use crate::models::ItemKind;
    match kind {
        ItemKind::Show => {
            if let Some(tv) = tvmaze
                && let Ok(Some(meta)) = tv.lookup_show(title).await
            {
                let _ = queries::apply_show_metadata_tvmaze(pool, item_id, &meta).await;
            }
            if let Some(tv) = tvdb
                && let Ok(Some(meta)) = tv.lookup_show(title, year).await
            {
                let _ = queries::apply_show_metadata_tvdb(pool, item_id, &meta).await;
            }
        }
        ItemKind::Movie => {
            if let Some(tv) = tvdb
                && let Ok(Some(meta)) = tv.lookup_movie(title, year).await
            {
                let _ = queries::apply_movie_metadata_tvdb(pool, item_id, &meta).await;
            }
        }
    }
    Ok(())
}

/// Backfill cast+crew and YouTube extras for items the
/// `refresh_item_metadata` path just identified. Best-effort: any
/// failure logs and moves on so the refresh still completes.
///
/// Note: scan-time dispatch goes through the trait — `TmdbAgent` and
/// other agents populate `MovieData.people` / `videos` / `reviews`
/// directly during `fetch_show` / `fetch_movie`, and the apply helpers
/// run from the scanner's main loop. This helper exists so the
/// admin-driven Refresh button (which doesn't go through scan
/// dispatch) still gets the same cast/videos/reviews behavior.
async fn enrich_credits_and_extras(
    pool: &SqlitePool,
    client: &TmdbClient,
    item_id: i64,
    tmdb_id: i64,
    is_show: bool,
) {
    let kind = if is_show {
        chimpflix_metadata::TmdbKind::Show
    } else {
        chimpflix_metadata::TmdbKind::Movie
    };
    // Run credits / videos / reviews in parallel — three independent
    // endpoints. Each translates into the common shape, then writes
    // via the source-scoped apply helpers.
    let (credits, videos, reviews) = tokio::join!(
        client.fetch_credits(kind, tmdb_id),
        client.fetch_videos(kind, tmdb_id),
        client.fetch_reviews(kind, tmdb_id),
    );
    if let Ok(c) = credits {
        let people = tmdb_credits_to_common(c);
        if let Err(e) =
            queries::apply_item_credits_for_source(pool, item_id, &people, "tmdb").await
        {
            warn!(error = %format!("{e:#}"), "apply credits");
        }
    } else if let Err(e) = credits {
        warn!(error = %format!("{e:#}"), tmdb_id, "TMDB credits fetch failed");
    }
    if let Ok(vs) = videos {
        let links: Vec<chimpflix_metadata::VideoLink> =
            vs.into_iter().map(tmdb_video_to_common).collect();
        if let Err(e) = queries::apply_item_extras(pool, item_id, &links).await {
            warn!(error = %format!("{e:#}"), "apply extras");
        }
    } else if let Err(e) = videos {
        warn!(error = %format!("{e:#}"), tmdb_id, "TMDB videos fetch failed");
    }
    if let Ok(rs) = reviews {
        let entries: Vec<chimpflix_metadata::ReviewEntry> =
            rs.into_iter().map(tmdb_review_to_common).collect();
        if let Err(e) =
            queries::apply_item_reviews_for_source(pool, item_id, &entries, "tmdb").await
        {
            warn!(error = %format!("{e:#}"), "apply reviews");
        }
    } else if let Err(e) = reviews {
        warn!(error = %format!("{e:#}"), tmdb_id, "TMDB reviews fetch failed");
    }
}

/// Local translators mirror the ones in `chimpflix_metadata::agents`.
/// Duplicated here so the refresh path (which calls TmdbClient directly,
/// not through TmdbAgent) doesn't need to go through the trait just for
/// the common-shape conversion.
fn tmdb_credits_to_common(
    credits: chimpflix_metadata::TmdbCredits,
) -> Vec<chimpflix_metadata::PersonCredit> {
    let mut out = Vec::new();
    for (idx, m) in credits.cast.into_iter().enumerate() {
        out.push(chimpflix_metadata::PersonCredit {
            external_id: Some(format!("tmdb:{}", m.tmdb_person_id)),
            name: m.name,
            role: "actor".to_string(),
            character: m.character,
            order: if m.order != 0 { m.order } else { idx as i32 },
            profile_url: m
                .profile_path
                .map(|p| chimpflix_metadata::tmdb::tmdb_image_url(&p, "w185")),
        });
    }
    for (idx, m) in credits.crew.into_iter().enumerate() {
        let role = match m.job.as_str() {
            "Director" => "director",
            "Writer" | "Screenplay" => "writer",
            "Producer" | "Executive Producer" => "producer",
            _ => "crew",
        }
        .to_string();
        out.push(chimpflix_metadata::PersonCredit {
            external_id: Some(format!("tmdb:{}", m.tmdb_person_id)),
            name: m.name,
            role,
            character: None,
            order: idx as i32,
            profile_url: m
                .profile_path
                .map(|p| chimpflix_metadata::tmdb::tmdb_image_url(&p, "w185")),
        });
    }
    out
}

fn tmdb_video_to_common(v: chimpflix_metadata::TmdbVideo) -> chimpflix_metadata::VideoLink {
    let kind = match v.kind.as_str() {
        "Trailer" => "trailer",
        "Teaser" => "teaser",
        "Featurette" => "featurette",
        "Clip" => "clip",
        "Behind the Scenes" => "behind-the-scenes",
        _ => "other",
    }
    .to_string();
    chimpflix_metadata::VideoLink {
        provider_key: v.key,
        name: v.name,
        kind,
        official: v.official,
        published_at_ms: None, // refresh path doesn't bother parsing; scan-time path does
    }
}

fn tmdb_review_to_common(r: chimpflix_metadata::TmdbReview) -> chimpflix_metadata::ReviewEntry {
    chimpflix_metadata::ReviewEntry {
        source_id: r.source_id,
        author: r.author,
        author_url: r.author_url,
        avatar_url: r.avatar_url,
        rating: r.rating,
        body: r.body,
        created_at_ms: r.created_at,
    }
}

/// Walk every local season+episode of `show_id` and overwrite the
/// episode rows with TMDB's metadata. Used by the `/items/{id}/refresh`
/// path — scan-time enrichment already runs per-file, but the
/// Refresh button doesn't go through that path and would otherwise
/// leave episode titles stuck at whatever the parser pulled out of
/// the filename. Best-effort: any per-season TMDB failure is logged
/// and we move on so a single bad fetch doesn't abort the whole
/// refresh.
async fn refresh_show_episodes(
    pool: &SqlitePool,
    client: &TmdbClient,
    show_id: i64,
    show_tmdb_id: i64,
) {
    let seasons: Vec<i32> = match sqlx::query(
        "SELECT DISTINCT season_number FROM seasons WHERE show_id = ? ORDER BY season_number",
    )
    .bind(show_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows
            .iter()
            .filter_map(|r| sqlx::Row::try_get::<i32, _>(r, "season_number").ok())
            .collect(),
        Err(e) => {
            warn!(error = %format!("{e:#}"), show_id, "refresh: list local seasons failed");
            return;
        }
    };

    for season_number in seasons {
        let season_meta = match client.fetch_season(show_tmdb_id, season_number).await {
            Ok(s) => s,
            Err(e) => {
                // 404 here is "TMDB doesn't have this season" — extremely
                // common for split-cour anime (Frieren / JJK / Demon
                // Slayer S2+) where the user's library has Season 2 but
                // TMDB tracks it all under Season 1. Demote to debug so
                // refresh-on-a-multi-season-anime-show doesn't flood the
                // Logs page with a row per missing season. Other errors
                // (5xx, network) still warn.
                let msg = format!("{e:#}");
                if msg.contains("404") {
                    debug!(
                        show_id,
                        season_number,
                        "refresh: TMDB has no record of this season; episode titles left as-is"
                    );
                } else {
                    warn!(
                        error = %msg,
                        show_id,
                        season_number,
                        "refresh: TMDB season fetch failed; episode titles left as-is"
                    );
                }
                continue;
            }
        };
        // Fetch all local episode ids for this season in one query so
        // we can loop in-process rather than per-episode SELECTs.
        let local_eps = match sqlx::query(
            "SELECT e.id, e.episode_number FROM episodes e
             JOIN seasons s ON e.season_id = s.id
             WHERE s.show_id = ? AND s.season_number = ?",
        )
        .bind(show_id)
        .bind(season_number)
        .fetch_all(pool)
        .await
        {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|r| {
                    let id: i64 = sqlx::Row::try_get(&r, "id").ok()?;
                    let n: i32 = sqlx::Row::try_get(&r, "episode_number").ok()?;
                    Some((id, n))
                })
                .collect::<Vec<_>>(),
            Err(e) => {
                warn!(error = %format!("{e:#}"), show_id, season_number, "refresh: list local episodes failed");
                continue;
            }
        };

        for (ep_id, ep_num) in local_eps {
            let Some(ep_meta) = season_meta
                .episodes
                .iter()
                .find(|e| e.episode_number == ep_num)
            else {
                continue;
            };
            if let Err(e) = queries::apply_episode_metadata(pool, ep_id, ep_meta).await {
                warn!(
                    error = %format!("{e:#}"),
                    show_id,
                    season_number,
                    episode_number = ep_num,
                    "refresh: apply episode metadata"
                );
            }
        }
    }
}

/// Walk TMDB's per-season episode counts to convert an absolute
/// episode number into a season-relative `(season, episode)` pair.
///
/// Anime libraries frequently use absolute numbering ("Show - 29.mkv")
/// that the parser stores as `season=1, episode=29, absolute_number=29`.
/// When TMDB's S1 has only 12 episodes, that lookup 404s and the
/// episode row never gets metadata. This helper does the bookkeeping:
/// walks seasons 1..N, subtracting each season's `episodes.len()` from
/// the absolute counter until what's left fits inside one season.
///
/// Returns `Some((season, episode))` when resolution succeeds, `None`
/// when the absolute number exceeds the total episode count TMDB has
/// listed (e.g. a brand-new episode TMDB hasn't catalogued yet, or
/// the show is genuinely numbered season-relative and the parser
/// mis-identified it as absolute).
async fn tmdb_resolve_absolute_episode(
    season_cache: &SeasonCache,
    client: &TmdbClient,
    show_tmdb_id: i64,
    absolute_number: i32,
    max_seasons_to_walk: i32,
) -> Option<(i32, i32)> {
    // Cap the walk so a misconfigured show with a wildly wrong
    // absolute number doesn't fan out into a 100-season TMDB hammer.
    let cap = max_seasons_to_walk.clamp(1, 50);
    let mut remaining = absolute_number;
    for season_n in 1..=cap {
        let season = match fetch_season_cached(season_cache, client, show_tmdb_id, season_n).await {
            Ok(Some(s)) => s,
            // Confirmed-missing season — we've exhausted what TMDB
            // has for this show.
            Ok(None) => return None,
            Err(_) => return None,
        };
        let count = season.episodes.len() as i32;
        if count <= 0 {
            return None;
        }
        if remaining <= count {
            return Some((season_n, remaining));
        }
        remaining -= count;
    }
    None
}

/// Episode-only TMDB enrichment. Show-level metadata is written via
/// the trait dispatch (`TmdbAgent::fetch_show` + `apply_show_data`);
/// this helper picks up at the per-episode level so the legacy episode
/// path keeps working until Slice 5 lifts it into the trait too.
///
/// Called from the dispatch loop only when TMDB returned a show match
/// in this scan; the `tmdb_id` argument is taken directly from
/// `ShowData.tmdb_id` so we avoid an extra `SELECT tmdb_id` round-trip.
async fn tmdb_apply_episodes_for_show(
    pool: &SqlitePool,
    client: &TmdbClient,
    season_cache: &SeasonCache,
    hint: &ShowHint,
    show_tmdb_id: i64,
) {
    match fetch_season_cached(season_cache, client, show_tmdb_id, hint.season_number).await {
        Ok(Some(season)) => {
            if let Some(ep_meta) = season
                .episodes
                .iter()
                .find(|e| e.episode_number == hint.episode_number)
            {
                if let Err(e) =
                    queries::apply_episode_metadata(pool, hint.episode_id, ep_meta).await
                {
                    warn!(error = %format!("{e:#}"), "apply episode metadata");
                }
            }
        }
        Ok(None) => {
            // Confirmed-missing season — cached for the rest of the scan.
        }
        Err(e) => warn!(
            error = %format!("{e:#}"),
            show = %hint.show_title,
            season = hint.season_number,
            "TMDB season fetch failed"
        ),
    }
}

/// Materialize placeholder `episodes` rows for every episode the
/// chain's PRIMARY agent lists for `show_id`, so an in-progress or
/// future season is COMPLETE in the DB even before the files arrive.
///
/// A placeholder is an `episodes` row with NO `media_files` row. It is
/// informational only — excluded by construction from every
/// "content you have" surface (those JOIN/EXISTS on `media_files`) — but
/// it makes `MAX(episode_number)` reflect the true season length (so the
/// finale flag is correct) and gives the local calendar its air dates.
///
/// Cost discipline (no per-file fetch storm):
///   * Runs at most ONCE per show per scan, gated on `placeholder_shows`.
///   * Reuses the same cached episode list the per-file dispatch already
///     pulled: TMDB via `season_cache` (`fetch_season_cached`), so no new
///     network call; TVDB / TVMaze do one `fetch_episodes` per show.
///   * Only the PRIMARY agent is consulted — we don't fan out across the
///     whole chain. The primary owns episode numbering for the library
///     kind (anime → tvdb, shows → tvdb/tvmaze/tmdb, etc.).
///
/// Idempotent: [`queries::upsert_episode_placeholder`] keys on
/// `(season_id, episode_number)` — the same key the file-backed
/// [`queries::upsert_episode`] uses — so a placeholder reconciles in
/// place when a file later arrives (no dupes, no clobbering).
#[allow(clippy::too_many_arguments)]
async fn populate_show_placeholders(
    pool: &SqlitePool,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    tvmaze: Option<&TvMazeClient>,
    agents: &AgentChain,
    library_kind: LibraryKind,
    show_id: i64,
    show_lookup: &chimpflix_metadata::ShowLookup,
    season_cache: &SeasonCache,
    placeholder_shows: &PlaceholderShows,
) {
    // Once per show per scan. Insert-then-check so the first worker to
    // reach a given show wins and every later file of that show bails.
    {
        let mut guard = placeholder_shows.lock().await;
        if !guard.insert(show_id) {
            return;
        }
    }

    // External ids to address the show by. Prefer the ids the in-flight
    // chain just resolved (`show_lookup`); fall back to whatever a prior
    // scan persisted on the `items` row. The fallback matters on re-scans
    // where the live title search hiccups but the id is already known —
    // we still want to complete the season rather than skip the show.
    let (mut tvdb_id, mut tmdb_id, mut tvmaze_id) =
        (show_lookup.tvdb_id, show_lookup.tmdb_id, show_lookup.tvmaze_id);
    if tvdb_id.is_none() || tmdb_id.is_none() || tvmaze_id.is_none() {
        if let Ok(Some(row)) =
            sqlx::query("SELECT tmdb_id, tvdb_id, tvmaze_id FROM items WHERE id = ?")
                .bind(show_id)
                .fetch_optional(pool)
                .await
        {
            tvdb_id = tvdb_id.or_else(|| {
                sqlx::Row::try_get::<Option<i64>, _>(&row, "tvdb_id")
                    .ok()
                    .flatten()
            });
            tmdb_id = tmdb_id.or_else(|| {
                sqlx::Row::try_get::<Option<i64>, _>(&row, "tmdb_id")
                    .ok()
                    .flatten()
            });
            tvmaze_id = tvmaze_id.or_else(|| {
                sqlx::Row::try_get::<Option<i64>, _>(&row, "tvmaze_id")
                    .ok()
                    .flatten()
            });
        }
    }

    // Walk the chain in priority order — `ordered()` puts the PRIMARY
    // first, so the primary agent is consulted first and wins if it can
    // supply a season-aware episode list. We fall through to the next
    // capable agent only when an earlier one can't (e.g. the primary is
    // `anilist`, which never drives placeholders — its `streamingEpisodes`
    // listing has no per-season numbering or air dates — so an anime
    // library still gets placeholders from `tvdb` further down the
    // chain). AniList is also skipped on non-anime libraries to match
    // the dispatch loops.
    for agent_name in agents.ordered() {
        if agent_name == "anilist" && !matches!(library_kind, LibraryKind::Anime) {
            continue;
        }
        let upserted = match agent_name {
            "tvdb" => {
                let (Some(client), Some(id)) = (tvdb, tvdb_id) else {
                    continue;
                };
                placeholders_from_tvdb(pool, client, show_id, id).await
            }
            "tvmaze" => {
                let (Some(client), Some(id)) = (tvmaze, tvmaze_id) else {
                    continue;
                };
                placeholders_from_tvmaze(pool, client, show_id, id).await
            }
            "tmdb" => {
                let (Some(client), Some(id)) = (tmdb, tmdb_id) else {
                    continue;
                };
                placeholders_from_tmdb(pool, client, season_cache, show_id, id).await
            }
            // anilist / omdb don't expose a season-aware episode list with
            // air dates suitable for placeholders.
            _ => continue,
        };
        match upserted {
            // First agent that produced rows wins; stop walking the chain.
            Ok(n) if n > 0 => {
                debug!(show_id, agent = agent_name, count = n, "populated episode placeholders");
                return;
            }
            Ok(_) => {
                // Agent reachable but listed nothing new — still authoritative
                // for placeholders; don't double-fetch from a lower-priority
                // source.
                return;
            }
            Err(e) => {
                // Transient failure (network / rate limit / circuit breaker).
                // Try the next capable agent in the chain. The show stays
                // flagged for the REST OF THIS SCAN (so its other files
                // don't re-storm the failing endpoint); the NEXT scan
                // re-checks from a fresh `placeholder_shows` set and retries.
                warn!(
                    error = %format!("{e:#}"),
                    show_id,
                    agent = agent_name,
                    "placeholder population failed; trying next agent"
                );
            }
        }
    }
}

/// TVDB placeholder source. One `fetch_episodes` returns every episode
/// across every season (with season + episode numbers, air dates,
/// absolute numbers, and the TVDB episode id), so a single call covers
/// the whole show. `tvdb_id` here is the EPISODE's tvdb id, kept distinct
/// from `tmdb_id` (TVDB doesn't surface TMDB ids).
async fn placeholders_from_tvdb(
    pool: &SqlitePool,
    client: &TvdbClient,
    show_id: i64,
    tvdb_id: i64,
) -> Result<usize> {
    let episodes = client.fetch_episodes(tvdb_id).await?;
    let mut count = 0usize;
    for ep in &episodes {
        // Skip TVDB "season 0" specials and any episode TVDB couldn't
        // number — they don't belong on the main season timeline.
        if ep.season_number <= 0 || ep.episode_number <= 0 {
            continue;
        }
        let season_id = queries::upsert_season(pool, show_id, ep.season_number).await?;
        queries::upsert_episode_placeholder(
            pool,
            season_id,
            &queries::PlaceholderEpisode {
                episode_number: ep.episode_number,
                title: Some(ep.title.clone()).filter(|s| !s.trim().is_empty()),
                summary: ep.summary.clone(),
                air_date: ep.air_date.clone(),
                tmdb_id: None,
                tvdb_id: Some(ep.tvdb_id),
                absolute_number: ep.absolute_number,
            },
        )
        .await?;
        count += 1;
    }
    Ok(count)
}

/// TVMaze placeholder source. Mirrors TVDB — one `fetch_episodes` lists
/// every season. TVMaze carries no TMDB/TVDB episode ids.
async fn placeholders_from_tvmaze(
    pool: &SqlitePool,
    client: &TvMazeClient,
    show_id: i64,
    tvmaze_id: i64,
) -> Result<usize> {
    let episodes = client.fetch_episodes(tvmaze_id).await?;
    let mut count = 0usize;
    for ep in &episodes {
        if ep.season_number <= 0 || ep.episode_number <= 0 {
            continue;
        }
        let season_id = queries::upsert_season(pool, show_id, ep.season_number).await?;
        queries::upsert_episode_placeholder(
            pool,
            season_id,
            &queries::PlaceholderEpisode {
                episode_number: ep.episode_number,
                title: Some(ep.title.clone()).filter(|s| !s.trim().is_empty()),
                summary: ep.summary.clone(),
                air_date: ep.air_date.clone(),
                tmdb_id: None,
                tvdb_id: None,
                absolute_number: None,
            },
        )
        .await?;
        count += 1;
    }
    Ok(count)
}

/// TMDB placeholder source. TMDB groups episodes per season behind
/// `/tv/{id}/season/{n}`, so we walk every season already present locally
/// for this show (the per-file dispatch will have created at least the
/// season the current file belongs to) and reuse the per-scan
/// `season_cache` — a season the dispatch already fetched costs zero
/// extra network calls here.
async fn placeholders_from_tmdb(
    pool: &SqlitePool,
    client: &TmdbClient,
    season_cache: &SeasonCache,
    show_id: i64,
    show_tmdb_id: i64,
) -> Result<usize> {
    let local_seasons: Vec<i32> = sqlx::query_scalar::<_, i32>(
        "SELECT season_number FROM seasons WHERE show_id = ? AND season_number > 0 \
         ORDER BY season_number",
    )
    .bind(show_id)
    .fetch_all(pool)
    .await?;
    let mut count = 0usize;
    for season_number in local_seasons {
        let season = match fetch_season_cached(season_cache, client, show_tmdb_id, season_number)
            .await
        {
            Ok(Some(s)) => s,
            // Confirmed-missing season (cached 404) — common for
            // split-cour anime; nothing to materialize.
            Ok(None) => continue,
            Err(e) => return Err(e),
        };
        let season_id = queries::upsert_season(pool, show_id, season_number).await?;
        for ep in &season.episodes {
            if ep.episode_number <= 0 {
                continue;
            }
            queries::upsert_episode_placeholder(
                pool,
                season_id,
                &queries::PlaceholderEpisode {
                    episode_number: ep.episode_number,
                    title: Some(ep.title.clone()).filter(|s| !s.trim().is_empty()),
                    summary: ep.summary.clone(),
                    air_date: ep.air_date.clone(),
                    tmdb_id: Some(ep.tmdb_id),
                    tvdb_id: None,
                    absolute_number: None,
                },
            )
            .await?;
            count += 1;
        }
    }
    Ok(count)
}

/// Memoised wrapper around `TmdbClient::fetch_season`. Same season
/// requested twice within a scan only hits the network once. Returns
/// the cached `Arc<TmdbSeason>` so callers don't have to deep-clone.
///
/// On error, propagates the network error to the caller without
/// caching — a transient TMDB failure on episode 1 shouldn't poison
/// episode 2 for the rest of the scan.
async fn fetch_season_cached(
    cache: &SeasonCache,
    client: &TmdbClient,
    show_tmdb_id: i64,
    season_number: i32,
) -> anyhow::Result<Option<Arc<TmdbSeason>>> {
    let key = (show_tmdb_id, season_number);
    {
        let guard = cache.lock().await;
        if let Some(hit) = guard.get(&key) {
            return Ok(match hit {
                CachedSeason::Found(season) => Some(season.clone()),
                CachedSeason::Missing => None,
            });
        }
    }
    match client.fetch_season(show_tmdb_id, season_number).await {
        Ok(season) => {
            let arc = Arc::new(season);
            let mut guard = cache.lock().await;
            // Two parallel workers can race to populate the same
            // key. Either wins — they'd have produced equivalent
            // results — so we keep what's already there.
            let entry = guard
                .entry(key)
                .or_insert_with(|| CachedSeason::Found(arc.clone()));
            Ok(match entry {
                CachedSeason::Found(season) => Some(season.clone()),
                CachedSeason::Missing => None,
            })
        }
        Err(e) => {
            // 404 is "this season doesn't exist on TMDB" — cache
            // the negative result so the rest of the scan doesn't
            // re-trigger the same lookup for every episode of the
            // show. Anything else (5xx, network, parse failure) is
            // transient; propagate without caching so a future
            // episode might succeed.
            let msg = format!("{e:#}");
            if msg.contains("404") {
                let mut guard = cache.lock().await;
                guard.entry(key).or_insert(CachedSeason::Missing);
                // Log the first 404 once so the operator sees it
                // happened, but as debug — the warn that fires from
                // the call site (`tmdb_apply_show`) handles the
                // user-visible surface.
                tracing::debug!(
                    show_tmdb_id,
                    season_number,
                    "TMDB returned 404 for season; cached negative result for this scan"
                );
                return Ok(None);
            }
            Err(e)
        }
    }
}


// `apply_tvdb_for_show` removed in Slice 3 — TVDB show lookups now go
// through `TvdbAgent::fetch_show` + `queries::apply_show_data` via the
// trait dispatch loop above.

// ordinal_suffix + season_candidate_queries tests live in
// `chimpflix_metadata::agents` now (Deferred B moved them when AniList
// became a fully-trait-driven agent).
