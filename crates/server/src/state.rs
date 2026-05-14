//! Application state shared across handlers.

use std::sync::Arc;

use chimpflix_library::ServerSettings;
use chimpflix_metadata::{TmdbClient, TvMazeClient};
use chimpflix_transcoder::{FfmpegConfig, TranscodeManager, TranscoderCapabilities};
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::auth::AuthConfig;
use crate::events::Hub;

/// Hot-reloadable cache over the `server_settings` singleton row. Every
/// admin `PATCH` to settings updates this in-place so subscribers (the
/// transcoder, CORS layer, network layer, scheduler) can re-read without
/// the round-trip to SQLite.
pub type SettingsCache = Arc<RwLock<ServerSettings>>;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub ffmpeg: FfmpegConfig,
    pub tmdb: Option<TmdbClient>,
    /// TVMaze fallback provider for shows. Always constructed (no key
    /// required); `None` only if HTTP client init fails.
    pub tvmaze: Option<TvMazeClient>,
    pub hub: Hub,
    pub auth: AuthConfig,
    pub transcoder: TranscodeManager,
    /// On-disk path of the DATA_DIR. Used by the admin backup endpoint to
    /// write VACUUM INTO snapshots and serve them back to the owner.
    pub data_dir: std::path::PathBuf,
    /// Cached server-wide settings; reloaded on every admin PATCH.
    pub settings: SettingsCache,
    /// Epoch ms when the server process started. Used by the dashboard
    /// to report uptime.
    pub started_at_ms: i64,
    /// Detected ffmpeg hardware accelerators + encoders at startup. The
    /// admin UI uses this to grey out options the host can't run.
    pub transcoder_caps: Arc<TranscoderCapabilities>,
    /// In-memory ring buffer of recent `tracing` events for the Logs page.
    pub log_buffer: crate::log_buffer::LogBuffer,
}
