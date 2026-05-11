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
use chimpflix_metadata::TmdbClient;
use chimpflix_transcoder::FfmpegConfig;
use sqlx::SqlitePool;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::events::{ScanEmitter, ScanEvent};
use crate::models::{ItemKind, LibraryKind};
use crate::parser::{self, Classification};
use crate::queries;

const PROGRESS_INTERVAL: i64 = 25;

pub async fn run_scan(
    pool: SqlitePool,
    ffmpeg: FfmpegConfig,
    tmdb: Option<TmdbClient>,
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
    roots: &[String],
    library_kind: LibraryKind,
    library_id: i64,
    job_id: i64,
    emitter: &ScanEmitter,
) -> Result<Counters> {
    let existing = queries::existing_media_files(pool, library_id).await?;
    let candidates = collect_candidates(roots).await?;
    debug!(
        library_id,
        count = candidates.len(),
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
        tmdb_apply_movie(pool, tmdb, &hint).await;
    }
    if let Some(hint) = show_hint {
        tmdb_apply_show(pool, tmdb, &hint).await;
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
            if let Err(e) = queries::apply_movie_metadata(pool, hint.id, &meta).await {
                warn!(error = %format!("{e:#}"), "apply movie metadata");
            }
        }
        Ok(None) => debug!(title = %hint.title, "no TMDB match"),
        Err(e) => warn!(error = %format!("{e:#}"), title = %hint.title, "TMDB lookup failed"),
    }
}

async fn tmdb_apply_show(pool: &SqlitePool, tmdb: Option<&TmdbClient>, hint: &ShowHint) {
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
