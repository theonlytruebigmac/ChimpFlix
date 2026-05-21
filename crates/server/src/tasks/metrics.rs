//! Live, in-memory per-kind counters + recent-run ring buffer.
//!
//! Read API (in_flight, in_flight_snapshot, recent, RunRecord fields)
//! is consumed by the admin activity endpoint in Phase 7 — silenced
//! here so the upgrade lands cleanly without intermediate noise.

#![allow(dead_code)]

//!
//! Backs the admin activity screen. Reset on restart — these are
//! "what's happening right now" metrics, not history. The
//! historical view (last 30 days per kind) reads from the
//! `task_kind_metrics_daily` rollup table populated by the
//! nightly flush task.
//!
//! Why in-memory:
//!   - The activity screen polls every 5s; reading from disk per
//!     poll would either dominate query latency or be wildly
//!     stale.
//!   - Live counters are append-mostly on hot paths (worker
//!     pickup, worker completion) — using `parking_lot` mutexes or
//!     atomics avoids the contention you'd hit on a SQLite write
//!     under load.
//!   - Numbers that lose their value across a restart (in-flight
//!     count, queue depth right now) are fine to drop — the
//!     historical rollup keeps yesterday.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use std::sync::Mutex;

/// One run's worth of summary data — kept in a per-kind ring
/// buffer so the activity screen can render the last N completions
/// without a DB hit.
#[derive(Debug, Clone)]
pub struct RunRecord {
    pub finished_at_ms: i64,
    pub duration_ms: i64,
    /// True iff the handler returned `Ok(())`.
    pub success: bool,
    /// When `success == false`, the classified error class (see
    /// [`crate::jobs::error_class::ErrorClass`]). `None` on
    /// success or for legacy / unclassified failures.
    pub error_class: Option<&'static str>,
}

/// Bounded ring buffer of run records. Pushes evict the oldest
/// entry when full. Lock-protected via parking_lot for fast,
/// uncontended access on the worker hot path.
#[derive(Debug, Default)]
pub struct RingBuffer {
    inner: Mutex<RingBufferInner>,
}

#[derive(Debug, Default)]
struct RingBufferInner {
    buf: Vec<RunRecord>,
    next: usize,
}

const RECENT_CAP_PER_KIND: usize = 100;

impl RingBuffer {
    pub fn push(&self, r: RunRecord) {
        let mut g = self.inner.lock().expect("metrics ringbuf mutex poisoned");
        if g.buf.len() < RECENT_CAP_PER_KIND {
            g.buf.push(r);
        } else {
            let slot = g.next;
            g.buf[slot] = r;
            g.next = (slot + 1) % RECENT_CAP_PER_KIND;
        }
    }

    /// Snapshot the ring, newest-first. Callers receive a `Vec`
    /// they own — no lock held after return. Cheap because the
    /// buffer is bounded at 100 per kind.
    pub fn snapshot_newest_first(&self) -> Vec<RunRecord> {
        let g = self.inner.lock().expect("metrics ringbuf mutex poisoned");
        if g.buf.len() < RECENT_CAP_PER_KIND {
            // Not yet wrapped — newest is the last pushed entry.
            g.buf.iter().rev().cloned().collect()
        } else {
            // Wrapped — `next` points at the *oldest*, so walk
            // backwards from `next-1` modulo length.
            let len = g.buf.len();
            (0..len)
                .map(|i| (g.next + len - 1 - i) % len)
                .map(|i| g.buf[i].clone())
                .collect()
        }
    }
}

/// Server-wide live metrics. One instance lives on `AppState` for
/// the process lifetime; cloned freely as an `Arc`.
#[derive(Debug, Default, Clone)]
pub struct LiveMetrics {
    inner: Arc<LiveMetricsInner>,
}

#[derive(Debug, Default)]
struct LiveMetricsInner {
    /// Per-kind count of jobs currently in `status = 'running'`.
    /// Bumped on worker pickup, decremented on completion (success
    /// or failure). Atomic — no lock contention on the hot path.
    in_flight: Mutex<HashMap<&'static str, Arc<AtomicU32>>>,
    /// Per-kind ring buffer of recent run records (last 100).
    /// Each kind gets its own buffer so contention is per-kind.
    recent: Mutex<HashMap<&'static str, Arc<RingBuffer>>>,
}

