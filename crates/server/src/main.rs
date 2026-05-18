//! ChimpFlix server entrypoint.

mod api;
mod auth;
mod events;
mod file_watcher;
mod log_buffer;
mod mailer;
mod notifier;
mod scheduler;
mod session_watcher;
mod state;
mod totp;
mod trakt_sync;
mod webhooks;

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use chimpflix_common::Vault;
use chimpflix_library::queries;
use chimpflix_metadata::{
    AniListClient, OpenSubtitlesClient, OpenSubtitlesCreds, TmdbClient, TraktClient, TraktCreds,
    TvdbClient,
};
use chimpflix_transcoder::{FfmpegConfig, TranscodeManager};
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
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

    let vault = Arc::new(load_vault());

    let pool = chimpflix_library::open(&data_dir).await?;
    queries::ensure_default_user(&pool).await?;

    // Migrate any legacy plaintext webhook secrets into the encrypted
    // columns. Idempotent — once every row is converted this is a no-op.
    match queries::backfill_webhook_secrets(&pool, &vault).await {
        Ok(0) => {}
        Ok(n) => info!(count = n, "encrypted legacy webhook secrets at rest"),
        Err(e) => warn!(error = %format!("{e:#}"), "webhook secret backfill failed"),
    }
    // Forward-upgrade rows that were stored before CHIMPFLIX_SECRET_KEY was
    // set. Without this the first vault_get on a plaintext row would
    // crash boot when encryption gets turned on after the fact.
    match queries::upgrade_plaintext_secrets(&pool, &vault).await {
        Ok((0, 0)) => {}
        Ok((named, hooks)) => info!(
            named_secrets = named,
            webhook_secrets = hooks,
            "re-encrypted previously-plaintext secrets"
        ),
        Err(e) => warn!(error = %format!("{e:#}"), "plaintext-secret upgrade failed"),
    }

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
    maybe_import_tmdb_from_env(&pool, &vault).await?;
    let tmdb = build_tmdb_from_vault(&pool, &vault).await?;
    match &tmdb {
        Some(_) => info!("TMDB enrichment enabled"),
        None => warn!(
            "TMDB key not set — metadata enrichment is disabled (configure under \
             /admin/server/credentials, or set TMDB_READ_TOKEN for a one-time import)"
        ),
    }
    let tmdb = Arc::new(RwLock::new(tmdb));

    let tvdb = build_tvdb_from_vault(&pool, &vault).await?;
    match &tvdb {
        Some(_) => info!("TVDB backfill enabled"),
        None => info!("TVDB backfill disabled — no key under /admin/server/credentials"),
    }
    let tvdb = Arc::new(RwLock::new(tvdb));

    // AniList works unauthenticated (30 req/min). If the operator has
    // stored a token in the vault we use the higher-limit auth path.
    let anilist = build_anilist_from_vault(&pool, &vault).await?;
    match &anilist {
        Some(_) => info!("AniList enrichment enabled (anime libraries)"),
        None => warn!("AniList client init failed; anime enrichment is disabled"),
    }
    let anilist = Arc::new(RwLock::new(anilist));

    let opensubtitles = build_opensubtitles_from_vault(&pool, &vault).await?;
    match &opensubtitles {
        Some(_) => info!("OpenSubtitles enabled"),
        None => info!("OpenSubtitles disabled — no credentials under /admin/server/credentials"),
    }
    let opensubtitles = Arc::new(RwLock::new(opensubtitles));

    let trakt = build_trakt_from_vault(&pool, &vault).await?;
    match &trakt {
        Some(_) => info!("Trakt OAuth app loaded"),
        None => info!("Trakt disabled — no credentials under /admin/server/credentials"),
    }
    let trakt = Arc::new(RwLock::new(trakt));
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

    let session_secret = auth::secret::load_or_migrate(&pool, &vault, &data_dir).await?;
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

    // Probe capabilities before constructing the transcoder so the
    // manager can use the detected per-card decoder list when
    // deciding whether to add `-hwaccel <name>` per session. The
    // probe is in-process — slow on a cold container (every codec
    // test makes a libavcodec call) but bounded by SMOKE_TIMEOUT.
    let transcoder_caps = Arc::new(chimpflix_transcoder::detect_capabilities(&ffmpeg).await);
    info!(
        ffmpeg = ?transcoder_caps.ffmpeg_version,
        hwaccels = ?transcoder_caps.hwaccels,
        h264_encoders = ?transcoder_caps.h264_encoders,
        hevc_encoders = ?transcoder_caps.hevc_encoders,
        cuda_decoders = ?transcoder_caps.decoders.cuda,
        vaapi_decoders = ?transcoder_caps.decoders.vaapi,
        qsv_decoders = ?transcoder_caps.decoders.qsv,
        videotoolbox_decoders = ?transcoder_caps.decoders.videotoolbox,
        "ffmpeg capability probe complete"
    );

    let transcoder = TranscodeManager::new(
        data_dir.join("cache/sessions"),
        ffmpeg.clone(),
        transcoder_caps.clone(),
    )?;
    // Reap orphaned sessions: 5-minute idle threshold, polled every 30s.
    // The client touches the session on every HLS manifest/segment
    // request, but HLS.js stops polling once its buffer is full — so a
    // user who pauses for a few minutes goes silent. 20s used to kill
    // those sessions; raise the floor to comfortably cover a coffee
    // break. Cost of the higher floor is ~5 minutes of stale ffmpeg
    // after a lost DELETE, which is acceptable.
    transcoder.spawn_reaper(300_000, 30);

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
        tvdb,
        anilist,
        opensubtitles,
        trakt,
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
        vault,
        login_attempts: crate::api::rate_limit::AttemptTracker::new(),
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
    file_watcher::spawn(state.clone());

    let app = api::router(state);
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("bind {bind_addr}"))?;
    info!(%bind_addr, "listening");

    // into_make_service_with_connect_info exposes the peer SocketAddr
    // to handlers/middleware via ConnectInfo — required by the
    // per-IP rate limiter on auth routes. The limiter still honors
    // X-Forwarded-For / X-Real-IP if set by a trusted proxy upstream.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
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

