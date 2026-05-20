//! `build_chapter_thumbs` — probe chapter metadata and extract one
//! thumbnail per chapter. Payload: `{ "file_id": i64 }`. Stamps the
//! file as processed even when it has no chapters so the discovery
//! pipeline doesn't re-probe it.

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path as StdPath;
use tracing::{debug, info, warn};

use crate::state::AppState;

pub const KIND: &str = "build_chapter_thumbs";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub file_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { file_id } =
        serde_json::from_value(payload).context("invalid payload")?;

    let Some((path, generated_at)) =
        sqlx::query_as::<_, (String, Option<i64>)>(
            "SELECT path, chapter_thumbs_generated_at
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
    if generated_at.is_some() {
        return Ok(());
    }

    let path_ref = StdPath::new(&path);
    let chapters = match chimpflix_transcoder::probe_chapters(&state.ffmpeg, path_ref).await {
        Ok(c) => c,
        Err(e) => {
            // ffprobe failures are usually "no chapters" or
            // container-doesn't-expose-them — debug, not warn.
            debug!(file_id, error = %format!("{e:#}"), "chapter probe failed");
            Vec::new()
        }
    };

    if chapters.is_empty() {
        // Stamp 0 so subsequent ticks don't re-probe.
        queries::record_chapter_thumbs_generated(&state.pool, file_id, 0)
            .await
            .context("stamp no-chapters")?;
        return Ok(());
    }

    let root = state.data_dir.join("chapter_thumbs");
    let mut produced = 0usize;
    for (idx, ch) in chapters.iter().enumerate() {
        let output = chimpflix_transcoder::chapter_thumbs::thumb_path(
            &root,
            file_id,
            idx as u32,
        );
        match chimpflix_transcoder::chapter_thumbs::extract_chapter_thumb(
            &state.ffmpeg,
            path_ref,
            &output,
            ch,
            chimpflix_transcoder::chapter_thumbs::DEFAULT_WIDTH,
        )
        .await
        {
            Ok(()) => produced += 1,
            Err(e) => warn!(
                file_id,
                chapter = idx,
                error = %format!("{e:#}"),
                "chapter thumb extraction failed",
            ),
        }
    }
    queries::record_chapter_thumbs_generated(&state.pool, file_id, chapters.len() as i64)
        .await
        .context("stamp chapter thumbs")?;
    info!(file_id, chapters = chapters.len(), produced, "chapter thumbs built");
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
