//! ffmpeg/ffprobe orchestration for probing and HLS transcoding.

pub mod capabilities;
pub mod chapter_thumbs;
pub mod hwaccel;
pub mod loudness;
pub mod markers;
pub mod previews;
pub mod probe;
pub mod session;

pub use capabilities::{TranscoderCapabilities, detect_capabilities};
pub use hwaccel::{EncoderPreset, HwAccel, VideoCodec};
pub use markers::{DetectedMarker, MarkerKind, detect_markers};
pub use previews::{DEFAULT_INTERVAL_S, DEFAULT_TILE_WIDTH, SpriteInfo, generate_sprite};
pub use probe::{
    Chapter, GopProbe, ProbeResult, ProbeStream, StreamKind, probe, probe_chapters, probe_gop,
    probe_subtitle_codec,
};
pub use session::{
    AudioTreatment, ContainerFormat, HLS_SEGMENT_DURATION_S, LoudnessTarget, Session,
    SessionSnapshot, SubExtractionStatus, TonemapConfig, TranscodeManager, VideoTreatment,
    WebVttSidecar, evict_text_subs_cache, is_text_subtitle_codec, scan_prewarm_text_subs,
};

#[derive(Debug, Clone)]
pub struct FfmpegConfig {
    pub ffmpeg: String,
    pub ffprobe: String,
    /// When set (1..=19), background work — scheduled tasks, scanner
    /// probes, marker detection, preview/thumb extraction, loudness
    /// analysis — wraps ffmpeg/ffprobe in `nice -n <level>` so it
    /// yields to live transcode sessions and the rest of the system.
    /// `None` = run at default priority.
    pub background_nice_level: Option<i32>,
}

impl Default for FfmpegConfig {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".to_string(),
            ffprobe: "ffprobe".to_string(),
            background_nice_level: None,
        }
    }
}

impl FfmpegConfig {
    pub fn from_env() -> Self {
        Self {
            ffmpeg: std::env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".to_string()),
            ffprobe: std::env::var("FFPROBE_BIN").unwrap_or_else(|_| "ffprobe".to_string()),
            background_nice_level: None,
        }
    }

    /// A `Command` builder for background ffmpeg work. When
    /// `background_nice_level` is set, the returned command is the
    /// `nice` wrapper with ffmpeg as its first arg — child runs at
    /// the requested priority irrespective of which tokio worker
    /// thread forked it. Use for scheduled tasks; live sessions
    /// should keep using `Command::new(&cfg.ffmpeg)` directly.
    pub fn background_ffmpeg(&self) -> tokio::process::Command {
        match self.background_nice_level {
            Some(n) => {
                let mut cmd = tokio::process::Command::new("nice");
                cmd.args(["-n", &n.to_string()]).arg(&self.ffmpeg);
                cmd
            }
            None => tokio::process::Command::new(&self.ffmpeg),
        }
    }

    pub fn background_ffprobe(&self) -> tokio::process::Command {
        match self.background_nice_level {
            Some(n) => {
                let mut cmd = tokio::process::Command::new("nice");
                cmd.args(["-n", &n.to_string()]).arg(&self.ffprobe);
                cmd
            }
            None => tokio::process::Command::new(&self.ffprobe),
        }
    }
}
