//! Unified audio-analysis entry point (perf-plan Phase C).
//!
//! [`analyze_audio`] is the single API consumers should call when they
//! want any combination of intro/credits detection and loudness
//! measurement on a media file.
//!
//! When both markers and loudness are requested, the implementation
//! uses a **single-decode fan-out** ([`fused_decode`]): one symphonia
//! decode pass over the file feeds an `ebur128` analyser, an intro-
//! window mono PCM buffer, and a credits-window mono PCM buffer in
//! parallel. The loudness measurement is finalized after EOF;
//! intro/credits buffers are resampled to the fingerprint rate and
//! matched against the supplied references. This cuts per-file CPU
//! roughly in half vs. running markers + loudness as two separate
//! decode passes — the redundant audio decode was what made
//! loudness-enabled imports the slowest path.
//!
//! When only one of markers or loudness is requested, [`analyze_audio`]
//! falls through to the existing single-purpose path. There's no
//! sharing benefit when there's only one consumer.
//!
//! Why this lives in its own module rather than `lib.rs`: as the
//! decode pipeline grows (silence detection, ad-break candidates,
//! dialog-level analysis), the orchestration logic gets non-trivial.
//! Keeping it self-contained avoids `lib.rs` turning into a kitchen
//! sink.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use ebur128::{EbuR128, Mode};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, info, warn};

use crate::audio::{self, AudioRegion};
use crate::detection;
use crate::fingerprint::{self, FingerprintKind};
use crate::loudness::{
    Cancelled, CancellationToken, LoudnessMeasurement, ProgressEvent, ProgressSink,
};
use crate::matching::ReferenceFingerprint;
use crate::{Config, Segment, SegmentMarkers};

/// What the caller wants from one [`analyze_audio`] invocation.
///
/// Every field is independently optional. An all-`None` / `false`
/// request returns an empty [`AnalysisResult`] without opening the
/// file — useful as a sanity check in callers that build the request
/// dynamically.
#[derive(Debug, Default, Clone)]
pub struct AnalysisRequest {
    /// When `Some`, run intro/credits detection against the supplied
    /// reference fingerprints. `None` skips markers entirely.
    pub markers: Option<MarkerRequest>,
    /// When `true`, run EBU R 128 loudness measurement.
    pub loudness: bool,
}

/// Marker-detection sub-request. Window hints narrow the search range
/// when the caller already knows where the intro/credits *should* be
/// (e.g. from container chapter timestamps that lacked semantic
/// labels). Empty refs + window hints mean "blackframe heuristic only
/// in this window" — tacet falls through automatically.
#[derive(Debug, Default, Clone)]
pub struct MarkerRequest {
    pub intro_refs: Vec<ReferenceFingerprint>,
    pub credits_refs: Vec<ReferenceFingerprint>,
    /// Stable episode id passed through to the markers output. Mirrors
    /// the field on tacet's existing single-episode entry point — kept
    /// here so callers can map markers back to their domain keys
    /// without an extra lookup.
    pub episode_id: String,
    /// Optional `(start_seconds, end_seconds)` overrides for the
    /// intro/credits decode windows. When `Some`, tacet decodes only
    /// the supplied range instead of the default window from
    /// `Config::intro_scan_minutes` / `Config::credits_scan_minutes`.
    /// Useful when the caller has container chapter boundaries that
    /// suggest *where* but not *what* (unlabeled chapters); narrowing
    /// the decode range cuts CPU + I/O proportionally.
    pub intro_window_hint: Option<(f64, f64)>,
    pub credits_window_hint: Option<(f64, f64)>,
}

/// What [`analyze_audio`] returns.
///
/// Each `Option` corresponds to the matching field in the request:
/// requested + succeeded → `Some(_)`; not requested → `None`; requested
/// but failed → see [`AnalysisResult::warnings`].
#[derive(Debug, Clone, Default)]
pub struct AnalysisResult {
    pub markers: Option<SegmentMarkers>,
    pub loudness: Option<LoudnessMeasurement>,
    pub stage_timings: StageTimings,
    /// Non-fatal sub-failures during analysis. E.g. loudness measurement
    /// raised an error while markers succeeded — the caller still gets
    /// the marker output and can decide what to do about the missing
    /// loudness. Empty when everything requested succeeded.
    pub warnings: Vec<String>,
}

