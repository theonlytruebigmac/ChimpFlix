//! HLS transcode session manager.
//!
//! v0.1 scope: single-variant (720p H.264 + AAC) software transcode,
//! spawned via ffmpeg subprocess. Each session has its own output
//! directory under the configured cache root. Sessions are kept alive
//! by recent access; the reaper kills idle ones.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use chimpflix_common::now_ms;
use serde::Serialize;
use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use crate::hwaccel::{EncoderPreset, HwAccel, VideoCodec};
use crate::FfmpegConfig;

/// What ffmpeg is doing with the source video stream for this session.
/// `Copy` skips the encoder entirely (just remuxes source packets into
/// the HLS container), `Reencode` runs the full filter + encoder
/// pipeline. The decision is made by the API layer based on whether
/// the source codec already matches what the client can play and
/// whether anything in the request requires modifying frames
/// (subtitles, scaling, tonemap).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoTreatment {
    Copy,
    Reencode,
}

/// Same shape as [`VideoTreatment`] but for the audio stream. `Copy`
/// remuxes source audio packets straight into the HLS container,
/// `Reencode` runs ffmpeg's AAC encoder. Saves a bit of CPU per
/// session when the source is already AAC (the vast majority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioTreatment {
    Copy,
    Reencode,
}

/// HLS segment container. Two real choices in practice:
///
/// * `Ts` (MPEG-TS) — the historical default. Universally playable,
///   self-contained per segment (no init segment needed), but only
///   carries a narrow set of codecs without complaint (H.264, HEVC,
///   AAC, MP3, AC-3, E-AC-3).
///
/// * `Fmp4` (fragmented MP4) — wider codec support (H.264, HEVC,
///   AV1, VP9 for video; AAC, AC-3, E-AC-3, Opus, FLAC for audio).
///   Requires an `init.mp4` init segment served alongside the
///   `.m4s` media segments. Modern HLS (Apple's recommendation
///   since HLS v7) and what unlocks stream-copy of AV1, VP9, Opus,
///   FLAC sources for browsers that can decode them.
///
/// The container is picked per session — re-encode-only sessions
/// stay on TS (cheaper segments, fewer moving parts), copy
/// sessions whose source codecs don't fit TS get bumped to Fmp4
/// so the copy fast-path stays available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerFormat {
    Ts,
    Fmp4,
}

/// Plain-data snapshot of a transcode session. Returned by
/// `TranscodeManager::list_sessions` so callers can build admin/dashboard
/// views without holding the sessions lock or peeking at `Session`'s
/// non-serializable internals.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSnapshot {
    pub id: String,
    pub user_id: i64,
    pub media_file_id: i64,
    pub start_position_ms: i64,
    pub duration_ms: Option<i64>,
    pub created_at: i64,
    pub last_seen_at: i64,
    /// Human label for the active encoder ("NVIDIA NVENC", "Intel
    /// QuickSync", "software (libx264)", …). Surfaced in the admin
    /// dashboard so the operator can verify the hardware path is
    /// actually being taken instead of silently falling back to CPU.
    pub encoder: String,
    /// `copy` when the session is just remuxing source video into
    /// HLS (audio swap only, no scale/subtitle/tonemap), `reencode`
    /// otherwise. Visible in the admin dashboard so the operator can
    /// see at a glance whether the CPU-cheap path got picked.
    pub video_treatment: VideoTreatment,
    /// `copy` when audio is being remuxed instead of AAC-encoded.
    pub audio_treatment: AudioTreatment,
    /// Source / target heights (px). Lets the dashboard render
    /// "4K → 1080p" so the operator can see at a glance what each
    /// session is actually doing. `source_height` is None when the
    /// source row didn't carry a `height` value.
    pub source_height: Option<u32>,
    pub target_height: u32,
    /// Encoder bitrate target in bps. Combined with target_height
    /// this matches the BANDWIDTH/RESOLUTION pair the master
    /// playlist advertises to the player.
    pub target_video_bitrate_bps: u64,
    /// Operator's speed-vs-quality preset for this session ("speed",
    /// "balanced", or "quality"). Surfaced so the admin dashboard
    /// can show what tradeoff is in effect, and so an "this session
    /// looks pixelated" report can be cross-checked against the
    /// preset before reaching for harder fixes.
    pub encoder_preset: String,
    /// True when the ffmpeg child has been SIGSTOP'd because the
    /// player reported pause. Lets the admin dashboard show a
    /// "▌▌ paused" pill and gives the reaper enough info to avoid
    /// killing sessions that intentionally aren't producing.
    pub paused: bool,
    /// Cumulative bytes served from this session (segment + playlist
    /// GETs). Resets to zero on session start; updated by the stream
    /// handlers via `Session::add_bytes_served`. Surfaced in the
    /// admin Now Playing tile and flushed to the playback_events
    /// table on session close.
    pub bytes_served: i64,
}

const SESSION_ID_BYTES: usize = 16;
const VARIANT_NAME: &str = "v1";
/// Directory name (under the session dir) and master-playlist URI
/// prefix for a WebVTT subtitle sidecar. Contains `index.m3u8`
/// (the subtitle media playlist) and `sub.vtt` (the WebVTT data).
const SUBTITLE_VARIANT_NAME: &str = "sub";
/// Group ID used on the `#EXT-X-MEDIA:TYPE=SUBTITLES` line and the
/// `SUBTITLES=` attribute of `#EXT-X-STREAM-INF`. HLS lets a single
/// stream variant point at multiple subtitle tracks by group; we only
/// expose one at a time so this stays constant.
const SUBTITLE_GROUP: &str = "subs";
/// Secondary variant directory name used when ABR is in play. Two
/// variants is the right amount of ABR for our deployment: most
/// consumer NVENC cards permit 2-3 concurrent encodes per process,
/// and a 1080p + 720p (or 720p + 480p) ladder covers the bandwidth
/// swings that matter (broadband flap, hotspot tether). Adding 480p
/// on top of 1080p + 720p would help dial-up users but the HLS
/// overhead per variant outweighs the benefit for the common case.
const FALLBACK_VARIANT_NAME: &str = "v2";
/// Default tier when the caller doesn't specify a quality target.
/// Picked to be safely playable on most broadband links without
/// rebuffering, and to keep the single-encode cost predictable.
const DEFAULT_TARGET_HEIGHT: u32 = 720;
const DEFAULT_TARGET_VIDEO_BITRATE_BPS: u64 = 2_500_000;
pub const HLS_SEGMENT_DURATION_S: u32 = 6;

/// HDR → SDR tonemap settings, threaded from `server_settings` into
/// the per-session filter chain. Bundled so adding new tonemap dials
/// doesn't grow `start()` / `build_command()`'s already-long parameter
/// list further.
#[derive(Debug, Clone)]
pub struct TonemapConfig {
    /// When false the tonemap filter is omitted entirely — HDR
    /// sources go through the SDR pipeline as-is (washed out, but
    /// lower CPU cost and what some operators prefer when their
    /// clients render HDR client-side).
    pub enabled: bool,
    /// Algorithm passed to ffmpeg's `tonemap=tonemap=<algo>` filter.
    /// One of: hable | reinhard | mobius | bt2390 | clip | linear.
    /// Validated upstream so we don't bother re-validating here.
    pub algorithm: String,
}

impl Default for TonemapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            algorithm: "hable".to_string(),
        }
    }
}

/// Per-file EBU R 128 measurements from the `analyze_loudness`
/// scheduled task. When present at session-create time, the
/// transcoder runs loudnorm in two-pass mode (linear=true) using
/// these as the `measured_*` parameters — produces sample-accurate
/// normalisation without runtime dynamic-range compression.
#[derive(Debug, Clone, Copy)]
pub struct LoudnessTarget {
    pub measured_i: f64,
    pub measured_tp: f64,
    pub measured_lra: f64,
    pub measured_thresh: f64,
}

/// Build the loudnorm filter chain. When measurements are supplied,
/// runs in two-pass mode with `linear=true` (the high-fidelity path
/// — no real-time dynamic-range compression). Without measurements,
/// falls back to the single-pass approximation (`I=-16:LRA=11:TP=-1.5`)
/// which works on any input but uses streaming-window estimates.
///
/// Targets are the EBU R 128 broadcast defaults (I = -16 LUFS for
/// stereo, LRA = 11 LU, TP = -1.5 dBTP) — matches Plex's "volume
/// leveling" output level so a library normalized by either tool
/// plays at roughly the same loudness.
fn build_loudnorm_filter(target: Option<&LoudnessTarget>) -> String {
    match target {
        Some(t) => format!(
            "loudnorm=I=-16:LRA=11:TP=-1.5:\
             measured_I={:.2}:measured_TP={:.2}:\
             measured_LRA={:.2}:measured_thresh={:.2}:\
             linear=true:print_format=summary",
            t.measured_i, t.measured_tp, t.measured_lra, t.measured_thresh
        ),
        None => "loudnorm=I=-16:LRA=11:TP=-1.5".to_string(),
    }
}

impl TonemapConfig {
    /// Build the prefix filter chain that maps HDR → SDR. Empty
    /// string when tonemap is off or the source isn't HDR — both
    /// cases mean "no extra filtering, just feed the encoder direct".
    pub fn build_chain(&self, hdr_format: Option<&str>) -> String {
        let is_hdr = matches!(hdr_format, Some("hdr10" | "hlg" | "dovi"));
        if !is_hdr || !self.enabled {
            return String::new();
        }
        // Linearize → tonemap → convert back to BT.709 SDR.
        // `desat=0` keeps colors saturated; ffmpeg's default 2.0
        // looks dull. Trailing comma matters — the caller splices
        // this string in front of `scale=...` and expects a clean
        // filter boundary.
        format!(
            "zscale=t=linear:npl=100,format=gbrpf32le,tonemap=tonemap={}:desat=0,zscale=p=bt709:t=bt709:m=bt709:r=tv,format=yuv420p,",
            self.algorithm
        )
    }
}

/// Sidecar WebVTT subtitle exposed via the master playlist as an
/// `#EXT-X-MEDIA:TYPE=SUBTITLES` track. When present, the main
/// video pipeline runs *without* a subtitle filter — the player
/// overlays the WebVTT itself, which avoids the libavfilter
/// `subtitles=` filter's whole-file scan and the `-copyts` +
/// setpts/asetpts dance that the burn path requires.
///
/// Only used for text subtitles (SRT / ASS / SSA / mov_text /
/// WebVTT). Picture-format subs (PGS / DVD / DVB / VobSub) still
/// take the burn-in path because browsers can't render bitmaps as
/// a separate text track.
#[derive(Debug, Clone)]
pub struct WebVttSidecar {
    /// BCP-47 / RFC 5646 language tag (e.g. "en", "ja"). Falls back
    /// to "und" when the source didn't tag the stream — players
    /// will still show the track, just without a language label.
    pub language: String,
    /// Human-readable display name (e.g. "English", "Netflix eng
    /// subrip"). Sourced from the embedded MKV title when present.
    pub display_name: String,
    /// Tracks the background extraction's progress. Subtitle
    /// extraction can take minutes on a 30 GB Bluray remux — long
    /// enough to time out the HTTP request that triggered the
    /// session. We do it in a tokio task instead and let the sub
    /// playlist / sub.vtt handlers wait on this channel when the
    /// player asks for them. Variant: `Pending` while running,
    /// `Ok(())` on success, `Err(message)` on failure.
    pub progress: Arc<tokio::sync::watch::Receiver<SubExtractionStatus>>,
}

/// Shared progress signal for the background WebVTT extraction.
/// Cloned across the `Session` (read-only) and the spawned task
/// (write-only via [`tokio::sync::watch::Sender`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubExtractionStatus {
    /// Extraction is still running.
    Pending,
    /// WebVTT + media playlist are on disk and ready to serve.
    Ready,
    /// Extraction failed. The string is a short reason for logs;
    /// the HTTP handler returns 404 to the player.
    Failed(String),
}

