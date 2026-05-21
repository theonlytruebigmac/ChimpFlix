//! ChimpFlix server entrypoint.

mod api;
mod auth;
mod client_ip;
mod events;
mod file_watcher;
mod jobs;
mod log_buffer;
mod mail_template;
mod mailer;
mod net;
mod notifier;
mod scheduler;
mod session_watcher;
mod ssrf;
mod state;
mod subtitles_lookup;
mod tasks;
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
    AniListClient, OmdbClient, OpenSubtitlesClient, OpenSubtitlesCreds, TmdbClient, TraktClient,
    TraktCreds, TvdbClient,
};
use chimpflix_transcoder::{FfmpegConfig, TranscodeManager};
use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, Layer as _, fmt, prelude::*};

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
    let mut bind_addr: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
        .parse()
        .context("BIND_ADDR is not a valid socket address")?;

    info!(?data_dir, %bind_addr, "starting chimpflix-server");

    let vault = Arc::new(load_vault());

    // Apply any operator-staged restore before opening the DB. When a
    // `chimpflix.db.pending-restore` file is present in data_dir, this
    // moves the current `chimpflix.db` aside to
    // `chimpflix.db.pre-restore-<stamp>.db` and renames the pending
    // file into place. Idempotent + no-op when nothing is staged.
    if let Err(e) = crate::api::admin::backup::apply_pending_restore_if_present(&data_dir).await {
        warn!(error = %format!("{e:#}"), "pending-restore application failed; booting against current DB");
    }

    // Two-pass open so the operator-configured `database_cache_size_mb`
    // is baked into every pooled connection from the start. The probe
    // pool is opened with SQLite defaults, runs migrations, and reads
    // the setting; if the operator wants a non-default cache, we close
    // the probe pool and reopen with the value pinned via
    // `PRAGMA cache_size`. The double-open cost is microseconds at
    // boot and is far simpler than juggling per-connection PRAGMA
    // application against a shared atomic.
    let probe_pool = chimpflix_library::open(&data_dir).await?;
    let configured_cache_mb = match queries::get_server_settings(&probe_pool).await {
        Ok(s) => s.database_cache_size_mb,
        Err(_) => 0,
    };
    let pool = if configured_cache_mb > 0 {
        probe_pool.close().await;
        info!(
            cache_size_mb = configured_cache_mb,
            "reopening database with operator-configured PRAGMA cache_size",
        );
        chimpflix_library::open_with(&data_dir, Some(configured_cache_mb)).await?
    } else {
        probe_pool
    };
    queries::ensure_default_user(&pool).await?;

    // Migrate any legacy plaintext webhook secrets into the encrypted
    // columns. Idempotent — once every row is converted this is a no-op.
    match queries::backfill_webhook_secrets(&pool, &vault).await {
        Ok(0) => {}
        Ok(n) => info!(count = n, "encrypted legacy webhook secrets at rest"),
        Err(e) => warn!(error = %format!("{e:#}"), "webhook secret backfill failed"),
    }
    // Same pattern for Trakt OAuth tokens (Phase 53). Per-user access
    // + refresh tokens were stored as plaintext TEXT; backfill encrypts
    // them via the vault on every boot until the legacy columns are
    // empty everywhere.
    match queries::backfill_trakt_tokens(&pool, &vault).await {
        Ok(0) => {}
        Ok(n) => info!(count = n, "encrypted legacy Trakt OAuth tokens at rest"),
        Err(e) => warn!(error = %format!("{e:#}"), "trakt token backfill failed"),
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

    let mut ffmpeg = FfmpegConfig::from_env();
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

    let omdb = build_omdb_from_vault(&pool, &vault).await?;
    match &omdb {
        Some(_) => info!("OMDb ratings enabled"),
        None => info!("OMDb disabled — no key under /admin/server/credentials"),
    }
    let omdb = Arc::new(RwLock::new(omdb));
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

    // Hydrate the server settings cache. The migration guarantees a row
    // exists (id = 1) with defaults; a missing row here is a corruption
    // bug, not a missing-config one. Loading early so the reaper can
    // honour the operator's configured idle threshold from the first
    // tick instead of starting at the hard-coded default.
    let initial_settings = queries::get_server_settings(&pool)
        .await
        .context("load server_settings singleton")?;
    // Wire the scanner nice level into the FfmpegConfig that scheduled
    // tasks and the file watcher will use. Live transcode sessions
    // call `Command::new(&cfg.ffmpeg)` directly and bypass the nice
    // wrapper — so the prior `transcoder.clone()` of `ffmpeg` (which
    // is what `TranscodeManager` keeps) doesn't pick up the nice
    // level, which is intentional. The state.ffmpeg used by scanner
    // and tasks gets the nice level applied here.
    if (1..=19).contains(&initial_settings.scanner_nice_level) {
        ffmpeg.background_nice_level = Some(initial_settings.scanner_nice_level as i32);
        info!(
            level = initial_settings.scanner_nice_level,
            "background ffmpeg/ffprobe will run under `nice -n N`"
        );
    }
    // Operator-set bind override takes precedence over the env when
    // non-empty. Parsed at write-time so a malformed value is rejected
    // before storage; the parse here is just to surface it; failure
    // falls back to the env-derived value with a warning.
    let bi = initial_settings.bind_interface.trim();
    if !bi.is_empty() {
        match bi.parse::<SocketAddr>() {
            Ok(addr) => {
                info!(env = %bind_addr, override = %addr, "honoring settings bind_interface");
                bind_addr = addr;
            }
            Err(e) => {
                warn!(value = %bi, error = %e, "ignoring malformed bind_interface, using env");
            }
        }
    }
    // Operator-controllable worker count for the durable job queue.
    // Captured here before `initial_settings` is moved into the
    // Arc<RwLock> so the jobs::start call below doesn't need to take
    // a read lock. Clamped defensively in case a hand-edited DB row
    // bypassed the schema CHECK.
    let job_workers = initial_settings.job_workers.clamp(1, 16) as usize;
    // Reap orphaned sessions on the operator's configured idle window.
    // The client sends a keepalive ping every 60s (and on every HLS
    // manifest/segment request), so the default 90s floor catches a
    // single missed beat plus reaper interval slack. Aggressive cleanup
    // matters most on mobile, where force-closing the PWA doesn't
    // reliably fire any unload event the server can observe — the only
    // signal is the keepalive going silent. The threshold is a startup-
    // time read (spawn_reaper takes an i64, not a settings handle);
    // changing it via the admin UI takes effect on next restart.
    // Reaper with a stats hook: every time the reaper kills an idle
    // session, fan out a `stop` event to playback_events with the
    // final cumulative bytes_served. Gives the admin Stats page
    // per-stream bandwidth without any per-segment DB write.
    let pool_for_reaper = pool.clone();
    transcoder.spawn_reaper_with_hook(
        initial_settings.transcoder_reaper_idle_threshold_ms,
        15,
        move |snap| {
            let pool = pool_for_reaper.clone();
            tokio::spawn(async move {
                emit_session_stop_event(&pool, &snap).await;
            });
        },
    );

    // Broadcast capacity for the in-process event hub. Sized for the
    // worst-case burst: a freshly-dropped season pack (typically up to
    // ~50 files) triggers a scan that emits per-file Added / Updated
    // events plus the eventual Completed. With 4096 slots a slow
    // subscriber (e.g. a paused WebSocket client) has plenty of room
    // before tokio::broadcast starts dropping for laggards. The old
    // 256-slot ceiling was tight enough that webhook deliveries and
    // scan-progress events would silently disappear under burst.
    let hub = Hub::new(4096);

    let settings = std::sync::Arc::new(tokio::sync::RwLock::new(initial_settings));

    // Trusted-proxy allowlist. Without this, the app refuses to honour
    // `X-Forwarded-For` / `CF-Connecting-IP` / `X-Real-IP` and falls
    // back to the peer socket address. Operators behind Traefik should
    // set this to the Docker bridge (e.g. `172.16.0.0/12`); behind
    // Cloudflare-fronted Traefik, additionally include Cloudflare's
    // published ranges so `CF-Connecting-IP` is preferred.
    let trusted_proxies = std::sync::Arc::new(net::parse_cidr_list(
        &env::var("TRUSTED_PROXIES").unwrap_or_default(),
    ));
    if trusted_proxies.is_empty() {
        warn!(
            "TRUSTED_PROXIES is empty — proxy headers (X-Forwarded-For, CF-Connecting-IP, \
             X-Real-IP) will be ignored. The recorded client IP will be the immediate peer \
             (likely the Docker bridge). Set TRUSTED_PROXIES=<cidr-list> before exposing \
             behind a reverse proxy."
        );
    } else {
        info!(
            count = trusted_proxies.len(),
            "trusted upstream proxies configured; proxy headers will be honored from listed peers"
        );
    }

    let state = AppState {
        pool,
        ffmpeg,
        tmdb,
        tvdb,
        anilist,
        opensubtitles,
        trakt,
        omdb,
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
        reset_email_limiter: crate::api::rate_limit::reset_email_limiter(),
        report_issue_limiter: crate::api::rate_limit::report_issue_limiter(),
        trusted_proxies,
        library_scans_in_progress: Arc::new(tokio::sync::RwLock::new(
            std::collections::HashSet::new(),
        )),
        trakt_refresh_locks: Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        task_metrics: crate::tasks::metrics::LiveMetrics::new(),
        worker_pool: Arc::new(tokio::sync::RwLock::new(None)),
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
    // Durable background job queue (see `crates/server/src/jobs/`).
    // Runs reclaim on startup so anything left as `running` from a
    // previous crash gets put back in the queue. Worker count is
    // operator-controllable (server_settings.job_workers, default 2)
    // — bump it on hosts with CPU headroom to let more pipeline
    // kinds run in parallel; lower it on small boxes where live
    // transcodes need the cores.
    let worker_pool =
        jobs::start(state.clone(), jobs::build_router(), job_workers).await;
    *state.worker_pool.write().await = Some(worker_pool);
    webhooks::spawn(state.clone());
    session_watcher::spawn(state.hub.clone(), state.transcoder.clone());
    // Filesystem watcher is always spawned — it consults the live
    // `scan_automatically` setting on every event tick so admins can
    // toggle real-time ingestion without restarting. When the toggle
    // is off, the watcher still receives kernel inotify events (near-
    // zero CPU at idle) but drops them without queueing scans, so
    // flipping the switch back on resumes immediately.
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
    // Per-layer filters: the stdout fmt layer honors RUST_LOG (so
    // container/syslog stays tidy), but the in-memory buffer that powers
    // the admin Logs page captures everything at TRACE-and-above. Without
    // this split, applying RUST_LOG=info globally meant the buffer never
    // saw DEBUG/TRACE — and changing the UI's "Min level" dropdown from
    // INFO down to TRACE looked like a no-op because there was nothing
    // below INFO to reveal.
    //
    // The buffer cap (5k lines in log_buffer.rs) keeps memory bounded
    // even when a chatty crate spams DEBUG; oldest evicts first.
    let stdout_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,tower_http=info"));
    // Buffer captures TRACE+ globally so the UI dropdown is meaningful,
    // but silences a few notoriously chatty deps (sqlx at the statement
    // level, the hyper/reqwest HTTP plumbing) to keep the 5k-line ring
    // useful instead of swamped with one-shot request frames.
    let buffer_filter = EnvFilter::new("trace,sqlx=info,hyper=info,h2=info,reqwest=info");
    tracing_subscriber::registry()
        .with(fmt::layer().with_filter(stdout_filter))
        .with(log_buffer::LogBufferLayer::new(buffer).with_filter(buffer_filter))
        .init();
}

/// Load the credential vault from `CHIMPFLIX_SECRET_KEY`. If the env var
/// is unset we boot in plaintext mode and print a ready-to-paste
/// suggested key so the operator can harden later without hunting for a
/// generator. Public deployments (any `APP_PUBLIC_ORIGIN` over https://)
/// REFUSE to boot in plaintext mode — there's no defensible reason for
/// an internet-facing instance to store SMTP creds, TOTP secrets, and
/// the session HMAC in the clear. The operator can opt back in by
/// setting `CHIMPFLIX_ALLOW_PLAINTEXT_VAULT=1` (intended for testing
/// only).
fn load_vault() -> Vault {
    match Vault::from_env() {
        Ok((vault, true)) => {
            info!("credential vault: encrypted at rest");
            vault
        }
        Ok((vault, false)) => {
            let is_public = env::var("APP_PUBLIC_ORIGIN")
                .ok()
                .is_some_and(|origin| origin.starts_with("https://"));
            let opt_in = env::var("CHIMPFLIX_ALLOW_PLAINTEXT_VAULT")
                .ok()
                .is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes"));
            if is_public && !opt_in {
                let suggested = chimpflix_common::generate_master_key_hex();
                eprintln!(
                    "FATAL: APP_PUBLIC_ORIGIN looks public (https://...) but \
                     {} is unset. Plaintext vault mode is forbidden for internet-\
                     exposed deployments because SMTP creds, TOTP secrets, and the \
                     session HMAC would land in chimpflix.db in the clear.\n\n\
                     Restart with:\n    {}={}\n\n\
                     (Or set CHIMPFLIX_ALLOW_PLAINTEXT_VAULT=1 to override — \
                     not recommended.)",
                    chimpflix_common::MASTER_KEY_ENV,
                    chimpflix_common::MASTER_KEY_ENV,
                    suggested,
                );
                std::process::exit(78); // EX_CONFIG
            }
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

/// Pull a vault slot, returning None on either "no row" OR "decrypt
/// failed". Decrypt failures are logged with a loud actionable message
/// pointing the operator at the recovery path (re-enter via admin UI,
/// or wipe the row with a DELETE).
///
/// This is the boot-resilient wrapper around `queries::vault_get`. A
/// rotated `CHIMPFLIX_SECRET_KEY` invalidates every previously-encrypted
/// row; without this helper the first `?` propagation would panic the
/// whole boot, leaving the operator with a server stuck in a restart
/// loop instead of one that's up-but-with-broken-integrations.
async fn vault_get_or_warn(
    pool: &SqlitePool,
    vault: &Vault,
    slot: &'static str,
) -> Option<String> {
    match queries::vault_get(pool, vault, slot).await {
        Ok(value) => value,
        Err(e) => {
            warn!(
                slot,
                error = %format!("{e:#}"),
                "vault slot is unreadable — integration disabled for this boot. \
                 Most likely cause: CHIMPFLIX_SECRET_KEY was rotated. Recover by \
                 re-entering the credential under Admin → Server → Credentials, \
                 or clear the slot: `sqlite3 chimpflix.db \"DELETE FROM secrets \
                 WHERE name='{slot}'\";`",
            );
            None
        }
    }
}

/// First-boot one-shot: if `secrets.tmdb` is empty but the legacy
/// `TMDB_READ_TOKEN` env var is set, copy the value into the vault so the
/// operator can rotate it from the admin UI from then on. The env var
/// continues to work for the lifetime of this process.
async fn maybe_import_tmdb_from_env(pool: &SqlitePool, vault: &Vault) -> anyhow::Result<()> {
    // If the slot is present-and-readable we're done. If the slot is
    // present-but-unreadable (rotated key), we DON'T silently overwrite
    // it — the operator needs to explicitly DELETE so the import is
    // intentional rather than a sneaky env-var override of admin-set
    // credentials. So: only import when the row genuinely doesn't exist.
    match queries::vault_get(pool, vault, "tmdb").await {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(e) => {
            warn!(
                error = %format!("{e:#}"),
                "tmdb vault slot exists but couldn't decrypt — skipping TMDB_READ_TOKEN \
                 env import to avoid clobbering admin-set creds. Re-enter the token \
                 via Admin → Server → Credentials, or clear the slot first.",
            );
            return Ok(());
        }
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
    let Some(token) = vault_get_or_warn(pool, vault, "tmdb").await else {
        return Ok(None);
    };
    // Read the operator's preferred metadata language inline rather
    // than threading it through main()'s startup sequence — the
    // settings row is tiny and this avoids reordering against the
    // later `initial_settings` load that several other subsystems
    // depend on. Settings changes here require a server restart since
    // TmdbClient is a process-wide singleton consumed by the scanner
    // and on-demand fix-match path.
    let language = match queries::get_server_settings(pool).await {
        Ok(s) => s.metadata_language,
        Err(_) => "en-US".to_string(),
    };
    Ok(Some(TmdbClient::with_language(&token, &language)?))
}

async fn build_tvdb_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<TvdbClient>> {
    let Some(apikey) = vault_get_or_warn(pool, vault, "tvdb").await else {
        return Ok(None);
    };
    Ok(Some(TvdbClient::new(&apikey, None)?))
}

async fn build_anilist_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<AniListClient>> {
    let token = vault_get_or_warn(pool, vault, "anilist").await;
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
    let Some(raw) = vault_get_or_warn(pool, vault, "opensubtitles").await else {
        return Ok(None);
    };
    let creds = OpenSubtitlesCreds::parse(&raw)?;
    Ok(Some(OpenSubtitlesClient::new(creds)?))
}

async fn build_trakt_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<TraktClient>> {
    let Some(raw) = vault_get_or_warn(pool, vault, "trakt").await else {
        return Ok(None);
    };
    let creds = TraktCreds::parse(&raw)?;
    Ok(Some(TraktClient::from_creds(creds)?))
}

async fn build_omdb_from_vault(
    pool: &SqlitePool,
    vault: &Vault,
) -> anyhow::Result<Option<OmdbClient>> {
    let Some(key) = vault_get_or_warn(pool, vault, "omdb").await else {
        return Ok(None);
    };
    Ok(Some(OmdbClient::new(key)?))
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

/// Emit a `stop` event with the closing session's cumulative
/// bandwidth + final position. Called from both the reaper (via the
/// hook registered above) and the explicit `DELETE /sessions/{id}`
/// path so admin Stats reflects every terminated stream. Resolves the
/// owning item / episode via `media_file_owner` — the same pattern
/// the start-event recorder uses.
pub(crate) async fn emit_session_stop_event(
    pool: &SqlitePool,
    snap: &chimpflix_transcoder::SessionSnapshot,
) {
    let (item_id, episode_id) =
        chimpflix_library::queries::media_file_owner(pool, snap.media_file_id)
            .await
            .unwrap_or((None, None));
    let ev = chimpflix_library::queries::PlaybackEventInput {
        item_id,
        episode_id,
        media_file_id: Some(snap.media_file_id),
        duration_ms: snap.duration_ms,
        decision: Some("transcode"),
        bytes_sent: Some(snap.bytes_served),
        session_token: Some(snap.id.as_str()),
        ..chimpflix_library::queries::PlaybackEventInput::new(snap.user_id, "stop")
    };
    if let Err(e) = chimpflix_library::queries::record_playback_event(pool, ev).await {
        warn!(
            session_id = %snap.id,
            error = %format!("{e:#}"),
            "record playback stop event",
        );
    }
}
