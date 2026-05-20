//! Chromaprint-based audio fingerprinting for the intro detector.
//!
//! Two cooperating operations:
//!
//! 1. **Extract** — pipe a slice of a media file's audio through
//!    ffmpeg (PCM s16le mono @ 11025 Hz) and feed it to chromaprint's
//!    `Fingerprinter`. Returns the resulting `Vec<u32>` — each u32
//!    represents roughly 124 ms of audio (4096-sample frames stepping
//!    1365 at 11025 Hz, see the `chromaprint` paper for the why).
//!
//! 2. **Match** — slide a reference fingerprint across a target
//!    fingerprint, computing the Hamming distance (XOR + popcount) at
//!    each offset. The best-scoring offset is the match position, in
//!    frames. Convert to milliseconds via `FRAME_STEP_MS`.
//!
//! The intro detector uses this pair as follows:
//!
//!   * Capture: when an operator saves a manual intro marker, we
//!     extract the fingerprint of that exact audio range and store
//!     it as the show's canonical intro signature.
//!   * Match: when `detect_markers` runs on another episode of the
//!     same show, we extract the fingerprint of the first ~10 min and
//!     search for the canonical signature inside it. A successful
//!     match anchors the new intro marker; failure falls back to the
//!     existing blackdetect/chapter pipeline.
//!
//! No state is owned by this module — the caller persists fingerprints
//! and decides when to call extract vs match.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use rusty_chromaprint::{Configuration, Fingerprinter};
use tokio::io::AsyncReadExt;
use tracing::debug;

use crate::FfmpegConfig;

/// Sample rate chromaprint expects. The Fingerprinter is also tunable
/// but every preset in the upstream lib is calibrated at this rate;
/// resample-at-extraction is cheaper than running a higher SR through
/// the algorithm.
pub const FINGERPRINT_SAMPLE_RATE: u32 = 11025;

/// Per-frame step in ms — i.e. how much real time advances per u32
/// hash in the resulting fingerprint. With `Configuration::preset_test1`
/// (the upstream default) frames are 4096 samples wide and step 1365
/// samples each, so at 11025 Hz the step is 1365/11025 ≈ 123.81 ms.
/// Codified as an integer for arithmetic; the small rounding error is
/// far below the threshold of human-visible skip-button placement.
pub const FRAME_STEP_MS: i64 = 124;

/// Default scan length when looking for an intro signature in a new
/// file — 10 minutes covers cold-opens + intro reliably.
pub const DEFAULT_MATCH_WINDOW_MS: i64 = 10 * 60 * 1000;

/// Confidence threshold for `match_fingerprint`. The algorithm emits
/// average bits flipped per 32-bit frame, so 0..=32. The chromaprint
/// paper suggests <14 is a confident match; we use 14.4 (45% of bits)
/// as a conservative ceiling. Tunable from match call site.
pub const DEFAULT_MATCH_THRESHOLD: f64 = 14.4;

