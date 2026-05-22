//! Discovery-triggered processing pipeline.
//!
//! When the scanner (scheduled, manual, or file-watcher path)
//! emits `ScanEvent::FileAdded`, we enqueue the per-file jobs that
//! make the new file fully playable: marker detection and loudness
//! analysis. They all run through the durable queue so a server
//! crash mid-processing resumes on next startup.
//!
//! Each kind is enqueued via `enqueue_job_unique` keyed on file_id
//! so a repeated FileAdded (e.g. a file that was deleted and
//! re-added quickly) doesn't pile up duplicate jobs.
//!
//! ## Gating
//!
//! Both per-file kinds enqueued here ([`PIPELINE_KINDS`]) and the
//! safety-net sweeps in [`crate::scheduler`] consult
//! [`crate::tasks::is_kind_allowed`]. That means flipping a
//! `*_enabled` setting in admin → tasks stops both entry points
//! immediately — the bug fix in `docs/pipelines/backend-plan.md` §2.
//!
//! ## Batching
//!
//! Per `docs/pipelines/backend-plan.md` §5, the scanner-driven
//! path buffers FileAdded events through an mpsc channel and a
//! dedicated drainer task that batches up to [`PIPELINE_BATCH_MAX`]
//! file_ids or flushes every [`PIPELINE_FLUSH_INTERVAL`] —
//! whichever first. One batch = one BEGIN IMMEDIATE = one fsync,
//! so a 10k-file scan becomes ~10 batches instead of 10k
//! transactions.

use std::time::Duration;

use chimpflix_library::ScanEvent;
use tokio::sync::mpsc;
use tracing::warn;

use crate::jobs::handlers;
use crate::state::AppState;
use crate::tasks::is_kind_allowed;

/// Flush every batch when this many events accumulate. Chosen so
/// the resulting transaction has predictable size — at 1000 file
/// events × 4 kinds × ~100B each, the WAL slice stays well under
/// 1MB and SQLite's per-statement parse cache stays hot.
const PIPELINE_BATCH_MAX: usize = 1000;

/// Maximum wall-clock between flushes. Bursts smaller than the
/// batch limit still drain promptly so admins see jobs appear in
/// the queue within the next polling interval.
const PIPELINE_FLUSH_INTERVAL: Duration = Duration::from_millis(100);

/// Bounded channel capacity for the file-id buffer between the
/// scan emitter and the drainer task. Big enough to absorb a
/// burst of `PIPELINE_BATCH_MAX × 4` events (one full flush in
/// flight + three waiting); if a stuck DB connection backs up the
/// drainer for longer than that, we'd rather drop new events on
/// the floor than grow memory without bound — the safety-net
/// scheduled tasks (`detect_markers`, `analyze_loudness`, …) will
/// catch any missed files on their next sweep.
const PIPELINE_CHANNEL_CAPACITY: usize = PIPELINE_BATCH_MAX * 4;

/// The set of per-file kinds the discovery pipeline fans out into
/// on `FileAdded`. Kept here (not in the tasks registry) because
/// the registry is the *what* and *which gate*; this is the
/// *what runs in the pipeline*. A periodic-only kind (e.g.
/// `refresh_trending`) would never appear here.
const PIPELINE_KINDS: &[&str] = &[
    handlers::detect_markers_file::KIND,
    handlers::analyze_loudness::KIND,
];