/// Per-stage timing for the activity-feed display. Stages that didn't
/// run report [`Duration::ZERO`].
#[derive(Debug, Clone, Default)]
pub struct StageTimings {
    pub markers: Duration,
    pub loudness: Duration,
}

/// Run the requested analyses on `path`. Synchronous — callers in an
/// async context should wrap in `tokio::task::spawn_blocking` (tacet's
/// internal DSP uses rayon and is meant for blocking pools).
///
/// Cancellation is cooperative: the token is polled at coarse
/// boundaries inside each sub-analysis (per audio packet for loudness,
/// per file for markers). When cancellation fires partway through,
/// already-completed stages are kept in the result, and stages still
/// running are reported in `warnings` so the caller can distinguish
/// "didn't ask" from "asked but got cancelled."
pub fn analyze_audio(
    path: &Path,
    request: AnalysisRequest,
    progress: Option<&dyn ProgressSink>,
    cancel: &CancellationToken,
    config: &Config,
) -> Result<AnalysisResult> {
    // Fused path: both markers + loudness requested. Single symphonia
    // decode pass feeds all three consumers (loudness analyser, intro
    // window buffer, credits window buffer). When only one consumer
    // is requested there's no sharing benefit; fall through to the
    // existing single-purpose helpers.
    if let (Some(marker_req), true) = (request.markers.as_ref(), request.loudness) {
        return fused_path(path, marker_req, progress, cancel, config);
    }

    let mut out = AnalysisResult::default();

    // ── Markers (single-purpose path) ─────────────────────────────
    if let Some(marker_req) = request.markers.as_ref() {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        if let Some(p) = progress {
            p.emit(ProgressEvent::MarkersStarted);
        }
        let start = Instant::now();
        match detection::detect_single_episode_with_hints(
            path,
            &marker_req.episode_id,
            &marker_req.intro_refs,
            &marker_req.credits_refs,
            marker_req.intro_window_hint,
            marker_req.credits_window_hint,
            config,
        ) {
            Ok(seg) => {
                out.markers = Some(seg);
            }
            Err(e) => {
                let msg = format!("markers: {e:#}");
                warn!(path = %path.display(), error = %msg, "marker detection failed in analyze_audio");
                out.warnings.push(msg);
            }
        }
        out.stage_timings.markers = start.elapsed();
        if let Some(p) = progress {
            p.emit(ProgressEvent::MarkersFinalizing);
        }
    }

    // ── Loudness (single-purpose path) ────────────────────────────
    if request.loudness {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let start = Instant::now();
        match crate::loudness::measure_loudness(path, cancel, progress) {
            Ok(Some(m)) => {
                out.loudness = Some(m);
            }
            Ok(None) => {
                // Benign: file had no audio stream / produced non-finite
                // measurements. Caller stamps the file as analysed.
            }
            Err(e) => {
                if e.downcast_ref::<Cancelled>().is_some() {
                    return Err(e);
                }
                let msg = format!("loudness: {e:#}");
                warn!(path = %path.display(), error = %msg, "loudness measurement failed in analyze_audio");
                out.warnings.push(msg);
            }
        }
        out.stage_timings.loudness = start.elapsed();
    }

    if let Some(p) = progress {
        p.emit(ProgressEvent::Completed);
    }
    info!(
        path = %path.display(),
        markers_requested = request.markers.is_some(),
        loudness_requested = request.loudness,
        markers_ms = out.stage_timings.markers.as_millis(),
        loudness_ms = out.stage_timings.loudness.as_millis(),
        warnings = out.warnings.len(),
        "analyze_audio completed"
    );
    Ok(out)
}

