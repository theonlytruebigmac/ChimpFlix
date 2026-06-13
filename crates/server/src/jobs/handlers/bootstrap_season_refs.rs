//! `bootstrap_season_refs` — build the intro + credits audio
//! fingerprint reference set for one season AND emit the per-episode
//! markers from the same decode pass. Payload:
//! `{ "show_id": i64, "season_number": i32 }`.
//!
//! Triggered automatically when a season accumulates its third
//! episode (see [`crate::jobs::handlers::detect_markers_file`]) and
//! manually from the admin UI's "Rebuild season refs" action.
//!
//! Phase B of the perf plan (`docs/PERF_PLAN.md`): this handler used
//! to call `tacet::detection::bootstrap_season` (refs only) and then
//! re-enqueue per-episode `detect_markers_file` jobs, which re-decoded
//! every episode in the season a second time to produce its markers.
//! That redundant decode is gone — we now call `detect_season`, which
//! produces refs + per-episode markers from one decode pass, and the
//! handler writes both in one transaction.
//!
//! Heavy work — symphonia decode + FFT per episode in parallel via
//! tacet's internal rayon pool. We run the whole call inside
//! `tokio::task::spawn_blocking` so the async runtime stays
//! responsive.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chimpflix_library::queries::{self, EpisodeForDetection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tacet::detection::{DetectionResult, EpisodeFile, Season};
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "bootstrap_season_refs";

/// Minimum number of episodes required before tacet can build
/// usable references. Tacet itself also enforces this internally;
/// duplicating it here lets us short-circuit before spawning the
/// heavy worker.
const MIN_EPISODES_FOR_BOOTSTRAP: i64 = 3;

/// Source values written to `markers.source` by this handler. Must
/// stay in lock-step with the detect_markers_file taxonomy.
const SOURCE_TACET: &str = "tacet";
const SOURCE_BLACKFRAME: &str = "blackframe";

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

    let episode_count = queries::count_episodes_in_season(&state.pool, show_id, season_number)
        .await
        .context("count episodes in season")?;
    if episode_count < MIN_EPISODES_FOR_BOOTSTRAP {
        // Not enough episodes to bootstrap yet. A future detect
        // call will re-trigger us once the season reaches the
        // threshold.
        return Ok(());
    }

    // Race-condition guard for the same "all episodes already have
    // markers" optimization `maybe_enqueue_bootstrap` checks. The
    // trigger-side check could race with new markers landing between
    // enqueue and dequeue (e.g. Phase A wrote labels for late-
    // arriving episodes while this job was queued). If every episode
    // is already covered by now there's nothing for us to do.
    let pending = queries::count_episodes_needing_markers_in_season(
        &state.pool,
        show_id,
        season_number,
    )
    .await
    .context("count episodes still needing marker detection")?;
    if pending == 0 {
        info!(
            show_id,
            season_number,
            episode_count,
            "skipping bootstrap_season_refs run: every episode already has markers"
        );
        return Ok(());
    }

    let episodes: Vec<EpisodeForDetection> =
        queries::list_episodes_in_season_for_detection(&state.pool, show_id, season_number)
            .await
            .context("list episodes for detection")?;
    if episodes.len() < MIN_EPISODES_FOR_BOOTSTRAP as usize {
        // Episode count check above counts every episode row; this
        // list filters to ones with a usable media_file (duration
        // probed, not removed). If too many are unprobed we can't
        // bootstrap — bail and let a future trigger retry.
        return Ok(());
    }

    // Build the tacet Season. Episode ids are the file_id stringified
    // so we can map markers back without an extra lookup.
    let tacet_episodes: Vec<EpisodeFile> = episodes
        .iter()
        .map(|e| EpisodeFile {
            id: e.file_id.to_string(),
            path: PathBuf::from(&e.path),
            episode_number: e.episode_number.max(0) as u32,
        })
        .collect();
    let series_id = format!("show-{show_id}");
    let season_label = season_number.max(0) as u32;
    let season = Season {
        series_id,
        season_number: season_label,
        episodes: tacet_episodes,
    };
    let config = tacet::Config::default();

    // Use the adaptive variant: tries a narrow 5/4-min scan window
    // first, only falls back to the configured wide window when the
    // narrow attempt fails to find cross-episode consensus. Common
    // case (intro in first 3-5 min) decodes ~6× less audio; worst
    // case (long cold open, e.g. Silo S01E05 at 14:28) pays the
    // narrow attempt then re-runs at full width.
    let detect_started = std::time::Instant::now();
    let result: DetectionResult = tokio::task::spawn_blocking(move || {
        tacet::detection::detect_season_adaptive(&season, &config)
    })
    .await
    .context("detect_season join")?
    .context("tacet detect_season_adaptive failed")?;
    let detect_elapsed = detect_started.elapsed();

    let empty_refs =
        result.intro_references.is_empty() && result.credits_references.is_empty();
    let intro_blob = bincode::serialize(&result.intro_references)
        .context("bincode serialize intro refs")?;
    let credits_blob = bincode::serialize(&result.credits_references)
        .context("bincode serialize credits refs")?;

    // Persist refs first. Even an empty result still gets persisted:
    // it acts as a "we tried, nothing came back" sentinel that stops
    // `detect_markers_file::maybe_enqueue_bootstrap` from re-enqueueing
    // us on every newly-added episode. The detect-markers handler
    // decodes the empty blob into an empty refs slice, which puts
    // tacet into blackframe-only mode for that season. An admin can
    // rebuild later by clearing the row.
    queries::upsert_season_refs_blobs(
        &state.pool,
        show_id,
        season_number,
        &intro_blob,
        &credits_blob,
    )
    .await
    .context("upsert season refs")?;

    // Persist per-episode markers from the same decode pass. Map
    // tacet's episode_id string back to our file_id via the original
    // list, then write each row + stamp `markers_detected_at`. This
    // is the work `detect_markers_file` used to re-derive — keeping
    // it here saves N redundant audio decodes per season bootstrap.
    let file_id_by_tacet_id: HashMap<String, i64> = episodes
        .iter()
        .map(|e| (e.file_id.to_string(), e.file_id))
        .collect();

    let mut wrote_markers = 0usize;
    let mut stamped = 0usize;
    for seg_markers in &result.markers {
        let Some(&file_id) = file_id_by_tacet_id.get(&seg_markers.episode_id) else {
            warn!(
                episode_id = %seg_markers.episode_id,
                "tacet returned markers for an unrecognized episode_id; skipping"
            );
            continue;
        };

        let mut rows: Vec<(String, i64, i64, String)> = Vec::new();
        if let Some(seg) = &seg_markers.intro {
            rows.push((
                "intro".to_string(),
                seg.start_ms(),
                seg.end_ms(),
                tacet_source_str(seg.source),
            ));
        }
        if let Some(seg) = &seg_markers.credits {
            rows.push((
                "credits".to_string(),
                seg.start_ms(),
                seg.end_ms(),
                tacet_source_str(seg.source),
            ));
        }

        // `replace_detected_markers` clears any prior auto rows for
        // this file before writing the new ones, so re-runs of the
        // bootstrap (e.g. operator "Rebuild season refs") correctly
        // overwrite stale tacet output. Manual rows are preserved.
        queries::replace_detected_markers(&state.pool, file_id, &rows)
            .await
            .with_context(|| format!("save markers for file_id={file_id}"))?;
        if !rows.is_empty() {
            wrote_markers += 1;
        }

        // Stamp the watermark whether or not we produced rows. An
        // empty result means tacet couldn't find anything (blackframe
        // also missed) — we still don't want detect_markers_file to
        // re-run on this file.
        stamp_detected(&state.pool, file_id).await?;
        stamped += 1;
    }

    // Diagnostic: per-episode wall-clock for the analysis. With
    // tacet running episodes through a rayon pool we can't break
    // out true per-episode timings from inside the spawn_blocking
    // (the work is parallel), but the total elapsed + episodes-
    // analysed gives the operator an average. Useful for answering
    // "why did this bootstrap take 14 minutes" — divide by episode
    // count to see if any single episode is way out of family.
    let per_episode_avg_ms = if episodes.is_empty() {
        0
    } else {
        detect_elapsed.as_millis() as u64 / episodes.len() as u64
    };

    if empty_refs {
        warn!(
            show_id,
            season_number,
            episode_count,
            stamped,
            detect_elapsed_ms = detect_elapsed.as_millis() as u64,
            per_episode_avg_ms,
            "tacet returned empty references; persisted sentinel row + stamped episodes (blackframe-only detection from here)"
        );
    } else {
        info!(
            show_id,
            season_number,
            intro_refs = result.intro_references.len(),
            credits_refs = result.credits_references.len(),
            wrote_markers,
            stamped,
            detect_elapsed_ms = detect_elapsed.as_millis() as u64,
            per_episode_avg_ms,
            "season references bootstrapped + per-episode markers persisted"
        );
    }

    Ok(())
}

