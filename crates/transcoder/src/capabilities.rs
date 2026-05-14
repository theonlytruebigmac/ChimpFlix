//! Detect ffmpeg's installed hwaccels + hardware encoders at startup so
//! the admin UI can offer only the options that will actually work.
//!
//! We invoke `ffmpeg -hide_banner -hwaccels` for the accel list and
//! `ffmpeg -hide_banner -encoders` for the encoder set, scanning for the
//! six h264/hevc hardware encoders shipped by upstream ffmpeg today.
//! Failures here are non-fatal — we just return an empty capability set
//! and the UI greys out the relevant options.

use serde::Serialize;
use std::collections::HashSet;

use crate::FfmpegConfig;

#[derive(Debug, Clone, Default, Serialize)]
pub struct TranscoderCapabilities {
    pub ffmpeg_version: Option<String>,
    pub hwaccels: Vec<String>,
    pub h264_encoders: Vec<String>,
    pub hevc_encoders: Vec<String>,
}

pub async fn detect_capabilities(cfg: &FfmpegConfig) -> TranscoderCapabilities {
    let mut caps = TranscoderCapabilities::default();
    caps.ffmpeg_version = ffmpeg_version(cfg).await;
    caps.hwaccels = ffmpeg_hwaccels(cfg).await;
    let encoders = ffmpeg_encoders(cfg).await;
    let h264_candidates = [
        "h264_vaapi",
        "h264_nvenc",
        "h264_qsv",
        "h264_videotoolbox",
        "h264_amf",
        "h264_v4l2m2m",
    ];
    let hevc_candidates = [
        "hevc_vaapi",
        "hevc_nvenc",
        "hevc_qsv",
        "hevc_videotoolbox",
        "hevc_amf",
        "hevc_v4l2m2m",
    ];
    caps.h264_encoders = h264_candidates
        .iter()
        .filter(|name| encoders.contains(**name))
        .map(|s| (*s).to_string())
        .collect();
    caps.hevc_encoders = hevc_candidates
        .iter()
        .filter(|name| encoders.contains(**name))
        .map(|s| (*s).to_string())
        .collect();
    caps
}

async fn ffmpeg_version(cfg: &FfmpegConfig) -> Option<String> {
    let out = tokio::process::Command::new(&cfg.ffmpeg)
        .args(["-hide_banner", "-version"])
        .output()
        .await
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines().next().map(|line| {
        // The first line is like "ffmpeg version 6.1.2 …" — trim noise to
        // the first whitespace-separated chunk after "version".
        line.split_whitespace()
            .nth(2)
            .unwrap_or(line)
            .to_string()
    })
}

async fn ffmpeg_hwaccels(cfg: &FfmpegConfig) -> Vec<String> {
    let Some(out) = tokio::process::Command::new(&cfg.ffmpeg)
        .args(["-hide_banner", "-hwaccels"])
        .output()
        .await
        .ok()
    else {
        return Vec::new();
    };
    let s = String::from_utf8_lossy(&out.stdout);
    // First line is a header ("Hardware acceleration methods:"); each
    // subsequent non-blank line is one method.
    s.lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

async fn ffmpeg_encoders(cfg: &FfmpegConfig) -> HashSet<String> {
    let Some(out) = tokio::process::Command::new(&cfg.ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .await
        .ok()
    else {
        return HashSet::new();
    };
    let s = String::from_utf8_lossy(&out.stdout);
    let mut set = HashSet::new();
    // Each encoder row looks like " V..... libx264              H.264 …"
    // — the second whitespace-separated token is the encoder name.
    for line in s.lines() {
        let mut it = line.split_whitespace();
        let flags = it.next();
        let Some(name) = it.next() else { continue };
        let Some(_) = flags else { continue };
        if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            set.insert(name.to_string());
        }
    }
    set
}
