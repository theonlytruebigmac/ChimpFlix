//! Application state shared across handlers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chimpflix_common::Vault;
use chimpflix_library::ServerSettings;
use chimpflix_metadata::{
    AniListClient, MalClient, OmdbClient, OpenSubtitlesClient, PlexOAuthClient, TmdbClient,
    TraktClient, TvMazeClient, TvdbClient,
};
use chimpflix_transcoder::{FfmpegConfig, SharedCapabilities, TranscodeManager};
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

/// Hot-swappable OMDb client. `None` until the operator stores an
/// OMDb API key in the vault (`omdb` slot). Consumed by the
/// `fetch_external_ratings` per-item handler.
pub type OmdbHandle = Arc<RwLock<Option<OmdbClient>>>;

/// Hot-swappable MyAnimeList client (anime ranking, X-MAL-CLIENT-ID).
/// `None` until the operator stores a `mal` client id in the vault.
pub type MalHandle = Arc<RwLock<Option<MalClient>>>;

/// Hot-swappable Plex OAuth client. Built lazily from the per-install
/// client identifier stored on `server_settings`; the `/auth/plex/start`
/// endpoint ensures one is constructed on first use. Cleared by the
/// admin "rotate identifier" action
/// (`POST /admin/plex/rotate-identifier`) so future PINs are issued
/// against a freshly-minted client identity.
pub type PlexOAuthHandle = Arc<RwLock<Option<PlexOAuthClient>>>;

/// In-memory pending-PIN store. Plex returns a numeric PIN id we have
/// to poll until the user approves; we keep an opaque handle around
/// instead of returning the raw id to the browser. The handle expires
/// alongside the underlying PIN (Plex defaults to 30 minutes for
/// `strong=true` PINs).
///
/// One handle ⇒ one intent. The `intent` field tells the poll handler
/// which side-effect to apply when the PIN flips to `Ready`:
///
///   * `Login`  — look up the linked ChimpFlix user, issue a session.
///   * `Signup` — validate the invite, create a user + provider link,
///     consume the invite, issue a session.
///   * `Link`   — attach the Plex identity to an already-signed-in
///     user's account. No session is issued (the existing
///     one is preserved).
pub type PlexPinCache = Arc<tokio::sync::Mutex<std::collections::HashMap<String, PendingPlexPin>>>;

#[derive(Debug, Clone)]
pub struct PendingPlexPin {
    pub plex_pin_id: i64,
    pub intent: PlexPinIntent,
    pub expires_at: std::time::Instant,
}

