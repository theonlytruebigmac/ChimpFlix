//! `analyze_loudness` — EBU R 128 loudness measurement per file.
//! Payload: `{ "file_id": i64 }`.
//!
//! Phase C of the perf plan (`docs/PERF_PLAN.md`): this handler used
//! to shell out to ffmpeg's `loudnorm` filter via
//! `chimpflix_transcoder::loudness::measure`. It now goes through
//! tacet's unified [`tacet::analyze::analyze_audio`] entry point,
//! which uses a pure-Rust EBU R 128 implementation (the `ebur128`
//! crate, a Rust port of libebur128) with an ffmpeg fallback for
//! codecs symphonia can't decode. Same measurement standard, same
//! output shape, no extra ffmpeg process per file when symphonia can
//! handle the codec.
//!
//! Kept as a distinct job kind for two reasons:
//!   1. Operator gating (`loudness_analysis_enabled`) — flipping the
//!      switch off needs a single place to stop new work.
//!   2. Safety-net sweep — files that pre-date the discovery
//!      pipeline, or files where the gate was flipped on after
//!      import, get loudness via this handler's `enqueue_for_files`
//!      bulk path rather than waiting for a marker re-detection.
//!
//! `detect_markers_file` opportunistically calls
//! `tacet::analyze_audio` with `loudness: true` when the gate is on
//! and the file hasn't been measured yet, so steady-state new-file
//! ingest does both passes from one tacet invocation. This handler
//! is the explicit-trigger / backfill path.

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path as StdPath;
use tacet::analyze::{AnalysisRequest, analyze_audio};
use tacet::loudness::{CancellationToken, ProgressSink};
use tracing::{info, warn};

use crate::jobs::progress::JobContext;
use crate::state::AppState;

pub const KIND: &str = "analyze_loudness";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub file_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { file_id } = serde_json::from_value(payload).context("invalid payload")?;

    let Some((path, analyzed_at)) = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT path, loudnorm_analyzed_at
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

    // Idempotent skip — `loudnorm_analyzed_at` is set even for silent
    // / no-audio files, so the check covers both "measured" and
    // "checked, nothing to measure" cases.
    if analyzed_at.is_some() {
        return Ok(());
    }

    // Tacet's analysis path is CPU-bound + uses rayon internally —
    // keep it off the tokio runtime so other workers stay responsive.
    let media_path = StdPath::new(&path).to_path_buf();
    let config = tacet::Config::default();
    let cancel = CancellationToken::new();
    let request = AnalysisRequest {
        markers: None,
        loudness: true,
    };
    // Pull the per-job progress sink from the worker's task-local so
    // the activity UI can render "Loudness · decoding · 42%" while
    // this handler runs.
    let progress_sink: Option<std::sync::Arc<dyn ProgressSink>> =
        JobContext::current().map(|c| c.progress_sink);
    let result = tokio::task::spawn_blocking(move || {
        let sink_ref: Option<&dyn ProgressSink> = progress_sink.as_deref();
        analyze_audio(&media_path, request, sink_ref, &cancel, &config)
    })
    .await
    .context("analyze_audio join")?;
    let analysis = match result {
        Ok(r) => {
            for warning in &r.warnings {
                warn!(file_id, %warning, "analyze_audio reported a warning during loudness measurement");
            }
            r
        }
        Err(e) => {
            warn!(file_id, error = %format!("{e:#}"), "loudness analysis failed");
            return Ok(());
        }
    };
    let measurement = analysis.loudness;

    let measurement_record = measurement.map(|m| queries::LoudnessMeasurement {
        integrated: m.integrated,
        true_peak: m.true_peak,
        lra: m.lra,
        threshold: m.threshold,
    });

    queries::record_loudness_measurement(&state.pool, file_id, measurement_record)
        .await
        .context("record loudness measurement")?;

    // Persist per-stage timings for the operator UI. Best-effort.
    if let Some(ctx) = JobContext::current() {
        let stage_payload = serde_json::json!({
            "loudness_ms": analysis.stage_timings.loudness.as_millis() as u64,
        });
        if let Ok(s) = serde_json::to_string(&stage_payload) {
            if let Err(e) = queries::record_job_stage_timings(&state.pool, ctx.job_id, &s).await {
                warn!(file_id, error = %format!("{e:#}"), "record_job_stage_timings failed");
            }
        }
    }

    info!(file_id, "loudness analyzed");
    Ok(())
}

/// Batched per-file enqueue — single `BEGIN IMMEDIATE` transaction
/// for all `file_ids`. See [`queries::enqueue_jobs_for_files_batched`]
/// for the rationale (avoids the per-file 517 race the previous
/// loop-of-enqueue_job_unique form was triggering under load).
pub async fn enqueue_for_files(pool: &sqlx::SqlitePool, file_ids: &[i64]) -> Result<usize> {
    queries::enqueue_jobs_for_files_batched(pool, KIND, file_ids).await
}
