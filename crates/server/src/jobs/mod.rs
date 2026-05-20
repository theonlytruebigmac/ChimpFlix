//! Durable background job queue.
//!
//! Backed by the `jobs` table (see migration `phase57_jobs_queue.sql`).
//! Workers poll `claim_next_job`, dispatch via [`JobRouter`] to the
//! per-kind handler registered at boot, then call `mark_job_succeeded`
//! or `mark_job_failed` based on the result. On startup the server
//! runs `reclaim_orphan_jobs` to recover anything left as `running`
//! after a previous crash.
//!
//! ## Adding a new job kind
//!
//! 1. Pick a `kind` string (`detect_markers_file`, `library_refresh`,
//!    etc.) — keep it stable; the value is persisted.
//! 2. Write an `async fn handle(state: AppState, payload: Value) ->
//!    Result<()>` in a new submodule under `handlers/`.
//! 3. Register it in [`build_router`] below.
//! 4. From the request handler, call
//!    `chimpflix_library::queries::enqueue_job(pool, JobInput::new(...))`
//!    instead of `tokio::spawn`.
//!
//! ## Concurrency
//!
//! The pool runs `n_workers` tokio tasks (default 1). They all poll
//! the same table, so SQLite's serialized writes guarantee at-most-
//! once delivery per claim. Two workers might dispatch *different*
//! jobs concurrently — fine for IO-bound work, but if a kind pegs a
//! CPU core (ffmpeg) you'll want either a per-kind semaphore or to
//! keep `n_workers = 1`.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chimpflix_library::queries::{
    claim_next_job_excluding_kinds, mark_job_dead, mark_job_failed, mark_job_succeeded,
    reclaim_orphan_jobs, touch_job_lease,
};
use serde_json::Value;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use crate::state::AppState;

pub mod handlers;
pub mod pipeline;

/// Default lease ttl. A job whose worker hasn't updated `locked_at`
/// in this long is treated as orphaned on the next startup reclaim.
/// Long enough to cover the worst-case ffmpeg detect_markers run on
/// a multi-hour bluray rip; short enough that a real crash is
/// recovered within an hour of the next start.
pub const DEFAULT_LEASE_TTL_MS: i64 = 60 * 60 * 1000;

/// How long the worker sleeps between empty-poll cycles. Short
/// enough that an enqueue feels responsive; long enough that an
/// idle server isn't waking up 10x/sec for nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Heartbeat cadence — long enough not to spam the DB, short
/// enough that a job running close to the lease TTL gets its
/// timestamp refreshed comfortably before reclaim would consider
/// it orphaned. Default lease is 1h; refreshing every 5 minutes
/// gives 12 retries before a lease ever risks expiring on a live
/// worker.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Per-kind concurrency limits. The worker pool's global limit
/// (number of tokio worker tasks) caps total concurrency, but
/// these per-kind caps prevent CPU-bound kinds from monopolizing
/// the pool when a backlog of one kind sits at the head of the
/// queue.
///
/// Rationale per kind:
///   - `detect_markers_file` (CPU/ffmpeg-bound, slow ~30-60s)
///     → 1: one blackdetect at a time avoids encoder thrash with
///     live transcodes.
///   - `generate_preview_sprite` (CPU/ffmpeg-bound, slow)
///     → 1: same.
///   - `build_chapter_thumbs` (CPU/ffmpeg-bound, medium)
///     → 1: same.
///   - `analyze_loudness` (CPU/ffmpeg-bound, fast ~5-10s)
///     → 1: same; loudnorm can spike CPU briefly but is short.
///   - `fetch_subtitles_item` (network-bound, latency-dominated)
///     → 4: parallelizes fine; OpenSubtitles rate limit is generous.
///
/// With 2 workers + these limits, a typical backlog scenario where
/// the queue has 1000 detect_markers + 500 subtitle jobs ends up
/// running 1 detect + N subtitles concurrently (instead of 2
/// detects competing for ffmpeg). New kinds default to 1 unless
/// added here.
const KIND_LIMITS: &[(&str, usize)] = &[
    (handlers::detect_markers_file::KIND, 1),
    (handlers::generate_preview_sprite::KIND, 1),
    (handlers::build_chapter_thumbs::KIND, 1),
    (handlers::analyze_loudness::KIND, 1),
    (handlers::fetch_subtitles_item::KIND, 4),
];

/// Build the per-kind semaphore map. Unknown kinds (not listed in
/// `KIND_LIMITS`) get a permit count equal to `default_limit` —
/// effectively unbounded by default so future kinds work without
/// touching this file.
fn build_kind_semaphores(default_limit: usize) -> HashMap<String, Arc<Semaphore>> {
    let mut map = HashMap::new();
    for (kind, limit) in KIND_LIMITS {
        map.insert((*kind).to_string(), Arc::new(Semaphore::new(*limit)));
    }
    // A wildcard fallback isn't possible (we don't know future
    // kinds in advance), so unknown kinds will use `default_limit`
    // via the get-or-insert path at claim time.
    let _ = default_limit;
    map
}