/// Extract a fingerprint from `path`'s audio between `start_ms` and
/// `start_ms + duration_ms`. Spawns one ffmpeg subprocess that:
///   * seeks to `start_ms` with `-ss` (fast, demuxer-level)
///   * caps the read to `duration_ms` with `-t`
///   * downmixes to mono + resamples to FINGERPRINT_SAMPLE_RATE
///   * emits raw PCM s16le on stdout
///
/// We pipe stdout directly into the chromaprint Fingerprinter instead
/// of writing a temp WAV — the audio buffer is small (60s × 11025 Hz
/// × 2 bytes ≈ 1.3 MB) and a temp file adds disk IO + cleanup.
pub async fn extract_fingerprint(
    cfg: &FfmpegConfig,
    path: &Path,
    start_ms: i64,
    duration_ms: i64,
) -> Result<Vec<u32>> {
    if duration_ms <= 0 {
        return Err(anyhow!(
            "extract_fingerprint: duration_ms must be > 0 (got {})",
            duration_ms,
        ));
    }
    let start_secs = (start_ms as f64) / 1000.0;
    let duration_secs = (duration_ms as f64) / 1000.0;
    debug!(
        path = %path.display(),
        start_ms,
        duration_ms,
        "fingerprint extract"
    );

    let mut child = cfg
        .background_ffmpeg()
        .args(["-hide_banner", "-nostats", "-loglevel", "error"])
        // Seek before -i: demuxer-level seek is orders of magnitude
        // faster than the post-decode `-ss` placement and accurate
        // enough for our 10-minute scan windows.
        .args(["-ss", &format!("{start_secs:.3}")])
        .args(["-t", &format!("{duration_secs:.3}")])
        .arg("-i")
        .arg(crate::safe_ffmpeg_input(path))
        // No video, no subtitles — fingerprinting is audio only.
        .args(["-vn", "-sn"])
        // Resample + downmix in one filter to avoid two filterchain
        // passes; `aresample` is the standard ffmpeg path for both.
        .args(["-ar", &FINGERPRINT_SAMPLE_RATE.to_string()])
        .args(["-ac", "1"])
        .args(["-f", "s16le", "-acodec", "pcm_s16le"])
        .arg("pipe:1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn ffmpeg audio extract for {}", path.display()))?;

    // Drain stdout into a buffer. We don't try to stream into
    // chromaprint frame-by-frame because the Fingerprinter wants its
    // samples in chunks at most a frame wide, and the all-at-once
    // approach simplifies error handling. Memory cap is the
    // 60s×11kHz×2B ≈ 1.3MB note above; the longest manual intro range
    // we'd accept (30 min) lands at ~40 MB, still fine.
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut bytes = Vec::with_capacity(
        ((duration_secs * FINGERPRINT_SAMPLE_RATE as f64 * 2.0) as usize).min(64 * 1024 * 1024),
    );
    stdout
        .read_to_end(&mut bytes)
        .await
        .with_context(|| "read ffmpeg pcm stdout")?;

    let status = child
        .wait()
        .await
        .with_context(|| "wait ffmpeg audio extract")?;
    if !status.success() {
        return Err(anyhow!(
            "ffmpeg audio extract failed (status {}): truncated or unreadable input",
            status,
        ));
    }
    if bytes.is_empty() {
        return Err(anyhow!(
            "ffmpeg produced no audio bytes — file may have no audio stream in range",
        ));
    }

    // Reinterpret as little-endian i16 samples. We control the format
    // via `-f s16le` so endianness is fixed.
    let samples: Vec<i16> = bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    fingerprint_samples(&samples)
}

/// Lower-level: feed a buffer of i16 mono samples (at
/// [`FINGERPRINT_SAMPLE_RATE`]) to chromaprint and return the
/// fingerprint. Exposed for unit tests; production callers use
/// [`extract_fingerprint`].
pub fn fingerprint_samples(samples: &[i16]) -> Result<Vec<u32>> {
    let config = Configuration::preset_test1();
    let mut printer = Fingerprinter::new(&config);
    printer
        .start(FINGERPRINT_SAMPLE_RATE, 1)
        .map_err(|e| anyhow!("chromaprint start: {e}"))?;
    printer.consume(samples);
    printer.finish();
    Ok(printer.fingerprint().to_vec())
}

/// Result of a successful fingerprint search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FingerprintMatch {
    /// Match position in the target audio, in milliseconds.
    pub start_ms: i64,
    /// Average bits flipped per 32-bit frame across the whole
    /// reference window. Lower = better. 0 is identical; ≥16 is noise.
    pub score: f64,
}

/// Slide `reference` across `target` and return the offset (in
/// target-frame units → milliseconds) where the Hamming distance is
/// lowest, provided the average distance is below `threshold`.
///
/// Returns `None` if either fingerprint is shorter than the other, or
/// if no offset clears the threshold. The default threshold is at
/// [`DEFAULT_MATCH_THRESHOLD`].
pub fn match_fingerprint(
    reference: &[u32],
    target: &[u32],
    threshold: f64,
) -> Option<FingerprintMatch> {
    if reference.is_empty() || target.len() < reference.len() {
        return None;
    }
    let ref_len = reference.len();
    let max_offset = target.len() - ref_len;
    let mut best_offset = 0usize;
    // Initialise to "worse than anything possible" so the first
    // candidate always wins.
    let mut best_total_distance: u64 = u64::MAX;
    for offset in 0..=max_offset {
        let mut distance: u64 = 0;
        for i in 0..ref_len {
            // popcount of XOR = number of bits that differ — the
            // chromaprint paper's prescribed similarity metric.
            distance += (reference[i] ^ target[offset + i]).count_ones() as u64;
        }
        if distance < best_total_distance {
            best_total_distance = distance;
            best_offset = offset;
        }
    }
    let avg_per_frame = best_total_distance as f64 / ref_len as f64;
    if avg_per_frame > threshold {
        return None;
    }
    Some(FingerprintMatch {
        start_ms: best_offset as i64 * FRAME_STEP_MS,
        score: avg_per_frame,
    })
}

