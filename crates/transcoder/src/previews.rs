//! Scrub-preview thumbnail sprite generator.
//!
//! One JPEG per media file, laid out as a grid of small frames captured
//! at a fixed-interval cadence (default: every 10s). The player
//! computes the tile index from playback position on hover and renders
//! the matching slice via CSS `background-position`. Lighter and
//! cacheable, vs. fetching a thumbnail per second.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, bail};
use tracing::debug;

use crate::FfmpegConfig;

/// Default interval between sampled frames. 10 seconds is the Plex/Roku
/// convention and gives ~720 tiles for a 2-hour movie — manageable as a
/// single sprite at 240×135 (~300 KB JPEG).
pub const DEFAULT_INTERVAL_S: u32 = 10;

/// Default tile width in pixels. Height tracks the source aspect ratio.
pub const DEFAULT_TILE_WIDTH: u32 = 240;

#[derive(Debug, Clone)]
pub struct SpriteInfo {
    pub path: PathBuf,
    pub interval_ms: i64,
    pub tile_width: u32,
    pub tile_height: u32,
    pub tile_cols: u32,
    pub tile_count: u32,
}

/// Generate a preview sprite for `input` at `output` (parent dir is
/// created). Computes a near-square tile grid sized to fit
/// `duration_ms / interval_s` thumbnails. Returns dimensions the caller
/// should persist on `media_files` so the player can index into the
/// sprite without re-probing.
pub async fn generate_sprite(
    ffmpeg: &FfmpegConfig,
    input: &Path,
    output: &Path,
    duration_ms: i64,
    interval_s: u32,
    tile_width: u32,
) -> Result<SpriteInfo> {
    if duration_ms <= 0 {
        bail!("source has unknown duration; cannot build preview sprite");
    }
    let interval_s = interval_s.max(1);
    let total_tiles = compute_tile_count(duration_ms, interval_s);
    if total_tiles == 0 {
        bail!("source is too short for any preview tiles at interval {interval_s}s");
    }
    let cols = grid_cols(total_tiles);
    let rows = total_tiles.div_ceil(cols);

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create preview dir {}", parent.display()))?;
    }

    // The fps filter samples one frame every interval_s seconds; scale
    // normalises the width (height auto-derived; trunc to even avoids
    // ffmpeg's "height not divisible by 2" warning on odd aspect ratios);
    // tile composes them into a single image.
    let vf = format!(
        "fps=1/{interval_s},scale={tile_width}:-2,tile={cols}x{rows}"
    );
    let args = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-skip_frame",
        "nokey",
        "-i",
        input
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF8 input path"))?,
        "-vf",
        &vf,
        "-frames:v",
        "1",
        "-qscale:v",
        "5",
        output
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF8 output path"))?,
    ];

    debug!(?args, "ffmpeg preview sprite");
    let status = ffmpeg
        .background_ffmpeg()
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn ffmpeg ({})", ffmpeg.ffmpeg))?
        .wait_with_output()
        .await
        .context("await ffmpeg")?;
    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr).into_owned();
        bail!(
            "ffmpeg preview sprite failed ({}): {}",
            status.status,
            stderr.lines().rev().take(3).collect::<Vec<_>>().join(" / ")
        );
    }

    // tile_height: we know cols/rows from generation; re-probe the sprite
    // and divide its overall height by the known row count to get the
    // exact per-tile height (handles 16:9, 4:3, and tall sources alike).
    let tile_height = probe_tile_height(ffmpeg, output, rows).await?;

    Ok(SpriteInfo {
        path: output.to_path_buf(),
        interval_ms: i64::from(interval_s) * 1000,
        tile_width,
        tile_height,
        tile_cols: cols,
        tile_count: total_tiles,
    })
}

async fn probe_tile_height(
    ffmpeg: &FfmpegConfig,
    sprite: &Path,
    rows: u32,
) -> Result<u32> {
    let output = ffmpeg
        .background_ffprobe()
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
            sprite
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 sprite path"))?,
        ])
        .output()
        .await
        .context("spawn ffprobe for sprite dimensions")?;
    if !output.status.success() {
        bail!("ffprobe failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let _w: u32 = lines.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let h: u32 = lines.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if h == 0 || rows == 0 {
        bail!("ffprobe returned no usable sprite dimensions");
    }
    Ok(h / rows)
}

fn compute_tile_count(duration_ms: i64, interval_s: u32) -> u32 {
    let secs = (duration_ms / 1000) as u64;
    let interval = u64::from(interval_s.max(1));
    (secs / interval).max(1) as u32
}

fn grid_cols(total: u32) -> u32 {
    if total <= 1 {
        return 1;
    }
    let approx = (total as f64).sqrt().ceil() as u32;
    approx.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_count_short_clip() {
        // 30s clip at 10s interval = 3 tiles.
        assert_eq!(compute_tile_count(30_000, 10), 3);
    }

    #[test]
    fn tile_count_long_clip() {
        // 2-hour clip at 10s interval = 720 tiles.
        assert_eq!(compute_tile_count(7_200_000, 10), 720);
    }

    #[test]
    fn tile_count_rounds_down() {
        // 35s at 10s = 3 tiles (we drop the partial trailing interval).
        assert_eq!(compute_tile_count(35_000, 10), 3);
    }

    #[test]
    fn grid_is_roughly_square() {
        assert_eq!(grid_cols(1), 1);
        assert_eq!(grid_cols(9), 3);
        assert_eq!(grid_cols(720), 27); // ceil(sqrt(720)) = 27
    }
}