/// Wrap an inner emitter with FileAdded handling. The inner
/// emitter is still called for every event (so the hub still gets
/// them); when a FileAdded comes through, the file_id is sent into
/// the batching channel.
///
/// A dedicated drainer task on the side accumulates ids, flushes
/// every [`PIPELINE_BATCH_MAX`] or [`PIPELINE_FLUSH_INTERVAL`] —
/// whichever first — and dispatches them via
/// [`enqueue_pipeline_batch`]. Net effect for a 10k-file initial
/// scan: ~10 transactions instead of 10k.
pub fn wrap_emitter_for_pipeline(
    state: AppState,
    inner: chimpflix_library::ScanEmitter,
) -> chimpflix_library::ScanEmitter {
    use std::sync::Arc;
    let (tx, rx) = mpsc::channel::<i64>(PIPELINE_CHANNEL_CAPACITY);
    spawn_pipeline_drainer(state, rx);

    Arc::new(move |evt: ScanEvent| {
        // Forward to the original consumer (hub publishes to WS).
        inner(evt.clone());
        if let ScanEvent::FileAdded { media_file_id, .. } = evt {
            // `try_send` never awaits — keeps the scanner's
            // per-file loop fast. On capacity overflow we log and
            // drop: the file still has a row in `media_files`, so
            // the safety-net sweeps (`detect_markers`,
            // `analyze_loudness`, …) pick it up on their next
            // tick. Dropped events here are a hint that the DB
            // pool is stuck or the worker pool is undersized.
            if let Err(e) = tx.try_send(media_file_id) {
                use tokio::sync::mpsc::error::TrySendError;
                match e {
                    TrySendError::Full(_) => warn!(
                        media_file_id,
                        capacity = PIPELINE_CHANNEL_CAPACITY,
                        "discovery pipeline channel full; dropping FileAdded (safety-net sweeps will catch this file)"
                    ),
                    TrySendError::Closed(_) => {
                        // Scanner outlived the drainer — only
                        // happens on shutdown races. Silent.
                    }
                }
            }
        }
    })
}

/// Spawn the batching drainer. Reads from `rx` until the channel
/// closes (scanner shutdown), buffering up to `PIPELINE_BATCH_MAX`
/// file_ids or `PIPELINE_FLUSH_INTERVAL` of wall time, then calls
/// [`enqueue_pipeline_batch`] for the buffered slice.
///
/// One drainer per emitter wrap — each library scan spawns its
/// own, scoped to the lifetime of that scan. No global
/// coordination needed.
fn spawn_pipeline_drainer(state: AppState, mut rx: mpsc::Receiver<i64>) {
    tokio::spawn(async move {
        let mut buf: Vec<i64> = Vec::with_capacity(PIPELINE_BATCH_MAX);
        loop {
            // Block until either the next event arrives or the
            // flush timer expires. `select!` returns whichever
            // branch fires first; the timer is re-created each
            // iteration (cheap — Sleep is a stack value).
            let timeout = tokio::time::sleep(PIPELINE_FLUSH_INTERVAL);
            tokio::pin!(timeout);
            tokio::select! {
                maybe = rx.recv() => {
                    match maybe {
                        Some(file_id) => {
                            buf.push(file_id);
                            if buf.len() >= PIPELINE_BATCH_MAX {
                                flush_pipeline_batch(&state, &mut buf).await;
                            }
                        }
                        // Channel closed: scanner finished. Drain
                        // anything left and exit.
                        None => {
                            if !buf.is_empty() {
                                flush_pipeline_batch(&state, &mut buf).await;
                            }
                            break;
                        }
                    }
                }
                _ = &mut timeout => {
                    if !buf.is_empty() {
                        flush_pipeline_batch(&state, &mut buf).await;
                    }
                }
            }
        }
    });
}

async fn flush_pipeline_batch(state: &AppState, buf: &mut Vec<i64>) {
    let to_flush = std::mem::take(buf);
    let count = to_flush.len();
    if let Err(e) = enqueue_pipeline_batch(state, &to_flush).await {
        warn!(
            error = %format!("{e:#}"),
            batch_size = count,
            "discovery pipeline batch enqueue failed",
        );
    }
}

