//! `extract_embedded_subs` — pull subtitle streams out of a media
//! container into .vtt sidecars next to the source. Payload:
//! `{ "file_id": i64 }`.
//!
//! Many MKVs ship subtitles inside the container (PGS bitmap or
//! SRT/ASS text). The HLS player can't seek into a container sub
//! efficiently, so we extract each unique (stream, language) pair
//! into a sidecar `.vtt` once per file. Result is the same shape as
//! [`fetch_subtitles_item`] writes — both feed the player's subtitle
//! picker.
//!
//! Gated by `embedded_subs_extract_enabled` — a separate switch from
//! `subtitle_fetch_enabled` (external OpenSubtitles fetch). Operators
//! often want one but not the other: embedded extract is free, while
//! external fetch costs rate-limited API calls. Per-language dedup
//! means turning the gate on later doesn't redo what's already there.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "extract_embedded_subs";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub file_id: i64,
}

/// One subtitle stream extracted from ffprobe output.
#[derive(Debug)]
struct SubStream {
    index: i32,
    codec: String,
    /// ISO-639 tag from the stream's `language` tag. Defaults to
    /// "und" (undetermined) when the container omits it.
    lang: String,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { file_id } =
        serde_json::from_value(payload).context("invalid payload")?;

    let Some((path, embedded_subs_extracted_at)) =
        sqlx::query_as::<_, (String, Option<i64>)>(
            "SELECT path, embedded_subs_extracted_at
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

    // Idempotent skip — the column is set after a successful
    // extraction. If a new sub-track gets added to the source (rare,
    // but possible if the user re-encodes), the scanner will update
    // media_files and the column gets cleared by that path.
    if embedded_subs_extracted_at.is_some() {
        return Ok(());
    }

    let media_path = PathBuf::from(&path);
    let streams = match probe_subtitle_streams(&state, &media_path).await {
        Ok(s) => s,
        Err(e) => {
            warn!(
                file_id,
                error = %format!("{e:#}"),
                "extract_embedded_subs: ffprobe failed"
            );
            return Ok(());
        }
    };

    if streams.is_empty() {
        // Stamp anyway so the sweep doesn't keep picking this file.
        stamp_extracted(&state, file_id).await?;
        return Ok(());
    }

    let mut extracted = 0usize;
    let mut skipped = 0usize;
    for stream in &streams {
        let dest = sidecar_path(&media_path, &stream.lang);
        if dest.exists() {
            skipped += 1;
            continue;
        }
        // PGS bitmap subs need OCR (not just a format conversion).
        // Skip them in this handler — a future enhancement can
        // shell out to a tesseract pipeline. For now, leaving them
        // un-extracted means external fetch (if enabled) can still
        // fill in the language.
        if is_pgs(&stream.codec) {
            skipped += 1;
            continue;
        }
        match extract_one(&state, &media_path, stream, &dest).await {
            Ok(()) => extracted += 1,
            Err(e) => {
                warn!(
                    file_id,
                    stream_index = stream.index,
                    lang = %stream.lang,
                    error = %format!("{e:#}"),
                    "extract_embedded_subs: ffmpeg extract failed"
                );
            }
        }
    }

    stamp_extracted(&state, file_id).await?;
    info!(file_id, extracted, skipped, "extract_embedded_subs done");
    Ok(())
}

async fn probe_subtitle_streams(state: &AppState, path: &Path) -> Result<Vec<SubStream>> {
    let mut cmd = state.ffmpeg.background_ffprobe();
    cmd.args([
        "-v", "error",
        "-select_streams", "s",
        "-show_entries", "stream=index,codec_name:stream_tags=language",
        "-of", "json",
    ])
    .arg(path)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let output = cmd.output().await.context("ffprobe spawn")?;
    if !output.status.success() {
        anyhow::bail!(
            "ffprobe exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value: Value = serde_json::from_slice(&output.stdout).context("ffprobe json")?;
    let streams = value
        .get("streams")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for s in streams {
        let index = s.get("index").and_then(|v| v.as_i64()).unwrap_or(-1) as i32;
        let codec = s
            .get("codec_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let lang = s
            .get("tags")
            .and_then(|t| t.get("language"))
            .and_then(|l| l.as_str())
            .unwrap_or("und")
            .to_string();
        if index >= 0 && !codec.is_empty() {
            out.push(SubStream { index, codec, lang });
        }
    }
    Ok(out)
}

fn is_pgs(codec: &str) -> bool {
    matches!(codec, "hdmv_pgs_subtitle" | "dvb_subtitle" | "dvd_subtitle")
}

/// Build the sidecar path: `<source stem>.<lang>.vtt`. Same
/// convention as `fetch_subtitles_item`, so the player picks both up
/// uniformly.
fn sidecar_path(source: &Path, lang: &str) -> PathBuf {
    let mut p = source.to_path_buf();
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("subs");
    p.set_file_name(format!("{stem}.{lang}.vtt"));
    p
}

async fn extract_one(
    state: &AppState,
    source: &Path,
    stream: &SubStream,
    dest: &Path,
) -> Result<()> {
    let mut cmd = state.ffmpeg.background_ffmpeg();
    cmd.args([
        "-y",
        "-loglevel", "error",
        "-i",
    ])
    .arg(source)
    .args([
        "-map",
        &format!("0:{}", stream.index),
        "-c:s",
        "webvtt",
    ])
    .arg(dest)
    .stdout(Stdio::null())
    .stderr(Stdio::piped());

    let output = cmd.output().await.context("ffmpeg spawn")?;
    if !output.status.success() {
        // Clean up partial output so we don't leave a zero-byte
        // file the player would try to read.
        let _ = tokio::fs::remove_file(dest).await;
        anyhow::bail!(
            "ffmpeg exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

async fn stamp_extracted(state: &AppState, file_id: i64) -> Result<()> {
    let now = chimpflix_common::now_ms();
    sqlx::query(
        "UPDATE media_files SET embedded_subs_extracted_at = ? WHERE id = ?",
    )
    .bind(now)
    .bind(file_id)
    .execute(&state.pool)
    .await
    .context("media_files embedded_subs_extracted_at update")?;
    Ok(())
}

/// Enqueue one `extract_embedded_subs` job per file. Deduped on
/// file_id so a re-trigger while jobs are in flight is safe.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_path_appends_lang_and_vtt() {
        let p = sidecar_path(Path::new("/movies/Inception.mkv"), "en");
        assert_eq!(p.to_string_lossy(), "/movies/Inception.en.vtt");
    }

    #[test]
    fn sidecar_path_handles_und_language() {
        let p = sidecar_path(Path::new("/show/S01E01.mkv"), "und");
        assert_eq!(p.to_string_lossy(), "/show/S01E01.und.vtt");
    }

    #[test]
    fn is_pgs_matches_known_bitmap_codecs() {
        assert!(is_pgs("hdmv_pgs_subtitle"));
        assert!(is_pgs("dvb_subtitle"));
        assert!(is_pgs("dvd_subtitle"));
        assert!(!is_pgs("subrip"));
        assert!(!is_pgs("ass"));
        assert!(!is_pgs("webvtt"));
    }
}
