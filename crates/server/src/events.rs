//! In-process pub/sub for events broadcast to WebSocket subscribers.
//!
//! v0.1: a single broadcast channel carries every event. The wire format
//! is the JSON serialization of `ScanEvent` directly (see
//! `chimpflix_library::ScanEvent`); a richer topic-scoped format lands
//! when more event types arrive in later phases.

use chimpflix_library::ScanEvent;
use tokio::sync::broadcast;

/// What goes onto the broadcast channel. Today scan events are the only
/// kind; the enum exists so we can add more without breaking subscribers.
#[derive(Debug, Clone)]
pub enum Event {
    Scan(ScanEvent),
}

#[derive(Clone)]
pub struct Hub {
    tx: broadcast::Sender<Event>,
}

impl Hub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publish an event. Returns the number of active subscribers reached.
    /// Errors on "no subscribers" are intentionally ignored.
    pub fn publish(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}
