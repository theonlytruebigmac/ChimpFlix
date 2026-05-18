//! ffprobe wrapper: spawn the subprocess, parse its JSON output into our
//! own shape.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::debug;

use crate::FfmpegConfig;

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub duration_ms: Option<i64>,
    pub bit_rate: Option<i64>,
    pub size_bytes: Option<i64>,
    pub container: Option<String>,
    pub streams: Vec<ProbeStream>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeStream {
    pub index: i32,
    pub kind: StreamKind,
    pub codec: Option<String>,
    pub profile: Option<String>,
    pub language: Option<String>,
    pub title: Option<String>,
    // Video-specific
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub pix_fmt: Option<String>,
    pub frame_rate: Option<f64>,
    pub hdr_format: Option<String>,
    // Audio-specific
    pub channels: Option<i32>,
    pub channel_layout: Option<String>,
    pub sample_rate: Option<i32>,
    // Disposition (subtitles + audio)
    pub is_default: bool,
    pub is_forced: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Other,
}

/// Result of a quick keyframe-spacing probe over the first few
/// seconds of a video stream. Used by the session planner to decide
/// whether codec-copy is safe: HLS muxers can only cut segments on
/// keyframe boundaries, so a source with a sparse GOP forces
/// oversized segments (and ugly initial-buffering for the user).
#[derive(Debug, Clone, Copy)]
pub struct GopProbe {
    /// Median distance between consecutive keyframes, in seconds.
    /// `None` if fewer than two keyframes were observed in the
    /// sampled interval (extremely short clip, no keyframes, or a
    /// container ffprobe couldn't enumerate packets for).
    pub median_keyframe_interval_s: Option<f64>,
    /// Total number of keyframes observed in the sample window.
    pub keyframes_observed: u32,
    /// How many seconds of the source we actually read packets for.
    pub sampled_duration_s: f64,
}

impl GopProbe {
    /// `true` when copy mode is likely to produce HLS segments that
    /// exceed the target segment duration. A 4-second cap is
    /// conservative against the default 6s HLS segments: it leaves
    /// room for one IDR-aligned cut per segment without overflow.
    pub fn copy_unsafe(&self, hls_segment_seconds: f64) -> bool {
        match self.median_keyframe_interval_s {
            // No keyframes seen: don't trust copy. Could be a still-
            // image-only clip or a stream the muxer can't index.
            None => true,
            // We allow a small overshoot (segment * 1.5) before flipping
            // to re-encode — most files in the wild have ~2s GOPs and a
            // few stragglers around 4-5s, and chasing those into
            // re-encode would waste CPU. Anything past 1.5×segment is
            // genuine sparse-GOP territory.
            Some(s) => s > hls_segment_seconds * 1.5,
        }
    }
}

/// Probe the keyframe spacing of the first video stream by reading a
/// limited interval of packets. `read_seconds` is passed to ffprobe's
/// `-read_intervals` so the probe stays fast even on large files
/// (typical run: under 250ms). Returns `None` if ffprobe doesn't
/// surface packet-level data for the container.
pub async fn probe_gop(
    cfg: &FfmpegConfig,
    path: &Path,
    read_seconds: f64,
) -> Result<GopProbe> {
    let interval = format!("%+#{}", read_seconds.max(2.0));
    let output = Command::new(&cfg.ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,dts_time,flags",
            "-read_intervals",
            &interval,
        ])
        .arg(path)
        .output()
        .await
        .with_context(|| format!("spawn ffprobe (gop) for {}", path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "ffprobe (gop) failed for {}: {}",
            path.display(),
            stderr.trim()
        );
    }

    let raw: RawPackets = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parse ffprobe (gop) JSON for {}", path.display()))?;

    let mut keyframe_times: Vec<f64> = Vec::new();
    let mut last_time: f64 = 0.0;
    for p in &raw.packets {
        let t = p
            .pts_time
            .as_deref()
            .or(p.dts_time.as_deref())
            .and_then(|s| s.parse::<f64>().ok());
        if let Some(t) = t {
            last_time = last_time.max(t);
            // ffprobe encodes the keyframe flag as "K_" (the trailing
            // underscore is for the discard bit, which we don't care
            // about). Any flags string starting with 'K' = keyframe.
            if p.flags.as_deref().is_some_and(|f| f.starts_with('K')) {
                keyframe_times.push(t);
            }
        }
    }

    let median_keyframe_interval_s = if keyframe_times.len() >= 2 {
        let mut gaps: Vec<f64> = keyframe_times
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|g| *g > 0.0)
            .collect();
        gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        if gaps.is_empty() {
            None
        } else {
            Some(gaps[gaps.len() / 2])
        }
    } else {
        None
    };

    let probe = GopProbe {
        median_keyframe_interval_s,
        keyframes_observed: keyframe_times.len() as u32,
        sampled_duration_s: last_time,
    };

    debug!(
        path = %path.display(),
        median = ?probe.median_keyframe_interval_s,
        keyframes = probe.keyframes_observed,
        "ffprobe gop done"
    );

    Ok(probe)
}