#[derive(Clone)]
struct KindLimiter {
    semaphores: Arc<tokio::sync::RwLock<HashMap<String, Arc<Semaphore>>>>,
    default_limit: usize,
}

impl KindLimiter {
    fn new(default_limit: usize) -> Self {
        Self {
            semaphores: Arc::new(tokio::sync::RwLock::new(build_kind_semaphores(
                default_limit,
            ))),
            default_limit,
        }
    }

    /// Snapshot the set of kinds whose permits are fully exhausted
    /// right now. Used to filter the claim query so the worker
    /// doesn't grab a row it can't run.
    async fn saturated_kinds(&self) -> Vec<String> {
        let semaphores = self.semaphores.read().await;
        semaphores
            .iter()
            .filter(|(_, sem)| sem.available_permits() == 0)
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Acquire a permit for `kind`. Creates the semaphore on first
    /// use for kinds not declared in `KIND_LIMITS`.
    async fn acquire(&self, kind: &str) -> tokio::sync::OwnedSemaphorePermit {
        // Fast path: read lock, look up existing semaphore.
        {
            let semaphores = self.semaphores.read().await;
            if let Some(sem) = semaphores.get(kind) {
                let sem = sem.clone();
                drop(semaphores);
                return sem.acquire_owned().await.expect("semaphore closed");
            }
        }
        // Slow path: insert a default-limit semaphore for an
        // unknown kind. Done under write lock so a concurrent
        // worker doesn't double-insert.
        let mut semaphores = self.semaphores.write().await;
        let sem = semaphores
            .entry(kind.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(self.default_limit)))
            .clone();
        drop(semaphores);
        sem.acquire_owned().await.expect("semaphore closed")
    }
}

/// Backoff applied to the next retry after a handler returns Err.
/// Exponential up to a cap, computed from the existing `attempts`
/// counter (which `claim_next_job` already incremented).
fn backoff_ms(attempts: i64) -> i64 {
    // attempts=1 → 1s, 2 → 4s, 3 → 16s, 4 → 64s, capped at 5min.
    let base = 1_000i64;
    let pow = 4i64.saturating_pow(attempts.saturating_sub(1).clamp(0, 8) as u32);
    base.saturating_mul(pow).min(5 * 60 * 1000)
}

/// A boxed async handler. Closures-returning-futures (rather than
/// the `async-trait` macro) so we don't need an extra dep — the
/// allocator overhead per dispatch is irrelevant compared to the
/// ffmpeg work the handlers do.
pub type JobFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
pub type JobHandler = Arc<dyn Fn(AppState, Value) -> JobFuture + Send + Sync>;

#[derive(Clone, Default)]
pub struct JobRouter {
    handlers: Arc<HashMap<String, JobHandler>>,
}

impl JobRouter {
    pub fn builder() -> JobRouterBuilder {
        JobRouterBuilder {
            inner: HashMap::new(),
        }
    }

    fn lookup(&self, kind: &str) -> Option<JobHandler> {
        self.handlers.get(kind).cloned()
    }
}

pub struct JobRouterBuilder {
    inner: HashMap<String, JobHandler>,
}

impl JobRouterBuilder {
    /// Register a handler for a kind. `f` is an `async fn` (or
    /// equivalent) — the macro-free way to wrap it is
    /// `|state, payload| Box::pin(my_handler_fn(state, payload))`.
    pub fn register<F, Fut>(mut self, kind: &str, f: F) -> Self
    where
        F: Fn(AppState, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let h: JobHandler = Arc::new(move |s, p| Box::pin(f(s, p)));
        self.inner.insert(kind.to_string(), h);
        self
    }

    pub fn build(self) -> JobRouter {
        JobRouter {
            handlers: Arc::new(self.inner),
        }
    }
}

/// Construct the production router with every registered handler.
/// New job kinds get added here so all start-up wiring stays in one
/// place.
pub fn build_router() -> JobRouter {
    JobRouter::builder()
        // Discovery-pipeline per-file handlers. The scanner enqueues
        // one of each per new media_file row.
        .register(handlers::detect_markers_file::KIND, |s, p| {
            handlers::detect_markers_file::run(s, p)
        })
        .register(handlers::generate_preview_sprite::KIND, |s, p| {
            handlers::generate_preview_sprite::run(s, p)
        })
        .register(handlers::analyze_loudness::KIND, |s, p| {
            handlers::analyze_loudness::run(s, p)
        })
        .register(handlers::build_chapter_thumbs::KIND, |s, p| {
            handlers::build_chapter_thumbs::run(s, p)
        })
        .register(handlers::fetch_subtitles_item::KIND, |s, p| {
            handlers::fetch_subtitles_item::run(s, p)
        })
        .build()
}

