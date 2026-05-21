//! `bootstrap_season_refs` — build the intro + credits audio
//! fingerprint reference set for one season, via tacet's
//! [`tacet::detection::bootstrap_season`]. Payload:
//! `{ "show_id": i64, "season_number": i32 }`.
//!
//! Triggered automatically when a season accumulates its third
//! episode (see [`crate::jobs::handlers::detect_markers_file`]) and
//! manually from the admin UI's "Rebuild season refs" action.
//!
//! Heavy work — symphonia decode + FFT per episode in parallel via
//! tacet's internal rayon pool. We run the whole call inside
//! `tokio::task::spawn_blocking` so the async runtime stays
//! responsive.
//!
//! Successful runs persist the bincoded references and enqueue
//! per-episode detection so already-imported episodes pick up the
//! new refs without an operator click.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use tacet::detection::SeasonReferences;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "bootstrap_season_refs";

/// Minimum number of episodes required before tacet can build
/// usable references. Tacet itself also enforces this internally;
/// duplicating it here lets us short-circuit before spawning the
/// heavy worker.
const MIN_EPISODES_FOR_BOOTSTRAP: i64 = 3;

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub show_id: i64,
    pub season_number: i32,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload {
        show_id,
        season_number,
    } = serde_json::from_value(payload).context("invalid payload")?;

    let episode_count =
        queries::count_episodes_in_season(&state.pool, show_id, season_number)
            .await
            .context("count episodes in season")?;
    if episode_count < MIN_EPISODES_FOR_BOOTSTRAP {
        // Not enough episodes to bootstrap yet. A future detect
        // call will re-trigger us once the season reaches the
        // threshold.
        return Ok(());
    }

    let raw_paths =
        queries::list_episode_paths_in_season(&state.pool, show_id, season_number)
            .await
            .context("list episode paths in season")?;
    let paths: Vec<PathBuf> = raw_paths.iter().map(PathBuf::from).collect();
    let config = tacet::Config::default();

    let refs: SeasonReferences = tokio::task::spawn_blocking(move || {
        // tacet's bootstrap takes &[&Path]; we own PathBufs so build
        // the borrowed slice inside the blocking closure.
        let borrowed: Vec<&std::path::Path> = paths.iter().map(PathBuf::as_path).collect();
        tacet::detection::bootstrap_season(&borrowed, &config)
    })
    .await
    .context("bootstrap_season join")?
    .context("tacet bootstrap_season failed")?;

    // Even an empty result still gets persisted: it acts as a
    // "we tried, nothing came back" sentinel that stops
    // `detect_markers_file::maybe_enqueue_bootstrap` from
    // re-enqueueing us on every newly-added episode. Without the
    // sentinel, a season tacet can't fingerprint (e.g. live-action
    // with unique audio every episode) would re-trigger bootstrap
    // for every new file in perpetuity — heavy CPU each time. The
    // detect-markers handler decodes the empty blob into an empty
    // refs slice, which puts tacet into blackframe-only mode for
    // that season. An admin can rebuild later by clearing the row.
    let empty_refs = refs.intro.is_empty() && refs.credits.is_empty();
    let intro_blob = bincode::serialize(&refs.intro)
        .context("bincode serialize intro refs")?;
    let credits_blob = bincode::serialize(&refs.credits)
        .context("bincode serialize credits refs")?;

    queries::upsert_season_refs_blobs(
        &state.pool,
        show_id,
        season_number,
        &intro_blob,
        &credits_blob,
    )
    .await
    .context("upsert season refs")?;

    if empty_refs {
        warn!(
            show_id,
            season_number,
            episode_count,
            "tacet returned empty references; persisted sentinel row to suppress retries (blackframe-only detection from here)"
        );
        // No per-episode re-enqueue — there's nothing new for them
        // to pick up. They'll naturally hit the empty-refs path via
        // the standard handler when next triggered.
        return Ok(());
    }

    info!(
        show_id,
        season_number,
        intro_refs = refs.intro.len(),
        credits_refs = refs.credits.len(),
        "season references bootstrapped"
    );

    // Re-enqueue per-episode detection for everything in this
    // season so existing files pick up the new references. The
    // per-file handler is idempotent + cheap when refs match.
    let file_ids = sqlx::query(
        "SELECT mf.id AS id
         FROM media_files mf
         JOIN episodes e ON e.id = mf.episode_id
         JOIN seasons s ON s.id = e.season_id
         WHERE s.show_id = ? AND s.season_number = ?
           AND mf.removed_at IS NULL",
    )
    .bind(show_id)
    .bind(season_number)
    .fetch_all(&state.pool)
    .await
    .context("list season episode ids for re-detection")?;
    let ids: Vec<i64> = file_ids
        .iter()
        .filter_map(|r| r.try_get::<i64, _>("id").ok())
        .collect();
    // Clear markers_detected_at so the per-file handler re-runs.
    // (Otherwise the early-exit check would short-circuit.)
    if !ids.is_empty() {
        // build "?,?,?,..." placeholder string for the IN clause
        let placeholders = std::iter::repeat_n("?", ids.len()).collect::<Vec<_>>().join(",");
        let sql = format!(
            "UPDATE media_files SET markers_detected_at = NULL WHERE id IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql);
        for id in &ids {
            q = q.bind(id);
        }
        q.execute(&state.pool)
            .await
            .context("clear markers_detected_at for re-detection")?;
    }
    let _enqueued = crate::jobs::handlers::detect_markers_file::enqueue_for_files(
        &state.pool,
        &ids,
    )
    .await?;
    Ok(())
}

/// Enqueue one bootstrap job per (show_id, season_number). Deduped
/// on a synthetic key combining both values so re-triggers while a
/// job is in flight are no-ops.
pub async fn enqueue_for_season(
    pool: &sqlx::SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<bool> {
    let payload = serde_json::json!({
        "show_id": show_id,
        "season_number": season_number,
    });
    // The dedup column on job_queue is an i64; pack (show_id,
    // season_number) into one. Show ids fit in 47 bits (way more
    // than any reasonable install will reach); season numbers are
    // typically 1-100 but some scanners use 0 or -1 for "specials"
    // / "absolute". `rem_euclid` keeps negatives non-overlapping
    // with positives in the low 16 bits — so season=-1 hashes
    // distinctly from season=65535 (which doesn't exist in
    // practice but defends the dedup key shape against a future
    // exotic season-numbering scheme).
    let season_lo = (season_number as i64).rem_euclid(1 << 16);
    let dedup_key = (show_id << 16) | season_lo;
    let res = queries::enqueue_job_unique(
        pool,
        queries::JobInput::new(KIND, payload),
        "show_season",
        dedup_key,
    )
    .await?;
    Ok(res.is_some())
}
