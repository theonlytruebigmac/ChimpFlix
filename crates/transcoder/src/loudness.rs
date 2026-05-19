//! EBU R 128 loudness measurement via ffmpeg's `loudnorm` filter.
//!
//! Runs ffmpeg in pass-1 print mode — the filter analyses the whole
//! audio stream and writes a JSON summary to stderr. We parse the JSON
//! out and return the four scalars the second-pass loudnorm needs to
//! produce a precisely-normalised stream.
//!
//! One pass takes roughly `0.05 * duration_s` on a typical x86 box for
//! stereo AAC (audio-only decode + filter, no encode). A 45-minute
//! episode is ~2 minutes of wall clock.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::FfmpegConfig;

#[derive(Debug, Clone, Copy)]
pub struct LoudnessMeasurement {
    /// Integrated loudness, LUFS.
    pub integrated: f64,
    /// True peak, dBTP.
    pub true_peak: f64,
    /// Loudness range, LU.
    pub lra: f64,
    /// Noise floor / threshold, LUFS.
    pub threshold: f64,
}

#[derive(Debug, Deserialize)]
struct LoudnormJson {
    input_i: String,
    input_tp: String,
    input_lra: String,
    input_thresh: String,
}

/// Measure the integrated loudness of the first audio stream in
/// `input`. Returns `None` when the file has no audio or the filter
/// can't produce useful numbers (e.g. clipped input that scores `-inf`).
pub async fn measure(
    cfg: &FfmpegConfig,
    input: &Path,
) -> Result<Option<LoudnessMeasurement>> {
    // -vn drops video, -sn drops subs, -dn drops data — we only need
    // audio. The loudnorm filter in print mode emits JSON to stderr
    // and a null muxer keeps the encoder from doing work.
    let output = cfg
        .background_ffmpeg()
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(input)
        .args([
            "-vn",
            "-sn",
            "-dn",
            "-map",
            "0:a:0?",
            "-af",
            "loudnorm=print_format=json",
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .with_context(|| format!("spawn ffmpeg loudnorm for {}", input.display()))?;

    if !output.status.success() {
        // No audio stream is a benign result, not an error — return
        // None and let the caller stamp the file as analysed.
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Stream map '0:a:0?' matches no streams")
            || stderr.contains("does not contain any stream")
        {
            return Ok(None);
        }
        anyhow::bail!(
            "ffmpeg loudnorm exited non-zero for {}: {}",
            input.display(),
            stderr.trim()
        );
    }

    let stderr = String::from_utf8(output.stderr)
        .context("ffmpeg loudnorm stderr was not utf-8")?;
    let json = extract_json_block(&stderr).ok_or_else(|| {
        anyhow!(
            "no loudnorm JSON block in ffmpeg stderr for {}",
            input.display()
        )
    })?;
    let parsed: LoudnormJson = serde_json::from_str(json)
        .with_context(|| format!("parse loudnorm JSON for {}", input.display()))?;

    // ffmpeg prints "-inf" for completely silent inputs — treat as no
    // measurement so callers can decide whether to skip or default.
    let integrated = parse_lufs(&parsed.input_i)?;
    let true_peak = parse_lufs(&parsed.input_tp)?;
    let lra = parse_lufs(&parsed.input_lra)?;
    let threshold = parse_lufs(&parsed.input_thresh)?;
    if !integrated.is_finite() || !true_peak.is_finite() {
        return Ok(None);
    }
    Ok(Some(LoudnessMeasurement {
        integrated,
        true_peak,
        lra,
        threshold,
    }))
}

/// Find the `{ ... }` block in the stderr that loudnorm emits. The
/// preceding banner text is fixed-format ("[Parsed_loudnorm_0 @ ...]")
/// but the JSON itself is the only `{` in the output, so we slice
/// from the first `{` to the matching `}`.
fn extract_json_block(stderr: &str) -> Option<&str> {
    let start = stderr.find('{')?;
    let mut depth = 0i32;
    for (i, c) in stderr[start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&stderr[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_lufs(s: &str) -> Result<f64> {
    let trimmed = s.trim();
    if trimmed == "-inf" || trimmed == "inf" {
        return Ok(f64::NAN);
    }
    trimmed
        .parse::<f64>()
        .with_context(|| format!("parse LUFS value `{trimmed}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_simple_json_block() {
        let stderr = "[Parsed_loudnorm_0 @ 0x123] \n\
            {\n  \"input_i\" : \"-19.50\",\n  \"input_tp\" : \"-2.10\"\n}\n\
            size=N/A time=00:00:42.10";
        let block = extract_json_block(stderr).expect("should find JSON");
        assert!(block.contains("input_i"));
        assert!(block.ends_with('}'));
    }

    #[test]
    fn parses_negative_lufs() {
        assert!((parse_lufs("-19.50").unwrap() - (-19.50)).abs() < 1e-9);
    }

    #[test]
    fn parses_inf_as_nan() {
        assert!(parse_lufs("-inf").unwrap().is_nan());
    }
}