impl LiveMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Increment the in-flight counter for `kind`. Returns a
    /// guard that decrements on drop, so callers can't forget to
    /// decrement on early-return paths.
    pub fn enter(&self, kind: &'static str) -> InFlightGuard {
        let counter = self.counter_for(kind);
        counter.fetch_add(1, Ordering::Relaxed);
        InFlightGuard { counter }
    }

    /// Record one completed run.
    pub fn record(&self, kind: &'static str, run: RunRecord) {
        let buf = self.ring_for(kind);
        buf.push(run);
    }

    /// Snapshot the in-flight count for `kind`. 0 for kinds that
    /// have never run since this process started.
    pub fn in_flight(&self, kind: &str) -> u32 {
        let g = self
            .inner
            .in_flight
            .lock()
            .expect("metrics in_flight mutex poisoned");
        g.get(kind).map(|c| c.load(Ordering::Relaxed)).unwrap_or(0)
    }

    /// Snapshot every kind's in-flight count. Used by the admin
    /// API to render the activity screen in one call.
    pub fn in_flight_snapshot(&self) -> HashMap<String, u32> {
        let g = self
            .inner
            .in_flight
            .lock()
            .expect("metrics in_flight mutex poisoned");
        g.iter()
            .map(|(k, c)| ((*k).to_string(), c.load(Ordering::Relaxed)))
            .collect()
    }

    /// Snapshot recent runs for `kind`, newest first.
    pub fn recent(&self, kind: &str) -> Vec<RunRecord> {
        let g = self
            .inner
            .recent
            .lock()
            .expect("metrics recent mutex poisoned");
        match g.get(kind) {
            Some(buf) => buf.snapshot_newest_first(),
            None => Vec::new(),
        }
    }

    fn counter_for(&self, kind: &'static str) -> Arc<AtomicU32> {
        let mut g = self
            .inner
            .in_flight
            .lock()
            .expect("metrics in_flight mutex poisoned");
        g.entry(kind)
            .or_insert_with(|| Arc::new(AtomicU32::new(0)))
            .clone()
    }

    fn ring_for(&self, kind: &'static str) -> Arc<RingBuffer> {
        let mut g = self
            .inner
            .recent
            .lock()
            .expect("metrics recent mutex poisoned");
        g.entry(kind)
            .or_insert_with(|| Arc::new(RingBuffer::default()))
            .clone()
    }
}

/// RAII decrement of the in-flight counter. Dropped at the end of
/// the worker's per-job scope, regardless of whether the handler
/// succeeded, failed, or panicked.
pub struct InFlightGuard {
    counter: Arc<AtomicU32>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_increments_and_drop_decrements() {
        let m = LiveMetrics::new();
        assert_eq!(m.in_flight("k"), 0);
        let g = m.enter("k");
        assert_eq!(m.in_flight("k"), 1);
        drop(g);
        assert_eq!(m.in_flight("k"), 0);
    }

    #[test]
    fn enter_handles_concurrent_increments() {
        let m = LiveMetrics::new();
        let _g1 = m.enter("k");
        let _g2 = m.enter("k");
        let _g3 = m.enter("k");
        assert_eq!(m.in_flight("k"), 3);
    }

    #[test]
    fn ring_buffer_keeps_last_100_newest_first() {
        let m = LiveMetrics::new();
        for i in 0..150 {
            m.record(
                "k",
                RunRecord {
                    finished_at_ms: i,
                    duration_ms: 1,
                    success: true,
                    error_class: None,
                },
            );
        }
        let recent = m.recent("k");
        assert_eq!(recent.len(), 100);
        // Newest-first: index 0 should be the most recent (i=149)
        assert_eq!(recent[0].finished_at_ms, 149);
        // Oldest in the snapshot should be i=50.
        assert_eq!(recent[99].finished_at_ms, 50);
    }

    #[test]
    fn ring_buffer_unwrapped_returns_inserted_order_reversed() {
        let m = LiveMetrics::new();
        for i in 0..5 {
            m.record(
                "k",
                RunRecord {
                    finished_at_ms: i,
                    duration_ms: 1,
                    success: true,
                    error_class: None,
                },
            );
        }
        let recent = m.recent("k");
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].finished_at_ms, 4);
        assert_eq!(recent[4].finished_at_ms, 0);
    }

    #[test]
    fn in_flight_snapshot_includes_every_seen_kind() {
        let m = LiveMetrics::new();
        let _a = m.enter("alpha");
        let _b1 = m.enter("beta");
        let _b2 = m.enter("beta");
        let snap = m.in_flight_snapshot();
        assert_eq!(snap.get("alpha"), Some(&1));
        assert_eq!(snap.get("beta"), Some(&2));
    }

    #[test]
    fn recent_for_unknown_kind_returns_empty() {
        let m = LiveMetrics::new();
        assert!(m.recent("never_run").is_empty());
    }
}
