//! In-process pub/sub for events broadcast to WebSocket subscribers and
//! the webhook dispatcher.

use chimpflix_library::ScanEvent;
use chimpflix_transcoder::SessionSnapshot;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::broadcast;

/// What goes onto the broadcast channel. `Scan` is reserved for the legacy
/// scan-progress format; `Webhook` is the generic event envelope picked up
/// by the dispatcher to fan out to subscribed webhooks; `Sessions` is the
/// "active transcodes list changed" stream consumed by the admin dashboard.
#[derive(Debug, Clone)]
pub enum Event {
    Scan(ScanEvent),
    /// (event_name, JSON payload) for outbound webhook delivery.
    Webhook(WebhookEvent),
    /// Full snapshot of the current set of active transcode sessions.
    /// Emitted whenever the membership of the set changes (start / end /
    /// reap). Subscribers should treat this as the authoritative state.
    Sessions(SessionsEvent),
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionsEvent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub active: Vec<SessionSnapshot>,
}

impl SessionsEvent {
    pub fn snapshot(active: Vec<SessionSnapshot>) -> Self {
        Self {
            kind: "sessions",
            active,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct WebhookEvent {
    pub name: String,
    pub payload: Value,
}

impl WebhookEvent {
    pub fn new(name: impl Into<String>, payload: impl Serialize) -> Self {
        let name = name.into();
        // Serialize the payload up front. If it fails (a future event
        // type with a non-JSON-friendly nested value), substitute
        // `Null` so we still produce *some* event — but record the
        // failure at error level so the operator can spot it. The
        // previous `.unwrap_or(Value::Null)` made these silent.
        let payload = match serde_json::to_value(&payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(
                    event = %name,
                    error = %e,
                    "webhook event payload failed to serialize; emitting Null instead"
                );
                Value::Null
            }
        };
        Self { name, payload }
    }
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