#[derive(Debug, Clone)]
pub enum PlexPinIntent {
    Login,
    /// Bound to a specific invite code. Only PINs created with this
    /// intent can consume the invite — a stale Login handle can't be
    /// upgraded to signup mid-flight.
    Signup { invite_code_hash: String },
    /// Attach the resulting Plex identity to this user_id. The handler
    /// re-verifies the session at poll time so a logout between start
    /// and poll cleanly rejects rather than linking under the wrong
    /// account.
    Link { user_id: i64 },
}

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
    /// OMDb client for the `fetch_external_ratings` per-item handler.
    /// `None` until an OMDb API key is stored in the credential vault.
    pub omdb: OmdbHandle,
    /// MyAnimeList client for the anime "Top 10" ranking. `None` until a
    /// `mal` client id is stored in the credential vault — anime libraries
    /// fall back to local top-watched while unset.
    pub mal: MalHandle,
    /// Plex OAuth client. Lazily constructed from
    /// `server_settings.plex_client_identifier` on the first
    /// `/auth/plex/start` call.
    pub plex_oauth: PlexOAuthHandle,
    /// Server-side cache of in-flight Plex PIN handles. Keyed by an
    /// opaque random token the frontend polls on; never exposes the
    /// raw Plex PIN id to the browser.
    pub plex_pin_cache: PlexPinCache,
    /// TVMaze fallback provider for shows. Always constructed (no key
    /// required); `None` only if HTTP client init fails.
    pub tvmaze: Option<TvMazeClient>,
    pub hub: Hub,
    /// Per-provider circuit breakers for external metadata APIs. Job
    /// handlers wrap their TMDB/OMDb/Trakt/… calls in
    /// `circuit_breakers.<provider>.run(...)` so a rate-limited provider
    /// fails fast for all in-flight jobs instead of each burning a worker
    /// slot rediscovering the outage. Read-only from handlers.
    pub circuit_breakers: std::sync::Arc<crate::circuit_breaker::CircuitBreakers>,
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
    /// Detected ffmpeg hardware accelerators + encoders. Probed at
    /// startup and refreshable at runtime via the admin "re-probe"
    /// endpoint (POST /admin/transcoder/capabilities/reprobe). The admin
    /// UI uses this to grey out options the host can't run. This is the
    /// *same* `Arc<SharedCapabilities>` handle held inside
    /// `transcoder` (the `TranscodeManager`), so a re-probe `store`
    /// here is observed by both the admin GET and the live encoder-
    /// selection path. Readers call `.load()` for a lock-free snapshot.
    pub transcoder_caps: Arc<SharedCapabilities>,
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
    /// Per-user open WebSocket count. Used by the WS upgrade handler
    /// to enforce the per-user connection cap (MONTH 1 in
    /// `docs/PUBLIC_RELEASE_HARDENING.md`). A misbehaving or
    /// compromised client can otherwise open hundreds of WS
    /// connections; each one fans out events from the same broadcast
    /// channel. Reads are O(1) under the lock; the lock is held only
    /// briefly during upgrade/teardown.
    pub ws_connections_per_user: Arc<RwLock<HashMap<i64, u32>>>,
    /// Per-route HTTP request counters surfaced via `/metrics`.
    /// Recorded by `crate::api::http_metrics::track` (an outer
    /// router middleware) and read by the Prometheus exporter.
    pub http_metrics: crate::api::http_metrics::HttpMetricsRegistry,
    /// Per-user Trakt token-refresh serialization. Each entry is a
    /// dedicated `Mutex<()>` held for the lifetime of one user's
    /// refresh-and-upsert sequence so concurrent server tasks
    /// (push_history hook + scheduled trakt_pull + manual UI ping)
    /// don't all hit `/oauth/token` simultaneously when the access
    /// token is about to expire — that produces duplicate refreshes,
    /// each minting a new token pair, and a last-writer-wins upsert
    /// that can lose a valid refresh_token.
    pub trakt_refresh_locks: Arc<RwLock<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>>,
    /// Live, in-memory per-kind counters and recent-run ring
    /// buffer. Used by the admin activity screen to render
    /// "what's happening right now" without hitting SQLite on
    /// every 5s poll. Reset on restart — historical data lives
    /// in `task_kind_metrics_daily` instead.
    pub task_metrics: crate::tasks::metrics::LiveMetrics,
    /// Handle to the job-queue worker pool. Populated by main()
    /// after `jobs::start()` returns. Wrapped in `Option` because
    /// AppState is constructed before the pool is started (the
    /// pool needs `state` to dispatch handlers) and wrapped in
    /// RwLock so the settings PATCH handler can read it without
    /// mutating AppState. `None` only between construction and
    /// `jobs::start()` — a hard error elsewhere would still log
    /// and continue rather than panic.
    pub worker_pool: Arc<RwLock<Option<crate::jobs::WorkerPoolHandle>>>,
    /// Per-job live progress store. Workers insert an entry on job
    /// claim and remove it on completion; handlers update it via
    /// the `JobContext::current().progress_sink` task-local. The
    /// admin activity-feed endpoint reads snapshots to render
    /// "Decoding · 42%" inline for in-flight jobs.
    pub job_progress: Arc<crate::jobs::progress::JobProgressStore>,
    /// Library-first-scan exclusivity gate. Use
    /// [`Self::library_scan_exclusive`] for `acquire`/`release` calls
    /// from inside the scan trigger; consumers (worker pool,
    /// scheduler) subscribe via [`crate::jobs::scan_gate::LibraryScanGate::subscribe`].
    ///
    /// While at least one first-scan is in progress:
    ///   * The job worker pool's claim loop awaits a clear before
    ///     claiming new jobs (lets in-flight ones finish).
    ///   * The scheduler tick defers periodic-task dispatch.
    ///
    /// Cleared when the last in-progress first-scan completes
    /// (success or failure). Counter semantics correctly handle the
    /// case where the operator adds two new libraries back-to-back
    /// — the gate stays active until BOTH scans drain.
    pub library_scan_exclusive: Arc<crate::jobs::scan_gate::LibraryScanGate>,

    /// Application-level mutex around operations known to do many
    /// sequential writes (operator-initiated backfill sweep, library
    /// delete cascade, etc.). Held briefly during the write phase to
    /// prevent two such operations from racing each other into the
    /// SQLite writer slot — they'd both succeed eventually thanks to
    /// the retry helper, but holding the mutex up front means we
    /// don't burn retry budget against ourselves.
    ///
    /// Per-job worker writes (markers, loudness, scan inserts) do NOT
    /// take this lock — those are inherently parallel and rely on
    /// `BEGIN IMMEDIATE` + `with_busy_retry` for serialization at the
    /// SQLite layer. The bulk lock is for *operator-triggered* bulk
    /// operations that would otherwise blast thousands of writes into
    /// the queue while regular traffic is also writing.
    pub bulk_write_lock: Arc<tokio::sync::Semaphore>,

    /// Set of `optimized_versions.id`s the operator has requested be
    /// cancelled. The cancel route flips the DB row to `cancelled` and
    /// inserts the id here; the `optimize_versions` worker polls this
    /// set between ffmpeg progress reads and, when it sees its own row,
    /// kills the in-flight ffmpeg child + removes the partial output.
    /// Entries are removed by the worker once it has acted on them (or
    /// when it finishes a row, defensively). Queued-only cancels never
    /// reach the worker — the claim query skips non-`queued` rows — so
    /// this set only ever matters for the running row(s) of the current
    /// batch. Process-local; a restart drops the set, which is fine
    /// because a restart also kills every in-flight ffmpeg child.
    pub optimize_cancels: Arc<RwLock<HashSet<i64>>>,
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

    pub async fn omdb_snapshot(&self) -> Option<OmdbClient> {
        self.omdb.read().await.clone()
    }

    pub async fn set_omdb(&self, client: Option<OmdbClient>) {
        *self.omdb.write().await = client;
    }

    pub async fn mal_snapshot(&self) -> Option<MalClient> {
        self.mal.read().await.clone()
    }

    pub async fn set_mal(&self, client: Option<MalClient>) {
        *self.mal.write().await = client;
    }

    /// Return the live `PlexOAuthClient`, building it from the
    /// persisted `plex_client_identifier` on first use. Subsequent
    /// calls return the cached client.
    pub async fn plex_oauth(&self) -> anyhow::Result<PlexOAuthClient> {
        if let Some(client) = self.plex_oauth.read().await.clone() {
            return Ok(client);
        }
        // Cold path: generate-or-load the identifier and build the
        // client under the write lock. The double-check inside the
        // write lock guards against two callers crossing the same
        // boundary at the same time.
        let identifier =
            chimpflix_library::queries::ensure_plex_client_identifier(&self.pool).await?;
        let mut guard = self.plex_oauth.write().await;
        if let Some(existing) = guard.clone() {
            return Ok(existing);
        }
        let client = PlexOAuthClient::new(&identifier)?;
        *guard = Some(client.clone());
        Ok(client)
    }

    /// Stash a freshly-issued PIN under an opaque handle. The handle
    /// is what we hand back to the browser; the underlying Plex PIN id
    /// stays server-side so a hostile script can't poll someone else's
    /// in-flight authorization.
    pub async fn plex_pin_remember(&self, handle: String, pending: PendingPlexPin) {
        let mut guard = self.plex_pin_cache.lock().await;
        // Opportunistic GC — entries expire on `expires_at`, so a
        // sweep at every insert keeps the map small without needing a
        // background task.
        let now = std::time::Instant::now();
        guard.retain(|_, p| p.expires_at > now);
        guard.insert(handle, pending);
    }

    /// Lookup a pending PIN by its opaque handle. Returns the entry
    /// (cloned so the lock isn't held across the network call) or
    /// `None` if the handle is unknown / expired.
    pub async fn plex_pin_lookup(&self, handle: &str) -> Option<PendingPlexPin> {
        let guard = self.plex_pin_cache.lock().await;
        let entry = guard.get(handle)?;
        if entry.expires_at <= std::time::Instant::now() {
            return None;
        }
        Some(entry.clone())
    }

    /// Drop a PIN handle from the cache (called after a terminal
    /// outcome — Ready, Expired, or hard error).
    pub async fn plex_pin_forget(&self, handle: &str) {
        self.plex_pin_cache.lock().await.remove(handle);
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

    /// Try to claim a WebSocket connection slot for `user_id`. Returns
    /// `true` when under the per-user cap and the slot was claimed;
    /// `false` when the user already has `cap` connections open. The
    /// caller MUST pair a successful `true` return with a later call
    /// to [`release_ws_connection`] (RAII guard recommended) or the
    /// per-user count leaks. See MONTH 1 #1 in
    /// `docs/PUBLIC_RELEASE_HARDENING.md`.
    pub async fn try_acquire_ws_connection(&self, user_id: i64, cap: u32) -> bool {
        let mut guard = self.ws_connections_per_user.write().await;
        let entry = guard.entry(user_id).or_insert(0);
        if *entry >= cap {
            return false;
        }
        *entry += 1;
        true
    }

    /// Release a previously acquired WebSocket slot. Idempotent — a
    /// release without a matching acquire (e.g. after a panic that
    /// bypassed the RAII guard) decrements toward zero rather than
    /// underflowing.
    pub async fn release_ws_connection(&self, user_id: i64) {
        let mut guard = self.ws_connections_per_user.write().await;
        if let Some(n) = guard.get_mut(&user_id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                guard.remove(&user_id);
            }
        }
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

    /// Request cancellation of a running optimized-version re-encode.
    /// Inserts the id so the worker's poll loop sees it. Idempotent.
    pub async fn request_optimize_cancel(&self, id: i64) {
        self.optimize_cancels.write().await.insert(id);
    }

    /// True if the operator has requested cancellation of this
    /// optimized-version id. Polled by the worker between ffmpeg
    /// progress reads. A read-lock check kept off the hot path (called
    /// a handful of times per second, not per frame).
    pub async fn optimize_cancel_requested(&self, id: i64) -> bool {
        self.optimize_cancels.read().await.contains(&id)
    }

    /// Drop a cancel request once the worker has acted on it (killed the
    /// child) or finished the row. Keeps the set from accreting stale
    /// ids across batches. Idempotent.
    pub async fn clear_optimize_cancel(&self, id: i64) {
        self.optimize_cancels.write().await.remove(&id);
    }
}