/// Encode a fingerprint Vec<u32> as the BLOB layout we persist:
/// concatenated little-endian u32s. Symmetric with [`decode_blob`].
pub fn encode_blob(fp: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(fp.len() * 4);
    for v in fp {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode a stored fingerprint BLOB back into a Vec<u32>. Returns an
/// error if the byte length isn't a multiple of 4 (corrupt row).
pub fn decode_blob(bytes: &[u8]) -> Result<Vec<u32>> {
    if !bytes.len().is_multiple_of(4) {
        return Err(anyhow!(
            "fingerprint blob length {} is not a multiple of 4",
            bytes.len(),
        ));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ref_fp() -> Vec<u32> {
        // 8 frames. Specific bit patterns aren't important — only
        // that they're distinct so the search can localize them.
        vec![
            0x1234_5678, 0xDEAD_BEEF, 0xCAFE_BABE, 0xFEED_FACE, 0x0BAD_F00D, 0x8BAD_F00D,
            0xABAD_BABE, 0x1337_0042,
        ]
    }

    #[test]
    fn match_finds_exact_offset_zero() {
        let r = ref_fp();
        let target = r.clone();
        let m = match_fingerprint(&r, &target, DEFAULT_MATCH_THRESHOLD).unwrap();
        assert_eq!(m.start_ms, 0);
        assert_eq!(m.score, 0.0);
    }

    #[test]
    fn match_finds_exact_offset_n() {
        let r = ref_fp();
        let mut target = vec![0u32; 5];
        target.extend_from_slice(&r);
        target.extend_from_slice(&[0u32; 3]);
        let m = match_fingerprint(&r, &target, DEFAULT_MATCH_THRESHOLD).unwrap();
        assert_eq!(m.start_ms, 5 * FRAME_STEP_MS);
        assert_eq!(m.score, 0.0);
    }

    #[test]
    fn match_rejects_when_above_threshold() {
        let r = ref_fp();
        // Build a target that's all-zeros where r has high bit counts —
        // average distance will be high.
        let mut target = vec![0u32; 5];
        target.extend_from_slice(&vec![0u32; r.len()]); // zeros — bad match for r
        target.extend_from_slice(&[0u32; 3]);
        // Force a strict threshold so the noise can't sneak in.
        let m = match_fingerprint(&r, &target, 2.0);
        assert!(m.is_none(), "expected None, got {m:?}");
    }

    #[test]
    fn match_empty_inputs_return_none() {
        let r: Vec<u32> = vec![];
        let target = ref_fp();
        assert!(match_fingerprint(&r, &target, DEFAULT_MATCH_THRESHOLD).is_none());

        let r = ref_fp();
        let target: Vec<u32> = vec![];
        assert!(match_fingerprint(&r, &target, DEFAULT_MATCH_THRESHOLD).is_none());
    }

    #[test]
    fn match_target_shorter_than_reference_is_none() {
        let r = ref_fp();
        let target = vec![0u32; r.len() - 1];
        assert!(match_fingerprint(&r, &target, DEFAULT_MATCH_THRESHOLD).is_none());
    }

    #[test]
    fn match_picks_best_of_many_offsets() {
        // Plant the reference at offset 12, surrounded by noise that's
        // sometimes close but never exact. The best score should be
        // the planted offset.
        let r = ref_fp();
        let mut target = Vec::new();
        for i in 0..12u32 {
            // Pseudo-random "noise" values — distinct from r so they
            // don't accidentally tie the planted occurrence.
            target.push(i.wrapping_mul(0x9E37_79B1));
        }
        target.extend_from_slice(&r);
        for i in 0..7u32 {
            target.push((i + 100).wrapping_mul(0x9E37_79B1));
        }
        let m = match_fingerprint(&r, &target, DEFAULT_MATCH_THRESHOLD).unwrap();
        assert_eq!(m.start_ms, 12 * FRAME_STEP_MS);
        assert_eq!(m.score, 0.0);
    }

    #[test]
    fn blob_roundtrip() {
        let fp = ref_fp();
        let blob = encode_blob(&fp);
        assert_eq!(blob.len(), fp.len() * 4);
        let decoded = decode_blob(&blob).unwrap();
        assert_eq!(decoded, fp);
    }

    #[test]
    fn blob_decode_rejects_truncated() {
        let blob = vec![0u8, 0, 0]; // 3 bytes — not a multiple of 4
        assert!(decode_blob(&blob).is_err());
    }
}
