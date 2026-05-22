//! Per-job live progress tracking.
//!
//! When the worker picks up a job it stashes a [`JobContext`] in a
//! tokio task-local before calling the handler. Handlers that want to
//! emit progress (e.g. by passing a [`tacet::loudness::ProgressSink`]
//! into a tacet analysis call) can pull the active context with
//! [`JobContext::current`] without changing their function signature.
//!
//! The sink writes into an in-memory [`JobProgressStore`] keyed by
//! `job_id`. The admin "Activity" feed polls a snapshot of the store
//! and merges it with the in-flight job rows from the DB so the UI
//! can render "Decoding · 42%" inline for the job that's running
//! right now.
//!
//! The store is ephemeral — entries are inserted on job start and
//! removed on job completion (success or failure). Nothing is
//! persisted across server restarts; in-flight jobs that survived a
//! crash will be re-attempted by `claim_next_job` and pick up a
//! fresh progress entry on their next run.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tacet::loudness::{ProgressEvent, ProgressSink};

use chimpflix_common::now_ms;

/// Context the worker hands to the handler via a tokio task-local.
/// Handlers don't take this as a parameter — they pull it with
/// [`JobContext::current`] only if they care about progress, so
/// existing handler signatures don't need to change.
#[derive(Clone)]
pub struct JobContext {
    pub job_id: i64,
    /// Job kind — currently unused by callers but recorded here so
    /// handlers can emit kind-aware progress events (e.g. distinct
    /// labels for marker detection vs. loudness sweep) without an
    /// extra round trip through the queue.
    #[allow(dead_code)]
    pub kind: String,
    /// Shared sink that emits into the live progress store. Cheap
    /// clone (Arc); pass to `tacet::analyze::analyze_audio` as
    /// `Some(sink.as_ref())`.
    pub progress_sink: Arc<dyn ProgressSink>,
}

tokio::task_local! {
    /// Per-job task-local. Set by the worker around each handler
    /// dispatch; readable from anywhere inside the handler's async
    /// call tree via [`JobContext::current`].
    static JOB_CTX: JobContext;
}

impl JobContext {
    /// Run `f` with `ctx` installed as the active job context. Used by
    /// the worker dispatcher to scope a handler call.
    pub async fn scope<F, T>(ctx: JobContext, f: F) -> T
    where
        F: std::future::Future<Output = T>,
    {
        JOB_CTX.scope(ctx, f).await
    }

    /// Returns the active job context, if any. Returns `None` when
    /// called outside a worker (e.g. an HTTP request handler that
    /// happens to call into a function that also looks for context).
    pub fn current() -> Option<JobContext> {
        JOB_CTX.try_with(|ctx| ctx.clone()).ok()
    }
}

/// One in-flight job's live progress.
#[derive(Debug, Clone, Serialize)]
pub struct JobProgress {
    /// Human-readable stage label ("Decoding audio", "Computing
    /// loudness", "Matching fingerprint…"). Falls back to the
    /// machine name when no friendlier label is set.
    pub stage: String,
    /// 0.0..=1.0 when known, `None` when the stage doesn't have a
    /// duration the UI can normalize against.
    pub percent: Option<f32>,
    /// Epoch ms of the last update. The UI uses this to grey out
    /// progress that hasn't refreshed in a while (worker stuck /
    /// long-running stage without sub-stage events).
    pub updated_at_ms: i64,
    /// Internal: last-known total duration in seconds, used to
    /// convert position-in-seconds events into percent. Not
    /// serialized — the API response only needs the percent itself.
    #[serde(skip)]
    duration_secs: Option<f64>,
}

impl JobProgress {
    fn new_started(stage: impl Into<String>, duration_secs: Option<f64>) -> Self {
        Self {
            stage: stage.into(),
            percent: Some(0.0),
            updated_at_ms: now_ms(),
            duration_secs,
        }
    }
}

/// In-memory store of live progress for jobs currently executing.
/// Workers insert + update; the admin API reads snapshots.
#[derive(Default)]
pub struct JobProgressStore {
    inner: Mutex<HashMap<i64, JobProgress>>,
}

