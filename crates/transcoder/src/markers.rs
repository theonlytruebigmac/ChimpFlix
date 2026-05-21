//! Intro/credits detection.
//!
//! Two cooperating strategies:
//!
//! 1. **Chapter metadata** (best signal, ~free): ffprobe lists any
//!    `Chapter` entries embedded in the container. We name-match
//!    titles like "Intro" / "Opening" / "End Credits" / "Outro" to
//!    intro/credits markers. Authoritative when present — Bluray
//!    rips and well-mastered MKVs include these.
//!
//! 2. **`blackdetect` filter** (heuristic fallback): run ffmpeg in
//!    null-output mode with `blackdetect=d=1` to find every black
//!    sequence ≥ 1s. Bucket them:
//!      * "intro" — first sustained black run that occurs before 600s
//!        and lasts ≥ 2s (typical post-cold-open black before the
//!        title card).
//!      * "credits" — black run that begins within the last 8% of the
//!        duration (or final 60s, whichever is longer).
//!
//! Chapter-derived markers win over blackdetect for the same kind —
//! they're hand-curated. blackdetect fills in the gap for kinds the
//! chapters don't cover.
//!
//! Heuristic, not perfect — accurate enough to drive a "Skip Intro" /
//! "Skip Credits" button in the player. False positives are
//! unavoidable for movies with mid-film fades; the player treats
//! markers as soft hints.
//!
//! No state is persisted by this module — the caller writes detected
//! ranges into the `markers` table.

use std::path::Path;

use anyhow::{Context, Result};
use tracing::debug;

use crate::FfmpegConfig;
use crate::probe::{Chapter, probe_chapters};

/// blackdetect scan window per side (head + tail). The intro
/// classifier only considers runs within the first 10 minutes and
/// credits only within the last 10 minutes of the file, so decoding
/// the middle is pure waste. On a 2-hour movie this cuts the
/// blackdetect pass from ~120 minutes of decode work to ~20 — a 6×
/// speed-up. Files shorter than 2× this window get a single full-
/// file pass because head and tail would overlap anyway.
const SCAN_WINDOW_SECS: u64 = 600;
const SCAN_WINDOW_MS: i64 = (SCAN_WINDOW_SECS as i64) * 1000;

#[derive(Debug, Clone, PartialEq)]
pub struct DetectedMarker {
    pub kind: MarkerKind,
    /// Effective range used by the player. For chapter-derived intros
    /// this is anchored to 0 so the Skip Intro button advances past
    /// the cold open + title card; the original chapter sub-range
    /// lives in `signature_range`.
    pub start_ms: i64,
    pub end_ms: i64,
    /// Which detection strategy produced this marker. Drives the
    /// fingerprint auto-capture decision — only Chapter-source
    /// intros are trusted enough to seed a show's canonical theme
    /// signature without operator review.
    pub source: MarkerSource,
    /// Sub-range that contains the content signature (the actual
    /// intro audio without the cold-open prefix). When `Some`, the
    /// auto-capture path extracts the fingerprint from this slice
    /// rather than `[start_ms, end_ms]`; when `None`, the full
    /// marker range is used. Chapter-derived intros set this so
    /// the fingerprint is built from just the theme music, not
    /// the cold open that precedes it.
    pub signature_range: Option<(i64, i64)>,
}

/// Which detection strategy produced a `DetectedMarker`. Chapter
/// detection name-matches against ffprobe's chapter list — narrow
/// rules, high confidence. Blackdetect spots fade-to-black runs —
/// works on any source but produces noisier ranges. The distinction
/// matters for the fingerprint auto-capture path, which only trusts
/// chapter-derived intros to seed a show's canonical signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerSource {
    Chapter,
    BlackDetect,
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

