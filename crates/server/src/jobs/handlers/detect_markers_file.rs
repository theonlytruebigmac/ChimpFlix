//! `detect_markers_file` — single-file marker detection. Payload:
//! `{ "file_id": i64 }`.
//!
//! The discovery-triggered pipeline emits one of these per new
//! `media_files` row, replacing the legacy item-level handler when
//! the scanner finds the file. Manual triggers (admin "Detect
//! markers" on an item / library / bulk) fan out into one job per
//! file, which gives the worker pool finer-grained interleaving
//! between shows and per-file retry semantics.

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path as StdPath;
use tracing::{info, warn};

use crate::api::markers::{maybe_auto_capture_fingerprint, override_intro_via_fingerprint};
use crate::state::AppState;

pub const KIND: &str = "detect_markers_file";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub file_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { file_id } =
        serde_json::from_value(payload).context("invalid payload")?;

    // Resolve path + duration up front. A missing row is a no-op
    // success (file may have been deleted between enqueue and run).
    let Some((path, duration_ms)) = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT path, duration_ms FROM media_files WHERE id = ? AND removed_at IS NULL",
    )
    .bind(file_id)
    .fetch_optional(&state.pool)
    .await
    .context("media_files lookup")?
    else {
        return Ok(());
    };

    let path_buf = StdPath::new(&path).to_path_buf();
    let detected = match chimpflix_transcoder::detect_markers(
        &state.ffmpeg,
        &path_buf,
        duration_ms,
    )
    .await
    {
        Ok(d) => d,
        Err(e) => {
            // Per-file detection failure is best-effort — log and
            // succeed so we don't blanket-retry a corrupt mkv that
            // would keep failing. The safety-net scheduled task
            // (`detect_markers`) re-discovers files without markers
            // on a later tick if the file was repaired.
            warn!(file_id, error = %format!("{e:#}"), "marker detection failed");
            return Ok(());
        }
    };

    // Auto-capture a show fingerprint from chapter metadata when
    // available. The cache is single-shot here (one file = one
    // possible insert) so the HashSet API is overkill — we still
    // call the same helper for symmetry with the bulk path.
    let mut seen_with_fp = std::collections::HashSet::<i64>::new();
    maybe_auto_capture_fingerprint(
        &state.pool,
        &state.ffmpeg,
        file_id,
        &path_buf,
        &detected,
        &mut seen_with_fp,
    )
    .await;

    let mut rows: Vec<(String, i64, i64)> = detected
        .into_iter()
        .map(|m| (m.kind.as_str().to_string(), m.start_ms, m.end_ms))
        .collect();
    override_intro_via_fingerprint(
        &state.pool,
        &state.ffmpeg,
        file_id,
        &path_buf,
        &mut rows,
    )
    .await;

    if let Err(e) = queries::replace_auto_markers(&state.pool, file_id, &rows).await {
        warn!(file_id, error = %format!("{e:#}"), "marker save failed");
    } else {
        info!(file_id, count = rows.len(), "markers detected");
    }
    Ok(())
}

/// Helper for the manual trigger paths (item-level, library-level,
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
