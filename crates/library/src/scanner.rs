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
//!   * No removal of media_files for deleted-from-disk paths. Future work.
//!   * Title-only matching for items (`UNIQUE (library_id, kind, sort_title)`);
//!     two distinct movies with the same title in the same library collide.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use chimpflix_metadata::{AniListClient, TmdbClient, TmdbSeason, TvMazeClient, TvdbClient};
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
/// HTTP requests against TMDB/TVDB/TVMaze/AniList. 8 keeps the box
/// usefully busy without saturating either ffprobe spawns or the
/// metadata APIs' polite-client expectations.
const SCAN_PARALLELISM: usize = 8;

/// Per-scan cache of TMDB season fetches. Keyed on
/// `(show_tmdb_id, season_number)`. Lives only for the duration of a
/// single scan — we deliberately don't persist this across scans so
/// freshly-aired episodes show up on the next run without manual
/// invalidation.
type SeasonCache = Mutex<HashMap<(i64, i32), Arc<TmdbSeason>>>;

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

/// The set of metadata agents that should run for a given library, in
/// owner-configured priority order. Loaded once per scan to avoid querying
/// `library_agents` on every file.
#[derive(Debug, Clone, Default)]
struct AgentChain {
    enabled: std::collections::HashSet<String>,
}

impl AgentChain {
    async fn load(pool: &SqlitePool, library_id: i64) -> Self {
        match queries::list_library_agents(pool, library_id).await {
            Ok(agents) => Self {
                enabled: agents
                    .into_iter()
                    .filter(|a| a.enabled)
                    .map(|a| a.agent_name)
                    .collect(),
            },
            Err(e) => {
                warn!(error = %format!("{e:#}"), library_id, "failed to load library agents — falling back to defaults");
                // Defaults match the legacy hardcoded behavior so a
                // misconfigured table doesn't break enrichment.
                let mut set = std::collections::HashSet::new();
                set.insert("tmdb".into());
                set.insert("tvmaze".into());
                set.insert("tvdb".into());
                set.insert("anilist".into());
                Self { enabled: set }
            }
        }
    }