/// Detect intro and credits markers on the given file. Combines
/// chapter-metadata and blackdetect strategies (see module docs).
/// `duration_ms` is needed for credits detection (relative to the
/// end of the file). When `None`, only intro detection is attempted.
///
/// Errors propagate from the underlying ffprobe/ffmpeg invocations.
/// Most failures (no chapters, no black runs) are *not* errors —
/// they just produce empty results for that strategy.
pub async fn detect_markers(
    cfg: &FfmpegConfig,
    path: &Path,
    duration_ms: Option<i64>,
) -> Result<Vec<DetectedMarker>> {
    // Chapter pass first — cheapest and highest signal. We tolerate
    // probe failure (chapter pass returning Err) by treating it as
    // "no chapters" and falling through to blackdetect, rather than
    // failing the whole detection — many containers ffprobe doesn't
    // know how to enumerate chapters for still encode just fine.
    let chapter_markers = match probe_chapters(cfg, path).await {
        Ok(chapters) => classify_chapters(&chapters, duration_ms),
        Err(e) => {
            debug!(path = %path.display(), error = %e, "chapter probe failed; falling back to blackdetect");
            Vec::new()
        }
    };

    let runs = scan_blackdetect(cfg, path, duration_ms).await?;
    debug!(
        path = %path.display(),
        runs = runs.len(),
        "blackdetect parsed runs"
    );
    let blackdetect_markers = classify(runs, duration_ms);

    Ok(merge_markers(chapter_markers, blackdetect_markers))
}

/// Run the blackdetect ffmpeg pass(es) and return the merged black
/// runs in absolute file timestamps. Long files get a head-only +
/// tail-only split so we don't decode the middle that the classifier
/// would discard anyway; short/unknown-duration files fall back to a
/// single full-file pass.
async fn scan_blackdetect(
    cfg: &FfmpegConfig,
    path: &Path,
    duration_ms: Option<i64>,
) -> Result<Vec<BlackRun>> {
    match duration_ms {
        Some(dur) if dur > 2 * SCAN_WINDOW_MS => {
            // Head: decode first 10 minutes from the start.
            let head = scan_blackdetect_range(cfg, path, 0, Some(SCAN_WINDOW_SECS)).await?;
            // Tail: seek to (duration - 10 min) and decode 10 minutes.
            // `-ss` before `-i` is a fast (keyframe) seek — the resulting
            // blackdetect timestamps are relative to the seek point,
            // so we add the offset to convert back to absolute file
            // time before merging.
            let tail_offset_secs = ((dur - SCAN_WINDOW_MS) / 1000).max(0) as u64;
            let mut tail =
                scan_blackdetect_range(cfg, path, tail_offset_secs, Some(SCAN_WINDOW_SECS)).await?;
            let offset_ms = (tail_offset_secs as i64) * 1000;
            for r in tail.iter_mut() {
                r.start_ms += offset_ms;
                r.end_ms += offset_ms;
            }
            let mut runs = head;
            runs.extend(tail);
            Ok(runs)
        }
        _ => {
            // Single pass for short/unknown-duration files. Head + tail
            // would overlap anyway and the extra fork would only save
            // milliseconds.
            scan_blackdetect_range(cfg, path, 0, None).await
        }
    }
}

/// Invoke ffmpeg's blackdetect filter over a single range and parse
/// its stderr. `start_secs` becomes `-ss <N>` before `-i` (fast,
/// keyframe-accurate seek); `duration_secs` becomes `-t <N>` (decode
/// at most N seconds). When both are None/0 the whole file is
/// scanned. Returned timestamps are relative to `start_secs` — the
/// caller is responsible for translating them to absolute file time.
async fn scan_blackdetect_range(
    cfg: &FfmpegConfig,
    path: &Path,
    start_secs: u64,
    duration_secs: Option<u64>,
) -> Result<Vec<BlackRun>> {
    debug!(
        path = %path.display(),
        start_secs,
        duration_secs = ?duration_secs,
        "blackdetect start"
    );
    let mut cmd = cfg.background_ffmpeg();
    cmd.args(["-hide_banner", "-nostats", "-loglevel", "info"]);
    if start_secs > 0 {
        cmd.args(["-ss", &start_secs.to_string()]);
    }
    cmd.arg("-i")
        // file: prefix prevents a filename starting with `-` from being
        // parsed as a flag (see crate::safe_ffmpeg_input).
        .arg(crate::safe_ffmpeg_input(path));
    if let Some(d) = duration_secs {
        cmd.args(["-t", &d.to_string()]);
    }
    let output = cmd
        // d=1 → minimum 1s of black; pix_th low so fades are picked up.
        .args(["-vf", "blackdetect=d=1:pix_th=0.10"])
        .args(["-an", "-sn", "-f", "null"])
        .arg("-")
        .output()
        .await
        .with_context(|| format!("spawn ffmpeg blackdetect for {}", path.display()))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(parse_black_runs(&stderr))
}

