//! EBU R 128 loudness measurement.
//!
//! Built on the [`ebur128`] crate — a pure-Rust port of libebur128 that
//! matches the C reference implementation bit-exact when its `c-tests`
//! feature is enabled. We feed it the file's native PCM stream via
//! symphonia (the same decoder used by [`crate::audio`]); when symphonia
//! rejects the codec, we fall back to ffmpeg's `ebur128` filter through
//! a subprocess.
//!
//! The output shape matches what ffmpeg's `loudnorm` filter prints (in
//! print-mode JSON): integrated LUFS, true-peak dBTP, loudness range
//! (LU), and the relative gating threshold (LUFS). Values are within
//! ~0.1 LU of ffmpeg's `ebur128` filter on the same input — they share
//! the same reference algorithm.
//!
//! This module is part of the perf-plan Phase C work: pulling loudness
//! into tacet so it lives next to fingerprinting in the
//! "single-pass audio analysis" architecture. Today the decode pass for
//! loudness is separate from the fingerprint decode pass (different
//! sample-rate + channel requirements); future work in
//! [`crate::analyze_audio`] will share a single decode pass between
//! both consumers once the streaming PCM fan-out is wired up.

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use ebur128::{EbuR128, Mode};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, warn};

use crate::audio::ffmpeg as audio_ffmpeg;

/// Result of EBU R 128 analysis on one file.
///
/// All values are in LUFS (loudness) / dBTP (true peak) / LU (range).
/// Field names mirror the JSON ffmpeg's `loudnorm` filter emits so a
/// migrating consumer doesn't have to relabel.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct LoudnessMeasurement {
    /// Integrated program loudness, LUFS. Anchor value for normalization.
    pub integrated: f64,
    /// True peak, dBTP. Peak after 4× oversampling — the relevant value
    /// for asking "will this clip on consumer playback?"
    pub true_peak: f64,
    /// Loudness range, LU. Macro-dynamic-range measurement; a quiet
    /// dialogue scene followed by an explosion has high LRA.
    pub lra: f64,
    /// Relative gating threshold the integrated measurement used,
    /// LUFS. Helpful for diagnosing extremely quiet content — when
    /// this is near `integrated` minus 10 LU the integrated value
    /// is well-conditioned; far below it suggests most of the file
    /// was gated out.
    pub threshold: f64,
}

/// Cooperative cancellation handle for long-running analyses.
///
/// Polled at coarse boundaries (per audio packet decode, roughly every
/// ~20 ms of playback time). When triggered, the analysis returns
/// `Err(Cancelled)` and the partially-decoded state is dropped. Cheap
/// enough that callers can plumb one through every call site without
/// worrying about overhead.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Signal cancellation. Idempotent.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

/// One milestone in an analysis run. Surfaced to the optional progress
/// sink so callers can paint activity-feed updates ("decoding 42%",
/// "computing loudness", etc.) without polling.
///
/// Variants are coarse on purpose for this phase: per-stage Started /
/// Progress / Finalizing for loudness, plus coarser markers events.
/// Future work on the single-decode pipeline will add finer-grained
/// variants (DecodeProgress separate from FingerprintProgress, etc.)
/// without breaking existing callers — match arms can use `_ => {}`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ProgressEvent {
    // ── Loudness analysis ────────────────────────────────────────
    /// Loudness analysis started.
    LoudnessStarted {
        /// Source file duration in seconds, if known. `None` for live
        /// streams or files whose container doesn't report duration.
        duration_seconds: Option<f64>,
    },
    /// Progress through the file. Fires roughly every ~1s of source
    /// playback time — best for driving a smoothed progress bar, not a
    /// per-frame counter.
    LoudnessProgress {
        /// Source time position decoded so far, in seconds.
        position_seconds: f64,
    },
    /// All samples have been fed to the loudness analyser; final
    /// integration is happening.
    LoudnessFinalizing,

    // ── Marker detection (intro/credits) ─────────────────────────
    /// Marker detection started. No timing data yet — tacet's
    /// detection path opens the file internally and the surrounding
    /// orchestrator doesn't know the duration up front.
    MarkersStarted,
    /// Marker detection has finished its decode + matching work and
    /// is about to return. Useful for the activity feed to advance
    /// from "running" to "finalizing" without depending on the wall-
    /// clock duration.
    MarkersFinalizing,

    // ── Top-level orchestration ──────────────────────────────────
    /// All requested analyses have completed (or been skipped). Last
    /// event emitted by [`crate::analyze::analyze_audio`].
    Completed,
}

