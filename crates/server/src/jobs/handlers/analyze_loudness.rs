//! `analyze_loudness` — ffmpeg loudnorm measurement per file.
//! Payload: `{ "file_id": i64 }`.

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path as StdPath;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "analyze_loudness";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub file_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { file_id } =
        serde_json::from_value(payload).context("invalid payload")?;

    let Some((path, analyzed_at)) =
        sqlx::query_as::<_, (String, Option<i64>)>(
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

    // Idempotent skip — `loudnorm_analyzed_at` is set even for
    // silent / no-audio files, so the check covers both "measured"
    // and "checked, nothing to measure" cases.
    if analyzed_at.is_some() {
        return Ok(());
    }

    let measurement = match chimpflix_transcoder::loudness::measure(
        &state.ffmpeg,
        StdPath::new(&path),
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            warn!(file_id, error = %format!("{e:#}"), "loudness analysis failed");
            return Ok(());
        }
    };

    let measurement_record = measurement.map(|m| queries::LoudnessMeasurement {
        integrated: m.integrated,
        true_peak: m.true_peak,
        lra: m.lra,
        threshold: m.threshold,
    });

    queries::record_loudness_measurement(&state.pool, file_id, measurement_record)
        .await
        .context("record loudness measurement")?;
    info!(file_id, "loudness analyzed");
    Ok(())
}

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