    fn is_enabled(&self, name: &str) -> bool {
        self.enabled.contains(name)
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
async fn scan_inner(
    pool: &SqlitePool,
    ffmpeg: &FfmpegConfig,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    anilist: Option<&AniListClient>,
    tvmaze: Option<&TvMazeClient>,
    roots: &[String],
    library_kind: LibraryKind,
    library_id: i64,
    job_id: i64,
    cache_root: Option<&Path>,
    emitter: &ScanEmitter,
) -> Result<Counters> {
    let existing = queries::existing_media_files(pool, library_id).await?;
    let candidates = collect_candidates(roots).await?;
    let agents = AgentChain::load(pool, library_id).await;
    debug!(
        library_id,
        count = candidates.len(),
        enabled_agents = ?agents.enabled,
        "scan candidates collected"
    );

    let existing = Arc::new(existing);
    let agents = Arc::new(agents);
    let cache_root_owned: Option<PathBuf> = cache_root.map(Path::to_path_buf);
    let season_cache: Arc<SeasonCache> = Arc::new(Mutex::new(HashMap::new()));

    // Clone the network clients up front so each parallel worker owns
    // its own handle. reqwest::Client is internally Arc'd, so this is
    // a cheap refcount bump per worker — not duplicated state.
    let pool_owned = pool.clone();
    let ffmpeg_owned = ffmpeg.clone();
    let tmdb_owned = tmdb.cloned();
    let tvdb_owned = tvdb.cloned();
    let anilist_owned = anilist.cloned();
    let tvmaze_owned = tvmaze.cloned();

    let mut tasks = stream::iter(candidates.into_iter())
        .map(|(root, path)| {
            let pool = pool_owned.clone();
            let ffmpeg = ffmpeg_owned.clone();
            let tmdb = tmdb_owned.clone();
            let tvdb = tvdb_owned.clone();
            let anilist = anilist_owned.clone();
            let tvmaze = tvmaze_owned.clone();
            let agents = agents.clone();
            let existing = existing.clone();
            let cache_root = cache_root_owned.clone();
            let season_cache = season_cache.clone();
            async move {
                let res = process_file(
                    &pool,
                    &ffmpeg,
                    tmdb.as_ref(),
                    tvdb.as_ref(),
                    anilist.as_ref(),
                    tvmaze.as_ref(),
                    &agents,
                    &existing,
                    library_id,
                    library_kind,
                    &root,
                    &path,
                    cache_root.as_deref(),
                    &season_cache,
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
            queries::update_scan_counters(
                pool,
                job_id,
                counters.files_seen,
                counters.files_added,
                counters.files_updated,
                counters.files_removed,
            )
            .await?;
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

    Ok(counters)
}

async fn collect_candidates(roots: &[String]) -> Result<Vec<(PathBuf, PathBuf)>> {
    let roots: Vec<PathBuf> = roots.iter().map(PathBuf::from).collect();
    tokio::task::spawn_blocking(move || {
        let mut out = Vec::new();
        for root in &roots {
            if !root.exists() {
                warn!(root = %root.display(), "library root does not exist");
                continue;
            }
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
        out
    })
    .await
    .context("walk join failed")
}

#[allow(clippy::too_many_arguments)]
async fn process_file(
    pool: &SqlitePool,
    ffmpeg: &FfmpegConfig,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    anilist: Option<&AniListClient>,
    tvmaze: Option<&TvMazeClient>,
    agents: &AgentChain,
    existing: &HashMap<String, i64>,
    library_id: i64,
    library_kind: LibraryKind,
    root: &Path,
    path: &Path,
    cache_root: Option<&Path>,
    season_cache: &SeasonCache,
) -> Result<(queries::FileOutcome, Option<i64>)> {
    // Non-UTF8 paths used to fail silently up the error chain with
    // only the generic "non-UTF8 path" message in the scan job log.
    // Operators reported files disappearing from the library without
    // an obvious cause; the lossy display string here lets them see
    // *which* file got rejected (typically a Latin-1 filename that
    // the filesystem driver didn't normalize) so they can rename it.
    let path_str = path
        .to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| {
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
            let ep_id = queries::upsert_episode(pool, season_id, episode, &fallback_title).await?;

            // Same enrichment-skip rationale as the Movie arm.
            if auto_matched {
                show_hint = Some(ShowHint {
                    show_title,
                    show_year,
                    show_id,
                    season_number: season,
                    episode_number: episode,
                    episode_id: ep_id,
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

    if let Some(hint) = movie_hint {
        let tmdb = if agents.is_enabled("tmdb") { tmdb } else { None };
        let tvdb = if agents.is_enabled("tvdb") { tvdb } else { None };
        tmdb_apply_movie(pool, tmdb, tvdb, &hint).await;
    }
    if let Some(hint) = show_hint {
        let tmdb = if agents.is_enabled("tmdb") { tmdb } else { None };
        let tvdb = if agents.is_enabled("tvdb") { tvdb } else { None };
        let tvmaze = if agents.is_enabled("tvmaze") {
            tvmaze
        } else {
            None
        };
        // For anime libraries, AniList is the canonical primary; it runs
        // first so its title/summary/year stick, and the show-tail
        // enrichment treats TMDB/TVMaze/TVDB as null-fillers behind it.
        // The agent gate still applies — owners can disable AniList per
        // library if they prefer TMDB primary for a given catalogue.
        if matches!(library_kind, LibraryKind::Anime) && agents.is_enabled("anilist") {
            apply_anilist_for_show(pool, anilist, &hint).await;
        }
        tmdb_apply_show(pool, tmdb, tvdb, tvmaze, season_cache, &hint).await;
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
}

async fn tmdb_apply_movie(
    pool: &SqlitePool,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    hint: &MovieHint,
) {
    if let Some(client) = tmdb {
        match client.lookup_movie(&hint.title, hint.year).await {
            Ok(Some(meta)) => {
                let tmdb_id = meta.tmdb_id;
                let collection = meta.collection.clone();
                if let Err(e) = queries::apply_movie_metadata(pool, hint.id, &meta).await {
                    warn!(error = %format!("{e:#}"), "apply movie metadata");
                }
                enrich_credits_and_extras(pool, client, hint.id, tmdb_id, false).await;
                if let Some(stub) = collection {
                    apply_collection_for_item(pool, client, hint.id, &stub).await;
                }
            }
            Ok(None) => debug!(title = %hint.title, "no TMDB match"),
            Err(e) => warn!(error = %format!("{e:#}"), title = %hint.title, "TMDB lookup failed"),
        }
    }
    apply_tvdb_for_movie(pool, tvdb, hint).await;
}

async fn apply_tvdb_for_movie(
    pool: &SqlitePool,
    tvdb: Option<&TvdbClient>,
    hint: &MovieHint,
) {
    let Some(client) = tvdb else { return };
    match client.lookup_movie(&hint.title, hint.year).await {
        Ok(Some(meta)) => {
            if let Err(e) = queries::apply_movie_metadata_tvdb(pool, hint.id, &meta).await {
                warn!(error = %format!("{e:#}"), "apply TVDB movie metadata");
            }
        }
        Ok(None) => debug!(title = %hint.title, "no TVDB match"),
        Err(e) => warn!(error = %format!("{e:#}"), title = %hint.title, "TVDB movie lookup failed"),
    }
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
            if let Err(e) =
                queries::enrich_collection_overview(pool, collection_id, &full).await
            {
                warn!(error = %format!("{e:#}"), "enrich collection failed");
            }
        }
        Err(e) => warn!(error = %format!("{e:#}"), tmdb_id = stub.tmdb_id, "collection fetch failed"),
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
pub async fn refresh_item_metadata(
    pool: &SqlitePool,
    client: &TmdbClient,
    tvdb: Option<&TvdbClient>,
    tvmaze: Option<&TvMazeClient>,
    item_id: i64,
    override_tmdb_id: Option<i64>,
) -> anyhow::Result<()> {
    use crate::models::ItemKind;
    let row = sqlx::query("SELECT kind, title, year, tmdb_id FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_one(pool)
        .await?;
    let kind = ItemKind::from_db(sqlx::Row::try_get::<&str, _>(&row, "kind")?)?;
    let title: String = sqlx::Row::try_get(&row, "title")?;
    let year: Option<i32> = sqlx::Row::try_get(&row, "year")?;
    let existing_tmdb: Option<i64> = sqlx::Row::try_get(&row, "tmdb_id")?;
    let target_tmdb = override_tmdb_id.or(existing_tmdb);

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
            queries::apply_movie_metadata(pool, item_id, &meta).await?;
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
            queries::apply_show_metadata(pool, item_id, &meta).await?;
            // Per-episode enrichment. Without this, episode titles stay
            // at whatever the parser extracted from the filename
            // ("Spring Time Bluray-1080p") instead of the TMDB title
            // ("Spring Time"). Scan-time `tmdb_apply_show` only runs
            // once per file discovered, but the Refresh button enters
            // here — so we walk every local season + episode and
            // overwrite with TMDB's text.
            refresh_show_episodes(pool, client, item_id, tid).await;
            tid
        }
    };

    enrich_credits_and_extras(pool, client, item_id, tmdb_id, matches!(kind, ItemKind::Show))
        .await;

    // Run TVMaze + TVDB after TMDB so they only fill the holes TMDB left.
    match kind {
        ItemKind::Show => {
            if let Some(tv) = tvmaze {
                if let Ok(Some(meta)) = tv.lookup_show(&title).await {
                    let _ = queries::apply_show_metadata_tvmaze(pool, item_id, &meta).await;
                }
            }
            if let Some(tv) = tvdb {
                if let Ok(Some(meta)) = tv.lookup_show(&title, year).await {
                    let _ = queries::apply_show_metadata_tvdb(pool, item_id, &meta).await;
                }
            }
        }
        ItemKind::Movie => {
            if let Some(tv) = tvdb {
                if let Ok(Some(meta)) = tv.lookup_movie(&title, year).await {
                    let _ = queries::apply_movie_metadata_tvdb(pool, item_id, &meta).await;
                }
            }
        }
    }
    Ok(())
}

/// Backfill cast+crew and YouTube extras (trailers, featurettes, BTS) on
/// items the scanner just identified. Best-effort: any failure logs and
/// moves on so the scan still completes.
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
    match client.fetch_credits(kind, tmdb_id).await {
        Ok(credits) => {
            if let Err(e) = queries::apply_item_credits(pool, item_id, &credits).await {
                warn!(error = %format!("{e:#}"), "apply credits");
            }
        }
        Err(e) => warn!(error = %format!("{e:#}"), tmdb_id, "TMDB credits fetch failed"),
    }
    match client.fetch_videos(kind, tmdb_id).await {
        Ok(videos) => {
            if let Err(e) = queries::apply_item_extras(pool, item_id, &videos).await {
                warn!(error = %format!("{e:#}"), "apply extras");
            }
        }
        Err(e) => warn!(error = %format!("{e:#}"), tmdb_id, "TMDB videos fetch failed"),
    }
    match client.fetch_reviews(kind, tmdb_id).await {
        Ok(reviews) => {
            if let Err(e) = queries::apply_tmdb_reviews(pool, item_id, &reviews).await {
                warn!(error = %format!("{e:#}"), "apply reviews");
            }
        }
        Err(e) => warn!(error = %format!("{e:#}"), tmdb_id, "TMDB reviews fetch failed"),
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
                warn!(
                    error = %format!("{e:#}"),
                    show_id,
                    season_number,
                    "refresh: TMDB season fetch failed; episode titles left as-is"
                );
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

async fn tmdb_apply_show(
    pool: &SqlitePool,
    tmdb: Option<&TmdbClient>,
    tvdb: Option<&TvdbClient>,
    tvmaze: Option<&TvMazeClient>,
    season_cache: &SeasonCache,
    hint: &ShowHint,
) {
    let Some(client) = tmdb else { return };

    // Enrich the show row only if it doesn't yet have a tmdb_id. Avoids
    // hammering TMDB once per episode.
    let row = sqlx::query("SELECT tmdb_id FROM items WHERE id = ?")
        .bind(hint.show_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let existing_tmdb_id: Option<i64> = row.and_then(|r| {
        sqlx::Row::try_get::<Option<i64>, _>(&r, "tmdb_id")
            .ok()
            .flatten()
    });

    let show_tmdb_id = match existing_tmdb_id {
        Some(id) => Some(id),
        None => match client.lookup_show(&hint.show_title, hint.show_year).await {
            Ok(Some(meta)) => {
                let tid = meta.tmdb_id;
                if let Err(e) = queries::apply_show_metadata(pool, hint.show_id, &meta).await {
                    warn!(error = %format!("{e:#}"), "apply show metadata");
                }
                enrich_credits_and_extras(pool, client, hint.show_id, tid, true).await;
                Some(tid)
            }
            Ok(None) => {
                debug!(title = %hint.show_title, "no TMDB show match");
                None
            }
            Err(e) => {
                warn!(error = %format!("{e:#}"), title = %hint.show_title, "TMDB show lookup failed");
                None
            }
        },
    };

    // TVMaze fallback / null-filler. Runs whether or not TMDB matched —
    // when TMDB found nothing it provides primary identification, and
    // when TMDB matched it fills any remaining nulls (network, status,
    // imdb/tvdb cross-refs we didn't get from TMDB, etc.) without ever
    // overwriting.
    apply_tvmaze_for_show(pool, tvmaze, hint).await;
    apply_tvdb_for_show(pool, tvdb, hint).await;

    if let Some(show_tmdb_id) = show_tmdb_id {
        match fetch_season_cached(season_cache, client, show_tmdb_id, hint.season_number).await {
            Ok(season) => {
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
            Err(e) => warn!(
                error = %format!("{e:#}"),
                show = %hint.show_title,
                season = hint.season_number,
                "TMDB season fetch failed"
            ),
        }
    }
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
) -> anyhow::Result<Arc<TmdbSeason>> {
    let key = (show_tmdb_id, season_number);
    {
        let guard = cache.lock().await;
        if let Some(hit) = guard.get(&key) {
            return Ok(hit.clone());
        }
    }
    let season = client.fetch_season(show_tmdb_id, season_number).await?;
    let arc = Arc::new(season);
    let mut guard = cache.lock().await;
    // Two parallel workers can race to populate the same key. Either
    // wins — they'd have produced equivalent results — but we
    // deliberately don't overwrite a winner that beat us by handing
    // back what's already there.
    Ok(guard.entry(key).or_insert(arc).clone())
}

async fn apply_tvmaze_for_show(
    pool: &SqlitePool,
    tvmaze: Option<&TvMazeClient>,
    hint: &ShowHint,
) {
    let Some(client) = tvmaze else { return };
    // Only call TVMaze when there's still something for it to contribute:
    // skip when summary AND year AND imdb_id are all already set.
    let row = sqlx::query(
        "SELECT title, summary, year, imdb_id, tvdb_id FROM items WHERE id = ?",
    )
    .bind(hint.show_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some(row) = row else { return };
    let summary: Option<String> = sqlx::Row::try_get(&row, "summary").ok().flatten();
    let year: Option<i32> = sqlx::Row::try_get(&row, "year").ok().flatten();
    let imdb_id: Option<String> = sqlx::Row::try_get(&row, "imdb_id").ok().flatten();
    let tvdb_id: Option<i64> = sqlx::Row::try_get(&row, "tvdb_id").ok().flatten();
    if summary.is_some() && year.is_some() && imdb_id.is_some() && tvdb_id.is_some() {
        return;
    }
    match client.lookup_show(&hint.show_title).await {
        Ok(Some(meta)) => {
            if let Err(e) =
                queries::apply_show_metadata_tvmaze(pool, hint.show_id, &meta).await
            {
                warn!(error = %format!("{e:#}"), "apply TVMaze metadata");
            }
        }
        Ok(None) => debug!(title = %hint.show_title, "no TVMaze match"),
        Err(e) => warn!(error = %format!("{e:#}"), title = %hint.show_title, "TVMaze lookup failed"),
    }
}

async fn apply_anilist_for_show(
    pool: &SqlitePool,
    anilist: Option<&AniListClient>,
    hint: &ShowHint,
) {
    let Some(client) = anilist else { return };
    // Skip the API call if we already have an anilist_id stored — re-runs
    // of the scan shouldn't re-search every episode of every show.
    let row = sqlx::query("SELECT anilist_id FROM items WHERE id = ?")
        .bind(hint.show_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if let Some(row) = row {
        let existing: Option<i64> = sqlx::Row::try_get(&row, "anilist_id").ok().flatten();
        if existing.is_some() {
            return;
        }
    }
    match client.lookup_show(&hint.show_title, hint.show_year).await {
        Ok(Some(meta)) => {
            if let Err(e) =
                queries::apply_show_metadata_anilist(pool, hint.show_id, &meta).await
            {
                warn!(error = %format!("{e:#}"), "apply AniList show metadata");
            }
        }
        Ok(None) => debug!(title = %hint.show_title, "no AniList match"),
        Err(e) => warn!(error = %format!("{e:#}"), title = %hint.show_title, "AniList lookup failed"),
    }
}

async fn apply_tvdb_for_show(
    pool: &SqlitePool,
    tvdb: Option<&TvdbClient>,
    hint: &ShowHint,
) {
    let Some(client) = tvdb else { return };
    // Skip the API call when nothing TVDB can contribute remains. Same
    // null-check shape as TVMaze; original_title is the one TVDB-only
    // field we care about over and above what TVMaze can supply.
    let row = sqlx::query(
        "SELECT summary, year, imdb_id, tvdb_id, original_title FROM items WHERE id = ?",
    )
    .bind(hint.show_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some(row) = row else { return };
    let summary: Option<String> = sqlx::Row::try_get(&row, "summary").ok().flatten();
    let year: Option<i32> = sqlx::Row::try_get(&row, "year").ok().flatten();
    let imdb_id: Option<String> = sqlx::Row::try_get(&row, "imdb_id").ok().flatten();
    let tvdb_id: Option<i64> = sqlx::Row::try_get(&row, "tvdb_id").ok().flatten();
    let original_title: Option<String> =
        sqlx::Row::try_get(&row, "original_title").ok().flatten();
    if summary.is_some()
        && year.is_some()
        && imdb_id.is_some()
        && tvdb_id.is_some()
        && original_title.is_some()
    {
        return;
    }
    match client.lookup_show(&hint.show_title, hint.show_year).await {
        Ok(Some(meta)) => {
            if let Err(e) =
                queries::apply_show_metadata_tvdb(pool, hint.show_id, &meta).await
            {
                warn!(error = %format!("{e:#}"), "apply TVDB show metadata");
            }
        }
        Ok(None) => debug!(title = %hint.show_title, "no TVDB show match"),
        Err(e) => warn!(error = %format!("{e:#}"), title = %hint.show_title, "TVDB show lookup failed"),
    }
}
