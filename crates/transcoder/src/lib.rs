//! ffmpeg/ffprobe orchestration for probing and HLS transcoding.

pub mod probe;
pub mod session;

pub use probe::{ProbeResult, ProbeStream, StreamKind, probe};
pub use session::{Session, TranscodeManager};

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
