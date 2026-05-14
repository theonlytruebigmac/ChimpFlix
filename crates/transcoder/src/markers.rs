//! Intro/credits detection via ffmpeg's `blackdetect` filter.
//!
//! Strategy: run ffmpeg in null-output mode with `blackdetect=d=1` to find
//! every black sequence >= 1s. Parse the stderr log for the
//! `black_start/black_end/black_duration` triples it emits. We then bucket
//! the runs:
//!
//!   * "intro" — first sustained black run that occurs before 600s and
//!     lasts >= 2s (typical post-cold-open black before the title card).
//!   * "credits" — black run that begins within the last 8% of the
//!     duration (or final 60s, whichever is longer).
//!
//! This is heuristic, not chapter metadata — it's accurate enough to
//! drive a "Skip Intro" / "Skip Credits" button in the player. False
//! positives are unavoidable for movies with mid-film fades; the player
//! treats markers as soft hints.
//!
//! No state is persisted by this module — the caller writes detected
//! ranges into the `markers` table.

use std::path::Path;

use anyhow::{Context, Result};
use tokio::process::Command;
use tracing::debug;

use crate::FfmpegConfig;

#[derive(Debug, Clone, PartialEq)]
pub struct DetectedMarker {
    pub kind: MarkerKind,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerKind {
    Intro,
    Credits,
}

impl MarkerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Intro => "intro",
            Self::Credits => "credits",
        }
    }
}

/// Run ffmpeg blackdetect on the given file and classify the runs.
/// `duration_ms` is needed for credits detection (relative to the end of
/// the file). When `None`, only intro detection is attempted.
pub async fn detect_markers(
    cfg: &FfmpegConfig,
    path: &Path,
    duration_ms: Option<i64>,
) -> Result<Vec<DetectedMarker>> {
    debug!(path = %path.display(), "blackdetect start");
    let output = Command::new(&cfg.ffmpeg)
        .args(["-hide_banner", "-nostats", "-loglevel", "info"])
        .arg("-i")
        .arg(path)
        // d=1 → minimum 1s of black; pix_th low so fades are picked up.
        .args(["-vf", "blackdetect=d=1:pix_th=0.10"])
        .args(["-an", "-sn", "-f", "null"])
        .arg("-")
        .output()
        .await
        .with_context(|| format!("spawn ffmpeg blackdetect for {}", path.display()))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let runs = parse_black_runs(&stderr);
    debug!(
        path = %path.display(),
        runs = runs.len(),
        "blackdetect parsed runs"
    );
    Ok(classify(runs, duration_ms))
}

#[derive(Debug, Clone, Copy)]
struct BlackRun {
    start_ms: i64,
    end_ms: i64,
}

fn parse_black_runs(stderr: &str) -> Vec<BlackRun> {
    let mut out = Vec::new();
    for line in stderr.lines() {
        // Format: "[blackdetect @ 0x...] black_start:N black_end:M black_duration:D"
        let Some(rest) = line.split_once("black_start:") else {
            continue;
        };
        let rest = rest.1;
        let mut iter = rest.split_whitespace();
        let start = iter.next().and_then(|s| s.parse::<f64>().ok());
        let end = iter
            .next()
            .and_then(|s| s.strip_prefix("black_end:"))
            .and_then(|s| s.parse::<f64>().ok());
        if let (Some(s), Some(e)) = (start, end) {
            if e > s {
                out.push(BlackRun {
                    start_ms: (s * 1000.0) as i64,
                    end_ms: (e * 1000.0) as i64,
                });
            }
        }
    }
    out
}

fn classify(runs: Vec<BlackRun>, duration_ms: Option<i64>) -> Vec<DetectedMarker> {
    let mut out = Vec::new();

    // Intro: first black run that starts before 600s and is at least 2s.
    // We mark from 0 to the run's end so the "skip intro" button advances
    // the user past the title card, not just to the start of black.
    if let Some(run) = runs
        .iter()
        .find(|r| r.start_ms <= 600_000 && (r.end_ms - r.start_ms) >= 2_000)
    {
        out.push(DetectedMarker {
            kind: MarkerKind::Intro,
            start_ms: 0,
            end_ms: run.end_ms,
        });
    }

    // Credits: black run beginning within max(last 8%, last 60s) of the
    // file. The run's start_ms is the "you can skip from here" point.
    if let Some(dur) = duration_ms {
        let cutoff = dur - (dur / 12).max(60_000);
        if let Some(run) = runs.iter().find(|r| r.start_ms >= cutoff) {
            out.push(DetectedMarker {
                kind: MarkerKind::Credits,
                start_ms: run.start_ms,
                end_ms: dur,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blackdetect_output() {
        let s = "\
[blackdetect @ 0x7f8a] black_start:12.345 black_end:14.555 black_duration:2.21
[blackdetect @ 0x7f8a] black_start:1200.000 black_end:1208.000 black_duration:8.0
some unrelated log line
[blackdetect @ 0x7f8a] black_start:3550.0 black_end:3590.5 black_duration:40.5
";
        let runs = parse_black_runs(s);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].start_ms, 12_345);
        assert_eq!(runs[2].end_ms, 3_590_500);
    }

    #[test]
    fn classifies_intro_and_credits() {
        let runs = vec![
            BlackRun {
                start_ms: 30_000,
                end_ms: 35_000,
            },
            BlackRun {
                start_ms: 1_800_000,
                end_ms: 1_810_000,
            },
        ];
        let out = classify(runs, Some(1_900_000));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, MarkerKind::Intro);
        assert_eq!(out[0].end_ms, 35_000);
        assert_eq!(out[1].kind, MarkerKind::Credits);
        assert_eq!(out[1].start_ms, 1_800_000);
    }

    #[test]
    fn no_markers_for_quiet_file() {
        let out = classify(vec![], Some(60 * 60 * 1000));
        assert!(out.is_empty());
    }
}