#[derive(Debug)]
pub struct Session {
    pub id: String,
    pub user_id: i64,
    pub media_file_id: i64,
    pub output_dir: PathBuf,
    pub start_position_ms: i64,
    pub duration_ms: Option<i64>,
    pub created_at: i64,
    /// Encoded target height (e.g. 720, 1080) for the session. Captured
    /// at start so `master_playlist()` reflects the same numbers ffmpeg
    /// is producing.
    pub target_height: u32,
    pub target_video_bitrate_bps: u64,
    /// Secondary ABR variant when one is in play. None means single-
    /// variant session (the historical behavior). When Some, ffmpeg
    /// is running with `split` + two encoder branches producing both
    /// the primary tier and this lower tier in lockstep, and the
    /// master playlist advertises both — HLS.js picks adaptively
    /// based on its rolling bandwidth estimate.
    pub fallback_variant: Option<(u32, u64)>,
    /// Source video height in pixels. None when the source row is
    /// missing the `height` column (rare; very old scans). Used by
    /// the admin dashboard to show "Encoding 4K → 1080p" and by the
    /// player to grey out impractical quality tiers.
    pub source_height: Option<u32>,
    /// Which encoder ffmpeg is actually running. Captured at start so
    /// the admin dashboard can attribute per-session CPU usage.
    pub hwaccel: HwAccel,
    /// Speed-vs-quality preset applied to the encoder. Captured so the
    /// admin dashboard / stats overlay can show what tradeoff is in
    /// effect for any given session.
    pub encoder_preset: EncoderPreset,
    /// Whether ffmpeg is re-encoding the video stream or just copying
    /// source packets through. See [`VideoTreatment`].
    pub video_treatment: VideoTreatment,
    /// Same for audio.
    pub audio_treatment: AudioTreatment,
    /// HLS segment container. TS for the common copy/reencode-into-
    /// H.264-AAC case; fMP4 when we need wider codec support to
    /// keep a copy fast-path viable (Opus, FLAC, AV1, VP9 sources
    /// going to a browser that supports them in fMP4 HLS).
    pub container_format: ContainerFormat,
    /// Lowercase codec names of what the player actually receives —
    /// "h264" / "hevc" / "av1" / "vp9" for video, "aac" / "ac3" /
    /// "eac3" / "opus" / "flac" for audio. Resolved once at session
    /// start based on the treatment choice: reencode lands on the
    /// encoder's fixed output codec; copy passes the source through.
    /// Used by [`master_playlist`] to advertise accurate CODECS
    /// attributes so HLS.js's variant-pick logic doesn't reject the
    /// stream on a codec-mismatch parse.
    pub output_video_codec: String,
    pub output_audio_codec: String,
    /// Per-stream selection state, retained on the session so the
    /// [`TranscodeManager::find_compatible`] lookup can match a fresh
    /// request against an existing session by the same (user, file,
    /// audio, subtitle, quality, normalize) tuple — the case where a
    /// user refreshes the watch page mid-playback. None values mean
    /// "transcoder default" (first audio, no subtitle, etc.).
    pub audio_index: Option<u32>,
    pub subtitle_index: Option<u32>,
    pub audio_normalize: bool,
    /// When `Some`, the session emits a WebVTT subtitle track as a
    /// sidecar in the master playlist (`#EXT-X-MEDIA:TYPE=SUBTITLES`)
    /// instead of burning subtitles into the video. The video
    /// pipeline runs without a subtitle filter and starts almost
    /// immediately. See [`WebVttSidecar`].
    pub webvtt_sidecar: Option<WebVttSidecar>,
    last_seen: AtomicI64,
    /// Cumulative bytes served from this session (segment GETs +
    /// master / variant playlist GETs). Incremented by the stream
    /// handlers via [`Self::add_bytes_served`]; flushed to the
    /// `playback_events` table at session-close time so the admin
    /// Stats page can show per-stream bandwidth without per-segment
    /// DB writes.
    bytes_served: AtomicI64,
    /// Pause state — set true while the ffmpeg child is SIGSTOP'd.
    /// Used by the keepalive reaper to avoid killing a session that's
    /// intentionally suspended (paused for a long time), and surfaced
    /// in the snapshot so the admin dashboard can show "▌▌ paused"
    /// next to those sessions.
    paused: AtomicBool,
    _child: Mutex<Child>,
}

impl Drop for Session {
    fn drop(&mut self) {
        // Log when the Session struct is dropped so we can correlate
        // ffmpeg deaths with our own kill paths. If we see this BEFORE
        // the "ffmpeg child exited" log, our code (reaper, DELETE,
        // unmount cleanup, etc.) killed it. If we see "child exited"
        // BEFORE this Drop, ffmpeg died on its own (OOM, GPU crash,
        // internal error) — investigate at the kernel/driver level.
        let pid = self
            ._child
            .lock()
            .ok()
            .and_then(|c| c.id());
        info!(
            session_id = %self.id,
            ?pid,
            elapsed_ms = now_ms() - self.created_at,
            "session struct dropped (kill_on_drop will signal ffmpeg)",
        );
    }
}

impl Session {
    pub fn touch(&self) {
        self.last_seen.store(now_ms(), Ordering::Relaxed);
    }

    pub fn last_seen(&self) -> i64 {
        self.last_seen.load(Ordering::Relaxed)
    }

    /// Add `n` bytes to the rolling per-session bandwidth counter.
    /// Called from the segment + playlist handlers after a response
    /// body has been written. Relaxed ordering is fine here — we read
    /// the value once at session-close time and slight delivery-order
    /// reordering against a concurrent read doesn't affect the
    /// "bytes served by this stream" aggregate.
    pub fn add_bytes_served(&self, n: u64) {
        // Saturate at i64::MAX so a runaway counter can't wrap. The
        // i64 limit is ~9.2 EB, well beyond any realistic stream.
        let n = n.min(i64::MAX as u64) as i64;
        self.bytes_served.fetch_add(n, Ordering::Relaxed);
    }

    /// Snapshot of cumulative bytes served. Used by the manager when
    /// emitting the `stop` event on session close so the admin Stats
    /// page can show per-stream bandwidth.
    pub fn bytes_served(&self) -> i64 {
        self.bytes_served.load(Ordering::Relaxed)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    /// Suspend the ffmpeg child via SIGSTOP. Stops the encoder from
    /// burning CPU/GPU and writing new segments while the player is
    /// paused. Idempotent — re-pausing a paused session is a no-op.
    /// Returns `false` if we couldn't reach the pid (already exited,
    /// or non-Unix platform); the manager treats that as "best effort
    /// done, keep going".
    pub fn pause(&self) -> bool {
        if self.paused.swap(true, Ordering::AcqRel) {
            return true;
        }
        self.signal_child(signal::Pause)
    }

    /// Resume a paused ffmpeg child via SIGCONT. Idempotent. Also
    /// touches the session so the idle reaper doesn't decide to kill
    /// it right after resume (a long pause may have brought last_seen
    /// close to the threshold).
    pub fn resume(&self) -> bool {
        if !self.paused.swap(false, Ordering::AcqRel) {
            return true;
        }
        self.touch();
        self.signal_child(signal::Continue)
    }

    /// Send a signal to the ffmpeg child. Holds the child lock just
    /// long enough to read the pid; the actual kill() is outside the
    /// lock so we don't block other session operations.
    fn signal_child(&self, sig: signal::Kind) -> bool {
        let pid = {
            let guard = self._child.lock().expect("child lock");
            guard.id()
        };
        let Some(pid) = pid else {
            return false;
        };
        signal::send(pid, sig)
    }

    pub fn variant_name() -> &'static str {
        VARIANT_NAME
    }

    /// Returns true if the given directory name is one of this
    /// session's variants. Used by the variant_file handler to
    /// reject 404 paths quickly — with ABR off only `v1` is valid;
    /// with ABR on both `v1` and `v2` are. Sessions with a WebVTT
    /// sidecar additionally serve files under `sub/`.
    pub fn is_known_variant(&self, name: &str) -> bool {
        if name == VARIANT_NAME {
            return true;
        }
        if self.fallback_variant.is_some() && name == FALLBACK_VARIANT_NAME {
            return true;
        }
        self.webvtt_sidecar.is_some() && name == SUBTITLE_VARIANT_NAME
    }

    /// Synthesize the master playlist for this session. RESOLUTION
    /// width is derived from the target height assuming a 16:9 ratio,
    /// which is the common case and what scale=-2 will produce. Off-
    /// aspect content (4:3, 21:9) still plays — HLS just advertises
    /// a hint, not a requirement.
    ///
    /// When ABR is on, the master lists both variants — primary
    /// first so HLS.js starts there on good links and only drops to
    /// the fallback when its bandwidth estimate sustains below the
    /// primary's BANDWIDTH cutoff.
    ///
    /// HLS version 7 is required for fMP4 segments (`#EXT-X-MAP`).
    /// We bump the version uniformly so the difference between TS
    /// and fMP4 sessions doesn't matter to clients — every modern
    /// player understands v7+.
    pub fn master_playlist(&self) -> String {
        let codecs = format!(
            "{},{}",
            codec_string_for(&self.output_video_codec),
            codec_string_for(&self.output_audio_codec),
        );
        let mut out = String::from("#EXTM3U\n#EXT-X-VERSION:7\n");
        // Subtitle sidecar group. Emitted BEFORE the STREAM-INF lines
        // per HLS convention (media groups must be defined before
        // they're referenced). DEFAULT=YES + AUTOSELECT=YES tells
        // the player to enable this track on first load — there's
        // only one and the user explicitly picked it, so we want it
        // visible immediately. Without DEFAULT, HLS.js loads the
        // playlist but leaves the track in `disabled` mode, which
        // looks identical to "no subs" to the user.
        let subs_attr = if let Some(sc) = &self.webvtt_sidecar {
            let name = sanitize_for_playlist_attr(&sc.display_name);
            let lang = sanitize_for_playlist_attr(&sc.language);
            out.push_str(&format!(
                "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"{SUBTITLE_GROUP}\",NAME=\"{name}\",LANGUAGE=\"{lang}\",DEFAULT=YES,AUTOSELECT=YES,FORCED=NO,URI=\"{SUBTITLE_VARIANT_NAME}/index.m3u8\"\n",
            ));
            format!(",SUBTITLES=\"{SUBTITLE_GROUP}\"")
        } else {
            String::new()
        };
        let h = self.target_height;
        let w = (h as u64 * 16 / 9) as u32;
        let bw = self.target_video_bitrate_bps + 200_000;
        out.push_str(&format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={bw},RESOLUTION={w}x{h},CODECS=\"{codecs}\"{subs_attr}\n{VARIANT_NAME}/index.m3u8\n",
        ));
        if let Some((fh, fbps)) = self.fallback_variant {
            let fw = (fh as u64 * 16 / 9) as u32;
            let fbw = fbps + 200_000;
            out.push_str(&format!(
                "#EXT-X-STREAM-INF:BANDWIDTH={fbw},RESOLUTION={fw}x{fh},CODECS=\"{codecs}\"{subs_attr}\n{FALLBACK_VARIANT_NAME}/index.m3u8\n",
            ));
        }
        out
    }
}

/// Strip characters that would terminate or corrupt an HLS playlist
/// attribute value. The HLS spec restricts attribute string values to
/// quoted-string syntax with no embedded `"` or newline; in practice
/// we also strip `,` since some buggy clients split on it even inside
/// quotes. Display names like "Cantonese (Traditional, Hong Kong)"
/// become "Cantonese (Traditional Hong Kong)" — readable, won't
/// break parsing.
fn sanitize_for_playlist_attr(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '"' | '\n' | '\r' | ','))
        .collect()
}

/// RFC 6381 codec identifier for a given codec short-name. These
/// are the strings browsers expect inside the HLS `CODECS=` attr
/// (and inside an MSE `MediaSource.isTypeSupported` query). When
/// we don't have a known mapping the function returns the input
/// name lowercased — HLS spec allows arbitrary tokens, and most
/// players are forgiving about unknowns when the segment can be
/// successfully demuxed.
fn codec_string_for(codec: &str) -> String {
    match codec.to_ascii_lowercase().as_str() {
        "h264" | "avc" | "x264" => "avc1.4d401f".to_string(),
        "hevc" | "h265" | "x265" => "hev1.1.6.L93.B0".to_string(),
        "av1" | "av01" => "av01.0.04M.08".to_string(),
        "vp9" | "vp09" => "vp09.00.10.08".to_string(),
        "aac" => "mp4a.40.2".to_string(),
        "ac3" | "ac-3" => "ac-3".to_string(),
        "eac3" | "e-ac-3" | "ec-3" => "ec-3".to_string(),
        "opus" => "opus".to_string(),
        "flac" => "flac".to_string(),
        "mp3" | "mpga" => "mp4a.40.34".to_string(),
        other => other.to_string(),
    }
}

#[derive(Clone)]
pub struct TranscodeManager {
    inner: Arc<Inner>,
}

struct Inner {
    cache_root: PathBuf,
    ffmpeg: FfmpegConfig,
    /// Capabilities probed once at server startup (encoder list,
    /// per-hwaccel decoder list). Used by [`Self::start`] to decide
    /// whether to emit a `-hwaccel <name>` hint for the source
    /// codec — only do so if the runtime probe confirmed this card
    /// can actually decode the codec, otherwise software-decode and
    /// just use the GPU for the encode side.
    capabilities: Arc<crate::TranscoderCapabilities>,
    sessions: RwLock<HashMap<String, Arc<Session>>>,
}

