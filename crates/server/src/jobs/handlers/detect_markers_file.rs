//! `detect_markers_file` — single-file intro / credits detection.
//! Payload: `{ "file_id": i64 }`.
//!
//! Backed by [tacet](https://crates.io/crates/tacet-core) — audio
//! fingerprint matching against per-season reference sets, with a
//! blackframe-fade fallback for live-action shows.
//!
//! Flow per file:
//!   1. Resolve `show_id` + `season_number` for the file.
//!   2. Load the season's bincoded references (built by
//!      [`crate::jobs::handlers::bootstrap_season_refs`]). Movies
//!      and pre-bootstrap episodes get empty refs.
//!   3. Run `tacet::detection::detect_single_episode` in a blocking
//!      worker — even with empty refs the blackframe fallback can
//!      still produce a credits marker.
//!   4. Persist via `replace_auto_markers` and stamp
//!      `markers_detected_at`.
//!   5. If this is the third+ episode of a season and refs aren't
//!      built yet, enqueue `bootstrap_season_refs` for the season.

use std::path::Path as StdPath;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use tacet::matching::ReferenceFingerprint;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "detect_markers_file";

/// Threshold at which we bootstrap a season's references. Tacet's
/// own minimum (3 episodes) — duplicated as a constant so the
/// trigger logic doesn't have to dig into tacet's internals.
const BOOTSTRAP_THRESHOLD: i64 = 3;

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub file_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { file_id } =
        serde_json::from_value(payload).context("invalid payload")?;

    let Some((path, _duration_ms, markers_already_detected)) =
        sqlx::query_as::<_, (String, Option<i64>, Option<i64>)>(
            "SELECT path, duration_ms, markers_detected_at
             FROM media_files
             WHERE id = ? AND removed_at IS NULL",
        )
        .bind(file_id)
        .fetch_optional(&state.pool)
        .await
        .context("media_files lookup")?
    else {
        return Ok(());
    };

    // Idempotent skip — both the on-add and safety-net paths can
    // enqueue the same file; bail if a previous run already stamped
    // the watermark.
    if markers_already_detected.is_some() {
        return Ok(());
    }

    // Resolve the (show, season) for this file. None means a movie
    // or a file whose item match hasn't been resolved yet. In both
    // cases tacet still runs (empty refs → blackframe fallback only),
    // but we skip the bootstrap trigger.
    let season_ctx = queries::resolve_show_and_season_for_file(&state.pool, file_id)
        .await
        .context("resolve show + season for file")?;

    let (intro_refs, credits_refs) = match season_ctx {
        Some((show_id, season_number)) => {
            load_refs(&state.pool, show_id, season_number).await?
        }
        None => (vec![], vec![]),
    };

    // Tacet is CPU-bound + uses rayon internally — keep it off the
    // tokio runtime so other workers stay responsive.
    let episode_id = format!("file-{file_id}");
    let media_path: PathBuf = StdPath::new(&path).to_path_buf();
    let config = tacet::Config::default();
    let markers = tokio::task::spawn_blocking(move || {
        tacet::detection::detect_single_episode(
            &media_path,
            &episode_id,
            &intro_refs,
            &credits_refs,
            &config,
        )
    })
    .await
    .context("tacet detect join")?;
    let markers = match markers {
        Ok(m) => m,
        Err(e) => {
            // A failure here is "tacet couldn't decode the file" — log
            // and stamp the watermark anyway so the safety-net sweep
            // doesn't keep retrying a permanently-broken source.
            warn!(file_id, error = %format!("{e:#}"), "tacet detection failed");
            stamp_detected(&state.pool, file_id).await?;
            return Ok(());
        }
    };

    let mut rows: Vec<(String, i64, i64)> = Vec::new();
    if let Some(seg) = &markers.intro {
        rows.push(("intro".into(), seg.start_ms(), seg.end_ms()));
    }
    if let Some(seg) = &markers.credits {
        rows.push(("credits".into(), seg.start_ms(), seg.end_ms()));
    }
    queries::replace_auto_markers(&state.pool, file_id, &rows)
        .await
        .context("save markers")?;

    stamp_detected(&state.pool, file_id).await?;

    info!(
        file_id,
        intro_found = markers.intro.is_some(),
        credits_found = markers.credits.is_some(),
        intro_source = ?markers.intro.as_ref().map(|s| s.source),
        credits_source = ?markers.credits.as_ref().map(|s| s.source),
        "markers detected"
    );

    // Trigger bootstrap when this file pushes its season past the
    // threshold and references aren't built yet. Dedup happens at
    // job-queue enqueue (unique on synthetic (show, season) key) +
    // bootstrap_season_refs itself short-circuits when refs are
    // already present.
    if let Some((show_id, season_number)) = season_ctx {
        maybe_enqueue_bootstrap(&state.pool, show_id, season_number).await?;
    }
    Ok(())
}

