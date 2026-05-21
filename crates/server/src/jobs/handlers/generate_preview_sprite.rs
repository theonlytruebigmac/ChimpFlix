//! `generate_preview_sprite` — sprite sheet for the scrubber preview.
//! Payload: `{ "file_id": i64 }`.

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path as StdPath;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "generate_preview_sprite";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub file_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { file_id } =
        serde_json::from_value(payload).context("invalid payload")?;

    let Some((path, duration_ms, sprite_path)) =
        sqlx::query_as::<_, (String, Option<i64>, Option<String>)>(
            "SELECT path, duration_ms, preview_sprite_path
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

    // Idempotency at the file level — if a sprite already exists,
    // bail. The discovery enqueue path is deduped, but manual
    // retriggers (admin "Generate previews" on a whole library)
    // pile up jobs for files that don't need them; this short-
    // circuits without rerunning ffmpeg.
    if sprite_path.is_some() {
        return Ok(());
    }
    let Some(duration) = duration_ms else {
        // No duration — sprite generation needs it to space frames.
        // Stamp nothing and move on; the next scan with mediainfo
        // will fill in duration.
        return Ok(());
    };

    let dir = state.data_dir.join("previews");
    tokio::fs::create_dir_all(&dir).await?;
    let output = dir.join(format!("{file_id}.jpg"));

    let info = match chimpflix_transcoder::generate_sprite(
        &state.ffmpeg,
        StdPath::new(&path),
        &output,
        duration,
        chimpflix_transcoder::DEFAULT_INTERVAL_S,
        chimpflix_transcoder::DEFAULT_TILE_WIDTH,
    )
    .await
    {
        Ok(info) => info,
        Err(e) => {
            warn!(file_id, error = %format!("{e:#}"), "preview generation failed");
            return Ok(());
        }
    };

    queries::record_preview_sprite(
        &state.pool,
        queries::PreviewSpriteRecord {
            media_file_id: file_id,
            path: info.path.to_string_lossy().into_owned(),
            interval_ms: info.interval_ms,
            tile_width: i64::from(info.tile_width),
            tile_height: i64::from(info.tile_height),
            tile_cols: i64::from(info.tile_cols),
            tile_count: i64::from(info.tile_count),
        },
    )
    .await
    .context("record preview sprite")?;

    info!(file_id, "preview sprite generated");
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