impl TranscodeManager {
    pub fn new(
        cache_root: PathBuf,
        ffmpeg: FfmpegConfig,
        capabilities: Arc<crate::TranscoderCapabilities>,
    ) -> Result<Self> {
        std::fs::create_dir_all(&cache_root)
            .with_context(|| format!("create transcode cache dir {}", cache_root.display()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                cache_root,
                ffmpeg,
                capabilities,
                sessions: RwLock::new(HashMap::new()),
            }),
        })
    }

    /// Read accessor for the capability probe — callers (the session
    /// API in particular) need to know which encoders are actually
    /// available before deciding whether to request HEVC.
    pub fn capabilities(&self) -> Arc<crate::TranscoderCapabilities> {
        self.inner.capabilities.clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        media_file_id: i64,
        media_file_path: &Path,
        start_position_ms: i64,
        duration_ms: Option<i64>,
        user_id: i64,
        audio_index: Option<u32>,
        subtitle_index: Option<u32>,
        subtitle_codec: Option<&str>,
        subtitle_language: Option<&str>,
        subtitle_title: Option<&str>,
        subtitle_offset_ms: i64,
        hdr_format: Option<&str>,
        subtitle_style: Option<&str>,
        quality_target: Option<(u32, u64)>,
        hwaccel: HwAccel,
        encoder_preset: EncoderPreset,
        video_treatment: VideoTreatment,
        audio_treatment: AudioTreatment,
        audio_bitrate_bps: u64,
        audio_normalize: bool,
        source_height: Option<u32>,
        fallback_variant: Option<(u32, u64)>,
        container_format: ContainerFormat,
        source_video_codec: Option<&str>,
        source_audio_codec: Option<&str>,
        source_pix_fmt: Option<&str>,
        tonemap: TonemapConfig,
        target_video_codec: VideoCodec,
        gpu_device: &str,
        loudness_target: Option<LoudnessTarget>,
    ) -> Result<Arc<Session>> {
        let id = generate_id();
        let session_dir = self.inner.cache_root.join(&id);
        let variant_dir = session_dir.join(VARIANT_NAME);
        tokio::fs::create_dir_all(&variant_dir)
            .await
            .with_context(|| format!("create session dir {}", variant_dir.display()))?;

        // Clamp height and bitrate to sane bounds so a malformed client
        // request can't ask ffmpeg to emit 16K @ 10 Gbps. Defaults kick
        // in when the caller doesn't override.
        let (target_height, target_video_bitrate_bps) = match quality_target {
            Some((h, b)) => (
                h.clamp(144, 2160),
                b.clamp(100_000, 50_000_000),
            ),
            None => (DEFAULT_TARGET_HEIGHT, DEFAULT_TARGET_VIDEO_BITRATE_BPS),
        };

        // ABR is only safe on the reencode-without-burn path. A subtitle
        // BURN would require duplicating the `subtitles=` filter into
        // each split branch — workable but error-prone (the filter
        // re-opens the file for each branch); we save that for a future
        // refactor. Sidecar (text) subtitles don't touch the video
        // filter graph at all, so they compose fine with ABR. Copy
        // sessions can't ABR because there's no encoder to retarget.
        // Fallback also has to be strictly smaller than the primary or
        // it adds no value.
        let subtitle_is_burn = subtitle_index.is_some()
            && !subtitle_codec.is_some_and(is_text_subtitle_codec);
        let abr_eligible = matches!(video_treatment, VideoTreatment::Reencode)
            && !subtitle_is_burn;
        let resolved_fallback = if abr_eligible {
            fallback_variant.and_then(|(fh, fbps)| {
                let fh = fh.clamp(144, 2160);
                let fbps = fbps.clamp(100_000, 50_000_000);
                if fh < target_height && fbps < target_video_bitrate_bps {
                    Some((fh, fbps))
                } else {
                    None
                }
            })
        } else {
            None
        };
        if resolved_fallback.is_some() {
            let fb_dir = session_dir.join(FALLBACK_VARIANT_NAME);
            tokio::fs::create_dir_all(&fb_dir)
                .await
                .with_context(|| format!("create fallback variant dir {}", fb_dir.display()))?;
        }

        // Resolve the codec the player will actually see for each
        // stream. Reencode always lands on H.264 / AAC (the only
        // encoders we configure); copy passes the source codec
        // through. Used by the master playlist's CODECS attribute
        // and by the container-format gating below.
        let output_video_codec = match video_treatment {
            VideoTreatment::Reencode => target_video_codec.label().to_string(),
            VideoTreatment::Copy => source_video_codec
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_else(|| "h264".to_string()),
        };
        // HEVC inside MPEG-TS is fraught — every browser that
        // understands HEVC expects it inside fMP4 (#EXT-X-MAP +
        // hvc1 tag). Force-upgrade the container when the operator
        // (or auto-fallback) picked HEVC, regardless of what the
        // request asked for. H264 keeps the operator's container
        // choice.
        let container_format = if matches!(target_video_codec, VideoCodec::Hevc)
            && matches!(video_treatment, VideoTreatment::Reencode)
        {
            ContainerFormat::Fmp4
        } else {
            container_format
        };
        // HEVC ABR: enabled on hardware encoders (NVENC / QSV / VAAPI
        // / VideoToolbox / AMF — each session gets its own GPU
        // encoder context so two parallel sub-pipelines compose
        // fine) but DISABLED on software libx265 (single-process
        // x265 keeps shared state across encoder contexts and dual
        // sub-pipelines can deadlock on rate-control coordination).
        // The conservative choice — if the operator paid for an HEVC
        // GPU they get the better experience; software-fallback HEVC
        // sessions still play, just without the secondary variant.
        let resolved_fallback = if matches!(target_video_codec, VideoCodec::Hevc)
            && matches!(hwaccel, crate::HwAccel::None)
        {
            None
        } else {
            resolved_fallback
        };
        let output_audio_codec = match audio_treatment {
            AudioTreatment::Reencode => "aac".to_string(),
            AudioTreatment::Copy => source_audio_codec
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_else(|| "aac".to_string()),
        };

        // Decide whether to emit `-hwaccel <name>` based on the
        // runtime capability probe. We do this here (not inside
        // hwaccel.rs) so the decision can incorporate the actual
        // detected per-card decoder list — an RTX 5070 Ti's
        // capabilities probe will list `av1` under `cuda` and we
        // light up NVDEC for it; a GTX 1050's probe won't list
        // `av1` and we silently fall back to software decode for
        // the same source. No card-model database needed.
        let use_hwaccel_decode = match hwaccel.paired_decoder() {
            Some(name) => source_video_codec
                .map(|c| normalize_codec_for_decoder(c))
                .is_some_and(|c| self.inner.capabilities.decoders.supports(name, &c)),
            None => false,
        };

        // ffmpeg 5.1's `scale_cuda` cannot reliably convert 10-bit
        // CUDA frames (P010 / yuv420p10le_cuda from NVDEC AV1 or
        // HEVC Main10) — sessions crash with "Impossible to convert
        // between the formats supported by the filter 'graph 0
        // input from stream 0:0' and the filter 'auto_scale_0'".
        // The fix that landed in ffmpeg 6.0 isn't in Debian
        // bookworm's package, so we work around it here: when the
        // source is 10-bit, force the CPU scale path (decoded
        // frames still come from NVDEC but get downloaded to CPU
        // before scaling). Slight throughput hit on PCIe vs the
        // pure-GPU pipeline, but the alternative is the session
        // dying with no segments.
        let source_is_10bit = source_pix_fmt
            .map(|f| {
                let lower = f.to_ascii_lowercase();
                lower.contains("10le")
                    || lower.contains("10be")
                    || lower.starts_with("p010")
                    || lower.starts_with("p012")
                    || lower.starts_with("p016")
                    || lower.contains("12le")
                    || lower.contains("12be")
            })
            .unwrap_or(false);

        // For text subtitles, spawn the WebVTT extraction in the
        // background so the `/sessions` HTTP request returns
        // immediately. On a 30 GB Bluray remux extraction can take
        // multiple minutes (ffmpeg has to scan most of the file to
        // find every subtitle packet), and synchronously awaiting
        // that here would time out the proxy / fetch and surface
        // as a "Playback failed" error before the video even
        // starts.
        //
        // The master playlist always references the sidecar URI
        // when text subs are picked; the `sub/*` HTTP handlers
        // wait on the watch channel below before serving files,
        // with their own timeout. Player UX: video plays
        // immediately, subs appear when extraction completes.
        //
        // Picture subs (PGS / DVD / VobSub) skip the sidecar
        // entirely — browsers can't render bitmaps as text
        // tracks, so they still take the burn path.
        let start_seconds = (start_position_ms.max(0) as f64) / 1000.0;
        let webvtt_sidecar: Option<WebVttSidecar> = if let Some(si) = subtitle_index {
            if matches!(subtitle_kind(subtitle_codec), SubtitleKind::Text) {
                let language = subtitle_language
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "und".to_string());
                let display_name = subtitle_title
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| {
                        if language == "und" {
                            "Subtitle".to_string()
                        } else {
                            language.clone()
                        }
                    });
                let (tx, rx) = tokio::sync::watch::channel(SubExtractionStatus::Pending);
                let ffmpeg_cfg = self.inner.ffmpeg.clone();
                let media_path = media_file_path.to_path_buf();
                let session_dir_clone = session_dir.clone();
                // Remaining duration is used for the playlist's
                // EXTINF / TARGETDURATION. Over-stating is
                // harmless; under-stating cuts subs off early.
                let total_seconds = duration_ms
                    .map(|d| (d.max(0) as f64) / 1000.0)
                    .unwrap_or(3.0 * 3600.0);
                let remaining = (total_seconds - start_seconds).max(60.0);
                let session_id_for_log = id.clone();
                let captured_offset_ms = subtitle_offset_ms;
                tokio::spawn(async move {
                    let res = extract_webvtt_sidecar(
                        &ffmpeg_cfg,
                        &media_path,
                        si,
                        start_seconds,
                        remaining,
                        captured_offset_ms,
                        &session_dir_clone,
                    )
                    .await;
                    let status = match res {
                        Ok(()) => SubExtractionStatus::Ready,
                        Err(e) => {
                            warn!(
                                session_id = %session_id_for_log,
                                error = %e,
                                si,
                                "webvtt sidecar extraction failed"
                            );
                            SubExtractionStatus::Failed(e.to_string())
                        }
                    };
                    let _ = tx.send(status);
                });
                Some(WebVttSidecar {
                    language,
                    display_name,
                    progress: Arc::new(rx),
                })
            } else {
                None
            }
        } else {
            None
        };

        let child = spawn_ffmpeg(
            &self.inner.ffmpeg,
            media_file_path,
            &session_dir,
            start_position_ms,
            audio_index,
            subtitle_index,
            subtitle_codec,
            hdr_format,
            sanitize_subtitle_style(subtitle_style),
            target_height,
            target_video_bitrate_bps,
            hwaccel,
            encoder_preset,
            target_video_codec,
            video_treatment,
            audio_treatment,
            audio_bitrate_bps,
            audio_normalize,
            resolved_fallback,
            container_format,
            use_hwaccel_decode,
            source_is_10bit,
            webvtt_sidecar.is_some(),
            &id,
            &tonemap,
            gpu_device,
            loudness_target.as_ref(),
        )
        .await?;

        let now = now_ms();
        let session = Arc::new(Session {
            id: id.clone(),
            user_id,
            media_file_id,
            output_dir: session_dir,
            start_position_ms,
            duration_ms,
            created_at: now,
            target_height,
            target_video_bitrate_bps,
            fallback_variant: resolved_fallback,
            container_format,
            output_video_codec,
            output_audio_codec,
            source_height,
            hwaccel,
            encoder_preset,
            video_treatment,
            audio_treatment,
            audio_index,
            subtitle_index,
            audio_normalize,
            webvtt_sidecar,
            last_seen: AtomicI64::new(now),
            bytes_served: AtomicI64::new(0),
            paused: AtomicBool::new(false),
            _child: Mutex::new(child),
        });

        self.inner
            .sessions
            .write()
            .expect("sessions lock")
            .insert(id, session.clone());
        info!(
            session_id = %session.id,
            media_file_id,
            user_id,
            "transcode session started"
        );
        Ok(session)
    }

    pub fn get(&self, id: &str) -> Option<Arc<Session>> {
        self.inner
            .sessions
            .read()
            .expect("sessions lock")
            .get(id)
            .cloned()
    }

    /// Find an existing session matching all of the supplied
    /// parameters. Used to short-circuit `create_session` when a
    /// reload / re-mount lands on a still-running session — we hand
    /// the existing session back instead of spinning up a fresh
    /// ffmpeg. Match is by full parameter tuple because anything else
    /// (different audio track, different quality tier, etc.) would
    /// need re-encoding from scratch anyway.
    ///
    /// Position match deliberately allows the request to be *later*
    /// than the session's start position. The session started encoding
    /// at `session.start_position_ms` and has been producing segments
    /// forward at realtime ever since; any request whose start falls
    /// between the session start and (roughly) the encoded extent is
    /// safe to adopt — the player just seeks into the existing HLS
    /// buffer at `(request - session.start) / 1000`. A small backward
    /// tolerance accommodates resume-position bounce between the SSR
    /// read and the in-browser read.
    ///
    /// The upper bound (`MAX_AHEAD_MS`) protects against pathological
    /// adoptions of very-old sessions (e.g. user opened the page
    /// hours ago and the keepalive somehow stayed alive). The idle
    /// reaper handles the common case but the bound is defense in
    /// depth.
    #[allow(clippy::too_many_arguments)]
    pub fn find_compatible(
        &self,
        user_id: i64,
        media_file_id: i64,
        start_position_ms: i64,
        audio_index: Option<u32>,
        subtitle_index: Option<u32>,
        target_height: u32,
        target_video_bitrate_bps: u64,
        audio_normalize: bool,
        fallback_variant: Option<(u32, u64)>,
    ) -> Option<Arc<Session>> {
        const BACK_TOLERANCE_MS: i64 = 5_000;
        const MAX_AHEAD_MS: i64 = 30 * 60 * 1000;
        let map = self.inner.sessions.read().expect("sessions lock");
        map.values()
            .find(|s| {
                let pos_delta = start_position_ms - s.start_position_ms;
                s.user_id == user_id
                    && s.media_file_id == media_file_id
                    && s.audio_index == audio_index
                    && s.subtitle_index == subtitle_index
                    && s.target_height == target_height
                    && s.target_video_bitrate_bps == target_video_bitrate_bps
                    && s.audio_normalize == audio_normalize
                    // ABR shape has to match: adopting a single-variant
                    // session for an ABR request would leave HLS.js
                    // asking for `v2/index.m3u8` and 404'ing; the
                    // reverse just wastes the second encoder branch.
                    && s.fallback_variant == fallback_variant
                    && pos_delta >= -BACK_TOLERANCE_MS
                    && pos_delta <= MAX_AHEAD_MS
            })
            .cloned()
    }

    pub async fn delete(&self, id: &str) -> bool {
        self.delete_returning(id).await.is_some()
    }

    /// Same as `delete` but returns a snapshot of the session before
    /// it was destroyed — used by the server-crate hooks that emit
    /// `stop` events to the playback_events table so the admin Stats
    /// page can show per-stream bandwidth (`bytes_served`) and final
    /// session metadata. Returns None if there was no such session.
    pub async fn delete_returning(&self, id: &str) -> Option<SessionSnapshot> {
        let session = self
            .inner
            .sessions
            .write()
            .expect("sessions lock")
            .remove(id);
        let session = session?;
        // Build the snapshot before dropping the Arc so `bytes_served`
        // captures the cumulative count before the encoder dies.
        let snapshot = SessionSnapshot {
            id: session.id.clone(),
            user_id: session.user_id,
            media_file_id: session.media_file_id,
            start_position_ms: session.start_position_ms,
            duration_ms: session.duration_ms,
            created_at: session.created_at,
            last_seen_at: session.last_seen(),
            encoder: session.hwaccel.label().to_string(),
            video_treatment: session.video_treatment,
            audio_treatment: session.audio_treatment,
            source_height: session.source_height,
            target_height: session.target_height,
            target_video_bitrate_bps: session.target_video_bitrate_bps,
            encoder_preset: session.encoder_preset.label().to_string(),
            paused: session.is_paused(),
            bytes_served: session.bytes_served(),
        };
        let path = session.output_dir.clone();
        drop(session); // Dropping kills the ffmpeg child via kill_on_drop.
        let _ = tokio::fs::remove_dir_all(&path).await;
        info!(session_id = id, "transcode session deleted");
        Some(snapshot)
    }

    /// Snapshot of every currently-running session. Used by the admin
    /// dashboard; callers receive plain data and don't share state with
    /// the live `Session` objects.
    pub fn list_sessions(&self) -> Vec<SessionSnapshot> {
        let map = self.inner.sessions.read().expect("sessions lock");
        map.values()
            .map(|s| SessionSnapshot {
                id: s.id.clone(),
                user_id: s.user_id,
                media_file_id: s.media_file_id,
                start_position_ms: s.start_position_ms,
                duration_ms: s.duration_ms,
                created_at: s.created_at,
                last_seen_at: s.last_seen(),
                encoder: s.hwaccel.label().to_string(),
                video_treatment: s.video_treatment,
                audio_treatment: s.audio_treatment,
                source_height: s.source_height,
                target_height: s.target_height,
                target_video_bitrate_bps: s.target_video_bitrate_bps,
                encoder_preset: s.encoder_preset.label().to_string(),
                paused: s.is_paused(),
                bytes_served: s.bytes_served(),
            })
            .collect()
    }

    pub async fn reap_idle(&self, idle_threshold_ms: i64) -> Vec<SessionSnapshot> {
        let now = now_ms();
        let stale: Vec<String> = {
            let map = self.inner.sessions.read().expect("sessions lock");
            map.iter()
                // Reap if last_seen has fallen behind, regardless of
                // pause state. The player's keepalive ping bumps
                // last_seen every 60s while the page is alive (even
                // when the video is paused), so a genuinely-active
                // paused user stays safe. Skipping paused sessions
                // entirely (the previous policy) leaked them whenever
                // a mobile user backgrounded the app without an
                // explicit teardown — the SIGSTOP'd ffmpeg held GPU
                // resources indefinitely and starved subsequent
                // sessions on the same card.
                .filter(|(_, s)| now - s.last_seen() > idle_threshold_ms)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let mut reaped = Vec::with_capacity(stale.len());
        for id in stale {
            if let Some(snap) = self.delete_returning(&id).await {
                reaped.push(snap);
            }
        }
        if !reaped.is_empty() {
            debug!(count = reaped.len(), "reaped idle transcode sessions");
        }
        reaped
    }

    /// Cache root for this transcoder. Exposed so callers (the
    /// scanner, the orphan-cleanup job) can compute subtitle cache
    /// paths without going through a method round-trip.
    pub fn cache_root(&self) -> &Path {
        &self.inner.cache_root
    }

    /// Pre-warm the WebVTT subtitle cache for a source file. Called
    /// by the scanner after stream metadata is in the DB so that
    /// when a user picks a text subtitle later, the cache already
    /// has the extracted `.vtt` and we can serve it instantly
    /// (Plex parity). The cache lives at
    /// `<cache_root>/subs/<path-key>/<si>.vtt`.
    ///
    /// `text_sub_indices` are the 0-based subtitle-only stream
    /// indices to extract — the caller filters by codec, since
    /// only text codecs (SRT / ASS / etc.) can become WebVTT.
    /// Picture codecs (PGS / DVD / VobSub) are silently skipped
    /// even if passed in.
    ///
    /// Already-cached entries are no-ops; this is safe to call on
    /// every scan. Failures are logged and the function returns
    /// `Ok(())` — a failed pre-warm just means the user's first
    /// play will fall back to session-time extraction, not that
    /// playback is broken.
    pub async fn ensure_text_subs_cached(
        &self,
        input: &Path,
        text_sub_indices: &[u32],
    ) -> Result<()> {
        let cache_root = &self.inner.cache_root;
        let key = path_cache_key(input);
        for &si in text_sub_indices {
            let cache_vtt = cache_root.join("subs").join(&key).join(format!("{si}.vtt"));
            if tokio::fs::metadata(&cache_vtt).await.is_ok() {
                continue;
            }
            if let Err(e) = extract_full_webvtt_to(&self.inner.ffmpeg, input, si, &cache_vtt).await
            {
                warn!(
                    error = %e,
                    si,
                    input = %input.display(),
                    "scan-time webvtt extraction failed; session start will retry"
                );
                continue;
            }
            info!(
                cache = %cache_vtt.display(),
                si,
                "subtitle pre-warmed during scan"
            );
        }
        Ok(())
    }

    /// Delete cached subtitle files for a source. Called by the
    /// orphan-cleanup job when a media file is purged so we don't
    /// keep stale `.vtt` blobs forever.
    pub async fn evict_text_subs_cache(&self, input: &Path) -> Result<()> {
        let cache_root = &self.inner.cache_root;
        let key = path_cache_key(input);
        let dir = cache_root.join("subs").join(&key);
        if tokio::fs::metadata(&dir).await.is_ok() {
            tokio::fs::remove_dir_all(&dir).await.ok();
        }
        Ok(())
    }

    /// Spawn a background task that periodically reaps idle sessions.
    pub fn spawn_reaper(&self, idle_threshold_ms: i64, interval_s: u64) {
        self.spawn_reaper_with_hook(idle_threshold_ms, interval_s, |_| {});
    }

    /// Same as `spawn_reaper`, but invokes `on_reaped` once per
    /// reaped session with its final snapshot — server-crate uses
    /// this to emit `stop` events to `playback_events` so the admin
    /// Stats page can attribute bandwidth + final session metadata
    /// without a per-segment DB write.
    pub fn spawn_reaper_with_hook<F>(
        &self,
        idle_threshold_ms: i64,
        interval_s: u64,
        on_reaped: F,
    ) where
        F: Fn(SessionSnapshot) + Send + Sync + 'static,
    {
        let manager = self.clone();
        let on_reaped = std::sync::Arc::new(on_reaped);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(interval_s));
            tick.tick().await; // skip the immediate tick
            loop {
                tick.tick().await;
                let reaped = manager.reap_idle(idle_threshold_ms).await;
                for snap in reaped {
                    on_reaped(snap);
                }
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
async fn spawn_ffmpeg(
    cfg: &FfmpegConfig,
    input: &Path,
    session_dir: &Path,
    start_position_ms: i64,
    audio_index: Option<u32>,
    subtitle_index: Option<u32>,
    subtitle_codec: Option<&str>,
    hdr_format: Option<&str>,
    subtitle_style: Option<String>,
    target_height: u32,
    target_video_bitrate_bps: u64,
    hwaccel: HwAccel,
    encoder_preset: EncoderPreset,
    target_video_codec: VideoCodec,
    video_treatment: VideoTreatment,
    audio_treatment: AudioTreatment,
    audio_bitrate_bps: u64,
    audio_normalize: bool,
    fallback_variant: Option<(u32, u64)>,
    container_format: ContainerFormat,
    use_hwaccel_decode: bool,
    source_is_10bit: bool,
    // When true, `start()` already extracted the subtitle to a
    // WebVTT sidecar and wrote its HLS playlist; the main video
    // pipeline should skip the burn filter, `-copyts`, and
    // `setpts`/`asetpts`. The player will overlay the sidecar
    // directly.
    using_sidecar_subtitle: bool,
    session_id: &str,
    tonemap: &TonemapConfig,
    gpu_device: &str,
    loudness_target: Option<&LoudnessTarget>,
) -> Result<Child> {
    // fMP4 segments live under .m4s; TS segments under .ts. Init
    // segment (init.mp4) only exists for fMP4 — ffmpeg writes it
    // once on session start and the variant manifest references it
    // via `#EXT-X-MAP`.
    let segment_ext = match container_format {
        ContainerFormat::Ts => "ts",
        ContainerFormat::Fmp4 => "m4s",
    };
    let primary_dir = session_dir.join(VARIANT_NAME);
    let manifest = primary_dir.join("index.m3u8");
    let segment_pattern = primary_dir.join(format!("seg-%03d.{segment_ext}"));
    let fallback_paths = fallback_variant.map(|_| {
        let d = session_dir.join(FALLBACK_VARIANT_NAME);
        (d.join("index.m3u8"), d.join(format!("seg-%03d.{segment_ext}")))
    });
    let start_seconds = (start_position_ms.max(0) as f64) / 1000.0;

    // Two subtitle-burn strategies depending on codec:
    //   * Text-based (subrip/ass/ssa/mov_text/webvtt) — the `subtitles`
    //     filter reads from the file by stream index.
    //   * Picture-based (hdmv_pgs_subtitle/dvd_subtitle/dvb_subtitle/
    //     vobsub) — the `subtitles` filter doesn't accept them; instead
    //     we map the subtitle stream into a filter graph and overlay it
    //     onto the video.
    //
    // When subtitle_codec is unknown we default to the text path; ffmpeg
    // will fail loudly and the captured stderr will tell us what to do
    // next.
    let mut cmd = Command::new(&cfg.ffmpeg);
    // `-nostdin` is critical when ffmpeg's stdin is wired to /dev/null
    // (as we do via `cmd.stdin(Stdio::null())` further down). Without
    // it, ffmpeg's interactive-mode tty handling reads stdin looking
    // for keypresses, hits immediate EOF, and after some buffered
    // encoding has flushed it interprets that as "user signalled exit"
    // and terminates silently with no error in stderr. Symptom: the
    // session monitor sees segments grow to ~100-200, then ffmpeg
    // disappears around the 40-90s mark with stderr_tail showing only
    // the harmless init warnings. Same flag is already present on the
    // webvtt-extract and gop-probe ffmpeg calls (search the file).
    cmd.args(["-y", "-nostdin", "-loglevel", "warning"]);
    // Bigger probe window applied to every session. The default
    // 5 MB / 5 s probe routinely fails on Bluray rips with multiple
    // subtitle / audio / attachment streams — ffmpeg gives up with
    // "Could not find codec parameters for stream N" warnings that
    // are non-fatal on a video-only pipeline but block PGS overlay
    // entirely (overlay can't synth sub2video frames without
    // dimensions). 100 MB / 100 s is the conservative number the
    // PGS-overlay community settled on; on local-file I/O the
    // additional probe time is sub-100ms even on slow disks.
    cmd.args(["-analyzeduration", "100M", "-probesize", "100M"]);
    // Pre-input device init for encoders that need one (currently
    // only VAAPI's `-vaapi_device /dev/dri/renderD128`). Must come
    // before `-i` because ffmpeg ties hardware contexts to the input
    // they're declared near.
    hwaccel.pre_input_args_with_device(&mut cmd, use_hwaccel_decode, gpu_device);
    // Keep frames on GPU end-to-end for the common reencode case
    // when nothing in the filter graph needs CPU-side frames. This
    // saves the GPU→CPU→GPU roundtrip that would otherwise eat
    // PCIe bandwidth and add CPU memcpy cost. Subtitle burn-in
    // (libass) and HDR tonemap (zscale) both currently require CPU
    // frames, so we drop back to the default (download to CPU) when
    // either is in play. Copy sessions don't decode at all so the
    // flag is irrelevant.
    let needs_cpu_frames = matches!(video_treatment, VideoTreatment::Copy)
        || subtitle_index.is_some()
        || matches!(hdr_format, Some("hdr10" | "hlg" | "dovi"));
    // GPU-native pipeline only makes sense when we're actually
    // hwaccel-decoding — `-hwaccel_output_format cuda` says "keep
    // decoded frames in CUDA memory" which is meaningless if the
    // decoder is software (frames are already in CPU memory and
    // would need uploading first, defeating the optimization).
    //
    // Also disabled for 10-bit sources: ffmpeg 5.1's scale_cuda
    // can't bridge P010 → NV12 reliably (the bug was fixed in
    // ffmpeg 6.0 but Debian bookworm ships 5.1). The failure mode
    // is an "Impossible to convert between the formats supported
    // by the filter 'graph 0 input from stream 0:0' and the
    // filter 'auto_scale_0'" stall right after spawn. Falling back
    // to the CPU scale path adds a PCIe roundtrip but keeps
    // sessions alive on 10-bit anime (AV1 10bit + HEVC Main10).
    let gpu_native = hwaccel.supports_gpu_native_pipeline()
        && !needs_cpu_frames
        && use_hwaccel_decode
        && !source_is_10bit;
    if gpu_native {
        hwaccel.gpu_output_format_args(&mut cmd);
    }
    // Picture-subtitle pre-input args. The overlay filter that burns
    // Picture-subtitle pre-input args. The bigger probe window is
    // applied universally above; what's PGS-specific is:
    //
    //   * `-fix_sub_duration` makes subtitle durations explicit so
    //     sparse PGS streams don't confuse the overlay timeline.
    //   * `-canvas_size 1920x1080` primes sub2video with a fallback
    //     canvas size in case the PGS dimensions still aren't found
    //     after the bigger probe. Belt-and-suspenders.
    // Sidecar mode: subtitle was pre-extracted to a WebVTT file
    // exposed via the master playlist's `#EXT-X-MEDIA` subtitle
    // group. The video pipeline runs without ANY subtitle
    // involvement — no burn filter, no `-copyts`, no setpts/asetpts.
    // The player overlays the sidecar VTT itself, the way Plex does.
    let is_picture_subtitle = !using_sidecar_subtitle
        && subtitle_index.is_some()
        && matches!(subtitle_kind(subtitle_codec), SubtitleKind::Picture);
    let is_text_subtitle = !using_sidecar_subtitle
        && subtitle_index.is_some()
        && matches!(subtitle_kind(subtitle_codec), SubtitleKind::Text);
    if is_picture_subtitle {
        cmd.args(["-fix_sub_duration", "-canvas_size", "1920x1080"]);
    }
    // Burn-path optimization: extract the chosen text subtitle to a
    // small standalone file so the `subtitles=` filter doesn't have
    // to scan the whole source (a 30 GB remux can take 2-3 minutes
    // before the first segment lands). Skipped entirely in sidecar
    // mode because we don't burn at all there.
    //
    // Falls back gracefully: if extraction fails we use the
    // inline-read path with `-copyts` + setpts (same as before).
    let extracted_sub: Option<PathBuf> = if is_text_subtitle {
        let si = subtitle_index.unwrap_or(0);
        extract_text_subtitle(cfg, input, si, start_seconds, subtitle_codec, session_dir).await
    } else {
        None
    };
    // Only the inline burn path needs `-copyts`; the extracted-burn
    // path has already time-shifted the subs so video and subs share
    // PTS=0. Picture-subtitle overlay always uses the inline path
    // (no extraction step yet — bitmap subs need a different
    // container to round-trip cleanly). Sidecar mode needs neither.
    let needs_inline_burn_alignment =
        (is_text_subtitle && extracted_sub.is_none()) || is_picture_subtitle;
    if needs_inline_burn_alignment {
        cmd.arg("-copyts");
    }
    cmd.args(["-ss", &format!("{start_seconds:.3}")])
        .arg("-i")
        .arg(input);

    // Branch on whether the video stream gets re-encoded or just
    // remuxed. `Copy` is the fast path — no filters, no encoder, ~90%
    // less CPU. Picked when the source codec already matches client
    // caps and nothing in the request modifies frames (no subtitle
    // burn, no scaling, no HDR tonemap). See `pick_video_treatment`
    // in stream.rs.
    let kind = subtitle_kind(subtitle_codec);
    let needs_tonemap = matches!(hdr_format, Some("hdr10" | "hlg" | "dovi")) && tonemap.enabled;
    // Helper to emit one variant's audio + HLS muxer args. ffmpeg's
    // CLI grammar is "each output file is preceded by the args that
    // apply to it"; chaining outputs is just calling this twice with
    // different paths and bitrates.
    //
    // Pulled out as a closure (rather than a free function) so it
    // can capture the per-variant audio shape — we encode audio
    // once per variant rather than splitting on the input side; the
    // AAC cost is small enough that this is cheaper than a filter-
    // graph audio split, and it lets each HLS output have its own
    // independent muxer.
    let emit_output =
        |cmd: &mut Command, manifest: &Path, segments: &Path, audio_idx_arg: Option<u32>| {
            if let Some(ai) = audio_idx_arg {
                cmd.args(["-map", &format!("0:a:{ai}")]);
            } else {
                cmd.args(["-map", "0:a:0?"]);
            }
            match audio_treatment {
                AudioTreatment::Copy => {
                    cmd.args(["-c:a", "copy"]);
                }
                AudioTreatment::Reencode => {
                    // Chain the audio filter parts. Order matters:
                    // loudnorm (if requested) operates on the original
                    // PTS, then asetpts resets so the encoder + muxer
                    // see a 0-anchored stream. asetpts only matters
                    // when we used `-copyts` on the input side; the
                    // extracted-subtitle path doesn't need copyts so
                    // audio packets are already 0-anchored from the
                    // input seek.
                    let mut af: Vec<String> = Vec::new();
                    if audio_normalize {
                        // Two-pass mode when stored measurements
                        // are available — feeds the precise input
                        // characteristics so the filter produces
                        // truly linear (no dynamic-range adjustment)
                        // output at the target. Single-pass otherwise
                        // (analysis hasn't been run, or analysis
                        // found no audio); single-pass works fine but
                        // approximates because it has to estimate
                        // from a streaming window.
                        af.push(build_loudnorm_filter(loudness_target));
                    }
                    if needs_inline_burn_alignment {
                        af.push("asetpts=PTS-STARTPTS".to_string());
                    }
                    if !af.is_empty() {
                        let chain = af.join(",");
                        cmd.args(["-af", &chain]);
                    }
                    cmd.args(["-c:a", "aac"])
                        .args(["-b:a", &format!("{audio_bitrate_bps}")])
                        .args(["-ac", "2"]);
                }
            }
            // EVENT playlist + list_size 0 = growing-but-append-only
            // manifest. See the comment near the original single-
            // variant emit; same reasoning per output.
            cmd.args(["-f", "hls"])
                .args(["-hls_time", &HLS_SEGMENT_DURATION_S.to_string()])
                .args(["-hls_playlist_type", "event"])
                .args(["-hls_list_size", "0"]);
            // Container-specific segment plumbing. fMP4 needs an
            // explicit segment-type flag + an init.mp4 path; ffmpeg
            // writes init.mp4 once at session start and the variant
            // manifest references it via `#EXT-X-MAP`. TS is the
            // default and needs no extra flags.
            if matches!(container_format, ContainerFormat::Fmp4) {
                // ffmpeg writes init.mp4 next to the manifest by
                // default; the manifest references it via
                // `#EXT-X-MAP:URI="init.mp4"` automatically.
                cmd.args(["-hls_segment_type", "fmp4"])
                    .args(["-hls_fmp4_init_filename", "init.mp4"]);
            }
            cmd.args(["-hls_segment_filename"])
                .arg(segments)
                .args(["-hls_flags", "independent_segments+temp_file"])
                .arg(manifest);
        };

    if matches!(video_treatment, VideoTreatment::Reencode) {
        // Sidecar mode disables both burn paths — the WebVTT file
        // exposed in the master playlist is handled client-side by
        // the player, so the video filter chain stays subtitle-free.
        let needs_filter_complex = !using_sidecar_subtitle && matches!(kind, SubtitleKind::Picture);
        // Burn as text whenever the caller asked for a subtitle, the
        // codec isn't picture-based, AND we're not running the
        // sidecar fast-path. Earlier the gate was just "kind ==
        // Text"; we widened it so NULL-codec rows (common on old
        // scans) still get the filter, and now narrow it back when
        // sidecar mode is on.
        let burn_text =
            !using_sidecar_subtitle && subtitle_index.is_some() && !needs_filter_complex;
        // HDR → SDR tonemap, only when the source is actually HDR and
        // the operator hasn't disabled tonemap globally. The chain is
        // libzimg (zscale) + libavfilter's tonemap — both ship with
        // most builds of ffmpeg. Without it, libx264 + yuv420p gets a
        // flat, washed-out picture from HDR10/HLG/DV sources.
        //
        // Algorithm is operator-configurable via
        // `server_settings.transcoder_hdr_tonemap_algo`; default
        // `hable` matches the value this was hard-coded to before
        // phase 30.
        let tonemap_chain = tonemap.build_chain(hdr_format);
        let tonemap = tonemap_chain.as_str();
        // Encoders that need their frames in a specific GPU memory
        // format get a small filter appended at the end (e.g.
        // `,format=nv12,hwupload` for VAAPI). For software + NVENC +
        // QSV + VideoToolbox + AMF this is an empty string.
        let hw_suffix = hwaccel.vf_suffix();

        match (fallback_variant, needs_filter_complex) {
            (Some((fb_height, fb_bitrate)), false) => {
                // ABR path: filter_complex with `split` followed by
                // two scale branches feeding two named outputs ([v1],
                // [v2]). Each variant gets its own encoder + audio +
                // HLS muxer downstream. This guarantees a single
                // decode for both encodes — important for heavy
                // codecs like AV1 where decode is the bottleneck.
                //
                // GPU-native pipeline uses the encoder's native
                // scaler (`scale_cuda` / `scale_vaapi`) so frames
                // never round-trip to system memory between decode
                // and encode. Falls back to CPU `scale` when not
                // supported (tonemap + subtitle paths force CPU).
                //
                // `:format=nv12` is critical for 10-bit HEVC sources:
                // NVDEC outputs P010 (10-bit), scale_cuda preserves
                // it by default, and NVENC h264 silently fails to
                // encode P010 input — session hangs with no segments
                // and no error log. NV12 is the universal NVENC h264
                // input; no-op cost when source was already 8-bit.
                let scaler = if gpu_native {
                    hwaccel.gpu_scale_filter()
                } else {
                    "scale"
                };
                let scale_fmt = if gpu_native { ":format=nv12" } else { "" };
                // CPU-side pixel-format normalization for the non-
                // gpu-native case. Same root cause as the gpu_native
                // `:format=nv12`: 10-bit sources (P010 / yuv420p10le)
                // pass straight through `scale` and NVENC h264 +
                // libx264 + AMF either reject them or produce broken
                // output. yuv420p is the universal 8-bit format every
                // encoder accepts. We skip it when hw_suffix already
                // does its own format conversion (VAAPI/QSV's
                // `,format=nv12,hwupload`) — doubling up is harmless
                // but ugly in logs.
                let cpu_fmt = cpu_format_normalization(gpu_native, hw_suffix);
                let fc = format!(
                    "[0:v:0]split=2[v1in][v2in];\
                     [v1in]{tonemap}{scaler}=-2:'min({target_height},ih)'{scale_fmt}{cpu_fmt}{hw_suffix}[v1];\
                     [v2in]{tonemap}{scaler}=-2:'min({fb_height},ih)'{scale_fmt}{cpu_fmt}{hw_suffix}[v2]"
                );
                cmd.args(["-filter_complex", &fc]);

                // Variant 1 (primary).
                cmd.args(["-map", "[v1]"]);
                hwaccel.apply_encoder_for(&mut cmd, target_video_codec, target_video_bitrate_bps, encoder_preset);
                emit_output(&mut cmd, &manifest, &segment_pattern, audio_index);

                // Variant 2 (fallback).
                let (fb_manifest, fb_segments) = fallback_paths
                    .as_ref()
                    .expect("fallback_paths set when fallback_variant set");
                cmd.args(["-map", "[v2]"]);
                hwaccel.apply_encoder_for(&mut cmd, target_video_codec, fb_bitrate, encoder_preset);
                emit_output(&mut cmd, fb_manifest, fb_segments, audio_index);
            }
            (_, true) => {
                // Picture-based subtitle: overlay subtitle stream onto
                // video before scale. ABR is gated off upstream when
                // subtitle_index is set, so we know this is a single-
                // variant output even if fallback_variant somehow
                // arrived (defense in depth — the gate is in start()).
                //
                // `eof_action=pass` — when the subtitle stream ends
                // (no more subs), keep passing video through instead
                // of stopping. `repeatlast=1` keeps the last subtitle
                // visible until replaced. Together with the
                // `-canvas_size` / `-fix_sub_duration` priming on the
                // input side, this stops the overlay filter from
                // blocking on a sparse subtitle stream — the common
                // case where the first PGS event is minutes into the
                // movie.
                // `format=yuv420p` BEFORE the overlay handles the
                // common "source is 10-bit HEVC, NVDEC decoded into
                // P010, overlay can't blend RGBA-on-P010" failure
                // mode — the result is video that plays but with
                // invisible subtitles. Forcing yuv420p first costs
                // one CPU pixel conversion per frame and ensures
                // the PGS bitmap composites correctly.
                let si = subtitle_index.unwrap_or(0);
                let scale = format!("{tonemap}scale=-2:'min({target_height},ih)'");
                // `setpts=PTS-STARTPTS` resets the encoder/muxer-side
                // PTS to 0 after `-copyts` preserved original PTS
                // through the overlay. Without it the HLS muxer
                // writes segments whose first PTS = the seek time,
                // which trips HLS.js manifest-time accounting.
                let fc = format!(
                    "[0:v:0]format=yuv420p[vbase];\
                     [vbase][0:s:{si}]overlay=eof_action=pass:repeatlast=1[vs];\
                     [vs]{scale},setpts=PTS-STARTPTS{hw_suffix}[v]"
                );
                cmd.args(["-filter_complex", &fc]).args(["-map", "[v]"]);
                hwaccel.apply_encoder_for(&mut cmd, target_video_codec, target_video_bitrate_bps, encoder_preset);
                emit_output(&mut cmd, &manifest, &segment_pattern, audio_index);
            }
            (None, false) => {
                // Single-variant reencode, no picture subtitle. The
                // common case before ABR — and still the path taken
                // by sources that don't qualify for ABR (subtitle
                // burn, source already at fallback resolution, etc).
                // GPU-native scaler when applicable (see ABR branch).
                let scaler = if gpu_native {
                    hwaccel.gpu_scale_filter()
                } else {
                    "scale"
                };
                let scale_fmt = if gpu_native { ":format=nv12" } else { "" };
                let scale = format!(
                    "{tonemap}{scaler}=-2:'min({target_height},ih)'{scale_fmt}"
                );
                let mut vf = scale.clone();
                // Normalize to 8-bit YUV before subtitle compositing.
                // libass + the `subtitles=` filter render glyphs into
                // a temporary RGBA layer and composite onto the
                // input frame; with 10-bit (P010 / yuv420p10le)
                // input they're slower and on some builds produce
                // visibly broken output. Doing the conversion here
                // also guarantees the encoder sees 8-bit YUV, which
                // is what NVENC h264 / libx264 / AMF want — the
                // failure mode otherwise is a silent hang (NVENC
                // refuses P010 to its h264 encoder, ffmpeg writes
                // no segments, player spins forever). Skip on
                // gpu_native (scale_cuda already did the conversion)
                // and on VAAPI/QSV (hw_suffix contains the upload
                // chain that does its own format step).
                let cpu_fmt = cpu_format_normalization(gpu_native, hw_suffix);
                if !cpu_fmt.is_empty() {
                    vf = format!("{vf}{cpu_fmt}");
                }
                if burn_text {
                    if let Some(si) = subtitle_index {
                        // Prefer the pre-extracted standalone file
                        // when extraction succeeded. The temp file is
                        // ~few KB so the `subtitles=` filter mmaps it
                        // and starts emitting instantly, vs. a 30 GB
                        // remux it'd have to scan end-to-end. When
                        // extracted, the only stream is the subtitle
                        // we want, so `si=0`.
                        let (sub_path, sub_si) =
                            if let Some(p) = extracted_sub.as_ref() {
                                (p.as_path(), 0u32)
                            } else {
                                (input, si)
                            };
                        let escaped = escape_for_filter(&sub_path.to_string_lossy());
                        vf = format!("{vf},subtitles=filename='{escaped}':si={sub_si}");
                        if let Some(style) = subtitle_style.as_deref() {
                            vf = format!("{vf}:force_style='{style}'");
                        }
                    }
                }
                // Reset PTS to 0 only when we used `-copyts` for
                // sub-alignment. The extracted-subtitle path has both
                // video and subs starting at 0 from input-seek
                // alone, so it doesn't need (and shouldn't have)
                // setpts — that would just be a no-op chain link.
                if needs_inline_burn_alignment {
                    vf = format!("{vf},setpts=PTS-STARTPTS");
                }
                if !hw_suffix.is_empty() {
                    vf = format!("{vf}{hw_suffix}");
                }
                cmd.args(["-vf", &vf]).args(["-map", "0:v:0"]);
                hwaccel.apply_encoder_for(&mut cmd, target_video_codec, target_video_bitrate_bps, encoder_preset);
                emit_output(&mut cmd, &manifest, &segment_pattern, audio_index);
            }
        }
    } else {
        // Copy path: just remux source video into the HLS container.
        // The session was created because audio_index forced a swap;
        // we honor that mapping but leave the video stream untouched.
        //
        // We intentionally do NOT pass `-copyts` here. `-copyts`
        // preserves source PTS values, so with `-ss N -i input` the
        // output segments would start at PTS=N. HLS.js + the HLS
        // muxer don't always agree on how to reconcile that with
        // manifest media-time (which always starts at 0), and some
        // configurations end up never producing a playable segment.
        // ffmpeg's default — reset PTS to 0 after the input seek —
        // matches what the re-encode path does and works reliably.
        cmd.args(["-map", "0:v:0"]).args(["-c:v", "copy"]);
        emit_output(&mut cmd, &manifest, &segment_pattern, audio_index);
    }

    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Reconstruct the full command line for the log. tokio::Command
    // doesn't expose its args list as a string, so we walk it ourselves
    // via `get_program` / `get_args`. Worth the few lines because
    // "what exact ffmpeg invocation crashed?" is the single most
    // useful piece of info when diagnosing a stuck session — a single
    // info-level line lets the operator paste it straight into a
    // shell to reproduce.
    let std_cmd = cmd.as_std();
    let cmdline = std::iter::once(std_cmd.get_program())
        .chain(std_cmd.get_args())
        .map(|os| os.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    info!(
        session_id,
        hwaccel = %hwaccel.label(),
        video_treatment = ?video_treatment,
        audio_treatment = ?audio_treatment,
        audio_normalize,
        "ffmpeg cmdline: {cmdline}",
    );
    debug!(
        ffmpeg = %cfg.ffmpeg,
        input = %input.display(),
        session_dir = %session_dir.display(),
        start_s = start_seconds,
        subtitle_index = ?subtitle_index,
        subtitle_codec = ?subtitle_codec,
        subtitle_kind = ?kind,
        hdr_format = ?hdr_format,
        tonemap = needs_tonemap,
        "spawning ffmpeg"
    );

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn ffmpeg ({}) for transcode session", cfg.ffmpeg))?;

    // Drain stderr in the background. ffmpeg writes warnings + the final
    // exit reason here; without a reader the pipe blocks once its kernel
    // buffer fills (~64 KB) which silently stalls ffmpeg. We log at warn
    // so transcode failures show up in the admin Logs page. When the
    // pipe closes we also emit one terminal line — the child has
    // exited and the operator otherwise has to infer that from the
    // absence of further output. With the cmdline log above this lets
    // a stuck session ("HLS.js spinning forever") be traced back to
    // exactly what ffmpeg refused to do.
    //
    // We also keep a ring of the last few stderr lines so the
    // "child exited" message can echo them at warn level — useful
    // when ffmpeg crashes via signal (SEGV on a bad codec/decoder
    // pairing) and no error line gets printed, only the tail of
    // benign warnings preceding the crash.
    let pid_for_monitor = child.id();
    // Heartbeat monitor: log "still alive" every 15s with the wall-clock
    // age + segment count. When a session mysteriously stops producing
    // segments, this lets the operator see at a glance whether the
    // process is alive-but-stalled (segments same, pid still there) or
    // gone (no heartbeat after time T). Stops on its own when the pid
    // disappears.
    if let Some(pid_u32) = pid_for_monitor {
        let session_id_str = session_id.to_string();
        let session_dir_str = session_dir.to_path_buf();
        let started_at = std::time::Instant::now();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
            tick.tick().await; // skip immediate
            loop {
                tick.tick().await;
                // kill(pid, 0) returns 0 if process exists, -1 + ESRCH
                // if not. Doesn't actually signal anything.
                let alive = unsafe { libc::kill(pid_u32 as libc::pid_t, 0) == 0 };
                if !alive {
                    info!(
                        session_id = %session_id_str,
                        pid = pid_u32,
                        elapsed_s = started_at.elapsed().as_secs(),
                        "session monitor: ffmpeg pid no longer alive — stopping monitor",
                    );
                    break;
                }
                // Count segment files across all variant dirs.
                let mut seg_count = 0_usize;
                if let Ok(mut entries) = tokio::fs::read_dir(&session_dir_str).await {
                    while let Ok(Some(e)) = entries.next_entry().await {
                        if let Ok(ft) = e.file_type().await {
                            if ft.is_dir() {
                                if let Ok(mut inner) = tokio::fs::read_dir(e.path()).await {
                                    while let Ok(Some(f)) = inner.next_entry().await {
                                        if let Some(name) = f.file_name().to_str() {
                                            if name.starts_with("seg-") && name.ends_with(".ts") {
                                                seg_count += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                info!(
                    session_id = %session_id_str,
                    pid = pid_u32,
                    elapsed_s = started_at.elapsed().as_secs(),
                    seg_count,
                    "session monitor: ffmpeg still alive",
                );
            }
        });
    }

    if let Some(stderr) = child.stderr.take() {
        let session_id_str = session_id.to_string();
        let pid = child.id();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            const RING_CAP: usize = 8;
            let mut ring: std::collections::VecDeque<String> =
                std::collections::VecDeque::with_capacity(RING_CAP);
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                // Filter known-benign noise so the admin Logs page
                // isn't drowned in identical warnings on every
                // session. ffmpeg always probes every stream in the
                // container even when we only `-map` a subset, and
                // bitmap subtitles (PGS / DVD / VobSub) often don't
                // carry dimensions in their header — so the probe
                // emits "Could not find codec parameters … :
                // unspecified size" + a "Consider increasing
                // analyzeduration" follow-up. The warnings are
                // harmless because we don't decode the streams.
                // Suppressed here but still kept in the ring buffer
                // so the post-mortem `stderr_tail` log on child
                // exit can include them for real debugging.
                if !is_benign_ffmpeg_line(&line) {
                    warn!(session_id = %session_id_str, ffmpeg = %line, "transcoder");
                }
                if ring.len() == RING_CAP {
                    ring.pop_front();
                }
                ring.push_back(line);
            }
            // Quote the tail of stderr in one line so the operator
            // doesn't have to scroll the Logs page hunting for the
            // last few warnings before the child died.
            let tail = if ring.is_empty() {
                "(no stderr output)".to_string()
            } else {
                ring.into_iter().collect::<Vec<_>>().join(" | ")
            };
            // Capture exit status via waitpid(WNOHANG). We don't call
            // tokio's Child::wait anywhere (the Child handle sits in the
            // Session mutex unawaited), so the child becomes a zombie
            // after exit until something reaps it. waitpid here reaps
            // and gives us the exit code or signal — critical for
            // diagnosing "ffmpeg silently died after N seconds" cases
            // where the stderr_tail is just innocent init warnings.
            // Tiny PID-reuse race with the eventual kill_on_drop is
            // acceptable: the alternative is never knowing what killed
            // ffmpeg.
            let exit_detail = if let Some(pid_u32) = pid {
                let mut status: libc::c_int = 0;
                let r = unsafe {
                    libc::waitpid(
                        pid_u32 as libc::pid_t,
                        &mut status,
                        libc::WNOHANG,
                    )
                };
                if r > 0 {
                    let low = status & 0x7f;
                    if low == 0 {
                        format!("exited code={}", (status >> 8) & 0xff)
                    } else if low != 0x7f {
                        let signal = low;
                        let coredump = (status & 0x80) != 0;
                        format!(
                            "killed signal={signal}{}",
                            if coredump { " (coredump)" } else { "" },
                        )
                    } else {
                        format!("raw_status={status}")
                    }
                } else if r == 0 {
                    "still running (stderr closed without exit?)".to_string()
                } else {
                    let errno = std::io::Error::last_os_error();
                    format!("waitpid_err={errno}")
                }
            } else {
                "no pid captured".to_string()
            };
            warn!(
                session_id = %session_id_str,
                pid = ?pid,
                exit = %exit_detail,
                stderr_tail = %tail,
                "ffmpeg child exited (stderr closed) — session will produce no further segments",
            );
        });
    }

    Ok(child)
}

#[derive(Debug, Clone, Copy)]
enum SubtitleKind {
    /// No subtitle burn requested.
    None,
    /// Text-based (subrip/ass/mov_text/webvtt) — `subtitles=` filter.
    Text,
    /// Picture-based (PGS/DVD/VobSub) — `overlay` via filter_complex.
    Picture,
}

/// Scanner-side entry point for WebVTT cache pre-warming. Mirrors
/// [`TranscodeManager::ensure_text_subs_cached`] but takes the
/// cache root explicitly so the scanner (which doesn't hold a
/// `TranscodeManager` handle, only a [`FfmpegConfig`] + the cache
/// path) can use it directly. Idempotent — already-cached subs
/// are no-ops, so re-scans don't repeat work.
pub async fn scan_prewarm_text_subs(
    cfg: &FfmpegConfig,
    cache_root: &Path,
    input: &Path,
    text_sub_indices: &[u32],
) -> Result<()> {
    let key = path_cache_key(input);
    for &si in text_sub_indices {
        let cache_vtt = cache_root.join("subs").join(&key).join(format!("{si}.vtt"));
        if tokio::fs::metadata(&cache_vtt).await.is_ok() {
            continue;
        }
        if let Err(e) = extract_full_webvtt_to(cfg, input, si, &cache_vtt).await {
            warn!(
                error = %e,
                si,
                input = %input.display(),
                "scan-time webvtt extraction failed; session start will retry"
            );
            continue;
        }
        info!(
            cache = %cache_vtt.display(),
            si,
            "subtitle pre-warmed during scan"
        );
    }
    Ok(())
}

/// Eviction counterpart to [`scan_prewarm_text_subs`]. Called from
/// the orphan-cleanup path so deleted media files don't leave
/// stale `.vtt` blobs in the cache forever.
pub async fn evict_text_subs_cache(cache_root: &Path, input: &Path) -> Result<()> {
    let key = path_cache_key(input);
    let dir = cache_root.join("subs").join(&key);
    if tokio::fs::metadata(&dir).await.is_ok() {
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
    Ok(())
}

/// True when an ffmpeg stderr line is known-noise that we want to
/// suppress from the per-line admin Logs feed. Suppressed lines
/// are still included in the post-mortem `stderr_tail` summary on
/// child exit — the goal is just to stop the Logs page from
/// drowning in identical "Could not find codec parameters" lines
/// (one per non-mapped PGS stream, on every session). They're
/// harmless because we don't map those streams; they appear only
/// because the demuxer probes every stream regardless.
fn is_benign_ffmpeg_line(line: &str) -> bool {
    // PGS / DVD / VobSub probe noise on Bluray remuxes. Match by
    // substring so we catch both the "Could not find codec
    // parameters" warning and the "Consider increasing the value
    // for the 'analyzeduration'" follow-up that always pairs
    // with it.
    if line.contains("Could not find codec parameters")
        && (line.contains("hdmv_pgs_subtitle")
            || line.contains("pgssub")
            || line.contains("dvd_subtitle")
            || line.contains("dvb_subtitle")
            || line.contains("vobsub"))
    {
        return true;
    }
    if line.contains("Consider increasing the value for the 'analyzeduration'") {
        return true;
    }
    false
}

/// True when this codec name produces a stream the `webvtt` muxer
/// can read — SRT, ASS/SSA, mov_text, WebVTT itself. The scanner
/// uses this to decide which subtitle streams are worth pre-
/// extracting to the cache. Picture codecs (PGS / DVD / VobSub /
/// XSUB) and unknown / NULL codecs are excluded — they'd either
/// fail extraction or take the burn-in path at session start.
pub fn is_text_subtitle_codec(codec: &str) -> bool {
    matches!(subtitle_kind(Some(codec)), SubtitleKind::Text)
}

fn subtitle_kind(codec: Option<&str>) -> SubtitleKind {
    let Some(c) = codec else {
        return SubtitleKind::None;
    };
    let c = c.to_ascii_lowercase();
    // Picture-based subtitle codecs have to come first because some of
    // them go by multiple short names depending on which ffmpeg
    // version (or which tool) wrote the row. PGS in particular is
    // seen as `hdmv_pgs_subtitle` (modern ffprobe), `pgs` (some
    // shorthand), and `pgssub` (older ffmpeg + some metadata tools).
    // Misclassifying any of these as text routes the request through
    // the `subtitles=` filter, which only accepts ASS/SRT and either
    // emits "Subtitle codec ... not supported" or silently produces
    // no overlay.
    if matches!(
        c.as_str(),
        "hdmv_pgs_subtitle"
            | "pgs"
            | "pgssub"
            | "dvd_subtitle"
            | "dvdsub"
            | "dvb_subtitle"
            | "dvbsub"
            | "vobsub"
            | "xsub"
    ) || c.contains("pgs")
        || c.contains("vobsub")
    {
        return SubtitleKind::Picture;
    }
    match c.as_str() {
        "subrip" | "srt" | "ass" | "ssa" | "mov_text" | "webvtt" | "text" => SubtitleKind::Text,
        // Unknown — try text first; if ffmpeg complains we'll see it in
        // the captured stderr.
        _ => SubtitleKind::Text,
    }
}

/// Validate and pass-through a client-supplied ASS `force_style`
/// argument. Each entry must be `Key=Value` where both sides are
/// alphanumeric, hex (`&H...&`), or `+`/`-` for numeric prefixes.
/// Returns None on anything fishy so we never splice arbitrary input
/// into ffmpeg's filter graph (which closes the single-quote with
/// `'`, breaks out of the filter with `,` or `;`, etc).
fn sanitize_subtitle_style(input: Option<&str>) -> Option<String> {
    let s = input?.trim();
    if s.is_empty() || s.len() > 512 {
        return None;
    }
    let safe = |c: char| {
        c.is_ascii_alphanumeric() || matches!(c, '=' | ',' | '&' | '.' | '+' | '-')
    };
    if !s.chars().all(safe) {
        return None;
    }
    // Reject if any entry doesn't look like `Key=Value` with both sides
    // non-empty. Cheap shape check, not a full ASS validator.
    for entry in s.split(',') {
        let mut parts = entry.splitn(2, '=');
        let key = parts.next()?.trim();
        let val = parts.next()?.trim();
        if key.is_empty() || val.is_empty() {
            return None;
        }
    }
    Some(s.to_string())
}

/// Extract a subtitle stream from `input` to a WebVTT file at
/// `dest`, using ffmpeg's `webvtt` muxer. No `-ss` — we cache the
/// full subtitle so a single extraction serves every seek
/// position; the per-session copy shifts cue timestamps at write
/// time. Used by both the scanner (preheats cache) and the
/// session-start fallback (cold cache).
async fn extract_full_webvtt_to(
    cfg: &FfmpegConfig,
    input: &Path,
    si: u32,
    dest: &Path,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("create cache dir {}", parent.display()))?;
    }
    let status = Command::new(&cfg.ffmpeg)
        .args(["-y", "-loglevel", "error", "-nostdin"])
        .arg("-i")
        .arg(input)
        .args(["-map", &format!("0:s:{si}")])
        .args(["-c:s", "webvtt"])
        .arg(dest)
        .status()
        .await
        .with_context(|| "spawn ffmpeg for webvtt extraction")?;
    if !status.success() {
        anyhow::bail!("ffmpeg webvtt extraction exited {:?}", status.code());
    }
    if tokio::fs::metadata(dest).await.is_err() {
        anyhow::bail!("webvtt extraction reported success but {} is missing", dest.display());
    }
    Ok(())
}

/// Time-shift every cue in a WebVTT body by `-shift_seconds`,
/// drop cues that end before the new zero, and clamp negative
/// starts to zero. Returns the rewritten body ready to write
/// next to the session's sub/index.m3u8.
///
/// The cache stores subs at their original (full-file) PTS, but
/// the HLS video stream the player is watching has its PTS reset
/// to 0 at the seek point — so without this shift, a cue at
/// 00:15:00 in the cache would only appear when the player's
/// video clock reaches 15 minutes (i.e. 15 minutes after the
/// user clicked play, regardless of where they seeked to).
///
/// We do this in Rust rather than re-running ffmpeg per session
/// because the WebVTT is text (tens of KB) and string-rewriting
/// is microseconds vs. seconds of ffmpeg spawn + extract.
fn shift_webvtt_timestamps(body: &str, shift_seconds: f64) -> String {
    // Block-aware parse. WebVTT separates blocks (header, STYLE,
    // NOTE, REGION, cue) with one or more blank lines. We need to
    // operate at block granularity because dropping a cue that
    // ends before the seek means dropping the WHOLE cue (optional
    // ID line + timing line + text lines) — emitting just the
    // text lines without their timing would corrupt subsequent
    // cue parsing (the orphan text becomes a non-cue block, and
    // worse, the next ID line gets glued to the previous cue's
    // tail). That bug was the cause of "subs work but are out of
    // sync" reports.
    let mut out = String::with_capacity(body.len());
    let normalized = body.replace("\r\n", "\n").replace('\r', "\n");
    let blocks: Vec<&str> = normalized.split("\n\n").collect();
    let mut first = true;
    for block in &blocks {
        if block.is_empty() {
            continue;
        }
        // Try to find a timing line in this block. If there is one,
        // it's a cue block we may need to shift or drop. Otherwise
        // (header, NOTE, STYLE, REGION) pass through verbatim.
        let lines: Vec<&str> = block.lines().collect();
        let timing_idx = lines.iter().position(|l| l.contains("-->"));
        let (write_block, replacement) = match timing_idx {
            Some(idx) => {
                let line = lines[idx];
                let arrow = line.find("-->").unwrap();
                let (left, right) = line.split_at(arrow);
                let after_arrow = &right[3..]; // skip "-->"
                let trimmed = after_arrow.trim_start();
                let leading_ws_len = after_arrow.len() - trimmed.len();
                let leading_ws = &after_arrow[..leading_ws_len];
                let (right_ts, tail) = match trimmed.find(|c: char| c == ' ' || c == '\t') {
                    Some(i) => (&trimmed[..i], &trimmed[i..]),
                    None => (trimmed, ""),
                };
                let s = parse_vtt_timestamp(left.trim());
                let e = parse_vtt_timestamp(right_ts);
                match (s, e) {
                    (Some(s), Some(e)) => {
                        let new_end = e - shift_seconds;
                        if new_end <= 0.0 {
                            // Drop the entire cue block.
                            (false, None)
                        } else {
                            let new_start = (s - shift_seconds).max(0.0);
                            let new_line = format!(
                                "{} --> {}{}{}",
                                format_vtt_timestamp(new_start),
                                format_vtt_timestamp(new_end),
                                leading_ws,
                                tail,
                            );
                            (true, Some((idx, new_line)))
                        }
                    }
                    _ => (true, None), // unparseable, pass through
                }
            }
            None => (true, None), // not a cue block, pass through
        };
        if !write_block {
            continue;
        }
        if !first {
            out.push_str("\n\n");
        }
        first = false;
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            if let Some((replace_idx, ref new_line)) = replacement {
                if i == replace_idx {
                    out.push_str(new_line);
                    continue;
                }
            }
            out.push_str(line);
        }
    }
    // Trailing newline so the file ends cleanly.
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Remove ASS-style override blocks (`{\an5\c&HFFFFFF&\pos(...)}`
/// and friends) from WebVTT cue text. ffmpeg's `webvtt` muxer
/// translates the structural parts of ASS (timing, basic
/// formatting) but leaves complex override codes — karaoke
/// effects, animations, per-character color shifts — as raw
/// `{...}` blocks in the cue body. They show up as a wall of
/// garbled text on screen, which is the most visible "subs are
/// broken" symptom on anime openings/endings.
///
/// We strip every `{...}` block from the cue text. The remaining
/// text might be a single character per cue (karaoke effects
/// often split lyrics letter-by-letter), which is still ugly but
/// at least readable. A cleaner fix would be to filter to the
/// non-effect dialogue layer of the ASS source, but that requires
/// understanding the source's layer/style hierarchy and isn't
/// possible at the WebVTT level.
fn strip_ass_overrides(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut depth = 0u32;
    for ch in body.chars() {
        match ch {
            '{' => {
                depth = depth.saturating_add(1);
            }
            '}' if depth > 0 => {
                depth = depth.saturating_sub(1);
            }
            _ if depth == 0 => {
                out.push(ch);
            }
            _ => {
                // Inside an override block; drop.
            }
        }
    }
    out
}

/// Insert (or replace) the HLS `X-TIMESTAMP-MAP` header right
/// after `WEBVTT`. The mapping `LOCAL:00:00:00.000,MPEGTS:0` tells
/// the player that WebVTT time 0 corresponds to MPEGTS 0 — i.e.
/// the start of the corresponding video segment. Since our cue
/// timestamps have already been shifted to start at zero
/// (relative to the user's seek + offset), this identity mapping
/// keeps everything aligned even on players that compute cue
/// times from MPEGTS rather than walking the WebVTT clock.
fn inject_timestamp_map(body: &str) -> String {
    const HEADER: &str = "X-TIMESTAMP-MAP=LOCAL:00:00:00.000,MPEGTS:0";
    // Find the WEBVTT header line and insert ours immediately
    // after. If the body already has an X-TIMESTAMP-MAP we
    // replace it; the muxer doesn't emit one by default but a
    // future ffmpeg upgrade might.
    let mut lines: Vec<String> = body.lines().map(|s| s.to_string()).collect();
    if let Some(first) = lines.first_mut() {
        if first.starts_with("WEBVTT") {
            // Check if next line is already a TIMESTAMP-MAP
            if lines.len() > 1 && lines[1].starts_with("X-TIMESTAMP-MAP") {
                lines[1] = HEADER.to_string();
            } else {
                lines.insert(1, HEADER.to_string());
            }
        }
    }
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn parse_vtt_timestamp(s: &str) -> Option<f64> {
    // Accepts MM:SS.mmm, HH:MM:SS.mmm. WebVTT uses '.' for ms
    // separator (SRT uses ',', ffmpeg's webvtt muxer emits '.').
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, rest) = match parts.len() {
        3 => (parts[0].parse::<f64>().ok()?, parts[1].parse::<f64>().ok()?, parts[2]),
        2 => (0.0, parts[0].parse::<f64>().ok()?, parts[1]),
        _ => return None,
    };
    let secs: f64 = rest.replace(',', ".").parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + secs)
}

fn format_vtt_timestamp(t: f64) -> String {
    let t = t.max(0.0);
    let total_ms = (t * 1000.0).round() as u64;
    let h = total_ms / 3_600_000;
    let m = (total_ms / 60_000) % 60;
    let s = (total_ms / 1000) % 60;
    let ms = total_ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// Set up the session's WebVTT sidecar by either copying from the
/// cache (instant) or extracting from source on demand (slow on
/// big remuxes; this is what the scanner pre-warms). After the
/// WebVTT is in place we shift its cue timestamps by the seek
/// offset so they align with the video's reset PTS, then write
/// the single-segment HLS subtitle playlist.
///
/// Cache hits make subsequent plays of the same file instant
/// regardless of seek position — the cache stores full-file
/// subs once, and we re-shift per session.
async fn extract_webvtt_sidecar(
    cfg: &FfmpegConfig,
    input: &Path,
    si: u32,
    start_seconds: f64,
    duration_seconds: f64,
    // User-controlled fine-tuning offset in milliseconds. Positive
    // values delay the subtitle relative to the video; negative
    // values advance it. Applied on top of the seek-based shift,
    // so the effective shift becomes `start_seconds - offset_s`.
    // (Smaller shift = cues appear later relative to current
    // playback time; that's "delayed subs".)
    offset_ms: i64,
    session_dir: &Path,
) -> Result<()> {
    // The session dir is e.g. `$CACHE_ROOT/sessions/<session_id>`;
    // walk up two levels to get the cache root. This works because
    // TranscodeManager::start creates the session dir under
    // `<cache_root>/<session_id>` consistently.
    let cache_root = session_dir
        .parent()
        .unwrap_or(session_dir);
    // media_file_id isn't visible here; instead key the cache by
    // the input file path's canonical form (resolved symlinks +
    // absolute) so different sessions of the same source hit the
    // same cache slot. Hashing the path bytes is enough — we don't
    // need cryptographic strength, just collision-resistance for
    // O(thousands of files).
    let cache_key = path_cache_key(input);
    let cache_vtt = cache_root
        .join("subs")
        .join(&cache_key)
        .join(format!("{si}.vtt"));

    let from_cache = tokio::fs::metadata(&cache_vtt).await.is_ok();
    let raw_vtt: String = if from_cache {
        info!(
            cache = %cache_vtt.display(),
            si,
            "webvtt cache hit; skipping extraction"
        );
        tokio::fs::read_to_string(&cache_vtt)
            .await
            .with_context(|| format!("read cached webvtt {}", cache_vtt.display()))?
    } else {
        extract_full_webvtt_to(cfg, input, si, &cache_vtt).await?;
        info!(
            cache = %cache_vtt.display(),
            si,
            "webvtt extracted and cached"
        );
        tokio::fs::read_to_string(&cache_vtt)
            .await
            .with_context(|| format!("read just-cached webvtt {}", cache_vtt.display()))?
    };

    // Shift cues so they line up with the seeked video's 0-based
    // PTS. The user's offset (in ms, defaults to 0) is folded in:
    // positive offset = subs delayed = LESS shift = cues stay at
    // larger times; negative offset = subs earlier = MORE shift.
    let effective_shift = start_seconds - (offset_ms as f64) / 1000.0;
    let shifted = if effective_shift.abs() > 0.001 {
        shift_webvtt_timestamps(&raw_vtt, effective_shift)
    } else {
        raw_vtt
    };

    // Drop ASS override codes the WebVTT muxer left in the cue
    // bodies. Anime karaoke / typesetting cues otherwise render
    // as a wall of "{\an5\c&HFFFFFF&\pos(...)\t(...)}" garbage.
    let shifted = strip_ass_overrides(&shifted);

    // Inject an HLS X-TIMESTAMP-MAP header as belt-and-suspenders
    // alignment. Even with our cue shift, some HLS.js versions /
    // browsers prefer the explicit MPEGTS↔LOCAL mapping for
    // sub-segment timing. We use the identity mapping (LOCAL 0 →
    // MPEGTS 0) because the cues have already been shifted to
    // start at zero relative to the video stream — both signals
    // agree, so the player can use either.
    let shifted = inject_timestamp_map(&shifted);

    let sub_dir = session_dir.join(SUBTITLE_VARIANT_NAME);
    tokio::fs::create_dir_all(&sub_dir)
        .await
        .with_context(|| format!("create subtitle sidecar dir {}", sub_dir.display()))?;
    let vtt_path = sub_dir.join("sub.vtt");
    tokio::fs::write(&vtt_path, shifted)
        .await
        .with_context(|| format!("write session webvtt {}", vtt_path.display()))?;

    let dur = duration_seconds.max(1.0);
    let target_dur = (dur.ceil() as u64).max(1);
    let playlist = format!(
        "#EXTM3U\n\
         #EXT-X-VERSION:7\n\
         #EXT-X-TARGETDURATION:{target_dur}\n\
         #EXT-X-PLAYLIST-TYPE:VOD\n\
         #EXTINF:{dur:.3},\n\
         sub.vtt\n\
         #EXT-X-ENDLIST\n"
    );
    let playlist_path = sub_dir.join("index.m3u8");
    tokio::fs::write(&playlist_path, playlist)
        .await
        .with_context(|| format!("write subtitle playlist {}", playlist_path.display()))?;
    info!(
        path = %vtt_path.display(),
        si,
        duration_s = duration_seconds,
        cache_hit = from_cache,
        "webvtt sidecar ready"
    );
    Ok(())
}

/// Filesystem-safe key for the subtitle cache directory. Strip
/// directory separators and other special characters from the
/// file path so the resulting string nests under
/// `$CACHE_ROOT/subs/<key>/<si>.vtt` without trying to escape the
/// cache root or create weird subdirectories.
fn path_cache_key(input: &Path) -> String {
    let s = input.to_string_lossy();
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    // Filesystem limits on filename length are usually 255 bytes;
    // for very long paths (deep nesting, long titles) the safe
    // upper bound is "fold the tail into a short hash". Crude but
    // collision-safe for normal libraries.
    if out.len() > 200 {
        let head: String = out.chars().take(160).collect();
        let mut h: u64 = 1469598103934665603;
        for b in out.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        format!("{head}_{h:016x}")
    } else {
        out
    }
}

/// Pull a single text-subtitle stream out of the source file into a
/// small standalone file (`.srt` / `.ass` / `.vtt` depending on
/// source codec). The whole point is to bypass the `subtitles=`
/// filter's startup behavior — when pointed at a 30-50 GB Bluray
/// remux, it scans the entire interleaved file looking for subtitle
/// packets before producing the first overlay frame, which on a
/// HDD can take 2-3 minutes. A pre-extracted file lets the filter
/// mmap a few KB and start instantly.
///
/// Using input-side `-ss <start_seconds>` in the extraction
/// command does two useful things:
///
///   * Limits the file scan to the section after the seek point —
///     much faster than starting from byte 0.
///   * Resets the extracted subtitle's timestamps to start at 0
///     (matching what input-side `-ss` does in the main pipeline).
///     This means the main ffmpeg call no longer needs `-copyts` +
///     `setpts=PTS-STARTPTS` / `asetpts=PTS-STARTPTS` — the subs
///     and the video are already on the same 0-based timeline.
///
/// Returns `None` (caller falls back to inline filter on the full
/// file) when extraction fails for any reason: wrong codec map,
/// ffmpeg error, missing output. We never raise — burning subs is
/// a best-effort feature and the inline path still works, just
/// slowly.
async fn extract_text_subtitle(
    cfg: &FfmpegConfig,
    input: &Path,
    si: u32,
    start_seconds: f64,
    codec: Option<&str>,
    output_dir: &Path,
) -> Option<PathBuf> {
    // Pick a container extension that matches the source so we can
    // stream-copy and avoid a transcode step. mov_text doesn't have
    // a standalone file format we can copy into, so we convert it
    // to SRT (cheap text→text transform). Unknown codecs fall back
    // to "convert to ASS" since libass can render anything once
    // it's in ASS form.
    let kind = codec.unwrap_or("").to_ascii_lowercase();
    let (ext, codec_arg) = match kind.as_str() {
        "ass" | "ssa" => ("ass", "copy"),
        "subrip" | "srt" => ("srt", "copy"),
        "webvtt" => ("vtt", "copy"),
        "mov_text" => ("srt", "srt"),
        _ => ("ass", "ass"),
    };
    let temp_path = output_dir.join(format!("sub.{ext}"));
    let result = Command::new(&cfg.ffmpeg)
        .args(["-y", "-loglevel", "error", "-nostdin"])
        .args(["-ss", &format!("{start_seconds:.3}")])
        .arg("-i")
        .arg(input)
        .args(["-map", &format!("0:s:{si}")])
        .args(["-c:s", codec_arg])
        .arg(&temp_path)
        .status()
        .await;
    match result {
        Ok(s) if s.success() && tokio::fs::metadata(&temp_path).await.is_ok() => {
            info!(
                path = %temp_path.display(),
                si,
                codec = ?codec,
                "subtitle pre-extracted for fast filter access"
            );
            Some(temp_path)
        }
        Ok(s) => {
            warn!(
                status = ?s,
                si,
                codec = ?codec,
                "subtitle extraction exited non-zero; will read inline from source"
            );
            None
        }
        Err(e) => {
            warn!(
                error = %e,
                si,
                codec = ?codec,
                "subtitle extraction failed to spawn; will read inline from source"
            );
            None
        }
    }
}

/// Returns `",format=yuv420p"` when the CPU filter chain needs to
/// downconvert to 8-bit YUV before handing frames off, or `""` when
/// some other part of the pipeline already handles the conversion.
///
/// Two failure modes this guards against on 10-bit sources (HEVC
/// Main10 / VP9 Profile 2 / AV1 10-bit — common for anime + 4K HDR
/// remuxes):
///
///   * NVENC h264 silently rejects P010 input. The session shows a
///     spawned ffmpeg child, no segments, no error in stderr — the
///     player spins forever. (NVENC hevc accepts P010 but we
///     transcode to h264 for browser compatibility.)
///   * libx264 + AMF produce broken output on 10-bit input even
///     when they nominally accept it.
///
/// Skipped when:
///
///   * `gpu_native` — the gpu_native pipeline (NVENC + scale_cuda,
///     VAAPI + scale_vaapi) does its own format conversion via the
///     scaler's `:format=nv12` argument.
///   * `hw_suffix.contains("format=")` — VAAPI/QSV's
///     `,format=nv12,hwupload` already converts before the encoder.
///     Doubling up is harmless (ffmpeg dedupes back-to-back format
///     filters) but wastes a chain link in logs.
fn cpu_format_normalization(gpu_native: bool, hw_suffix: &str) -> &'static str {
    if gpu_native || hw_suffix.contains("format=") {
        ""
    } else {
        ",format=yuv420p"
    }
}

/// Escape a path so it survives being interpolated inside ffmpeg's
/// single-quoted filter argument. ffmpeg's filtergraph parser is finicky
/// about `'`, `:`, `\`, `[`, `]`, and `,`.
fn escape_for_filter(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '\\' | '\'' | ':' | '[' | ']' | ',' | ';' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

fn generate_id() -> String {
    let mut buf = [0u8; SESSION_ID_BYTES];
    if let Err(e) = fill_random(&mut buf) {
        // Catastrophic OS RNG failure. Fall back to time-based ID; we
        // log loudly because this should never happen in practice.
        warn!(error = %e, "RNG failure during session id generation; using time-based fallback");
        let now = now_ms();
        for (i, byte) in buf.iter_mut().enumerate() {
            *byte = (now >> ((i % 8) * 8)) as u8;
        }
    }
    hex::encode(buf)
}

fn fill_random(buf: &mut [u8]) -> Result<()> {
    use rand_core::{OsRng, RngCore};
    let mut rng = OsRng;
    rng.fill_bytes(buf);
    Ok(())
}

/// Map the codec name we get from ffprobe / DB columns to the
/// canonical name used in the capability probe's per-hwaccel
/// decoder list. ffprobe writes `hevc`/`h264`/`vp9`/`av1`/`mpeg2video`
/// directly; this helper handles the handful of aliases (`h265` →
/// `hevc`, `mpeg2` → `mpeg2video`) so the cap lookup doesn't miss
/// matches.
fn normalize_codec_for_decoder(codec: &str) -> String {
    match codec.to_ascii_lowercase().as_str() {
        "h265" | "x265" => "hevc".to_string(),
        "x264" => "h264".to_string(),
        "vp09" => "vp9".to_string(),
        "av01" => "av1".to_string(),
        "mpeg2" => "mpeg2video".to_string(),
        other => other.to_string(),
    }
}

/// Cross-platform signal sender used by [`Session::pause`] /
/// [`Session::resume`]. On Unix we invoke `libc::kill` directly with
/// `SIGSTOP` / `SIGCONT`, which suspends ffmpeg in-kernel without
/// any cooperation from the child. On non-Unix targets the
/// operations are no-ops (the docker deployment is Linux-only; the
/// build still has to succeed on dev macOS/Windows hosts).
mod signal {
    #[derive(Copy, Clone)]
    pub enum Kind {
        Pause,
        Continue,
    }
    pub use Kind::Continue;
    pub use Kind::Pause;

    #[cfg(unix)]
    pub fn send(pid: u32, kind: Kind) -> bool {
        let sig = match kind {
            Kind::Pause => libc::SIGSTOP,
            Kind::Continue => libc::SIGCONT,
        };
        // SAFETY: `libc::kill` is safe to call with an arbitrary pid
        // and signal — kernel returns -1 + errno on misuse, never UB.
        // We check the return so a missing pid (already exited) is
        // observable rather than silently swallowed.
        let rc = unsafe { libc::kill(pid as libc::pid_t, sig) };
        rc == 0
    }

    #[cfg(not(unix))]
    pub fn send(_pid: u32, _kind: Kind) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{LoudnessTarget, TonemapConfig, build_loudnorm_filter};

    #[test]
    fn loudnorm_filter_without_measurement_is_single_pass() {
        let s = build_loudnorm_filter(None);
        assert!(s.starts_with("loudnorm=I=-16:LRA=11:TP=-1.5"));
        assert!(!s.contains("measured_I"));
        assert!(!s.contains("linear=true"));
    }

    #[test]
    fn loudnorm_filter_with_measurement_emits_two_pass_params() {
        let t = LoudnessTarget {
            measured_i: -19.5,
            measured_tp: -2.1,
            measured_lra: 9.3,
            measured_thresh: -29.4,
        };
        let s = build_loudnorm_filter(Some(&t));
        assert!(s.contains("measured_I=-19.50"));
        assert!(s.contains("measured_TP=-2.10"));
        assert!(s.contains("measured_LRA=9.30"));
        assert!(s.contains("measured_thresh=-29.40"));
        assert!(s.contains("linear=true"));
    }

    #[test]
    fn tonemap_chain_empty_for_sdr_source() {
        let cfg = TonemapConfig::default();
        assert_eq!(cfg.build_chain(None), "");
        assert_eq!(cfg.build_chain(Some("bt709")), "");
    }

    #[test]
    fn tonemap_chain_empty_when_disabled() {
        let cfg = TonemapConfig {
            enabled: false,
            algorithm: "hable".to_string(),
        };
        assert_eq!(cfg.build_chain(Some("hdr10")), "");
        assert_eq!(cfg.build_chain(Some("dovi")), "");
    }

    #[test]
    fn tonemap_chain_injects_algorithm_for_hdr_source() {
        let cfg = TonemapConfig {
            enabled: true,
            algorithm: "mobius".to_string(),
        };
        let chain = cfg.build_chain(Some("hdr10"));
        assert!(chain.contains("tonemap=tonemap=mobius"));
        assert!(chain.ends_with(','), "trailing comma keeps caller splice clean: {chain:?}");
    }

    #[test]
    fn tonemap_chain_default_matches_legacy_hable() {
        // Regression guard — phase 30 split the hard-coded chain into
        // this builder; the default output must match the previous
        // hard-coded value verbatim so existing HDR sources keep
        // looking identical.
        let cfg = TonemapConfig::default();
        let expected = "zscale=t=linear:npl=100,format=gbrpf32le,tonemap=tonemap=hable:desat=0,zscale=p=bt709:t=bt709:m=bt709:r=tv,format=yuv420p,";
        assert_eq!(cfg.build_chain(Some("hdr10")), expected);
    }
}
