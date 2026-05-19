//! Per-chapter thumbnail extraction.
//!
//! Plex's chapter-thumbs strip shows one frame per container chapter so
//! the seek menu has a poster for each act break. Built lazily by the
//! `generate_chapter_thumbs` scheduled task; lives on disk under
//! `<data_dir>/chapter_thumbs/<media_file_id>/<chapter_index>.jpg`.
//!
//! Distinct from BIF preview sprites (`previews.rs`) — sprites are a
//! fixed-cadence grid for hover-scrub, chapter thumbs are
//! variable-cadence and one-per-file with operator-curated titles.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tracing::debug;

use crate::FfmpegConfig;
use crate::probe::Chapter;

/// Default width for chapter thumbnails. Larger than scrub-sprite
/// tiles (240 px) since these render at full size in the chapter
/// menu, not as a tiny scrub overlay.
pub const DEFAULT_WIDTH: u32 = 320;

#[derive(Debug, Clone)]
pub struct ChapterThumbInfo {
    pub index: u32,
    pub path: PathBuf,
}

/// Extract a thumbnail for one chapter at `output`. The thumb is taken
/// from a point slightly past the chapter start (1.5s in by default) so
/// fade-ins from black don't yield a useless black frame.
pub async fn extract_chapter_thumb(
    ffmpeg: &FfmpegConfig,
    input: &Path,
    output: &Path,
    chapter: &Chapter,
    width: u32,
) -> Result<()> {
    // Skip 1.5s into the chapter; if the chapter is shorter than that,
    // sample at its midpoint instead so we don't seek past its end.
    let chapter_len_ms = (chapter.end_ms - chapter.start_ms).max(0);
    let offset_ms: i64 = if chapter_len_ms < 3_000 {
        chapter.start_ms + chapter_len_ms / 2
    } else {
        chapter.start_ms + 1_500
    };
    let seek_s = (offset_ms as f64) / 1000.0;

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create chapter thumb dir {}", parent.display()))?;
    }

    // `-ss` *before* `-i` triggers fast-seek to the nearest keyframe —
    // not pixel-accurate, but a chapter start is usually keyframe-
    // aligned by container convention, and "near-the-chapter" is fine
    // for a static poster. `-frames:v 1` extracts a single frame;
    // `scale=W:-2` keeps aspect ratio with an even-height constraint.
    let vf = format!("scale={width}:-2");
    let args = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-ss",
        &format!("{seek_s:.3}"),
        "-i",
    ];
    let status = ffmpeg
        .background_ffmpeg()
        .args(args)
        .arg(crate::safe_ffmpeg_input(input))
        .args(["-frames:v", "1", "-vf", &vf, "-q:v", "3"])
        .arg(output)
        .status()
        .await
        .with_context(|| {
            format!(
                "spawn ffmpeg (chapter thumb) input={} output={}",
                input.display(),
                output.display()
            )
        })?;
    if !status.success() {
        anyhow::bail!("ffmpeg chapter-thumb extraction exited non-zero for {}", input.display());
    }
    debug!(input = %input.display(), seek_s, output = %output.display(), "chapter thumb extracted");
    Ok(())
}

/// Build the on-disk path for a single chapter thumbnail under
/// `<root>/<media_file_id>/<chapter_index>.jpg`. Path-construction
/// helper kept here so the scheduler and the serve handler agree on
/// layout without duplicating the format string.
pub fn thumb_path(root: &Path, media_file_id: i64, chapter_index: u32) -> PathBuf {
    root.join(media_file_id.to_string())
        .join(format!("{chapter_index}.jpg"))
}