/// Run markers + loudness from a single symphonia decode pass.
/// Called by [`analyze_audio`] when both are requested. On unsupported
/// codecs (symphonia rejects the file), falls back to the legacy
/// sequential path so the caller still gets results — the fused
/// optimization is best-effort.
fn fused_path(
    path: &Path,
    marker_req: &MarkerRequest,
    progress: Option<&dyn ProgressSink>,
    cancel: &CancellationToken,
    config: &Config,
) -> Result<AnalysisResult> {
    if cancel.is_cancelled() {
        return Err(Cancelled.into());
    }
    let start = Instant::now();
    match fused_decode(path, marker_req, progress, cancel, config) {
        Ok(out) => {
            if let Some(p) = progress {
                p.emit(ProgressEvent::Completed);
            }
            info!(
                path = %path.display(),
                fused = true,
                markers_requested = true,
                loudness_requested = true,
                markers_ms = out.stage_timings.markers.as_millis(),
                loudness_ms = out.stage_timings.loudness.as_millis(),
                warnings = out.warnings.len(),
                "analyze_audio completed"
            );
            Ok(out)
        }
        Err(e) => {
            if e.downcast_ref::<Cancelled>().is_some() {
                return Err(e);
            }
            // Symphonia couldn't open the file; fall back to the
            // single-purpose path which has its own ffmpeg fallback
            // for exotic codecs. Loudness comes from
            // `loudness::measure_loudness` (which already tries
            // ffmpeg); markers comes from
            // `detection::detect_single_episode_with_hints` (which
            // tries ffmpeg via tacet's audio module).
            debug!(
                path = %path.display(),
                error = %format!("{e:#}"),
                "fused-decode path failed; falling back to sequential analyses"
            );
            let mut out = AnalysisResult::default();
            out.stage_timings.markers = start.elapsed(); // partial — failed fused attempt is markers-shaped
            // Re-dispatch as if loudness weren't requested? No — the
            // caller asked for both; we still want both. Run them
            // sequentially via the existing helpers.
            //
            // Markers
            if let Some(p) = progress {
                p.emit(ProgressEvent::MarkersStarted);
            }
            let m_start = Instant::now();
            match detection::detect_single_episode_with_hints(
                path,
                &marker_req.episode_id,
                &marker_req.intro_refs,
                &marker_req.credits_refs,
                marker_req.intro_window_hint,
                marker_req.credits_window_hint,
                config,
            ) {
                Ok(seg) => out.markers = Some(seg),
                Err(e) => {
                    out.warnings.push(format!("markers: {e:#}"));
                }
            }
            out.stage_timings.markers = m_start.elapsed();
            if let Some(p) = progress {
                p.emit(ProgressEvent::MarkersFinalizing);
            }

            // Loudness
            let l_start = Instant::now();
            match crate::loudness::measure_loudness(path, cancel, progress) {
                Ok(Some(m)) => out.loudness = Some(m),
                Ok(None) => {}
                Err(e) => {
                    if e.downcast_ref::<Cancelled>().is_some() {
                        return Err(e);
                    }
                    out.warnings.push(format!("loudness: {e:#}"));
                }
            }
            out.stage_timings.loudness = l_start.elapsed();
            if let Some(p) = progress {
                p.emit(ProgressEvent::Completed);
            }
            Ok(out)
        }
    }
}

