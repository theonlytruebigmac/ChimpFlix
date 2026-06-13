//! Detect ffmpeg's installed hwaccels + hardware encoders at startup so
//! the admin UI can offer only the options that will actually work.
//!
//! We invoke `ffmpeg -hide_banner -hwaccels` for the accel list and
//! `ffmpeg -hide_banner -encoders` for the encoder set, scanning for the
//! six h264/hevc hardware encoders shipped by upstream ffmpeg today.
//!
//! That on its own is not sufficient — the ffmpeg binary can advertise
//! `h264_nvenc` because it was compiled with NVENC support even when the
//! host's libcuda.so isn't reachable (the common docker-without-GPU
//! case). To avoid letting "Auto" pick an encoder that fails at session
//! start, we follow the listing scan with a one-frame smoke encode for
//! each candidate. Anything that fails the smoke test is dropped from
//! the reported list so the admin UI greys it out and `HwAccel::auto_pick`
//! never selects it.
//!
//! Failures here are non-fatal — we just return an empty capability set
//! and the UI greys out the relevant options.

use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Monotonically-increasing counter used to give each decoder smoke-test
/// temp file a unique name within the process. Eliminates path collisions
/// when two concurrent `detect_capabilities` calls (boot + admin reprobe)
/// probe the same (hwaccel, codec) pair simultaneously.
static PROBE_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

use tracing::debug;

use crate::FfmpegConfig;

/// Refreshable holder for the detected hardware capabilities.
///
/// Capabilities are probed once at boot, but a driver swap or GPU
/// hot-add can change what the host can do without the process
/// restarting. This wrapper lets the admin "re-probe" endpoint swap in
/// a fresh [`TranscoderCapabilities`] atomically while live readers
/// (the stream session path, the admin GET) keep seeing a consistent
/// snapshot.
///
/// Readers call [`SharedCapabilities::load`], which clones the inner
/// `Arc` under a very short read lock and hands it back — identical in
/// shape to the previous `Arc<TranscoderCapabilities>` the code held
/// directly, so the hot path keeps a lock-free snapshot for the rest of
/// its work. Writers call [`SharedCapabilities::store`] to publish a new
/// snapshot; in-flight readers that already loaded the old `Arc` keep
/// using it until they drop it, exactly like `ArcSwap` semantics but
/// with no extra dependency.
#[derive(Debug)]
pub struct SharedCapabilities {
    inner: RwLock<Arc<TranscoderCapabilities>>,
}

impl SharedCapabilities {
    /// Wrap an initial (boot-time) capability snapshot.
    pub fn new(caps: TranscoderCapabilities) -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(Arc::new(caps)),
        })
    }

    /// Current snapshot. Clones the inner `Arc` under a short read lock
    /// and releases the lock before returning, so callers never hold
    /// the lock across `.await`.
    pub fn load(&self) -> Arc<TranscoderCapabilities> {
        // A poisoned lock here just means a writer panicked mid-swap;
        // the held data is still a valid snapshot, so recover it rather
        // than propagate the panic into the playback path.
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Publish a fresh snapshot (used by the re-probe endpoint).
    pub fn store(&self, caps: TranscoderCapabilities) {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = Arc::new(caps);
    }
}

