//! Application state shared across handlers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chimpflix_common::Vault;
use chimpflix_library::ServerSettings;
use chimpflix_metadata::{
    AniListClient, OpenSubtitlesClient, TmdbClient, TraktClient, TvMazeClient, TvdbClient,
};
use chimpflix_transcoder::{FfmpegConfig, TranscodeManager, TranscoderCapabilities};
use ipnet::IpNet;
use sqlx::SqlitePool;
use tokio::sync::RwLock;

use crate::auth::AuthConfig;
use crate::events::Hub;

/// Hot-reloadable cache over the `server_settings` singleton row. Every
/// admin `PATCH` to settings updates this in-place so subscribers (the
/// transcoder, CORS layer, network layer, scheduler) can re-read without
/// the round-trip to SQLite.
pub type SettingsCache = Arc<RwLock<ServerSettings>>;

/// Hot-swappable TMDB client. Admin handlers replace the inner `Option`
/// whenever the TMDB credential is set or cleared, so scanners and
/// item-level callers see the new value on their next snapshot.
pub type TmdbHandle = Arc<RwLock<Option<TmdbClient>>>;

/// Hot-swappable TVDB client. Same semantics as [`TmdbHandle`] — set via
/// the credential vault, swapped in by the admin handler on save.
pub type TvdbHandle = Arc<RwLock<Option<TvdbClient>>>;

/// Hot-swappable AniList client. AniList works without a token (lower
/// rate limit) but the admin vault entry lets the operator upgrade to
/// authenticated traffic; either way the handle holds the most recent
/// build so the anime scanner path always sees the current client.
pub type AniListHandle = Arc<RwLock<Option<AniListClient>>>;

/// Hot-swappable OpenSubtitles client. `None` until the operator stores
/// the credential triple (api_key + username + password) in the vault.
pub type OpenSubtitlesHandle = Arc<RwLock<Option<OpenSubtitlesClient>>>;

/// Hot-swappable Trakt app client (server-wide; built from the vault's
/// client_id + client_secret). Per-user access tokens live in the
/// `user_trakt_tokens` table; the client itself is just the OAuth app
/// identity used to mint and refresh those tokens.
pub type TraktHandle = Arc<RwLock<Option<TraktClient>>>;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub ffmpeg: FfmpegConfig,
    pub tmdb: TmdbHandle,
    /// TVDB v4 backfill provider for shows and movies. `None` until the
    /// owner saves a credential under the `tvdb` slot.
    pub tvdb: TvdbHandle,
    /// AniList GraphQL client — primary for anime libraries. Always
    /// constructed (works without auth); `None` only if HTTP client init
    /// failed at boot.
    pub anilist: AniListHandle,
    /// OpenSubtitles client; the fetch_subtitles scheduled task pulls
    /// from this and writes results into the `external_subtitles` table.
    pub opensubtitles: OpenSubtitlesHandle,
    /// Trakt OAuth app client. Per-user tokens are read from the
    /// `user_trakt_tokens` table at request time and combined with this
    /// client to make scoped API calls.
    pub trakt: TraktHandle,
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
    /// Credential vault — owns the master key and the encrypt/decrypt
    /// primitives. The on-disk `secrets` table is the persistence layer.
    pub vault: Arc<Vault>,
    /// Per-username login-attempt tracker. Records failures + lockouts
    /// in-process. Process-local; horizontal scaling would lift this
    /// into a shared store.
    pub login_attempts: crate::api::rate_limit::AttemptTracker,
    /// Per-email throttle for password-reset requests. Independent of
    /// the per-IP limiter; together they defeat distributed email-
    /// bombing of any single inbox.
    pub reset_email_limiter: Arc<crate::api::rate_limit::StringLimiter>,
    /// Per-(user_id, item_id) throttle for `POST /items/{id}/report-issue`.
    /// Each report emails every admin and writes one notification row
    /// per admin, so unthrottled it's an amplification primitive.
    pub report_issue_limiter: Arc<crate::api::rate_limit::StringLimiter>,
    /// Operator-declared list of trusted upstream proxies (Traefik,
    /// Cloudflare ranges, Docker bridge). The client-IP middleware
    /// honours `X-Forwarded-For`/`CF-Connecting-IP` only when the
    /// immediate peer's socket address falls inside one of these CIDRs;
    /// otherwise the peer IP is used verbatim. Empty (default) =
    /// ignore proxy headers entirely. Sourced from the
    /// `TRUSTED_PROXIES` env var at startup.
    pub trusted_proxies: Arc<Vec<IpNet>>,
    /// Per-library scan mutex. Tracks which `library_id`s are currently
    /// being scanned (by the scheduled `scan_library` task, a manual
    /// admin trigger, or the filesystem watcher). Each pathway acquires
    /// the lock via `try_acquire_library_scan` before spawning ffmpeg
    /// / IO-heavy work and releases it via `release_library_scan` when
    /// done. Without this, the three pathways can run concurrently on
    /// the same library and saturate the disk that's also serving live
    /// transcode segments — the dominant cause of "smooth at 7pm, skips
    /// during the 2am maintenance window" reports.
    pub library_scans_in_progress: Arc<RwLock<HashSet<i64>>>,
    /// Per-user Trakt token-refresh serialization. Each entry is a
    /// dedicated `Mutex<()>` held for the lifetime of one user's
    /// refresh-and-upsert sequence so concurrent server tasks
    /// (push_history hook + scheduled trakt_pull + manual UI ping)
    /// don't all hit `/oauth/token` simultaneously when the access
    /// token is about to expire — that produces duplicate refreshes,
    /// each minting a new token pair, and a last-writer-wins upsert
    /// that can lose a valid refresh_token.
    pub trakt_refresh_locks:
        Arc<RwLock<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>>,
}