/// Reclaim orphans, then spawn `n_workers` polling loops. Workers
/// run for the process lifetime — there's no explicit shutdown hook
/// since process exit drops the tokio runtime which cancels them.
pub async fn start(state: AppState, router: JobRouter, n_workers: usize) {
    match reclaim_orphan_jobs(&state.pool, DEFAULT_LEASE_TTL_MS).await {
        Ok(0) => {}
        Ok(n) => info!(count = n, "reclaimed orphan jobs from previous run"),
        Err(e) => warn!(error = %format!("{e:#}"), "reclaim_orphan_jobs failed"),
    }
    let n = n_workers.max(1);
    // Limiter is shared across workers so a permit acquired by one
    // worker blocks others from claiming the same kind. Unknown
    // kinds default to `n_workers` permits — effectively unbounded
    // among the running workers — so adding a new kind doesn't
    // require touching `KIND_LIMITS`.
    let limiter = KindLimiter::new(n.max(1));
    for worker_id in 0..n {
        let state = state.clone();
        let router = router.clone();
        let limiter = limiter.clone();
        tokio::spawn(async move { worker_loop(worker_id, state, router, limiter).await });
    }
    info!(workers = n, "job queue workers spawned");
}

async fn worker_loop(
    worker_id: usize,
    state: AppState,
    router: JobRouter,
    limiter: KindLimiter,
) {
    loop {
        let saturated = limiter.saturated_kinds().await;
        let claim = claim_next_job_excluding_kinds(&state.pool, &saturated).await;
        match claim {
            Ok(Some(job)) => {
                let Some(handler) = router.lookup(&job.kind) else {
                    warn!(
                        worker = worker_id,
                        job_id = job.id,
                        kind = %job.kind,
                        "no handler registered; marking dead",
                    );
                    // Skip the retry path entirely — no handler will
                    // ever exist for this kind in this process, so
                    // burning attempts is pure churn. Go straight to
                    // terminal `dead`.
                    let _ = mark_job_dead(
                        &state.pool,
                        job.id,
                        "no handler registered for kind",
                    )
                    .await;
                    continue;
                };
                let payload: Value =
                    serde_json::from_str(&job.payload).unwrap_or(Value::Null);
                let kind = job.kind.clone();

                // Hold a per-kind concurrency permit for the
                // lifetime of the handler. The claim query
                // already filtered out saturated kinds, so this
                // `acquire` returns immediately in the common
                // case — but the `await` is correct: between
                // saturated_kinds() and acquire(), another worker
                // could have claimed the last permit. The await
                // then briefly blocks that worker, which is fine
                // — short-lived contention, no starvation.
                let _kind_permit = limiter.acquire(&kind).await;

                // Spawn a heartbeat task that refreshes `locked_at`
                // while the handler runs. Without this, a handler
                // that takes longer than the lease TTL would be
                // reclaimed mid-flight if the server restarted —
                // and a single chromaprint extract on a multi-hour
                // bluray can sit comfortably under the TTL but a
                // batched discover-pipeline tick could push close.
                // Cheap (one UPDATE per 5 min).
                let heartbeat_state = state.clone();
                let heartbeat_id = job.id;
                let heartbeat = tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(HEARTBEAT_INTERVAL).await;
                        if let Err(e) =
                            touch_job_lease(&heartbeat_state.pool, heartbeat_id).await
                        {
                            warn!(
                                job_id = heartbeat_id,
                                error = %format!("{e:#}"),
                                "heartbeat failed",
                            );
                        }
                    }
                });
                let result = handler(state.clone(), payload).await;
                heartbeat.abort();
                match result {
                    Ok(()) => {
                        if let Err(e) =
                            mark_job_succeeded(&state.pool, job.id).await
                        {
                            error!(
                                worker = worker_id,
                                job_id = job.id,
                                error = %format!("{e:#}"),
                                "mark_job_succeeded failed",
                            );
                        }
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        let backoff = backoff_ms(job.attempts);
                        warn!(
                            worker = worker_id,
                            job_id = job.id,
                            kind = %kind,
                            attempts = job.attempts,
                            backoff_ms = backoff,
                            error = %msg,
                            "job handler returned error",
                        );
                        if let Err(e) =
                            mark_job_failed(&state.pool, job.id, &msg, backoff)
                                .await
                        {
                            error!(
                                worker = worker_id,
                                job_id = job.id,
                                error = %format!("{e:#}"),
                                "mark_job_failed write failed",
                            );
                        }
                    }
                }
            }
            Ok(None) => {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            Err(e) => {
                warn!(
                    worker = worker_id,
                    error = %format!("{e:#}"),
                    "claim_next_job failed; backing off",
                );
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}
