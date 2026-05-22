//! Counter-backed exclusivity gate for library first-scans.
//!
//! The operator-initiated scan of a brand-new library needs to run
//! uncontended against the rest of the system — the failure mode
//! before this gate existed was the scanner racing 8 active workers
//! on the shared SQLite write lock, exhausting `busy_timeout` mid-
//! run, and bailing with a half-populated library (e.g. 75 files
//! visible out of 1560).
//!
//! Implementation: a counter wrapped behind a watch channel.
//!   * `acquire` bumps the counter; if it transitioned from 0 to 1,
//!     the watch flips to `true` and all subscribers wake.
//!   * `release` decrements; if back to 0, the watch flips to
//!     `false` and subscribers fall through to their normal work.
//!
//! Counter semantics (not a bool) so two overlapping first-scans
//! (operator adds library A, then library B before A's scan
//! finishes) both correctly hold the gate until both complete.
//! Using a bare watch::Sender<bool> would have the second scan's
//! completion release the gate even while the first is still
//! running.

use std::sync::Arc;

use tokio::sync::{Mutex, watch};

/// Library-first-scan exclusivity gate. Construct once at AppState
/// build time; share via `Arc`.
pub struct LibraryScanGate {
    count: Mutex<u32>,
    tx: watch::Sender<bool>,
}

impl LibraryScanGate {
    pub fn new() -> Arc<Self> {
        let (tx, _rx) = watch::channel(false);
        Arc::new(Self {
            count: Mutex::new(0),
            tx,
        })
    }

    /// Mark one first-scan as started. The gate is active for the
    /// rest of the system until a matching [`Self::release`] call.
    /// Idempotency is the caller's responsibility — multiple
    /// `acquire` calls without matching `release` will keep the
    /// counter elevated.
    pub async fn acquire(&self) {
        let mut count = self.count.lock().await;
        *count += 1;
        if *count == 1 {
            // `send_replace` (not `send`) so the value updates even
            // when no receivers are currently subscribed. The gate
            // is constructed early in AppState; subscribers attach
            // later from worker/scheduler tasks. Plain `send` would
            // see `is_closed = true` between construction and
            // first subscribe and silently drop the update.
            self.tx.send_replace(true);
        }
    }

    /// Mark one first-scan as finished. When the counter reaches 0,
    /// the gate flips to inactive and subscribers wake. Safe to
    /// call even if the counter is already 0 (defensive: server
    /// shutdown or duplicate completion callbacks don't underflow).
    pub async fn release(&self) {
        let mut count = self.count.lock().await;
        if *count > 0 {
            *count -= 1;
        }
        if *count == 0 {
            self.tx.send_replace(false);
        }
    }

    /// Subscribe to gate state changes. Receivers see `true` while
    /// any first-scan is active, `false` when all have drained.
    /// Consumers `await` on `changed()` to wake without polling.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.tx.subscribe()
    }

    /// Snapshot the gate state. Useful for callers that want a
    /// single-shot read without subscribing.
    pub fn is_active(&self) -> bool {
        *self.tx.subscribe().borrow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_release_cycle_flips_gate() {
        let gate = LibraryScanGate::new();
        assert!(!gate.is_active());
        gate.acquire().await;
        assert!(gate.is_active());
        gate.release().await;
        assert!(!gate.is_active());
    }

    #[tokio::test]
    async fn overlapping_acquires_hold_until_last_release() {
        // Adding library A, then library B before A finishes:
        // the gate should stay active until BOTH releases land.
        let gate = LibraryScanGate::new();
        gate.acquire().await; // A starts
        gate.acquire().await; // B starts
        assert!(gate.is_active());
        gate.release().await; // A finishes
        assert!(gate.is_active(), "B is still running; gate stays active");
        gate.release().await; // B finishes
        assert!(!gate.is_active());
    }

    #[tokio::test]
    async fn double_release_does_not_underflow() {
        let gate = LibraryScanGate::new();
        gate.acquire().await;
        gate.release().await;
        gate.release().await; // extra release — should be a no-op
        assert!(!gate.is_active());
        // Counter stays at zero; next acquire flips the watch normally.
        gate.acquire().await;
        assert!(gate.is_active());
    }
}