/// Receive [`ProgressEvent`]s during an analysis. Trait-objectified so
/// the API surface stays generic-free for callers that hold the sink
/// behind dyn dispatch (workers writing to a shared activity feed).
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: ProgressEvent);
}

impl<F> ProgressSink for F
where
    F: Fn(ProgressEvent) + Send + Sync,
{
    fn emit(&self, event: ProgressEvent) {
        (self)(event)
    }
}

/// Returned by [`measure_loudness`] when [`CancellationToken::cancel`]
/// fires mid-decode. Distinct from a real failure so callers can
/// distinguish "operator paused the queue" from "couldn't decode the
/// file."
#[derive(Debug, thiserror::Error)]
#[error("loudness analysis cancelled")]
pub struct Cancelled;

/// Measure EBU R 128 loudness for one file.
///
/// Returns `Ok(None)` when the file has no audio stream (still photos,
/// metadata-only files) or the measurement produces non-finite values
/// (e.g. completely silent input → `-inf` integrated). Both are benign
/// outcomes; the caller stamps the file as analysed and moves on.
///
/// Returns `Err` when:
/// - Symphonia and ffmpeg both fail to decode the file.
/// - The ebur128 analyser itself rejects the audio params (very rare —
///   typically unsupported channel layout).
/// - The caller's [`CancellationToken`] fires before completion (the
///   error downcasts to [`Cancelled`] in that case).
pub fn measure_loudness(
    path: &Path,
    cancel: &CancellationToken,
    progress: Option<&dyn ProgressSink>,
) -> Result<Option<LoudnessMeasurement>> {
    match measure_with_symphonia(path, cancel, progress) {
        Ok(measurement) => Ok(measurement),
        Err(symphonia_err) => {
            // Cancellation isn't a real failure — surface it as-is so
            // callers can distinguish "couldn't decode" from "operator
            // pressed pause."
            if symphonia_err.downcast_ref::<Cancelled>().is_some() {
                return Err(symphonia_err);
            }
            if audio_ffmpeg::is_available() {
                debug!(
                    path = %path.display(),
                    symphonia_error = format!("{symphonia_err:#}"),
                    "symphonia loudness path failed; falling back to ffmpeg ebur128 filter"
                );
                measure_with_ffmpeg(path, cancel).map_err(|ffmpeg_err| {
                    symphonia_err.context(format!("ffmpeg fallback also failed: {ffmpeg_err:#}"))
                })
            } else {
                Err(symphonia_err.context(
                    "ffmpeg fallback is unavailable (install ffmpeg to measure loudness for this codec)",
                ))
            }
        }
    }
}

