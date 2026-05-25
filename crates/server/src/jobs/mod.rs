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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use chimpflix_library::queries::{
    claim_next_job_excluding_kinds, mark_job_dead, mark_job_failed_with_class, mark_job_succeeded,
    reclaim_orphan_jobs, touch_job_lease,
};

use self::error_class::{backoff_for_class, classify};
use serde_json::Value;
use tokio::sync::{Semaphore, watch};
use tracing::{error, info, warn};

use crate::state::AppState;

pub mod error_class;
pub mod handlers;
pub mod pipeline;
pub mod progress;
pub mod scan_gate;

// Note: the old DEFAULT_LEASE_TTL_MS constant was removed when boot
// reclaim moved to `lease_ttl=0` (every `running` row at boot is by
// definition orphaned). The lease-touch heartbeat below uses its own
// fixed cadence and doesn't need a shared TTL constant.

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

/// Build the per-kind semaphore map from the tasks registry.
/// Unknown kinds (not in the registry) get a permit count equal to
/// `default_limit` at acquire-time — see [`KindLimiter::acquire`].
///
/// Single source of truth is now [`crate::tasks::registry`]; adding
/// a new kind there picks up the cap here automatically. With 2
/// workers + per-kind caps a typical backlog like 1000 marker jobs
/// and 500 subtitle jobs ends up running 1 marker + 4 subtitles in
/// flight instead of dogpiling either kind.
fn build_kind_semaphores(_default_limit: usize) -> HashMap<String, Arc<Semaphore>> {
    let mut map = HashMap::new();
    for k in crate::tasks::registry::all_kinds() {
        map.insert(
            k.job_kind.to_string(),
            Arc::new(Semaphore::new(k.concurrency as usize)),
        );
    }
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

    /// Swap the semaphore for `kind` to one with `new_cap` permits.
    /// Live jobs continue to hold permits on the *old* semaphore and
    /// finish normally — when they drop their permit, the old Arc is
    /// reclaimed. Newly arriving jobs acquire from the new semaphore.
    ///
    /// Transient overshoot: shrinking from N→M while N jobs are live
    /// briefly leaves N old + 0 new in flight. Once those drain, the
    /// new cap takes effect. No correctness issue — the cap is a
    /// throttle, not a hard limit.
    async fn resize(&self, kind: &str, new_cap: usize) {
        let cap = new_cap.max(1);
        let mut semaphores = self.semaphores.write().await;
        semaphores.insert(kind.to_string(), Arc::new(Semaphore::new(cap)));
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

/// RAII wrapper that aborts the underlying tokio `JoinHandle` when
/// the binding goes out of scope. Used to tie the lifetime of helper
/// tasks (heartbeats, watchdogs) to a parent scope so a panic inside
/// the parent can't leave the helper running indefinitely.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
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
        .register(handlers::analyze_loudness::KIND, |s, p| {
            handlers::analyze_loudness::run(s, p)
        })
        .register(handlers::fetch_subtitles_item::KIND, |s, p| {
            handlers::fetch_subtitles_item::run(s, p)
        })
        .register(handlers::refresh_logos_item::KIND, |s, p| {
            handlers::refresh_logos_item::run(s, p)
        })
        .register(handlers::detect_extras_item::KIND, |s, p| {
            handlers::detect_extras_item::run(s, p)
        })
        .register(handlers::extract_embedded_subs::KIND, |s, p| {
            handlers::extract_embedded_subs::run(s, p)
        })
        .register(handlers::fetch_external_ratings::KIND, |s, p| {
            handlers::fetch_external_ratings::run(s, p)
        })
        .register(handlers::bootstrap_season_refs::KIND, |s, p| {
            handlers::bootstrap_season_refs::run(s, p)
        })
        .build()
}

/// Resizeable handle into the worker pool. Each worker carries a
/// unique `id` and reads the desired pool size from `count_tx`; on
/// each iteration it exits if `id >= desired`, so shrinking the
/// pool drains naturally once each worker finishes its current
/// claim. Growing spawns fresh workers starting at the next id.
///
/// Lives in [`crate::state::AppState`] so the admin settings PATCH
/// handler can call [`Self::resize`] when the operator changes
/// `job_workers` — no restart required.
#[derive(Clone)]
pub struct WorkerPoolHandle {
    state: AppState,
    router: JobRouter,
    limiter: KindLimiter,
    count_tx: watch::Sender<usize>,
    /// Monotonic id allocator. We never reuse worker ids because a
    /// resize-down followed by resize-up shouldn't accidentally
    /// re-launch a worker with an id that an in-flight (draining)
    /// worker is still using.
    next_id: Arc<AtomicUsize>,
}

impl WorkerPoolHandle {
    /// Live desired worker count. Each worker's loop self-exits when
    /// its id moves outside this window. Clamped to [1, 32] to match
    /// the admin UI's slider range; 0 would mean "process no jobs",
    /// which is a footgun more easily expressed as "disable the
    /// kinds" via the gates.
    pub fn resize(&self, target: usize) {
        let clamped = target.clamp(1, 32);
        let current = *self.count_tx.borrow();
        if clamped == current {
            return;
        }
        // Publish the new desired count first. Workers above the
        // threshold notice on their next poll-or-changed wakeup and
        // bow out gracefully.
        let _ = self.count_tx.send(clamped);
        if clamped > current {
            // Growing: spawn (target - current) new workers. The id
            // allocator guarantees they get fresh ids past anything
            // already running, even if a previous shrink hasn't
            // finished draining yet.
            for _ in current..clamped {
                self.spawn_worker();
            }
        }
        info!(from = current, to = clamped, "job worker pool resized",);
    }

    /// Apply per-kind concurrency overrides without restart. Called
    /// from the admin settings PATCH path when `job_kind_concurrency`
    /// changes. Each key/value pair swaps that kind's in-flight
    /// semaphore for one with the new cap; unknown keys are ignored
    /// (forward-compat for kinds added/removed across versions).
    pub async fn apply_kind_concurrency(&self, overrides: &HashMap<String, usize>) {
        for (kind, cap) in overrides {
            self.limiter.resize(kind, *cap).await;
            info!(kind = %kind, cap = *cap, "kind concurrency cap applied");
        }
    }

    fn spawn_worker(&self) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let state = self.state.clone();
        let router = self.router.clone();
        let limiter = self.limiter.clone();
        let count_rx = self.count_tx.subscribe();
        tokio::spawn(async move {
            worker_loop(id, state, router, limiter, count_rx).await;
        });
    }
}