/// The real fused-decode implementation. One symphonia pass; the
/// caller is responsible for fallback semantics when this returns
/// `Err`.
fn fused_decode(
    path: &Path,
    marker_req: &MarkerRequest,
    progress: Option<&dyn ProgressSink>,
    cancel: &CancellationToken,
    config: &Config,
) -> Result<AnalysisResult> {
    let file = File::open(path).context("open media file")?;
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
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .cloned()
        .context("no audio track found")?;
    let track_id = track.id;
    let native_rate = track
        .codec_params
        .sample_rate
        .context("track has no sample rate")?;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .context("track has no channel layout")?;
    if channels == 0 {
        anyhow::bail!("track reports zero channels");
    }

    // Duration: we need this to resolve the credits window (last N
    // minutes) and for progress reporting. Falls back to None when
    // the container doesn't expose it; in that case credits window
    // is derived from the operator-supplied hint or skipped.
    let total_duration_seconds = track
        .codec_params
        .n_frames
        .and_then(|n| {
            track
                .codec_params
                .time_base
                .map(|tb| (n as f64) * (tb.numer as f64) / (tb.denom as f64))
        })
        .or_else(|| {
            track
                .codec_params
                .n_frames
                .map(|n| n as f64 / native_rate as f64)
        });

    // Resolve effective windows (hint overrides default, default
    // derives from config). `None` means "skip that side".
    let intro_window = resolve_intro_window(marker_req, config);
    let credits_window = resolve_credits_window(marker_req, config, total_duration_seconds);

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

    // Per-window native-rate mono buffers. Capacity hints based on
    // window length × native rate to skip a couple of reallocations
    // on the typical (~10-minute) window.
    let mut intro_native_mono: Vec<f32> = match &intro_window {
        Some((s, e)) => Vec::with_capacity(((e - s) * native_rate as f64) as usize),
        None => Vec::new(),
    };
    let mut credits_native_mono: Vec<f32> = match &credits_window {
        Some((s, e)) => Vec::with_capacity(((e - s) * native_rate as f64) as usize),
        None => Vec::new(),
    };
    let mut intro_offset_secs: Option<f64> = intro_window.map(|(s, _)| s);
    let mut credits_offset_secs: Option<f64> = credits_window.map(|(s, _)| s);

    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    // Running count of frames (one frame = one sample per channel)
    // decoded since file start. Drives the intro/credits window
    // overlap math.
    let mut frames_decoded: u64 = 0;
    let mut next_progress_at_frames: u64 = native_rate as u64;
    let stage_start = Instant::now();

    loop {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(_)) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "skipping undecodable audio packet during fused analyze"
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
        let frames_in_packet = (pcm.len() / channels) as u64;

        // Feed all channels to the loudness analyser at native rate.
        analyzer
            .add_frames_f32(pcm)
            .context("feed PCM to EBU R 128 analyser")?;

        // Window-overlap math: this packet covers frames in
        // [packet_start_frame, packet_end_frame). Project each
        // window onto that range and downmix the overlap into the
        // appropriate buffer.
        let packet_start_frame = frames_decoded;
        let packet_end_frame = frames_decoded + frames_in_packet;
        if let Some((ws, we)) = intro_window {
            tee_window(
                pcm,
                channels,
                native_rate,
                packet_start_frame,
                packet_end_frame,
                ws,
                we,
                &mut intro_native_mono,
            );
        }
        if let Some((ws, we)) = credits_window {
            tee_window(
                pcm,
                channels,
                native_rate,
                packet_start_frame,
                packet_end_frame,
                ws,
                we,
                &mut credits_native_mono,
            );
        }

        frames_decoded += frames_in_packet;
        if frames_decoded >= next_progress_at_frames {
            if let Some(p) = progress {
                p.emit(ProgressEvent::LoudnessProgress {
                    position_seconds: frames_decoded as f64 / native_rate as f64,
                });
            }
            next_progress_at_frames = frames_decoded + native_rate as u64;
        }
    }
    let decode_elapsed = stage_start.elapsed();
    if let Some(p) = progress {
        p.emit(ProgressEvent::LoudnessFinalizing);
    }

    // Finalize loudness.
    let loudness = read_loudness_from_analyzer(&analyzer, channels)?;

    // Bail if neither window collected samples — credits window
    // unresolvable on a no-duration file with no hint, intro window
    // similarly. We still got loudness; surface that.
    if intro_native_mono.is_empty() {
        intro_offset_secs = None;
    }
    if credits_native_mono.is_empty() {
        credits_offset_secs = None;
    }

    if let Some(p) = progress {
        p.emit(ProgressEvent::MarkersStarted);
    }
    let markers_stage_start = Instant::now();

    // Resample each window's mono buffer to the fingerprint rate
    // and run the fingerprint + match pipeline.
    let intro_region = match (intro_offset_secs, intro_native_mono.is_empty()) {
        (Some(start), false) => {
            let samples = if native_rate != config.sample_rate {
                audio::resample(&intro_native_mono, native_rate, config.sample_rate)?
            } else {
                intro_native_mono
            };
            Some(AudioRegion {
                samples,
                sample_rate: config.sample_rate,
                offset_seconds: start,
                total_duration: total_duration_seconds,
            })
        }
        _ => None,
    };
    let credits_region = match (credits_offset_secs, credits_native_mono.is_empty()) {
        (Some(start), false) => {
            let samples = if native_rate != config.sample_rate {
                audio::resample(&credits_native_mono, native_rate, config.sample_rate)?
            } else {
                credits_native_mono
            };
            Some(AudioRegion {
                samples,
                sample_rate: config.sample_rate,
                offset_seconds: start,
                total_duration: total_duration_seconds,
            })
        }
        _ => None,
    };

    let intro_marker = intro_region.as_ref().and_then(|region| {
        let fp = fingerprint::fingerprint(region, config);
        match_segment(
            &marker_req.intro_refs,
            region,
            &fp,
            config,
            FingerprintKind::Intro,
        )
    });
    let credits_marker = match credits_region.as_ref() {
        Some(region) => {
            let fp = fingerprint::fingerprint(region, config);
            match_segment(
                &marker_req.credits_refs,
                region,
                &fp,
                config,
                FingerprintKind::Credits,
            )
        }
        None => None,
    };
    // Blackframe fallback for credits is intentionally NOT applied
    // here in the fused path — it depends on the ffmpeg blackdetect
    // filter (separate from the audio decode), and we'd rather not
    // re-open the file twice. When the caller wants the fallback
    // they request markers-only, which goes through the existing
    // `detection::detect_single_episode_with_hints` path.

    let markers_elapsed = markers_stage_start.elapsed();
    if let Some(p) = progress {
        p.emit(ProgressEvent::MarkersFinalizing);
    }

    Ok(AnalysisResult {
        markers: Some(SegmentMarkers {
            episode_id: marker_req.episode_id.clone(),
            intro: intro_marker,
            credits: credits_marker,
        }),
        loudness,
        stage_timings: StageTimings {
            // Decode time is attributed mostly to loudness since the
            // loudness analyser consumed every sample; the tee-into-
            // mono work is a small fraction. Markers stage timing is
            // just the post-decode resample + fingerprint + match.
            markers: markers_elapsed,
            loudness: decode_elapsed,
        },
        warnings: Vec::new(),
    })
}

