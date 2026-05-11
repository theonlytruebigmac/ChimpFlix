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