/// Decode the season's bincoded references, or return empty vecs
/// when the season hasn't been bootstrapped yet. Decode failures are
/// surfaced as warnings + treated as "no refs" so a corrupt blob
/// doesn't break detection for the whole season.
async fn load_refs(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<(Vec<ReferenceFingerprint>, Vec<ReferenceFingerprint>)> {
    let Some(blobs) = queries::load_season_refs_blobs(pool, show_id, season_number)
        .await
        .context("load season refs")?
    else {
        return Ok((vec![], vec![]));
    };
    let intro: Vec<ReferenceFingerprint> = match bincode::deserialize(&blobs.intro) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                show_id,
                season_number,
                error = %e,
                "intro refs blob failed to deserialize; falling back to empty"
            );
            vec![]
        }
    };
    let credits: Vec<ReferenceFingerprint> = match bincode::deserialize(&blobs.credits) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                show_id,
                season_number,
                error = %e,
                "credits refs blob failed to deserialize; falling back to empty"
            );
            vec![]
        }
    };
    Ok((intro, credits))
}

async fn stamp_detected(pool: &SqlitePool, file_id: i64) -> Result<()> {
    let now = chimpflix_common::now_ms();
    sqlx::query("UPDATE media_files SET markers_detected_at = ? WHERE id = ?")
        .bind(now)
        .bind(file_id)
        .execute(pool)
        .await
        .context("stamp markers_detected_at")?;
    Ok(())
}

/// Enqueue `bootstrap_season_refs` when the season is past the
/// threshold and either has no refs yet OR the refs predate any
/// new episode. We accept the slight cost of an enqueue+short-circuit
/// to keep the trigger logic simple — the bootstrap handler dedups
/// internally.
async fn maybe_enqueue_bootstrap(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<()> {
    let episodes = queries::count_episodes_in_season(pool, show_id, season_number)
        .await
        .context("count episodes for bootstrap trigger")?;
    if episodes < BOOTSTRAP_THRESHOLD {
        return Ok(());
    }
    // Refs already built? Skip.
    let already = queries::load_season_refs_blobs(pool, show_id, season_number)
        .await
        .context("check existing season refs")?
        .is_some();
    if already {
        return Ok(());
    }
    let _ =
        super::bootstrap_season_refs::enqueue_for_season(pool, show_id, season_number)
            .await?;
    Ok(())
}

/// Helper for manual trigger paths (item-level, library-level,
/// bulk) — enqueues one detect_markers_file job per media_file_id,
/// deduped on file_id so a re-trigger while a job is in flight
/// doesn't pile up duplicates.
pub async fn enqueue_for_files(
    pool: &sqlx::SqlitePool,
    file_ids: &[i64],
) -> Result<usize> {
    let mut queued = 0usize;
    for &file_id in file_ids {
        let payload = serde_json::json!({ "file_id": file_id });
        let res = queries::enqueue_job_unique(
            pool,
            queries::JobInput::new(KIND, payload),
            "file_id",
            file_id,
        )
        .await?;
        if res.is_some() {
            queued += 1;
        }
    }
    Ok(queued)
}