/// Map chapter entries onto intro/credits markers via case-insensitive
/// title matching. The rules below are intentionally narrow — false
/// positives are worse than no marker (skip-intro yanking the user
/// into the wrong scene erodes trust).
///
/// - `intro` — chapter whose title contains "intro", "opening",
///   "opening credits", or "opening theme", AND starts in the first
///   600s of the file (most cold-open intros land there).
/// - `credits` — chapter whose title contains "credits", "end credits",
///   "outro", or "closing credits", AND starts in the last 30% of the
///   file (rules out musical numbers titled "credits" in the middle).
pub(crate) fn classify_chapters(
    chapters: &[Chapter],
    duration_ms: Option<i64>,
) -> Vec<DetectedMarker> {
    let mut out = Vec::new();
    let intro_cutoff = 600_000_i64;
    // For credits, prefer the last 30% of the file. When duration is
    // unknown, fall back to "any chapter whose title matches".
    let credits_threshold = duration_ms.map(|d| (d as f64 * 0.7) as i64);
    for ch in chapters {
        let Some(title) = ch.title.as_deref() else {
            continue;
        };
        let lower = title.to_ascii_lowercase();
        let is_intro =
            (lower.contains("opening") || lower.contains("intro")) && ch.start_ms <= intro_cutoff;
        let is_credits = (lower.contains("end credits")
            || lower.contains("closing credit")
            || lower == "credits"
            || lower.contains("outro"))
            && credits_threshold.is_none_or(|c| ch.start_ms >= c);
        if is_intro {
            out.push(DetectedMarker {
                kind: MarkerKind::Intro,
                // Anchor from 0 so the skip button advances past the
                // cold open + title card, not just the chapter mark.
                start_ms: 0,
                end_ms: ch.end_ms,
                source: MarkerSource::Chapter,
                // The chapter's actual range carries just the theme
                // song, suitable for fingerprint capture.
                signature_range: Some((ch.start_ms, ch.end_ms)),
            });
        } else if is_credits {
            let end = duration_ms.unwrap_or(ch.end_ms);
            out.push(DetectedMarker {
                kind: MarkerKind::Credits,
                start_ms: ch.start_ms,
                end_ms: end,
                source: MarkerSource::Chapter,
                signature_range: None,
            });
        }
    }
    out
}

