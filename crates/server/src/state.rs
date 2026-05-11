//! Application state shared across handlers.

use chimpflix_metadata::TmdbClient;
use chimpflix_transcoder::{FfmpegConfig, TranscodeManager};
use sqlx::SqlitePool;

use crate::auth::AuthConfig;
use crate::events::Hub;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub ffmpeg: FfmpegConfig,
    pub tmdb: Option<TmdbClient>,
    pub hub: Hub,
    pub auth: AuthConfig,
    pub transcoder: TranscodeManager,
}