/// Load the credential vault from `CHIMPFLIX_SECRET_KEY`. If the env var
/// is unset we boot in plaintext mode and print a ready-to-paste suggested
/// key so the operator can harden later without hunting for a generator.
fn load_vault() -> Vault {
    match Vault::from_env() {
        Ok((vault, true)) => {
            info!("credential vault: encrypted at rest");
            vault
        }
        Ok((vault, false)) => {
            let suggested = chimpflix_common::generate_master_key_hex();
            warn!(
                "credential vault is in PLAINTEXT mode — secrets in the SQLite file are not \
                 encrypted. To enable encryption-at-rest, restart with:"
            );
            warn!("    {}={}", chimpflix_common::MASTER_KEY_ENV, suggested);
            vault
        }
        Err(e) => {
            // Malformed env value — refuse to start. The plaintext fallback
            // only kicks in when the env is *absent*, not when it's set to
            // garbage. Booting silently would mask a typo.
            panic!("credential vault failed to load: {e:#}");
        }
    }
}

/// First-boot one-shot: if `secrets.tmdb` is empty but the legacy
/// `TMDB_READ_TOKEN` env var is set, copy the value into the vault so the
/// operator can rotate it from the admin UI from then on. The env var
/// continues to work for the lifetime of this process.
async fn maybe_import_tmdb_from_env(pool: &SqlitePool, vault: &Vault) -> anyhow::Result<()> {
    if queries::vault_get(pool, vault, "tmdb").await?.is_some() {
        return Ok(());
    }
    let Ok(raw) = std::env::var("TMDB_READ_TOKEN") else {
        return Ok(());
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    queries::vault_set(pool, vault, "tmdb", trimmed, None).await?;
    info!("imported TMDB_READ_TOKEN from env into credential vault");
    Ok(())
}

async fn build_tmdb_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<TmdbClient>> {
    let Some(token) = queries::vault_get(pool, vault, "tmdb").await? else {
        return Ok(None);
    };
    Ok(Some(TmdbClient::new(&token)?))
}

async fn build_tvdb_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<TvdbClient>> {
    let Some(apikey) = queries::vault_get(pool, vault, "tvdb").await? else {
        return Ok(None);
    };
    Ok(Some(TvdbClient::new(&apikey, None)?))
}

async fn build_anilist_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<AniListClient>> {
    let token = queries::vault_get(pool, vault, "anilist").await?;
    let client = match token {
        Some(t) => AniListClient::with_token(&t),
        None => AniListClient::unauthenticated(),
    };
    Ok(client.ok())
}

async fn build_opensubtitles_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<OpenSubtitlesClient>> {
    let Some(raw) = queries::vault_get(pool, vault, "opensubtitles").await? else {
        return Ok(None);
    };
    let creds = OpenSubtitlesCreds::parse(&raw)?;
    Ok(Some(OpenSubtitlesClient::new(creds)?))
}

async fn build_trakt_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<TraktClient>> {
    let Some(raw) = queries::vault_get(pool, vault, "trakt").await? else {
        return Ok(None);
    };
    let creds = TraktCreds::parse(&raw)?;
    Ok(Some(TraktClient::from_creds(creds)?))
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
