//! ChimpFlix server entrypoint.

mod api;
mod auth;
mod events;
mod state;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use chimpflix_library::queries;
use chimpflix_metadata::TmdbClient;
use chimpflix_transcoder::{FfmpegConfig, TranscodeManager};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use crate::auth::AuthConfig;
use crate::events::Hub;
use crate::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let data_dir = env::var("DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./data"));
    let bind_addr: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()
        .context("BIND_ADDR is not a valid socket address")?;

    info!(?data_dir, %bind_addr, "starting chimpflix-server");

    let pool = chimpflix_library::open(&data_dir).await?;
    queries::ensure_default_user(&pool).await?;

    let interrupted = queries::mark_interrupted_scans(&pool).await?;
    if interrupted > 0 {
        warn!(
            count = interrupted,
            "marked previously-running scan jobs as failed"
        );
    }
    let expired = queries::cleanup_expired_sessions(&pool).await?;
    if expired > 0 {
        info!(count = expired, "purged expired sessions");
    }

    let ffmpeg = FfmpegConfig::from_env();
    let tmdb = TmdbClient::from_env()?;
    match &tmdb {
        Some(_) => info!("TMDB enrichment enabled"),
        None => warn!("TMDB_READ_TOKEN unset — metadata enrichment is disabled"),
    }

    let session_secret = auth::secret::load_or_generate(&data_dir)?;
    let cookie_secure = env::var("APP_PUBLIC_ORIGIN")
        .ok()
        .is_some_and(|origin| origin.starts_with("https://"));
    if !cookie_secure {
        warn!(
            "session cookie will be set without the Secure flag — set APP_PUBLIC_ORIGIN=https://… \
             before exposing this server to the internet"
        );
    }

    if queries::is_in_setup_mode(&pool).await? {
        warn!(
            "first-run setup required — POST /api/v1/auth/setup with {{username, password}} to \
             create the owner account"
        );
    }

    let transcoder = TranscodeManager::new(data_dir.join("cache/sessions"), ffmpeg.clone())?;
    transcoder.spawn_reaper(60_000, 30);

    let hub = Hub::new(256);
    let state = AppState {
        pool,
        ffmpeg,
        tmdb,
        hub,
        auth: AuthConfig {
            session_secret: Arc::new(session_secret),
            cookie_secure,
        },
        transcoder,
    };

    let app = api::router(state);
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind {bind_addr}"))?;
    info!(%bind_addr, "listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .init();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        sigterm.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    info!("shutdown signal received");
}