fn resolve_intro_window(
    marker_req: &MarkerRequest,
    config: &Config,
) -> Option<(f64, f64)> {
    if let Some(hint) = marker_req.intro_window_hint {
        let (s, e) = hint;
        if s >= 0.0 && e > s {
            return Some((s, e));
        }
    }
    if marker_req.intro_refs.is_empty() {
        // No refs → nothing to match against. Skip the window so we
        // don't waste memory teeing samples we won't use.
        return None;
    }
    Some((0.0, config.intro_scan_minutes as f64 * 60.0))
}

fn resolve_credits_window(
    marker_req: &MarkerRequest,
    config: &Config,
    total_duration: Option<f64>,
) -> Option<(f64, f64)> {
    if let Some(hint) = marker_req.credits_window_hint {
        let (s, e) = hint;
        if s >= 0.0 && e > s {
            return Some((s, e));
        }
    }
    if marker_req.credits_refs.is_empty() {
        return None;
    }
    let dur = total_duration?;
    let credits_secs = config.credits_scan_minutes as f64 * 60.0;
    let start = (dur - credits_secs).max(0.0);
    Some((start, dur))
}

fn read_loudness_from_analyzer(
    analyzer: &EbuR128,
    channels: usize,
) -> Result<Option<LoudnessMeasurement>> {
    let integrated = analyzer
        .loudness_global()
        .context("read integrated loudness")?;
    let lra = analyzer.loudness_range().context("read loudness range")?;
    let threshold = analyzer
        .relative_threshold()
        .context("read relative threshold")?;
    let mut peak_lin = f64::NEG_INFINITY;
    for ch in 0..channels {
        let p = analyzer
            .true_peak(ch as u32)
            .context("read per-channel true peak")?;
        if p > peak_lin {
            peak_lin = p;
        }
    }
    let true_peak = if peak_lin > 0.0 {
        20.0 * peak_lin.log10()
    } else {
        f64::NEG_INFINITY
    };
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

/// Tee the overlap between `packet` (frames `[packet_start_frame,
/// packet_end_frame)` at `native_rate`) and the time-range window
/// `[window_start_secs, window_end_secs)` into `out` as downmixed
/// mono samples. No-op when there's no overlap.
///
/// 8 args is over clippy's default threshold but each one is doing
/// real work — packet bookkeeping (2), window bookkeeping (2),
/// decode params (3), output (1). Bundling them into a struct
/// would obscure that this is the per-packet hot loop, where every
/// indirection counts. Inline allow.
#[allow(clippy::too_many_arguments)]
fn tee_window(
    interleaved_pcm: &[f32],
    channels: usize,
    native_rate: u32,
    packet_start_frame: u64,
    packet_end_frame: u64,
    window_start_secs: f64,
    window_end_secs: f64,
    out: &mut Vec<f32>,
) {
    let rate = native_rate as f64;
    let window_start_frame = (window_start_secs * rate).floor() as u64;
    let window_end_frame = (window_end_secs * rate).ceil() as u64;
    if packet_end_frame <= window_start_frame || packet_start_frame >= window_end_frame {
        return;
    }
    let overlap_start = packet_start_frame.max(window_start_frame);
    let overlap_end = packet_end_frame.min(window_end_frame);
    if overlap_end <= overlap_start {
        return;
    }
    // Convert frame offsets to sample indices inside the
    // interleaved buffer.
    let frame_offset = (overlap_start - packet_start_frame) as usize;
    let frame_count = (overlap_end - overlap_start) as usize;
    let sample_start = frame_offset * channels;
    let sample_end = sample_start + frame_count * channels;
    // Defensive: a malformed packet could under-report its frame
    // count; clamp so we don't read past the slice.
    let sample_end = sample_end.min(interleaved_pcm.len());
    let slice = &interleaved_pcm[sample_start..sample_end];
    let mono = audio::downmix_to_mono(slice, channels);
    out.extend_from_slice(&mono);
}

/// Wrapper around the matching pipeline. Delegates to the canonical
/// scorer in `detection::match_region_to_segment` so the fused path
/// applies the same credits-tail-gap + min-duration validation as the
/// non-fused single-purpose path.
fn match_segment(
    refs: &[ReferenceFingerprint],
    region: &AudioRegion,
    fp: &crate::fingerprint::Fingerprint,
    config: &Config,
    kind: FingerprintKind,
) -> Option<Segment> {
    detection::match_region_to_segment(refs, region, fp, config, kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_request_returns_empty_result_without_touching_file() {
        let token = CancellationToken::new();
        let config = Config::default();
        // Path doesn't have to exist — the function should never open
        // it when nothing was requested.
        let path = std::path::Path::new("/nonexistent/file.mkv");
        let result = analyze_audio(path, AnalysisRequest::default(), None, &token, &config)
            .expect("should not error on empty request");
        assert!(result.markers.is_none());
        assert!(result.loudness.is_none());
        assert!(result.warnings.is_empty());
        assert_eq!(result.stage_timings.markers, Duration::ZERO);
        assert_eq!(result.stage_timings.loudness, Duration::ZERO);
    }

    #[test]
    fn pre_cancelled_token_short_circuits_loudness() {
        let token = CancellationToken::new();
        token.cancel();
        let config = Config::default();
        let path = std::path::Path::new("/nonexistent/file.mkv");
        let req = AnalysisRequest {
            markers: None,
            loudness: true,
        };
        let err = analyze_audio(path, req, None, &token, &config).expect_err("should cancel");
        assert!(err.downcast_ref::<Cancelled>().is_some());
    }
}