/// Enqueue per-file pipeline jobs for many file_ids in a single
/// transaction. Powers the scanner-emitter drainer
/// ([`spawn_pipeline_drainer`]) — turns a 10k-file initial scan
/// from 10k transactions into ~10. A single-file caller can pass a
/// one-element slice and pays only the constant startup overhead.
///
/// Each kind's gate is consulted once at the start; if a gate is
/// off, no rows for that kind get inserted regardless of how many
/// file_ids the batch contains.
///
/// Uses `BEGIN IMMEDIATE` rather than `pool.begin()` (which issues
/// `BEGIN DEFERRED`). The pipeline tx mixes SELECT (dedup check)
/// with INSERT; with a deferred BEGIN, the SELECT acquires a read
/// snapshot first and the INSERT then tries to upgrade to a writer
/// — and if the scanner has advanced the WAL in between (very
/// likely, since this drains while the scanner is still emitting),
/// SQLite returns `SQLITE_BUSY_SNAPSHOT` (517), which `busy_timeout`
/// does NOT retry. Acquiring the write lock upfront converts that
/// into a plain BUSY that `busy_timeout` *does* poll on. Same
/// trick used by `merge_items` in the library crate.
pub async fn enqueue_pipeline_batch(state: &AppState, file_ids: &[i64]) -> anyhow::Result<()> {
    use anyhow::Context;
    if file_ids.is_empty() {
        return Ok(());
    }

    // Gate-resolve up front so we don't repeat the settings-cache
    // RwLock read N times inside the tx.
    let mut kinds_to_enqueue: Vec<&'static str> = Vec::with_capacity(PIPELINE_KINDS.len());
    for kind in PIPELINE_KINDS {
        if is_kind_allowed(state, kind).await.is_allowed() {
            kinds_to_enqueue.push(*kind);
        }
    }
    if kinds_to_enqueue.is_empty() {
        return Ok(());
    }

    let mut conn = state.pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .context("BEGIN IMMEDIATE for pipeline batch enqueue")?;
    let result = enqueue_pipeline_batch_inner(&mut conn, file_ids, &kinds_to_enqueue).await;
    match &result {
        Ok(_) => {
            sqlx::query("COMMIT").execute(&mut *conn).await?;
        }
        Err(_) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
        }
    }
    result
}

async fn enqueue_pipeline_batch_inner(
    conn: &mut sqlx::SqliteConnection,
    file_ids: &[i64],
    kinds: &[&str],
) -> anyhow::Result<()> {
    use chimpflix_library::queries::{JobInput, enqueue_job_unique_tx};
    // Outer loop: file_id. Inner loop: kind. Keeps the JSON
    // payload allocation per-file, which is the larger of the two
    // (kind names are static).
    for &file_id in file_ids {
        let payload = serde_json::json!({ "file_id": file_id });
        for kind in kinds {
            enqueue_job_unique_tx(
                &mut *conn,
                JobInput::new(*kind, payload.clone()),
                "file_id",
                file_id,
            )
            .await?;
        }
    }
    Ok(())
}

/// Counts returned by [`enqueue_full_sweep`] — surfaced in the admin
/// "Process all pending" response so the operator sees how much was
/// queued without having to refresh the queue list afterward.
#[derive(Debug, Default, serde::Serialize)]
pub struct SweepCounts {
    pub markers: usize,
    pub loudness: usize,
}

/// Retroactively run the discovery pipeline against the existing
/// library: query for every file lacking each artifact, enqueue
/// the corresponding kind. Dedup means a re-trigger while jobs are
/// in flight is a no-op per file.
///
/// Use this to backfill items that existed before the pipeline
/// migration shipped — or after restoring from a backup that
/// didn't carry the artifact tables. The scheduled safety-net
/// tasks would eventually catch the same files, but this drains
/// the entire backlog in one click rather than across many ticks.
///
/// Gated kinds skip when their setting is off; the operator sees
/// `0` in the counts for those — clearer than failing or quietly
/// queueing work that won't run.
pub async fn enqueue_full_sweep(state: &AppState) -> anyhow::Result<SweepCounts> {
    use chimpflix_library::queries;
    // Generous caps — the queue is fine with thousands of rows and
    // each kind dedups on file_id so re-running this is harmless.
    const CAP: i64 = 100_000;
    let mut counts = SweepCounts::default();
    let pool = &state.pool;

    // Markers (always-on / Automatic): needs per-library iteration
    // because the existing query is scoped that way.
    if is_kind_allowed(state, handlers::detect_markers_file::KIND)
        .await
        .is_allowed()
    {
        for lib in queries::list_libraries(pool, None).await? {
            let rows = queries::list_media_files_needing_markers(pool, lib.id, CAP).await?;
            let ids: Vec<i64> = rows.into_iter().map(|(id, _, _)| id).collect();
            counts.markers += handlers::detect_markers_file::enqueue_for_files(pool, &ids).await?;
        }
    }

    // Loudness (Gated).
    if is_kind_allowed(state, handlers::analyze_loudness::KIND)
        .await
        .is_allowed()
    {
        let loud = queries::list_media_files_needing_loudness(pool, None, CAP).await?;
        let loud_ids: Vec<i64> = loud.iter().map(|c| c.id).collect();
        counts.loudness = handlers::analyze_loudness::enqueue_for_files(pool, &loud_ids).await?;
    }

    Ok(counts)
}