/// Map tacet's `SegmentSource` to the string written to
/// `markers.source`. Mirrors the helper in `detect_markers_file`;
/// kept inline rather than shared because the two handlers are the
/// only call sites and the mapping is one match arm long.
fn tacet_source_str(source: tacet::SegmentSource) -> String {
    match source {
        tacet::SegmentSource::AudioFingerprint => SOURCE_TACET.to_string(),
        tacet::SegmentSource::Blackframe => SOURCE_BLACKFRAME.to_string(),
    }
}

async fn stamp_detected(pool: &sqlx::SqlitePool, file_id: i64) -> Result<()> {
    let now = chimpflix_common::now_ms();
    sqlx::query("UPDATE media_files SET markers_detected_at = ? WHERE id = ?")
        .bind(now)
        .bind(file_id)
        .execute(pool)
        .await
        .with_context(|| format!("stamp markers_detected_at for file_id={file_id}"))?;
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
    // The dedup column on job_queue is an i64; combine (show_id,
    // season_number) into one stable key. We use a multiplicative
    // hash mix rather than bit-packing: bit-packing overflows i64
    // for show_id >= 2^47 (wraps silently in release builds,
    // causing collisions for imports with large external IDs).
    //
    // Mix: multiply show_id by a large odd prime (Knuth's golden-
    // ratio constant for 64-bit), then add the season number.
    // `wrapping_*` makes the modular arithmetic explicit. All
    // (show_id, season_number) pairs that fit in their declared
    // types map to distinct keys in the practical range.
    //
    // The key is also stamped onto the payload as `show_season`
    // so `enqueue_job_unique`'s `json_extract(payload,
    // '$.show_season')` lookup matches a real field. Without it,
    // dedup always misses — observed as 7-8 concurrent bootstrap
    // runs saturating the SQLite writer.
    let dedup_key = show_id
        .wrapping_mul(0x9e3779b97f4a7c15u64 as i64)
        .wrapping_add(season_number as i64);
    let payload = serde_json::json!({
        "show_id": show_id,
        "season_number": season_number,
        "show_season": dedup_key,
    });
    let res = queries::enqueue_job_unique(
        pool,
        queries::JobInput::new(KIND, payload),
        "show_season",
        dedup_key,
    )
    .await?;
    Ok(res.is_some())
}