impl JobProgressStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Translate a tacet [`ProgressEvent`] into a [`JobProgress`]
    /// update for `job_id`. Idempotent: late or out-of-order events
    /// only overwrite the entry's mutable fields, never re-insert
    /// completed entries (handled by the worker's `remove` on job
    /// completion).
    pub fn update(&self, job_id: i64, event: &ProgressEvent) {
        let mut guard = self.inner.lock().expect("JobProgressStore mutex poisoned");
        let now = now_ms();
        let entry = guard
            .entry(job_id)
            .or_insert_with(|| JobProgress::new_started("Starting", None));
        entry.updated_at_ms = now;

        match event {
            ProgressEvent::LoudnessStarted { duration_seconds } => {
                entry.stage = "Loudness · decoding".to_string();
                entry.duration_secs = *duration_seconds;
                entry.percent = Some(0.0);
            }
            ProgressEvent::LoudnessProgress { position_seconds } => {
                entry.stage = "Loudness · decoding".to_string();
                entry.percent = entry
                    .duration_secs
                    .filter(|d| *d > 0.0)
                    .map(|d| (*position_seconds / d).clamp(0.0, 1.0) as f32);
            }
            ProgressEvent::LoudnessFinalizing => {
                entry.stage = "Loudness · finalizing".to_string();
                entry.percent = Some(0.99);
            }
            ProgressEvent::MarkersStarted => {
                entry.stage = "Markers · analyzing".to_string();
                entry.percent = None;
                entry.duration_secs = None;
            }
            ProgressEvent::MarkersFinalizing => {
                entry.stage = "Markers · finalizing".to_string();
            }
            ProgressEvent::Completed => {
                // Worker handles final cleanup via `remove`; ignore
                // here so a stray Completed event doesn't leave the
                // entry stale.
            }
            // `ProgressEvent` is `#[non_exhaustive]` so tacet can grow
            // new variants without breaking us. Future variants
            // surface as the literal machine name until we add a
            // friendlier label here.
            _ => {
                entry.stage = format!("{event:?}");
            }
        }
    }

    /// Insert a placeholder entry for a freshly-started job. Used by
    /// the worker before any analysis-specific event has fired so the
    /// UI can show "Starting…" instead of nothing for the brief
    /// window between job claim and first event.
    pub fn begin(&self, job_id: i64) {
        self.inner
            .lock().expect("JobProgressStore mutex poisoned")
            .insert(job_id, JobProgress::new_started("Starting", None));
    }

    /// Remove a job's entry. Called by the worker on job completion
    /// (success or failure).
    pub fn finish(&self, job_id: i64) {
        self.inner.lock().expect("JobProgressStore mutex poisoned").remove(&job_id);
    }

    /// Snapshot the store for inclusion in the admin jobs response.
    /// Cheap clone of the whole map; the store stays bounded by the
    /// worker-pool size (at most `n_workers` live entries).
    pub fn snapshot(&self) -> HashMap<i64, JobProgress> {
        self.inner.lock().expect("JobProgressStore mutex poisoned").clone()
    }
}

/// Sink the worker installs into a [`JobContext`]. Forwards each
/// [`ProgressEvent`] to the [`JobProgressStore`] keyed by the
/// job_id captured at construction.
pub struct WorkerProgressSink {
    job_id: i64,
    store: Arc<JobProgressStore>,
}

impl WorkerProgressSink {
    pub fn new(job_id: i64, store: Arc<JobProgressStore>) -> Arc<Self> {
        Arc::new(Self { job_id, store })
    }
}

impl ProgressSink for WorkerProgressSink {
    fn emit(&self, event: ProgressEvent) {
        self.store.update(self.job_id, &event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loudness_progress_computes_percent_from_duration() {
        let store = JobProgressStore::new();
        store.begin(1);
        store.update(
            1,
            &ProgressEvent::LoudnessStarted {
                duration_seconds: Some(600.0),
            },
        );
        store.update(
            1,
            &ProgressEvent::LoudnessProgress {
                position_seconds: 300.0,
            },
        );
        let snap = store.snapshot();
        let p = snap.get(&1).expect("entry exists");
        assert!(p.percent.unwrap() > 0.49 && p.percent.unwrap() < 0.51);
        assert!(p.stage.contains("Loudness"));
    }

    #[test]
    fn progress_without_duration_leaves_percent_none() {
        let store = JobProgressStore::new();
        store.update(
            42,
            &ProgressEvent::LoudnessProgress {
                position_seconds: 100.0,
            },
        );
        let snap = store.snapshot();
        // The entry was auto-inserted by `update` since no `begin`
        // call preceded it. Without a known duration, percent stays
        // None.
        assert!(snap.get(&42).unwrap().percent.is_none());
    }

    #[test]
    fn finish_removes_entry() {
        let store = JobProgressStore::new();
        store.begin(7);
        assert!(store.snapshot().contains_key(&7));
        store.finish(7);
        assert!(!store.snapshot().contains_key(&7));
    }

    #[test]
    fn markers_started_resets_duration_tracking() {
        // Edge case: a single handler emits both LoudnessStarted (set
        // duration) and later MarkersStarted (clear duration). When a
        // subsequent MarkersFinalizing arrives we should not still
        // think percent is bound to the old duration.
        let store = JobProgressStore::new();
        store.update(
            5,
            &ProgressEvent::LoudnessStarted {
                duration_seconds: Some(120.0),
            },
        );
        store.update(5, &ProgressEvent::MarkersStarted);
        let snap = store.snapshot();
        let p = snap.get(&5).unwrap();
        assert!(p.percent.is_none());
        assert!(p.stage.contains("Markers"));
    }

    #[tokio::test]
    async fn task_local_scope_propagates_context() {
        let store = JobProgressStore::new();
        let sink = WorkerProgressSink::new(99, store.clone());
        let ctx = JobContext {
            job_id: 99,
            kind: "test_kind".into(),
            progress_sink: sink,
        };
        let observed = JobContext::scope(ctx.clone(), async move {
            JobContext::current().map(|c| c.job_id)
        })
        .await;
        assert_eq!(observed, Some(99));
        // Outside scope, no context.
        assert!(JobContext::current().is_none());
    }
}