fn measure_with_symphonia(
    path: &Path,
    cancel: &CancellationToken,
    progress: Option<&dyn ProgressSink>,
) -> Result<Option<LoudnessMeasurement>> {
    let file = std::fs::File::open(path).context("open media file")?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .context("unsupported audio format")?;

    let mut format = probed.format;
    let Some(track) = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .cloned()
    else {
        // Containers we open but find no audio in (video-only, broken
        // tracks, etc.) get a clean None — the caller stamps the file
        // as analysed and won't retry. Log so the operator can see why
        // a file completed in sub-ms with no measurement (the otherwise
        // mysterious `loudness_ms=0` in `analyze_audio completed`).
        debug!(
            path = %path.display(),
            "loudness: no decodable audio track in container; stamping file as checked with no measurement"
        );
        return Ok(None);
    };

    let track_id = track.id;
    let native_rate = track.codec_params.sample_rate.context("track has no sample rate")?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .context("track has no channel layout")?;

    if channels == 0 {
        bail!("track reports zero channels");
    }

    // Emit start event with best-effort duration.
    let total_duration_seconds = track
        .codec_params
        .n_frames
        .and_then(|n| track.codec_params.time_base.map(|tb| {
            (n as f64) * (tb.numer as f64) / (tb.denom as f64)
        }))
        .or_else(|| {
            track
                .codec_params
                .n_frames
                .map(|n| n as f64 / native_rate as f64)
        });
    if let Some(p) = progress {
        p.emit(ProgressEvent::LoudnessStarted {
            duration_seconds: total_duration_seconds,
        });
    }

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("create decoder")?;

    let mut analyzer = EbuR128::new(
        channels as u32,
        native_rate,
        Mode::I | Mode::LRA | Mode::TRUE_PEAK,
    )
    .context("initialize EBU R 128 analyser")?;

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut samples_seen: u64 = 0;
    // Progress emit cadence: every ~1s of source time. `samples_seen`
    // counts PCM frames (interleaved samples ÷ channels), so the threshold
    // is just native_rate frames — not native_rate × channels.
    let mut next_progress_at_samples: u64 = native_rate as u64;

    loop {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(_)) => break, // EOF
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(e) => {
                // Per-packet decode errors are non-fatal for the
                // measurement as a whole — log and skip the packet.
                warn!(
                    path = %path.display(),
                    error = %e,
                    "skipping undecodable audio packet during loudness measurement"
                );
                continue;
            }
        };

        let spec = *decoded.spec();
        if sample_buf.is_none() {
            sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let buf = sample_buf.as_mut().unwrap();
        buf.copy_interleaved_ref(decoded);
        let pcm = buf.samples();

        // ebur128 wants planar by default but exposes
        // `add_frames_*_interleaved` for the layout symphonia gives us.
        analyzer
            .add_frames_f32(pcm)
            .context("feed PCM to EBU R 128 analyser")?;

        samples_seen += (pcm.len() / channels) as u64;
        if samples_seen >= next_progress_at_samples {
            if let Some(p) = progress {
                let secs = samples_seen as f64 / native_rate as f64;
                p.emit(ProgressEvent::LoudnessProgress {
                    position_seconds: secs,
                });
            }
            next_progress_at_samples = samples_seen + native_rate as u64;
        }
    }

    if let Some(p) = progress {
        p.emit(ProgressEvent::LoudnessFinalizing);
    }

    let integrated = analyzer
        .loudness_global()
        .context("read integrated loudness")?;
    let lra = analyzer.loudness_range().context("read loudness range")?;
    let threshold = analyzer
        .relative_threshold()
        .context("read relative threshold")?;
    let mut peak_dbtp = f64::NEG_INFINITY;
    for ch in 0..channels {
        let p = analyzer
            .true_peak(ch as u32)
            .context("read per-channel true peak")?;
        if p > peak_dbtp {
            peak_dbtp = p;
        }
    }
    // ebur128 returns true peak as a linear amplitude; convert to dBTP.
    // (the C reference does the same; ffmpeg's loudnorm reports dBTP
    // already so the units match the existing storage layer)
    let true_peak_dbtp = if peak_dbtp > 0.0 {
        20.0 * peak_dbtp.log10()
    } else {
        f64::NEG_INFINITY
    };

    // Silent files / files that gated below the absolute threshold
    // produce non-finite results. Treat as "nothing to measure."
    if !integrated.is_finite() || !true_peak_dbtp.is_finite() {
        return Ok(None);
    }

    Ok(Some(LoudnessMeasurement {
        integrated,
        true_peak: true_peak_dbtp,
        lra,
        threshold,
    }))
}