/// Reclaim orphans, then spawn `n_workers` polling loops. Returns a
/// [`WorkerPoolHandle`] that can resize the pool live without a
/// process restart.
pub async fn start(state: AppState, router: JobRouter, n_workers: usize) -> WorkerPoolHandle {
    // Boot-time reclaim: any row left in `status='running'` is by
    // definition orphaned — the worker that claimed it lived in a
    // previous process that no longer exists. Passing `lease_ttl=0`
    // makes every `running` row qualify regardless of how recent
    // its `locked_at` is. The previous 1h TTL only caught long-dead
    // jobs and let zombie rows from a recent restart sit "running"
    // for an hour, double-counting against the worker-count math
    // on the admin dashboard.
    match reclaim_orphan_jobs(&state.pool, 0).await {
        Ok(0) => {}
        Ok(n) => info!(count = n, "reclaimed orphan jobs from previous run"),
        Err(e) => warn!(error = %format!("{e:#}"), "reclaim_orphan_jobs failed"),
    }
    let n = n_workers.max(1);
    // Limiter is shared across workers so a permit acquired by one
    // worker blocks others from claiming the same kind. The default
    // permit count for unknown kinds is intentionally fixed at the
    // initial worker count — it caps cross-worker concurrency for
    // operator-defined custom kinds, and changing it at runtime
    // doesn't matter because known (registry) kinds carry their own
    // explicit caps that don't scale with the worker count.
    let limiter = KindLimiter::new(n.max(1));
    let (count_tx, _) = watch::channel(n);
    let handle = WorkerPoolHandle {
        state,
        router,
        limiter,
        count_tx,
        next_id: Arc::new(AtomicUsize::new(0)),
    };
    for _ in 0..n {
        handle.spawn_worker();
    }
    info!(workers = n, "job queue workers spawned");
    handle
}

