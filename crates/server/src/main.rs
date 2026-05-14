//! ChimpFlix server entrypoint.

mod api;
mod auth;
mod events;
mod log_buffer;
mod scheduler;
mod session_watcher;
mod state;
mod webhooks;

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
    let log_buffer = log_buffer::LogBuffer::new();
    init_tracing(log_buffer.clone());

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
    // TVMaze is a free fallback for shows; no key required. We always
    // try to construct it — a failure here just means we skip the fallback
    // (it's not fatal to enrichment).
    let tvmaze = match chimpflix_metadata::TvMazeClient::new() {
        Ok(c) => Some(c),
        Err(e) => {
            warn!(error = %format!("{e:#}"), "TVMaze client init failed; fallback disabled");
            None
        }
    };

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
    // Reap orphaned sessions aggressively: 20s idle threshold, polled every
    // 10s. Worst-case ~30s of stale ffmpeg if the client's DELETE was
    // lost (page killed mid-flight). The client touches the session on
    // every HLS manifest/segment request, so 20s is well above the
    // 6-second segment cadence.
    transcoder.spawn_reaper(20_000, 10);

    let transcoder_caps = Arc::new(chimpflix_transcoder::detect_capabilities(&ffmpeg).await);
    info!(
        ffmpeg = ?transcoder_caps.ffmpeg_version,
        hwaccels = ?transcoder_caps.hwaccels,
        "ffmpeg capability probe complete"
    );

    let hub = Hub::new(256);

    // Hydrate the server settings cache. The migration guarantees a row
    // exists (id = 1) with defaults; a missing row here is a corruption
    // bug, not a missing-config one.
    let initial_settings = queries::get_server_settings(&pool)
        .await
        .context("load server_settings singleton")?;
    let settings = std::sync::Arc::new(tokio::sync::RwLock::new(initial_settings));

    let state = AppState {
        pool,
        ffmpeg,
        tmdb,
        tvmaze,
        hub,
        auth: AuthConfig {
            session_secret: Arc::new(session_secret),
            cookie_secure,
        },
        transcoder,
        data_dir: data_dir.clone(),
        settings,
        started_at_ms: chimpflix_common::now_ms(),
        transcoder_caps,
        log_buffer,
    };

    // Scheduled tasks: flip orphaned `running` rows, seed defaults, spawn
    // the runner loop. We do this after AppState is fully assembled.
    let interrupted_tasks = queries::mark_interrupted_tasks(&state.pool).await?;
    if interrupted_tasks > 0 {
        warn!(
            count = interrupted_tasks,
            "marked previously-running scheduled tasks as failed"
        );
    }
    if let Err(e) = scheduler::seed_defaults(&state.pool).await {
        warn!(error = %format!("{e:#}"), "scheduler seed failed; tasks can still be created manually");
    }
    scheduler::spawn(state.clone());
    webhooks::spawn(state.clone());
    session_watcher::spawn(state.hub.clone(), state.transcoder.clone());

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

fn init_tracing(buffer: log_buffer::LogBuffer) {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(log_buffer::LogBufferLayer::new(buffer))
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
