//! Hardware-encoder selection and ffmpeg argument plumbing.
//!
//! The operator picks one of {auto, none, nvenc, qsv, vaapi,
//! videotoolbox, amf} in admin → Server → Transcoder. `HwAccel::resolve`
//! turns that string into a concrete enum, falling back to `None`
//! (software libx264) whenever the chosen encoder isn't present in
//! the detected capabilities.
//!
//! Each variant brings its own argument syntax — NVENC's rate-control
//! presets are different from QSV's, VAAPI needs an explicit device,
//! VideoToolbox doesn't accept `-bufsize`, AMF uses `-quality` instead
//! of `-preset`. The mapping lives here so `spawn_ffmpeg` stays
//! readable.

use tokio::process::Command;

use crate::TranscoderCapabilities;

/// Operator-controlled speed-vs-quality dial. Each encoder maps the
/// three positions to its own preset vocabulary. Default `Balanced`
/// reproduces the exact arguments used before this enum existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderPreset {
    Speed,
    Balanced,
    Quality,
}

impl EncoderPreset {
    /// Case-insensitive parse from the server-settings string. Unknown
    /// values fall back to `Balanced` so a typo doesn't break sessions.
    pub fn resolve(setting: &str) -> Self {
        match setting.trim().to_ascii_lowercase().as_str() {
            "speed" | "fast" | "low" => Self::Speed,
            "quality" | "slow" | "high" => Self::Quality,
            _ => Self::Balanced,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Speed => "speed",
            Self::Balanced => "balanced",
            Self::Quality => "quality",
        }
    }
}

/// Concrete encoder choice for a single transcode session. Resolved
/// from the server-settings string + detected capabilities at session
/// start time, so capability changes (e.g. operator installs CUDA
/// drivers) take effect on the next session without restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwAccel {
    /// Software libx264. Always available; the fallback when the
    /// requested encoder isn't present.
    None,
    Nvenc,
    Qsv,
    Vaapi,
    Videotoolbox,
    Amf,
}

impl HwAccel {
    /// Translate the operator's setting + ffmpeg's detected
    /// capabilities into a concrete encoder choice. Any unknown or
    /// unavailable selection silently degrades to software so a typo
    /// or a driver removal can't kill playback.
    pub fn resolve(setting: &str, caps: &TranscoderCapabilities) -> Self {
        let normalized = setting.trim().to_ascii_lowercase();
        let want = match normalized.as_str() {
            "" | "none" | "off" | "software" | "cpu" | "libx264" => {
                return Self::None;
            }
            "auto" => return Self::auto_pick(caps),
            "nvenc" | "cuda" | "nvidia" => Self::Nvenc,
            "vaapi" => Self::Vaapi,
            "qsv" | "quicksync" | "intel" => Self::Qsv,
            "videotoolbox" | "vt" | "apple" => Self::Videotoolbox,
            "amf" | "amd" => Self::Amf,
            _ => return Self::None,
        };
        if want.is_available(caps) {
            want
        } else {
            Self::None
        }
    }

    /// Auto-pick prefers encoders by typical quality/speed/availability
    /// tradeoff: NVENC > QSV > VideoToolbox > VAAPI > AMF > software.
    /// Order is opinionated but reversible by setting a specific name.
    fn auto_pick(caps: &TranscoderCapabilities) -> Self {
        for candidate in [
            Self::Nvenc,
            Self::Qsv,
            Self::Videotoolbox,
            Self::Vaapi,
            Self::Amf,
        ] {
            if candidate.is_available(caps) {
                return candidate;
            }
        }
        Self::None
    }

    fn is_available(self, caps: &TranscoderCapabilities) -> bool {
        let encoder = match self {
            Self::None => return true,
            Self::Nvenc => "h264_nvenc",
            Self::Qsv => "h264_qsv",
            Self::Vaapi => "h264_vaapi",
            Self::Videotoolbox => "h264_videotoolbox",
            Self::Amf => "h264_amf",
        };
        caps.h264_encoders.iter().any(|e| e == encoder)
    }