/// Per-encoder time budget for the startup smoke test. Working
/// encoders complete in under 300 ms on every box we've tested;
/// 3 s is generous enough to ride out a slow first-encode initial-
/// alloc without making startup feel sluggish even if every
/// detected encoder times out (worst case ~18 s for the 6-encoder
/// candidate set).
const SMOKE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Default, Serialize)]
pub struct TranscoderCapabilities {
    pub ffmpeg_version: Option<String>,
    pub hwaccels: Vec<String>,
    pub h264_encoders: Vec<String>,
    pub hevc_encoders: Vec<String>,
    /// Per-hwaccel list of source codecs the GPU can decode in
    /// hardware. Keyed by hwaccel name (`cuda`, `vaapi`, `qsv`,
    /// `videotoolbox`), value is the canonical lowercase codec
    /// names probed (`h264`, `hevc`, `vp9`, `av1`, `mpeg2video`).
    /// Populated by [`detect_capabilities`] via the same smoke-test
    /// pattern as the encoder list — gives the operator an honest
    /// answer to "does my GPU actually decode this codec?" instead
    /// of inferring from card model and trusting it.
    pub decoders: HwDecoderCapabilities,
    /// Enumerated GPU devices for the dropdown in admin → Transcoder.
    /// NVIDIA enumerated via `nvidia-smi` when present; VAAPI via
    /// glob over `/dev/dri/renderD*`. Empty when neither yields any
    /// devices (single-GPU box or no GPU acceleration available).
    pub gpu_devices: Vec<GpuDevice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuDevice {
    /// Operator-facing label, e.g. "NVIDIA GeForce RTX 5070 Ti".
    pub name: String,
    /// Value to store in `server_settings.transcoder_gpu_device`.
    /// For NVENC: the index as a string ("0", "1"). For VAAPI: the
    /// full device path ("/dev/dri/renderD128").
    pub value: String,
    /// Which hwaccel this device serves. Helps the UI group/filter
    /// options when both NVENC + VAAPI cards are present on the
    /// same box (rare but real).
    pub backend: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HwDecoderCapabilities {
    pub cuda: Vec<String>,
    pub vaapi: Vec<String>,
    pub qsv: Vec<String>,
    pub videotoolbox: Vec<String>,
}

impl HwDecoderCapabilities {
    /// Does the named hwaccel support decoding the given source
    /// codec? Caller passes a lowercase codec name (the same form
    /// ffprobe stores in `media_streams.codec`).
    pub fn supports(&self, hwaccel: &str, normalized_codec: &str) -> bool {
        let list = match hwaccel {
            "cuda" => &self.cuda,
            "vaapi" => &self.vaapi,
            "qsv" => &self.qsv,
            "videotoolbox" => &self.videotoolbox,
            _ => return false,
        };
        list.iter().any(|c| c == normalized_codec)
    }
}

pub async fn detect_capabilities(cfg: &FfmpegConfig) -> TranscoderCapabilities {
    let mut caps = TranscoderCapabilities {
        ffmpeg_version: ffmpeg_version(cfg).await,
        hwaccels: ffmpeg_hwaccels(cfg).await,
        ..Default::default()
    };
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
    let h264_listed: Vec<String> = h264_candidates
        .iter()
        .filter(|name| encoders.contains(**name))
        .map(|s| (*s).to_string())
        .collect();
    let hevc_listed: Vec<String> = hevc_candidates
        .iter()
        .filter(|name| encoders.contains(**name))
        .map(|s| (*s).to_string())
        .collect();

    caps.h264_encoders = filter_to_working(cfg, h264_listed).await;
    caps.hevc_encoders = filter_to_working(cfg, hevc_listed).await;

    // Decoder smoke tests: per-hwaccel, per-source-codec. Each test
    // tries to decode a 1-frame synthetic input using the chosen
    // hwaccel; if ffmpeg exits zero, the GPU can actually decode
    // that codec. Catches the "card model has the silicon block but
    // the driver/container is missing the firmware" case, and
    // distinguishes pre-Ampere (no AV1 NVDEC) from Ampere+ (yes
    // AV1) without us having to maintain a card-model database.
    if caps.hwaccels.iter().any(|h| h == "cuda") {
        caps.decoders.cuda = probe_decoders_for(cfg, "cuda").await;
    }
    if caps.hwaccels.iter().any(|h| h == "vaapi") {
        caps.decoders.vaapi = probe_decoders_for(cfg, "vaapi").await;
    }
    if caps.hwaccels.iter().any(|h| h == "qsv") {
        caps.decoders.qsv = probe_decoders_for(cfg, "qsv").await;
    }
    if caps.hwaccels.iter().any(|h| h == "videotoolbox") {
        caps.decoders.videotoolbox = probe_decoders_for(cfg, "videotoolbox").await;
    }

    caps.gpu_devices = enumerate_gpu_devices().await;

    caps
}

/// Best-effort GPU device enumeration. Failures are silent — an empty
/// `gpu_devices` list just means the admin dropdown shows only the
/// "Auto" option, matching the previous behavior.
async fn enumerate_gpu_devices() -> Vec<GpuDevice> {
    let mut out: Vec<GpuDevice> = Vec::new();
    out.extend(enumerate_nvidia().await);
    out.extend(enumerate_vaapi().await);
    out
}

/// Run `nvidia-smi --query-gpu=index,name --format=csv,noheader,nounits`
/// and parse the output. Returns empty when the binary isn't on PATH
/// or when it exits non-zero (no NVIDIA driver, no permissions, etc).
async fn enumerate_nvidia() -> Vec<GpuDevice> {
    let output = tokio::process::Command::new("nvidia-smi")
        .args(["--query-gpu=index,name", "--format=csv,noheader,nounits"])
        .output()
        .await;
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = match String::from_utf8(output.stdout) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    text.lines()
        .filter_map(|line| {
            // Format: "0, NVIDIA GeForce RTX 5070 Ti"
            let mut parts = line.splitn(2, ',');
            let idx = parts.next()?.trim();
            let name = parts.next()?.trim();
            if idx.is_empty() || name.is_empty() {
                return None;
            }
            // Reject malformed index (non-digit) so we don't pass
            // junk to ffmpeg's `-gpu` flag later.
            if !idx.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            Some(GpuDevice {
                name: format!("NVIDIA: {name} (#{idx})"),
                value: idx.to_string(),
                backend: "nvenc".to_string(),
            })
        })
        .collect()
}

/// Glob `/dev/dri/renderD*` for VAAPI device nodes. Each render node
/// represents one card; D128 is canonical "first GPU" on Linux,
/// D129+ second card.
async fn enumerate_vaapi() -> Vec<GpuDevice> {
    let mut out: Vec<GpuDevice> = Vec::new();
    let mut entries = match tokio::fs::read_dir("/dev/dri").await {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    while let Ok(Some(ent)) = entries.next_entry().await {
        let name = ent.file_name();
        let name = match name.to_str() {
            Some(s) => s,
            None => continue,
        };
        if !name.starts_with("renderD") {
            continue;
        }
        let path = format!("/dev/dri/{name}");
        out.push(GpuDevice {
            name: format!("VAAPI: {name}"),
            value: path.clone(),
            backend: "vaapi".to_string(),
        });
    }
    // Stable order so the UI dropdown doesn't reshuffle between
    // restarts of the same hardware.
    out.sort_by(|a, b| a.value.cmp(&b.value));
    out
}

/// Codec set we care about for HW decode. Order is most-common
/// first so the typical "h264 source" probe completes before we
/// move on to the less-common codecs.
const DECODER_CODECS: &[&str] = &["h264", "hevc", "vp9", "av1", "mpeg2video"];

/// For each codec we care about, generate a 1-frame synthetic source
/// in that codec via the corresponding libavcodec encoder, then try
/// to decode it through the requested hwaccel. The handshake works
/// because ffmpeg can pipe its own output back to its input — no
/// external sample files needed.
async fn probe_decoders_for(cfg: &FfmpegConfig, hwaccel: &str) -> Vec<String> {
    let mut working = Vec::new();
    for codec in DECODER_CODECS {
        if smoke_test_decoder(cfg, hwaccel, codec).await {
            debug!(hwaccel = %hwaccel, codec = %codec, "decoder smoke test ok");
            working.push((*codec).to_string());
        }
    }
    working
}

/// Run `ffmpeg -hwaccel X -f lavfi -i color=...,Y -c:v <encoder>
/// -frames 1 -f rawvideo -` plus a second pipeline that takes the
/// encoded bytes and decodes them back. If both halves succeed, the
/// GPU can decode that codec through that hwaccel.
///
/// Implementation note: instead of two ffmpeg invocations + a pipe
/// (which is fiddly to set up correctly across platforms), we use
/// ffmpeg's `-init_hw_device` + force-decoder-name path: render a
/// short clip with the corresponding encoder to a temp file in
/// memory, then re-read it with the hwaccel decoder. For TS-friendly
/// codecs (h264/hevc/mpeg2) we mux to mpegts; for everything else
/// we use mp4 / matroska as appropriate.
///
/// The probe deliberately avoids producing real output: `-f null` and
/// `-frames:v 1` keep the test minimal. ~50ms per probe on a healthy
/// box; up to SMOKE_TIMEOUT on a hung driver.
async fn smoke_test_decoder(cfg: &FfmpegConfig, hwaccel: &str, codec: &str) -> bool {
    // Per-codec encoder + container shape for the test stream.
    // We render the test through software encoders only — the goal
    // is to produce a syntactically-valid bitstream we can hand to
    // the HW decoder; the test isn't measuring encoder capability.
    let (encoder, container) = match codec {
        "h264" => ("libx264", "mpegts"),
        "hevc" => ("libx265", "mpegts"),
        "vp9" => ("libvpx-vp9", "webm"),
        "av1" => ("libaom-av1", "matroska"),
        "mpeg2video" => ("mpeg2video", "mpegts"),
        _ => return false,
    };

    // Make a tiny test bitstream: 1-frame 64x64 black clip, ~kilobytes.
    // Use a per-call monotonic counter (not PID) so concurrent calls from
    // the boot probe and an admin reprobe don't share the same path.
    let tmp = std::env::temp_dir().join(format!(
        "chimpflix_dec_probe_{hwaccel}_{codec}_{}.dat",
        PROBE_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = tokio::fs::remove_file(&tmp).await;

    let mut enc_cmd = tokio::process::Command::new(&cfg.ffmpeg);
    enc_cmd
        .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
        .args(["-f", "lavfi", "-i", "color=c=black:s=64x64:d=0.1:r=1"])
        .args(["-vf", "format=yuv420p"])
        .args(["-c:v", encoder, "-frames:v", "1", "-f", container])
        .arg(&tmp);
    // libaom-av1's first encode is notoriously slow (5–30 s for initial
    // model load + allocation), so give it a much longer budget than
    // the standard smoke timeout to avoid falsely marking AV1 HW decode
    // as unavailable on GPUs that support it (e.g. NVDEC Ampere+).
    let enc_timeout = match codec {
        "av1" => Duration::from_secs(60),
        _ => SMOKE_TIMEOUT,
    };
    let enc_ok = matches!(
        tokio::time::timeout(enc_timeout, enc_cmd.output()).await,
        Ok(Ok(out)) if out.status.success()
    );
    if !enc_ok {
        // No way to make a sample → skip the codec. Operators
        // without libx265/libaom-av1 in their ffmpeg build will see
        // this for hevc/av1; that just means we can't probe those
        // codecs, not that NVDEC won't handle them. Falls back to
        // optimistic behavior (let pre_input_args allow the hint).
        let _ = tokio::fs::remove_file(&tmp).await;
        return false;
    }

    // Try to decode it via the hwaccel.
    let mut dec_cmd = tokio::process::Command::new(&cfg.ffmpeg);
    dec_cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    if hwaccel == "vaapi" {
        dec_cmd.args(["-vaapi_device", "/dev/dri/renderD128"]);
    }
    dec_cmd
        .args(["-hwaccel", hwaccel])
        .arg("-i")
        .arg(&tmp)
        .args(["-frames:v", "1", "-f", "null", "-"]);
    let dec_ok = matches!(
        tokio::time::timeout(SMOKE_TIMEOUT, dec_cmd.output()).await,
        Ok(Ok(out)) if out.status.success()
    );
    let _ = tokio::fs::remove_file(&tmp).await;
    dec_ok
}

/// Run a one-frame smoke encode for every candidate and keep only
/// the ones that exit zero. Each failure logs at `debug!` with the
/// encoder name (it's expected pruning of encoders ffmpeg advertises
/// but the host can't actually start — libcuda.so missing / VAAPI
/// render node missing / Intel iHD driver missing); the surviving set
/// is reported at `info!` by the caller.
async fn filter_to_working(cfg: &FfmpegConfig, candidates: Vec<String>) -> Vec<String> {
    let mut working = Vec::with_capacity(candidates.len());
    for enc in candidates {
        if smoke_test_encoder(cfg, &enc).await {
            debug!(encoder = %enc, "encoder smoke test ok");
            working.push(enc);
        } else {
            // Expected on most hosts: ffmpeg advertises encoders for hardware
            // that isn't present (e.g. VAAPI/QSV/V4L2 on an NVIDIA-only box,
            // where the VAAPI smoke test can't open /dev/dri/renderD128). This
            // is benign pruning, not a problem — logged at debug so it doesn't
            // spam WARN once per boot. The surviving set is reported at info
            // by the caller, so a genuinely-missing expected encoder (e.g. no
            // NVENC) is still visible as an empty/short capability list.
            debug!(
                encoder = %enc,
                "encoder advertised by ffmpeg but failed smoke test \
                 (missing driver / device); dropping from capability list"
            );
        }
    }
    working
}

/// Try to actually run the encoder on a 1-frame synthetic input. If
/// ffmpeg exits zero the encoder is genuinely usable; any non-zero
/// exit or timeout means we shouldn't surface it as a choice.
///
/// The lavfi `color` filter generates a single 320×240 black frame
/// — about as cheap as ffmpeg input gets, while still exercising
/// the full encoder init path that fails when libcuda / iHD /
/// renderD128 is missing. Output is muxed into the `null` muxer
/// (writes nothing to stdout/disk) so the test leaves no artifacts.
async fn smoke_test_encoder(cfg: &FfmpegConfig, encoder: &str) -> bool {
    let mut cmd = tokio::process::Command::new(&cfg.ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    // VAAPI needs a device handle declared before the input. Other
    // encoders accept software frames directly. We hardcode the
    // canonical Linux DRI path; operators with a non-standard render
    // node will still see the encoder fail the smoke test, which is
    // the correct signal to take "Auto" off VAAPI for them.
    if encoder.contains("_vaapi") {
        cmd.args(["-vaapi_device", "/dev/dri/renderD128"]);
    }
    cmd.args(["-f", "lavfi", "-i", "color=c=black:s=320x240:d=0.1:r=1"]);
    // VAAPI insists on NV12 frames uploaded to the GPU. Other
    // encoders are happy with plain yuv420p.
    let vf = if encoder.contains("_vaapi") {
        "format=nv12,hwupload"
    } else {
        "format=yuv420p"
    };
    cmd.args(["-vf", vf]);
    cmd.args(["-c:v", encoder, "-frames:v", "1", "-f", "null", "-"]);

    match tokio::time::timeout(SMOKE_TIMEOUT, cmd.output()).await {
        Ok(Ok(out)) => out.status.success(),
        Ok(Err(_)) => false, // spawn failed (ffmpeg binary missing, etc.)
        Err(_) => false,     // timed out — encoder is hung in init
    }
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
        line.split_whitespace().nth(2).unwrap_or(line).to_string()
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
