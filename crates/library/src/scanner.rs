//! Scanner orchestration: walk a library's root paths, classify each
//! video file, probe it, persist rows, optionally enrich via TMDB,
//! emit progress events along the way.
//!
//! v0.1 limitations:
//!   * Sequential processing (no concurrent ffprobe). Speed up later.
//!   * No removal of media_files for deleted-from-disk paths. Future work.
//!   * Title-only matching for items (`UNIQUE (library_id, kind, sort_title)`);
//!     two distinct movies with the same title in the same library collide.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use chimpflix_metadata::{TmdbClient, TvMazeClient};
use chimpflix_transcoder::FfmpegConfig;
use sqlx::SqlitePool;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::events::{ScanEmitter, ScanEvent};
use crate::models::{ItemKind, LibraryKind};
use crate::parser::{self, Classification};
use crate::queries;

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
                Self { enabled: set }
            }
        }
    }

    fn is_enabled(&self, name: &str) -> bool {
        self.enabled.contains(name)
    }
}

pub async fn run_scan(
    pool: SqlitePool,
    ffmpeg: FfmpegConfig,
    tmdb: Option<TmdbClient>,
    tvmaze: Option<TvMazeClient>,
    library_id: i64,
    job_id: i64,
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
        tvmaze.as_ref(),
        &library.paths,
        library.kind,
        library_id,
        job_id,
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
async fn scan_inner(
    pool: &SqlitePool,
    ffmpeg: &FfmpegConfig,
    tmdb: Option<&TmdbClient>,
    tvmaze: Option<&TvMazeClient>,
    roots: &[String],
    library_kind: LibraryKind,
    library_id: i64,
    job_id: i64,
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

    let mut counters = Counters::default();
    let mut since_progress = 0i64;

    for (root, path) in candidates {
        counters.files_seen += 1;
        match process_file(
            pool,
            ffmpeg,
            tmdb,
            tvmaze,
            &agents,
            &existing,
            library_id,
            library_kind,
            &root,
            &path,
        )
        .await
        {
            Ok(queries::FileOutcome::Added) => counters.files_added += 1,
            Ok(queries::FileOutcome::Updated) => counters.files_updated += 1,
            Ok(queries::FileOutcome::Unchanged) => {}
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
            for entry in WalkDir::new(root).follow_links(false) {
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
    tvmaze: Option<&TvMazeClient>,
    agents: &AgentChain,
    existing: &HashMap<String, i64>,
    library_id: i64,
    library_kind: LibraryKind,
    root: &Path,
    path: &Path,
) -> Result<queries::FileOutcome> {
    let path_str = path
        .to_str()
        .with_context(|| format!("non-UTF8 path: {}", path.display()))?
        .to_string();

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
        return Ok(queries::FileOutcome::Unchanged);
    }

    let classification = match parser::classify(path, root, library_kind) {
        Some(c) => c,
        None => {
            debug!(?path, "classifier could not extract info");
            return Ok(queries::FileOutcome::Unchanged);
        }
    };

    let probe = chimpflix_transcoder::probe(ffmpeg, path).await?;

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
            let id =
                queries::upsert_item(pool, library_id, ItemKind::Movie, &title, &sort_title, year)
                    .await?;
            movie_hint = Some(MovieHint { title, year, id });
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
            let show_id = queries::upsert_item(
                pool,
                library_id,
                ItemKind::Show,
                &show_title,
                &show_sort_title,
                show_year,
            )
            .await?;
            let season_id = queries::upsert_season(pool, show_id, season).await?;
            let fallback_title = title.unwrap_or_else(|| format!("Episode {episode}"));
            let ep_id = queries::upsert_episode(pool, season_id, episode, &fallback_title).await?;

            show_hint = Some(ShowHint {
                show_title,
                show_year,
                show_id,
                season_number: season,
                episode_number: episode,
                episode_id: ep_id,
            });
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

    if let Some(iid) = item_id {
        if let Some(d) = probe.duration_ms {
            queries::set_item_duration_if_null(pool, iid, d).await?;
        }
    }

    if let Some(hint) = movie_hint {
        let tmdb = if agents.is_enabled("tmdb") { tmdb } else { None };
        tmdb_apply_movie(pool, tmdb, &hint).await;
    }
    if let Some(hint) = show_hint {
        let tmdb = if agents.is_enabled("tmdb") { tmdb } else { None };
        let tvmaze = if agents.is_enabled("tvmaze") {
            tvmaze
        } else {
            None
        };
        tmdb_apply_show(pool, tmdb, tvmaze, &hint).await;
    }

    Ok(outcome)
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

async fn tmdb_apply_movie(pool: &SqlitePool, tmdb: Option<&TmdbClient>, hint: &MovieHint) {
    let Some(client) = tmdb else { return };
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
            tid
        }
    };

    enrich_credits_and_extras(pool, client, item_id, tmdb_id, matches!(kind, ItemKind::Show))
        .await;

    // Run TVMaze after TMDB so it only fills the holes TMDB left.
    if matches!(kind, ItemKind::Show) {
        if let Some(tv) = tvmaze {
            if let Ok(Some(meta)) = tv.lookup_show(&title).await {
                let _ = queries::apply_show_metadata_tvmaze(pool, item_id, &meta).await;
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

async fn tmdb_apply_show(
    pool: &SqlitePool,
    tmdb: Option<&TmdbClient>,
    tvmaze: Option<&TvMazeClient>,
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

    if let Some(show_tmdb_id) = show_tmdb_id {
        match client.fetch_season(show_tmdb_id, hint.season_number).await {
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