    /// Short label for logs / dashboard rendering.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "software (libx264)",
            Self::Nvenc => "NVIDIA NVENC",
            Self::Qsv => "Intel QuickSync",
            Self::Vaapi => "VAAPI",
            Self::Videotoolbox => "Apple VideoToolbox",
            Self::Amf => "AMD AMF",
        }
    }

    /// Args inserted BEFORE `-i input` to initialize a hardware
    /// device + plumb the source through a hardware decoder where
    /// possible. The decode-side hwaccel is just as load-bearing as
    /// the encoder for sources in the heavy codecs (AV1, HEVC,
    /// 10-bit anything) — without it ffmpeg software-decodes on the
    /// CPU and the encoder ends up starved, encoding well below
    /// realtime regardless of how fast the GPU encoder is. Symptom
    /// for the user: ffmpeg is alive and using CPU, but HLS
    /// segments land 1 every few seconds, the player buffers forever.
    ///
    /// We pass `-hwaccel` (not `-hwaccel_output_format`) so the
    /// decoder downloads frames back to CPU memory before the filter
    /// graph. Keeping frames on GPU end-to-end would be faster but
    /// the subtitle-burn `subtitles=` filter runs only on software
    /// frames — keeping decode on the GPU and download back is the
    /// pragmatic compromise.
    /// Name of the hwaccel string we'd pass to ffmpeg's `-hwaccel`
    /// for this encoder's natural decode pipeline. `None` for
    /// software (libx264 has no hwaccel) and AMF (AMD decode on
    /// Linux is VAAPI, not AMF — operators with AMD cards should
    /// pick VAAPI explicitly for decode acceleration).
    pub fn paired_decoder(self) -> Option<&'static str> {
        match self {
            Self::Vaapi => Some("vaapi"),
            Self::Nvenc => Some("cuda"),
            Self::Qsv => Some("qsv"),
            Self::Videotoolbox => Some("videotoolbox"),
            Self::Amf | Self::None => None,
        }
    }

    /// Args inserted BEFORE `-i input`. `use_hwaccel_decode` is
    /// the runtime decision made by the caller — typically:
    ///
    ///   "this encoder has a paired decoder AND the per-GPU
    ///   capability probe at startup confirmed it can decode
    ///   this specific source codec".
    ///
    /// Splitting the decision out of this function lets us probe
    /// per-card capability separately (so an RTX 5070 Ti gets NVDEC
    /// AV1 while a GTX 1050 doesn't) without baking a card-model
    /// database in here.
    pub fn pre_input_args(self, cmd: &mut Command, use_hwaccel_decode: bool) {
        // VAAPI always needs the device declaration even when we're
        // not asking for VAAPI decode — the encoder side uses it too.
        if matches!(self, Self::Vaapi) {
            cmd.args(["-vaapi_device", "/dev/dri/renderD128"]);
        }
        if !use_hwaccel_decode {
            return;
        }
        if let Some(name) = self.paired_decoder() {
            cmd.args(["-hwaccel", name]);
        }
    }

    /// Filter-graph suffix appended after the user-facing video
    /// filter chain. VAAPI requires frames to be uploaded to the
    /// GPU in NV12 format before the encoder can touch them. Other
    /// encoders accept software frames directly.
    pub fn vf_suffix(self) -> &'static str {
        match self {
            Self::Vaapi => ",format=nv12,hwupload",
            _ => "",
        }
    }

    /// True when this encoder can run a "frames stay on the GPU"
    /// pipeline — decode via hwaccel → scale on GPU → encode →
    /// without ever rounding trip to system memory. Saves the
    /// PCIe traffic + 2× memcpy per frame, which is meaningful on
    /// high-bitrate / high-resolution sources. Gated by:
    ///
    ///   * Encoder supports a GPU-side scaler (`scale_cuda`,
    ///     `scale_vaapi`, etc.). True for NVENC + VAAPI; QSV has
    ///     `scale_qsv` but the ffmpeg builds in our docker base
    ///     don't always include it; VideoToolbox + AMF don't have
    ///     equivalents on Linux.
    ///   * No filter that requires CPU-side frames. Subtitle burn
    ///     and HDR tonemap both currently need CPU frames (we
    ///     could rebuild them on GPU, but the refactor is large
    ///     and the win is small). Callers gate themselves.
    pub fn supports_gpu_native_pipeline(self) -> bool {
        matches!(self, Self::Nvenc | Self::Vaapi)
    }

    /// Args inserted between `-hwaccel` and `-i` to keep decoded
    /// frames on the GPU. Without these, ffmpeg downloads decoded
    /// frames to system memory before the filter graph runs.
    pub fn gpu_output_format_args(self, cmd: &mut Command) {
        match self {
            Self::Nvenc => {
                cmd.args(["-hwaccel_output_format", "cuda"]);
            }
            Self::Vaapi => {
                cmd.args(["-hwaccel_output_format", "vaapi"]);
            }
            _ => {}
        }
    }

    /// Name of the GPU-side scaler filter for this encoder. Used
    /// when [`supports_gpu_native_pipeline`] is true. Returns an
    /// empty string for encoders that don't have one.
    pub fn gpu_scale_filter(self) -> &'static str {
        match self {
            Self::Nvenc => "scale_cuda",
            Self::Vaapi => "scale_vaapi",
            _ => "",
        }
    }

    /// Push the encoder, rate-control, and profile args onto the
    /// command. Each branch is calibrated for HLS streaming — we
    /// want predictable segment sizes (rate-control caps) and low
    /// latency between input and segment-on-disk. Quality presets
    /// lean on the fast side because real-time encoding is the goal,
    /// not archival.
    pub fn apply_encoder(
        self,
        cmd: &mut Command,
        bitrate_bps: u64,
        preset: EncoderPreset,
    ) {
        let target = bitrate_bps.to_string();
        let maxrate = (bitrate_bps + bitrate_bps / 16).to_string();
        let bufsize = (bitrate_bps * 2).to_string();

        match self {
            Self::None => {
                // libx264 preset names are the documented ones — moving
                // from veryfast to medium roughly halves throughput but
                // shaves visible blocking on hard-to-encode frames.
                let p = match preset {
                    EncoderPreset::Speed => "ultrafast",
                    EncoderPreset::Balanced => "veryfast",
                    EncoderPreset::Quality => "medium",
                };
                cmd.args(["-c:v", "libx264"])
                    .args(["-preset", p])
                    .args(["-b:v", &target])
                    .args(["-maxrate", &maxrate])
                    .args(["-bufsize", &bufsize])
                    .args(["-pix_fmt", "yuv420p"])
                    .args(["-profile:v", "main"])
                    .args(["-level", "4.0"]);
            }
            Self::Nvenc => {
                // NVENC presets are p1 (fastest) through p7 (slowest).
                // p4 is the documented "medium" balance; p1 and p6
                // bracket the extremes the operator can pick. ll tune
                // favors low latency (right for live HLS), vbr +
                // maxrate/bufsize keeps segments within HLS bandwidth
                // hints regardless of preset.
                let p = match preset {
                    EncoderPreset::Speed => "p1",
                    EncoderPreset::Balanced => "p4",
                    EncoderPreset::Quality => "p6",
                };
                cmd.args(["-c:v", "h264_nvenc"])
                    .args(["-preset", p])
                    .args(["-tune", "ll"])
                    .args(["-rc", "vbr"])
                    .args(["-b:v", &target])
                    .args(["-maxrate", &maxrate])
                    .args(["-bufsize", &bufsize])
                    .args(["-profile:v", "main"]);
            }
            Self::Qsv => {
                let p = match preset {
                    EncoderPreset::Speed => "ultrafast",
                    EncoderPreset::Balanced => "veryfast",
                    EncoderPreset::Quality => "medium",
                };
                cmd.args(["-c:v", "h264_qsv"])
                    .args(["-preset", p])
                    .args(["-b:v", &target])
                    .args(["-maxrate", &maxrate])
                    .args(["-bufsize", &bufsize])
                    .args(["-profile:v", "main"]);
            }
            Self::Vaapi => {
                // VAAPI doesn't accept -preset; quality is governed by
                // -global_quality (QP, lower = higher quality). 23 is
                // the canonical "visually transparent at typical
                // bitrates" point; 28 trades quality for ~25% more
                // throughput, 18 spends more cycles for source-quality
                // output. Bitrate cap still applies so HLS bandwidth
                // hints stay honest.
                let q = match preset {
                    EncoderPreset::Speed => "28",
                    EncoderPreset::Balanced => "23",
                    EncoderPreset::Quality => "18",
                };
                cmd.args(["-c:v", "h264_vaapi"])
                    .args(["-global_quality", q])
                    .args(["-b:v", &target])
                    .args(["-maxrate", &maxrate])
                    .args(["-bufsize", &bufsize])
                    .args(["-profile:v", "main"]);
            }
            Self::Videotoolbox => {
                // VideoToolbox ignores -bufsize and treats -maxrate
                // loosely; the encoder picks its own buffer model on
                // Apple Silicon. Quality preset maps to -realtime:
                // realtime 1 = speed (skip B-frames, single pass),
                // realtime 0 = quality (default, slower but better).
                let realtime = match preset {
                    EncoderPreset::Speed => "1",
                    EncoderPreset::Balanced | EncoderPreset::Quality => "0",
                };
                cmd.args(["-c:v", "h264_videotoolbox"])
                    .args(["-realtime", realtime])
                    .args(["-b:v", &target])
                    .args(["-maxrate", &maxrate])
                    .args(["-profile:v", "main"]);
            }
            Self::Amf => {
                // AMD AMF uses -quality {speed|balanced|quality} —
                // direct 1:1 mapping with our enum.
                let q = match preset {
                    EncoderPreset::Speed => "speed",
                    EncoderPreset::Balanced => "balanced",
                    EncoderPreset::Quality => "quality",
                };
                cmd.args(["-c:v", "h264_amf"])
                    .args(["-quality", q])
                    .args(["-b:v", &target])
                    .args(["-maxrate", &maxrate])
                    .args(["-bufsize", &bufsize])
                    .args(["-profile:v", "main"]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TranscoderCapabilities;

    fn caps_with(h264: &[&str]) -> TranscoderCapabilities {
        TranscoderCapabilities {
            ffmpeg_version: None,
            hwaccels: vec![],
            h264_encoders: h264.iter().map(|s| s.to_string()).collect(),
            hevc_encoders: vec![],
            decoders: Default::default(),
        }
    }

    #[test]
    fn explicit_setting_picks_the_named_encoder_when_available() {
        let caps = caps_with(&["h264_nvenc", "h264_qsv"]);
        assert_eq!(HwAccel::resolve("nvenc", &caps), HwAccel::Nvenc);
        assert_eq!(HwAccel::resolve("qsv", &caps), HwAccel::Qsv);
    }

    #[test]
    fn explicit_setting_falls_back_to_software_when_unavailable() {
        let caps = caps_with(&[]);
        assert_eq!(HwAccel::resolve("nvenc", &caps), HwAccel::None);
        assert_eq!(HwAccel::resolve("videotoolbox", &caps), HwAccel::None);
    }

    #[test]
    fn auto_prefers_nvenc_over_others() {
        let caps = caps_with(&["h264_vaapi", "h264_qsv", "h264_nvenc"]);
        assert_eq!(HwAccel::resolve("auto", &caps), HwAccel::Nvenc);
    }

    #[test]
    fn auto_falls_through_priority_order() {
        assert_eq!(
            HwAccel::resolve("auto", &caps_with(&["h264_amf"])),
            HwAccel::Amf,
        );
        assert_eq!(
            HwAccel::resolve("auto", &caps_with(&["h264_videotoolbox"])),
            HwAccel::Videotoolbox,
        );
        assert_eq!(HwAccel::resolve("auto", &caps_with(&[])), HwAccel::None);
    }

    #[test]
    fn empty_or_none_returns_software() {
        let caps = caps_with(&["h264_nvenc"]);
        assert_eq!(HwAccel::resolve("", &caps), HwAccel::None);
        assert_eq!(HwAccel::resolve("none", &caps), HwAccel::None);
        assert_eq!(HwAccel::resolve("off", &caps), HwAccel::None);
        assert_eq!(HwAccel::resolve("software", &caps), HwAccel::None);
    }

    #[test]
    fn unknown_setting_returns_software() {
        let caps = caps_with(&["h264_nvenc"]);
        assert_eq!(HwAccel::resolve("magic", &caps), HwAccel::None);
    }

    #[test]
    fn encoder_preset_resolves_canonical_names() {
        assert_eq!(EncoderPreset::resolve("speed"), EncoderPreset::Speed);
        assert_eq!(EncoderPreset::resolve("balanced"), EncoderPreset::Balanced);
        assert_eq!(EncoderPreset::resolve("quality"), EncoderPreset::Quality);
    }

    #[test]
    fn encoder_preset_accepts_synonyms() {
        assert_eq!(EncoderPreset::resolve("fast"), EncoderPreset::Speed);
        assert_eq!(EncoderPreset::resolve("slow"), EncoderPreset::Quality);
        assert_eq!(EncoderPreset::resolve("HIGH"), EncoderPreset::Quality);
    }

    #[test]
    fn encoder_preset_unknown_falls_back_to_balanced() {
        // Default ensures a typo in admin settings doesn't break sessions.
        assert_eq!(EncoderPreset::resolve(""), EncoderPreset::Balanced);
        assert_eq!(EncoderPreset::resolve("ludicrous"), EncoderPreset::Balanced);
    }
}