async fn worker_loop(
    worker_id: usize,
    state: AppState,
    router: JobRouter,
    limiter: KindLimiter,
    mut count_rx: watch::Receiver<usize>,
) {
    // Library-first-scan exclusivity gate. While the gate's counter
    // is non-zero, this worker awaits a clear before claiming new
    // jobs — lets operator-initiated scans of brand-new libraries
    // run uncontended against the worker pool. Incremental scans
    // (re-scans, file watcher, scheduled) do not raise the gate.
    let mut scan_exclusive_rx = state.library_scan_exclusive.subscribe();
    loop {
        // Self-exit check. Each worker's id is monotonic; the
        // resize handler bumps the desired count and any worker
        // whose id is now out of range bows out as soon as it
        // notices. We re-check here (top of the loop) AND between
        // empty-poll sleeps, so a shrink doesn't have to wait for
        // the next claim to take effect.
        if worker_id >= *count_rx.borrow() {
            info!(worker = worker_id, "job worker draining (pool shrunk)");
            return;
        }
        // Park while a first-scan is in progress. `changed()`
        // resolves immediately if the flag was set before the
        // worker started, then we re-check; once cleared, we fall
        // through to the claim path. Polled at watch's event
        // granularity (not a sleep loop) so wake-up is prompt.
        while *scan_exclusive_rx.borrow() {
            if scan_exclusive_rx.changed().await.is_err() {
                // Sender dropped — server shutting down; bow out.
                return;
            }
        }
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
                    let _ =
                        mark_job_dead(&state.pool, job.id, "no handler registered for kind").await;
                    continue;
                };
                let payload: Value = serde_json::from_str(&job.payload).unwrap_or(Value::Null);
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

                // Bump the live in-flight counter; the guard
                // decrements on drop (after the handler returns
                // or panics). For kinds the registry doesn't know
                // about (operator-custom schedules), we skip
                // metrics — they aren't surfaced in the admin
                // activity screen anyway.
                let metric_kind: Option<&'static str> =
                    crate::tasks::registry::find_kind(&kind).map(|k| k.job_kind);
                let _live_guard = metric_kind.map(|k| state.task_metrics.enter(k));
                let started_at = chimpflix_common::now_ms();

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
                // RAII abort: if the surrounding handler future panics
                // or the worker loop unwinds for any reason, the guard
                // drops and aborts the heartbeat — without this the
                // heartbeat keeps touching the lease forever, blocking
                // orphan-reclaim from picking the job up after a panic.
                let _heartbeat = AbortOnDrop(tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(HEARTBEAT_INTERVAL).await;
                        if let Err(e) = touch_job_lease(&heartbeat_state.pool, heartbeat_id).await {
                            warn!(
                                job_id = heartbeat_id,
                                error = %format!("{e:#}"),
                                "heartbeat failed",
                            );
                        }
                    }
                }));

                // Per-job progress tracking. The worker installs a
                // `JobContext` in a tokio task-local before calling
                // the handler. Handlers that emit progress pull the
                // sink with `JobContext::current()` and pass it into
                // tacet's analysis API. We `begin()` here so the UI
                // shows "Starting…" for the brief gap between job
                // claim and the first progress event from inside the
                // handler. `finish()` runs in the `Ok(_) | Err(_)`
                // arms below regardless of outcome.
                state.job_progress.begin(job.id);
                let progress_sink =
                    progress::WorkerProgressSink::new(job.id, state.job_progress.clone());
                let job_ctx = progress::JobContext {
                    job_id: job.id,
                    kind: kind.clone(),
                    progress_sink,
                };
                let result = progress::JobContext::scope(job_ctx, async {
                    handler(state.clone(), payload).await
                })
                .await;
                state.job_progress.finish(job.id);
                // Heartbeat is aborted by `_heartbeat`'s Drop on scope exit
                // (covers both normal flow and handler panic).
                let finished_at = chimpflix_common::now_ms();
                let duration_ms = finished_at.saturating_sub(started_at);
                match result {
                    Ok(()) => {
                        if let Some(k) = metric_kind {
                            state.task_metrics.record(
                                k,
                                crate::tasks::metrics::RunRecord {
                                    finished_at_ms: finished_at,
                                    duration_ms,
                                    success: true,
                                    error_class: None,
                                },
                            );
                        }
                        if let Err(e) = mark_job_succeeded(&state.pool, job.id).await {
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
                        // Classify before computing backoff: rate-
                        // limited / auth / permanent / timeout each
                        // get their own retry curve from
                        // [`error_class::backoff_for_class`]. Falling
                        // back to the legacy exponential schedule
                        // when the class wants a normal retry but
                        // the curve doesn't have one (covered by
                        // `Transient`).
                        let class = classify(&e);
                        if let Some(k) = metric_kind {
                            state.task_metrics.record(
                                k,
                                crate::tasks::metrics::RunRecord {
                                    finished_at_ms: finished_at,
                                    duration_ms,
                                    success: false,
                                    error_class: Some(class.as_str()),
                                },
                            );
                        }
                        let class_backoff = backoff_for_class(class, job.attempts);
                        // For terminal classes `class_backoff` is
                        // None and the computed `backoff` value
                        // doesn't matter — `mark_job_failed_with_class`
                        // flips to `dead` and ignores backoff
                        // entirely. We still compute the legacy
                        // exponential curve as a fallback for the
                        // non-terminal-but-uncovered case (defensive),
                        // and so the log line below shows a useful
                        // number.
                        let backoff = class_backoff.unwrap_or_else(|| backoff_ms(job.attempts));
                        warn!(
                            worker = worker_id,
                            job_id = job.id,
                            kind = %kind,
                            attempts = job.attempts,
                            error_class = class.as_str(),
                            backoff_ms = backoff,
                            error = %msg,
                            "job handler returned error",
                        );
                        if let Err(e) = mark_job_failed_with_class(
                            &state.pool,
                            job.id,
                            &msg,
                            backoff,
                            Some(class.as_str()),
                        )
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
                // Race the empty-poll sleep against a count-change
                // notification so a shrink is observed promptly
                // instead of waiting up to POLL_INTERVAL for the
                // next loop iteration.
                tokio::select! {
                    _ = tokio::time::sleep(POLL_INTERVAL) => {}
                    _ = count_rx.changed() => {}
                }
            }
            Err(e) => {
                warn!(
                    worker = worker_id,
                    error = %format!("{e:#}"),
                    "claim_next_job failed; backing off",
                );
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    _ = count_rx.changed() => {}
                }
            }
        }
    }
}