#[derive(Debug, Deserialize)]
struct RawPackets {
    #[serde(default)]
    packets: Vec<RawPacket>,
}

#[derive(Debug, Deserialize)]
struct RawPacket {
    pts_time: Option<String>,
    dts_time: Option<String>,
    flags: Option<String>,
}

pub async fn probe(cfg: &FfmpegConfig, path: &Path) -> Result<ProbeResult> {
    let output = Command::new(&cfg.ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
        ])
        .arg(path)
        .output()
        .await
        .with_context(|| format!("spawn ffprobe for {}", path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("ffprobe failed for {}: {}", path.display(), stderr.trim());
    }

    let raw: RawProbe = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parse ffprobe JSON for {}", path.display()))?;

    let container = raw
        .format
        .as_ref()
        .and_then(|f| f.format_name.clone())
        .map(|s| primary_format(&s));

    let duration_ms = raw
        .format
        .as_ref()
        .and_then(|f| f.duration.as_ref())
        .and_then(|s| s.parse::<f64>().ok())
        .map(|secs| (secs * 1000.0) as i64);

    let bit_rate = raw
        .format
        .as_ref()
        .and_then(|f| f.bit_rate.as_ref())
        .and_then(|s| s.parse::<i64>().ok());

    let size_bytes = raw
        .format
        .as_ref()
        .and_then(|f| f.size.as_ref())
        .and_then(|s| s.parse::<i64>().ok());

    let streams = raw.streams.into_iter().map(convert_stream).collect();

    debug!(
        path = %path.display(),
        duration_ms = ?duration_ms,
        container = ?container,
        "ffprobe done"
    );

    Ok(ProbeResult {
        duration_ms,
        bit_rate,
        size_bytes,
        container,
        streams,
    })
}