impl AppState {
    /// Cheap clone of the current TMDB client, if any. Holds the read
    /// lock only for the clone itself, never across an `await`.
    pub async fn tmdb_snapshot(&self) -> Option<TmdbClient> {
        self.tmdb.read().await.clone()
    }

    /// Replace the TMDB client. Used by the credential-vault admin
    /// handler after a successful `set` or `delete`.
    pub async fn set_tmdb(&self, client: Option<TmdbClient>) {
        *self.tmdb.write().await = client;
    }

    pub async fn tvdb_snapshot(&self) -> Option<TvdbClient> {
        self.tvdb.read().await.clone()
    }

    pub async fn set_tvdb(&self, client: Option<TvdbClient>) {
        *self.tvdb.write().await = client;
    }

    pub async fn anilist_snapshot(&self) -> Option<AniListClient> {
        self.anilist.read().await.clone()
    }

    pub async fn set_anilist(&self, client: Option<AniListClient>) {
        *self.anilist.write().await = client;
    }

    pub async fn opensubtitles_snapshot(&self) -> Option<OpenSubtitlesClient> {
        self.opensubtitles.read().await.clone()
    }

    pub async fn set_opensubtitles(&self, client: Option<OpenSubtitlesClient>) {
        *self.opensubtitles.write().await = client;
    }

    pub async fn trakt_snapshot(&self) -> Option<TraktClient> {
        self.trakt.read().await.clone()
    }

    pub async fn set_trakt(&self, client: Option<TraktClient>) {
        *self.trakt.write().await = client;
    }

    /// Try to acquire the scan lock for `library_id`. Returns true if
    /// the caller now holds the lock and should proceed; false if a
    /// scan is already in progress and the caller should bail out
    /// cleanly. Pair with [`release_library_scan`] in the same task.
    /// Spawn-and-forget call sites should install a local RAII guard
    /// that re-spawns the release on `Drop` so a panic inside the
    /// scanner doesn't leak the entry (see `scheduler::run_task`).
    pub async fn try_acquire_library_scan(&self, library_id: i64) -> bool {
        let mut guard = self.library_scans_in_progress.write().await;
        guard.insert(library_id)
    }

    /// Release the scan lock previously taken by
    /// [`try_acquire_library_scan`]. Idempotent: a release without a
    /// matching acquire is a no-op.
    pub async fn release_library_scan(&self, library_id: i64) {
        self.library_scans_in_progress
            .write()
            .await
            .remove(&library_id);
    }

    /// Get-or-insert the per-user Trakt-refresh mutex. Callers hold
    /// the returned mutex across their refresh+upsert sequence; see
    /// [`trakt_refresh_locks`][AppState::trakt_refresh_locks] for the
    /// race that motivates this.
    pub async fn trakt_refresh_lock(&self, user_id: i64) -> Arc<tokio::sync::Mutex<()>> {
        // Fast path under the read lock for the common case (lock
        // already exists).
        {
            let guard = self.trakt_refresh_locks.read().await;
            if let Some(m) = guard.get(&user_id) {
                return m.clone();
            }
        }
        // Cold path: take the write lock and re-check (another
        // caller may have inserted between our read and write).
        let mut guard = self.trakt_refresh_locks.write().await;
        guard
            .entry(user_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }
}
