//! ffmpeg/ffprobe orchestration for probing and HLS transcoding.

pub mod capabilities;
pub mod hwaccel;
pub mod markers;
pub mod previews;
pub mod probe;
pub mod session;

pub use capabilities::{TranscoderCapabilities, detect_capabilities};
pub use hwaccel::{EncoderPreset, HwAccel};
pub use markers::{DetectedMarker, MarkerKind, detect_markers};
pub use previews::{DEFAULT_INTERVAL_S, DEFAULT_TILE_WIDTH, SpriteInfo, generate_sprite};
pub use probe::{GopProbe, ProbeResult, ProbeStream, StreamKind, probe, probe_gop, probe_subtitle_codec};
pub use session::{
    AudioTreatment, ContainerFormat, HLS_SEGMENT_DURATION_S, Session, SessionSnapshot,
    SubExtractionStatus, TranscodeManager, VideoTreatment, WebVttSidecar,
    evict_text_subs_cache, is_text_subtitle_codec, scan_prewarm_text_subs,
};

#[derive(Debug, Clone)]
pub struct FfmpegConfig {
    pub ffmpeg: String,
    pub ffprobe: String,
}

impl Default for FfmpegConfig {
    fn default() -> Self {
        Self {
            ffmpeg: "ffmpeg".to_string(),
            ffprobe: "ffprobe".to_string(),
        }
    }
}

impl FfmpegConfig {
    pub fn from_env() -> Self {
        Self {
            ffmpeg: std::env::var("FFMPEG_BIN").unwrap_or_else(|_| "ffmpeg".to_string()),
            ffprobe: std::env::var("FFPROBE_BIN").unwrap_or_else(|_| "ffprobe".to_string()),
        }
    }
}