/// Fallback for codecs symphonia can't decode (HE-AAC, E-AC3 / Atmos,
/// DTS, exotic containers). Runs ffmpeg's `ebur128` filter and parses
/// the summary line it writes to stderr.
///
/// Output matches `measure_with_symphonia` within typical EBU R 128
/// tolerances — both back ends implement the same standard.
fn measure_with_ffmpeg(path: &Path, cancel: &CancellationToken) -> Result<Option<LoudnessMeasurement>> {
    // No async runtime here — tacet runs inside spawn_blocking. Use
    // std::process and check cancellation between read attempts.
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    let mut child = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostats",
            "-i",
        ])
        .arg(format!("file:{}", path.display()))
        .args([
            "-vn",
            "-sn",
            "-dn",
            "-map",
            "0:a:0?",
            "-af",
            "ebur128=peak=true",
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawn ffmpeg ebur128 for {}", path.display()))?;

    // Best-effort cancellation — poll periodically. Most ebur128
    // analysis runs are short enough that this rarely fires, but a
    // 4-hour movie can take a minute and the operator deserves a
    // responsive pause button.
    let stderr = child.stderr.take().expect("stderr piped above");
    let stderr_handle = std::thread::spawn(move || {
        use std::io::Read;
        let mut s = String::new();
        let mut reader = stderr;
        let _ = reader.read_to_string(&mut s);
        s
    });

    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait(); // reap zombie — avoid defunct process entry
            drop(stderr_handle); // thread will exit once the pipe closes
            return Err(Cancelled.into());
        }
        match child.try_wait()? {
            Some(_) => break,
            None => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
    let status = child.wait()?;
    let stderr_text = stderr_handle.join().unwrap_or_default();

    if !status.success() {
        if stderr_text.contains("Stream map '0:a:0?' matches no streams")
            || stderr_text.contains("does not contain any stream")
        {
            return Ok(None);
        }
        bail!(
            "ffmpeg ebur128 exited non-zero for {}: {}",
            path.display(),
            stderr_text.trim()
        );
    }

    parse_ebur128_summary(&stderr_text).map(Some)
}

/// Parse the summary block ffmpeg's `ebur128` filter emits at end of
/// stream. The block looks like:
///
/// ```text
/// [Parsed_ebur128_0 @ 0x123]   Integrated loudness:
/// [Parsed_ebur128_0 @ 0x123]     I:         -19.5 LUFS
/// [Parsed_ebur128_0 @ 0x123]     Threshold: -29.5 LUFS
/// [Parsed_ebur128_0 @ 0x123]   Loudness range:
/// [Parsed_ebur128_0 @ 0x123]     LRA:         8.3 LU
/// [Parsed_ebur128_0 @ 0x123]   True peak:
/// [Parsed_ebur128_0 @ 0x123]     Peak:       -2.1 dBFS
/// ```
fn parse_ebur128_summary(stderr: &str) -> Result<LoudnessMeasurement> {
    let extract = |needle: &str| -> Option<f64> {
        for line in stderr.lines() {
            if let Some(rest) = line.split_once(needle).map(|(_, b)| b) {
                let token = rest
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches(',');
                if let Ok(v) = token.parse::<f64>() {
                    return Some(v);
                }
            }
        }
        None
    };
    let integrated = extract("I:")
        .ok_or_else(|| anyhow!("ffmpeg ebur128 summary missing `I:` line"))?;
    // Threshold appears in the Integrated block. Two `Threshold:` lines
    // exist (one for I, one for LRA); take the first.
    let threshold = extract("Threshold:")
        .ok_or_else(|| anyhow!("ffmpeg ebur128 summary missing `Threshold:` line"))?;
    let lra = extract("LRA:")
        .ok_or_else(|| anyhow!("ffmpeg ebur128 summary missing `LRA:` line"))?;
    // Peak may be reported as dBFS (older ffmpeg) or dBTP (newer with
    // peak=true). We requested peak=true above; treat the value as
    // dBTP since the units differ by oversampling, not by scale.
    let true_peak = extract("Peak:")
        .ok_or_else(|| anyhow!("ffmpeg ebur128 summary missing `Peak:` line"))?;
    Ok(LoudnessMeasurement {
        integrated,
        true_peak,
        lra,
        threshold,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ffmpeg_ebur128_summary() {
        let stderr = "\
[Parsed_ebur128_0 @ 0x55] Summary:
[Parsed_ebur128_0 @ 0x55]   Integrated loudness:
[Parsed_ebur128_0 @ 0x55]     I:         -19.5 LUFS
[Parsed_ebur128_0 @ 0x55]     Threshold: -29.5 LUFS
[Parsed_ebur128_0 @ 0x55]   Loudness range:
[Parsed_ebur128_0 @ 0x55]     LRA:         8.3 LU
[Parsed_ebur128_0 @ 0x55]     Threshold: -39.5 LUFS
[Parsed_ebur128_0 @ 0x55]     LRA low:   -22.0 LUFS
[Parsed_ebur128_0 @ 0x55]     LRA high:  -13.7 LUFS
[Parsed_ebur128_0 @ 0x55]   True peak:
[Parsed_ebur128_0 @ 0x55]     Peak:       -2.1 dBFS
";
        let parsed = parse_ebur128_summary(stderr).expect("parses");
        assert!((parsed.integrated - -19.5).abs() < 1e-6);
        assert!((parsed.threshold - -29.5).abs() < 1e-6);
        assert!((parsed.lra - 8.3).abs() < 1e-6);
        assert!((parsed.true_peak - -2.1).abs() < 1e-6);
    }

    #[test]
    fn cancellation_token_starts_clear() {
        let t = CancellationToken::new();
        assert!(!t.is_cancelled());
        t.cancel();
        assert!(t.is_cancelled());
    }

    #[test]
    fn closure_implements_progress_sink() {
        // Smoke test: ensure the blanket impl compiles for closures so
        // callers don't need to define a struct for one-off sinks.
        let count = std::sync::Mutex::new(0u32);
        let sink = |_: ProgressEvent| {
            *count.lock().unwrap() += 1;
        };
        sink.emit(ProgressEvent::LoudnessFinalizing);
        assert_eq!(*count.lock().unwrap(), 1);
    }
}