/// Combine chapter-derived markers with blackdetect-derived markers.
/// Chapter wins for any kind it covers — the title-match heuristic
/// is more reliable than fade-to-black timing. The blackdetect ones
/// fill in kinds the chapter list didn't cover.
pub(crate) fn merge_markers(
    chapter: Vec<DetectedMarker>,
    blackdetect: Vec<DetectedMarker>,
) -> Vec<DetectedMarker> {
    let mut out = chapter;
    for m in blackdetect {
        if out.iter().any(|c| c.kind == m.kind) {
            continue;
        }
        out.push(m);
    }
    // Stable order keeps the consuming UI predictable — intros before
    // credits, then by start_ms within each kind.
    out.sort_by(|a, b| {
        a.kind
            .as_str()
            .cmp(b.kind.as_str())
            .then(a.start_ms.cmp(&b.start_ms))
    });
    out
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
            source: MarkerSource::BlackDetect,
            // Blackdetect produces a fade range, not the intro audio
            // itself — signature is unknown, so leave `None` and the
            // fingerprint auto-capture path skips this marker.
            signature_range: None,
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
                source: MarkerSource::BlackDetect,
                signature_range: None,
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

    #[test]
    fn classify_chapters_matches_intro_title() {
        let chapters = vec![
            Chapter {
                start_ms: 0,
                end_ms: 90_000,
                title: Some("Opening Credits".into()),
            },
            Chapter {
                start_ms: 90_000,
                end_ms: 1_200_000,
                title: Some("Episode".into()),
            },
        ];
        let out = classify_chapters(&chapters, Some(1_300_000));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MarkerKind::Intro);
        assert_eq!(out[0].start_ms, 0);
        assert_eq!(out[0].end_ms, 90_000);
    }

    #[test]
    fn classify_chapters_matches_credits_title() {
        let chapters = vec![
            Chapter {
                start_ms: 0,
                end_ms: 60_000,
                title: Some("Cold Open".into()),
            },
            Chapter {
                start_ms: 60_000,
                end_ms: 1_700_000,
                title: Some("Main Show".into()),
            },
            Chapter {
                start_ms: 1_700_000,
                end_ms: 1_800_000,
                title: Some("End Credits".into()),
            },
        ];
        let out = classify_chapters(&chapters, Some(1_800_000));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MarkerKind::Credits);
        assert_eq!(out[0].start_ms, 1_700_000);
        assert_eq!(out[0].end_ms, 1_800_000);
    }

    #[test]
    fn classify_chapters_rejects_early_credits_chapter() {
        // A chapter named "Credits" in the first 70% of the file is
        // probably the cast intro for a documentary, not the
        // post-show roll. Reject so we don't yank the player.
        let chapters = vec![Chapter {
            start_ms: 30_000,
            end_ms: 60_000,
            title: Some("Credits".into()),
        }];
        let out = classify_chapters(&chapters, Some(1_800_000));
        assert!(out.is_empty(), "got {out:?}");
    }

    #[test]
    fn classify_chapters_ignores_untitled() {
        let chapters = vec![Chapter {
            start_ms: 0,
            end_ms: 60_000,
            title: None,
        }];
        let out = classify_chapters(&chapters, Some(1_800_000));
        assert!(out.is_empty());
    }

    #[test]
    fn merge_prefers_chapter_over_blackdetect_for_same_kind() {
        let chapter = vec![DetectedMarker {
            kind: MarkerKind::Intro,
            start_ms: 0,
            end_ms: 90_000,
            source: MarkerSource::Chapter,
            signature_range: Some((30_000, 90_000)),
        }];
        let blackdetect = vec![DetectedMarker {
            kind: MarkerKind::Intro,
            start_ms: 0,
            end_ms: 35_000,
            source: MarkerSource::BlackDetect,
            signature_range: None,
        }];
        let out = merge_markers(chapter.clone(), blackdetect);
        assert_eq!(out.len(), 1);
        // Chapter-derived value wins.
        assert_eq!(out[0].end_ms, 90_000);
        assert_eq!(out[0].source, MarkerSource::Chapter);
        assert_eq!(out[0].signature_range, Some((30_000, 90_000)));
    }

    #[test]
    fn merge_fills_in_kinds_chapters_missed() {
        let chapter = vec![DetectedMarker {
            kind: MarkerKind::Intro,
            start_ms: 0,
            end_ms: 90_000,
            source: MarkerSource::Chapter,
            signature_range: Some((30_000, 90_000)),
        }];
        let blackdetect = vec![DetectedMarker {
            kind: MarkerKind::Credits,
            start_ms: 1_700_000,
            end_ms: 1_800_000,
            source: MarkerSource::BlackDetect,
            signature_range: None,
        }];
        let out = merge_markers(chapter, blackdetect);
        assert_eq!(out.len(), 2);
        // Sorted: credits ("credits" < "intro" alphabetically).
        assert_eq!(out[0].kind, MarkerKind::Credits);
        assert_eq!(out[0].source, MarkerSource::BlackDetect);
        assert_eq!(out[1].kind, MarkerKind::Intro);
        assert_eq!(out[1].source, MarkerSource::Chapter);
    }

    #[test]
    fn classify_chapters_sets_signature_range_to_chapter_bounds() {
        let chapters = vec![Chapter {
            start_ms: 12_000,
            end_ms: 90_000,
            title: Some("Opening Credits".into()),
        }];
        let out = classify_chapters(&chapters, Some(1_800_000));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, MarkerKind::Intro);
        // Effective range anchored to 0 (skip button advances past
        // cold open), but signature range carries the actual theme.
        assert_eq!(out[0].start_ms, 0);
        assert_eq!(out[0].end_ms, 90_000);
        assert_eq!(out[0].signature_range, Some((12_000, 90_000)));
        assert_eq!(out[0].source, MarkerSource::Chapter);
    }

    #[test]
    fn blackdetect_intro_leaves_signature_range_unset() {
        // The blackdetect classifier produces an intro from a black
        // run, but the actual theme audio isn't bounded by the fade
        // edges. Signature stays None so the auto-capture path
        // refuses to seed a fingerprint from it.
        let runs = vec![BlackRun {
            start_ms: 60_000,
            end_ms: 65_000,
        }];
        let out = classify(runs, Some(1_800_000));
        let intro = out.iter().find(|m| m.kind == MarkerKind::Intro).unwrap();
        assert_eq!(intro.source, MarkerSource::BlackDetect);
        assert_eq!(intro.signature_range, None);
    }
}