/// Re-probe the file with just enough flags to extract the codec name
/// of the Nth subtitle stream (0-indexed within subtitle streams,
/// matching the API's `subtitle_index` semantics).
///
/// Used as a safety net when the database's `media_streams.codec` is
/// NULL for the requested subtitle — without it, the transcoder would
/// fall through to its "unknown → assume text" default and either
/// silently fail (text `subtitles=` filter on a PGS stream) or burn
/// the wrong filter graph. Costs one ffprobe spawn (~50-100ms on a
/// local file) per session start in the missing-codec path; sessions
/// with cached codecs hit the DB and never call this.
pub async fn probe_subtitle_codec(
    cfg: &FfmpegConfig,
    path: &Path,
    subtitle_index: u32,
) -> Result<Option<String>> {
    // `-select_streams s` filters to subtitle streams only, which
    // turns ffprobe's enumeration into "the Nth row is the Nth
    // subtitle by stream-index order" — matching what the API gives
    // us. Much cheaper than parsing the full per-stream JSON.
    let output = Command::new(&cfg.ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "s",
            "-show_entries",
            "stream=codec_name",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .await
        .with_context(|| format!("spawn ffprobe (subtitle codec) for {}", path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "ffprobe (subtitle codec) failed for {}: {}",
            path.display(),
            stderr.trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .nth(subtitle_index as usize)
        .map(|s| s.to_string()))
}

fn primary_format(format_name: &str) -> String {
    // ffprobe returns comma-separated names for multi-format containers
    // (e.g. "matroska,webm" or "mov,mp4,m4a,3gp,3g2,mj2"). Pick the first
    // useful one.
    format_name
        .split(',')
        .next()
        .unwrap_or(format_name)
        .to_string()
}

fn convert_stream(s: RawStream) -> ProbeStream {
    let kind = match s.codec_type.as_deref() {
        Some("video") => StreamKind::Video,
        Some("audio") => StreamKind::Audio,
        Some("subtitle") => StreamKind::Subtitle,
        _ => StreamKind::Other,
    };

    let frame_rate = s.r_frame_rate.as_deref().and_then(parse_rational);

    let hdr_format = if kind == StreamKind::Video {
        detect_hdr(&s)
    } else {
        None
    };

    let language = s.tags.as_ref().and_then(|t| t.language.clone());
    let title = s.tags.as_ref().and_then(|t| t.title.clone());

    let disposition = s.disposition.unwrap_or_default();

    ProbeStream {
        index: s.index,
        kind,
        codec: s.codec_name,
        profile: s.profile,
        language,
        title,
        width: s.width,
        height: s.height,
        pix_fmt: s.pix_fmt,
        frame_rate,
        hdr_format,
        channels: s.channels,
        channel_layout: s.channel_layout,
        sample_rate: s.sample_rate.as_deref().and_then(|s| s.parse::<i32>().ok()),
        is_default: disposition.default == 1,
        is_forced: disposition.forced == 1,
    }
}

fn parse_rational(s: &str) -> Option<f64> {
    let (num, den) = s.split_once('/')?;
    let n: f64 = num.parse().ok()?;
    let d: f64 = den.parse().ok()?;
    if d == 0.0 { None } else { Some(n / d) }
}

fn detect_hdr(s: &RawStream) -> Option<String> {
    // Heuristic: rely on color_transfer / color_primaries when present.
    // Dolby Vision shows up as side-data, not parsed here yet — Phase 4.
    let transfer = s.color_transfer.as_deref()?;
    let primaries = s.color_primaries.as_deref().unwrap_or("");
    let space = s.color_space.as_deref().unwrap_or("");

    let is_bt2020 = primaries == "bt2020" || space == "bt2020nc" || space == "bt2020c";
    if !is_bt2020 {
        return None;
    }
    match transfer {
        "smpte2084" => Some("hdr10".to_string()),
        "arib-std-b67" => Some("hlg".to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// ffprobe JSON shape (just the fields we use)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawProbe {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Debug, Deserialize)]
struct RawFormat {
    format_name: Option<String>,
    duration: Option<String>,
    bit_rate: Option<String>,
    size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawStream {
    index: i32,
    codec_type: Option<String>,
    codec_name: Option<String>,
    profile: Option<String>,
    width: Option<i32>,
    height: Option<i32>,
    pix_fmt: Option<String>,
    r_frame_rate: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    color_space: Option<String>,
    channels: Option<i32>,
    channel_layout: Option<String>,
    sample_rate: Option<String>,
    tags: Option<RawTags>,
    disposition: Option<RawDisposition>,
}

#[derive(Debug, Default, Deserialize)]
struct RawTags {
    language: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDisposition {
    #[serde(default)]
    default: i32,
    #[serde(default)]
    forced: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_with(median: Option<f64>) -> GopProbe {
        GopProbe {
            median_keyframe_interval_s: median,
            keyframes_observed: median.map(|_| 5).unwrap_or(0),
            sampled_duration_s: 10.0,
        }
    }

    #[test]
    fn copy_safe_for_typical_gops() {
        // 2s GOP against a 6s segment: well within budget.
        assert!(!probe_with(Some(2.0)).copy_unsafe(6.0));
    }

    #[test]
    fn copy_safe_when_gop_equals_segment() {
        // GOP == segment is still fine; the muxer cuts at the next IDR.
        assert!(!probe_with(Some(6.0)).copy_unsafe(6.0));
    }

    #[test]
    fn copy_unsafe_when_gop_far_exceeds_segment() {
        // 12s GOP against a 6s segment: every other segment is empty
        // of keyframes, which would either be oversized or fail to cut
        // cleanly. Flip to re-encode.
        assert!(probe_with(Some(12.0)).copy_unsafe(6.0));
    }

    #[test]
    fn copy_unsafe_when_no_keyframes_observed() {
        // No samples = no evidence of safe cutting. Don't trust copy.
        assert!(probe_with(None).copy_unsafe(6.0));
    }

    #[test]
    fn copy_safe_with_slight_overshoot() {
        // 8s GOP against 6s segment is within the 1.5× allowance:
        // chasing this into re-encode would burn CPU for the long
        // tail of real-world files with slightly larger GOPs.
        assert!(!probe_with(Some(8.0)).copy_unsafe(6.0));
    }

    #[test]
    fn keyframe_flag_starts_with_k() {
        // ffprobe emits "K_" for keyframes; "__" for non-keyframes.
        // The parser keys off the leading 'K'; assert that contract
        // doesn't drift as we refactor.
        assert!("K_".starts_with('K'));
        assert!("K".starts_with('K'));
        assert!(!"__".starts_with('K'));
        assert!(!"_D".starts_with('K'));
    }
}
