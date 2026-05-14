//! In-memory ring buffer that captures `tracing` events for the admin Logs
//! page. We register a custom `Layer` with the global subscriber; events are
//! pushed into a bounded deque so memory is capped regardless of log volume.
//!
//! Events captured by the buffer are also surfaced to the Alerts page when
//! their level is >= WARN, alongside admin audit entries.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use tracing::{Event, Subscriber};
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

const CAPACITY: usize = 5_000;

#[derive(Debug, Clone, Serialize)]
pub struct LogLine {
    pub timestamp_ms: i64,
    pub level: String,
    pub target: String,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct LogBuffer {
    inner: Arc<RwLock<VecDeque<LogLine>>>,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(VecDeque::with_capacity(CAPACITY))),
        }
    }

    /// Push a line into the buffer, evicting the oldest when full.
    pub fn push(&self, line: LogLine) {
        if let Ok(mut buf) = self.inner.write() {
            if buf.len() == CAPACITY {
                buf.pop_front();
            }
            buf.push_back(line);
        }
    }

    /// Newest-first snapshot, optionally filtered by minimum level and a
    /// hard cap on the number of returned entries.
    pub fn snapshot(&self, min_level: Option<&str>, limit: usize) -> Vec<LogLine> {
        let Ok(buf) = self.inner.read() else {
            return Vec::new();
        };
        let want_rank = min_level.map(level_rank).unwrap_or(0);
        buf.iter()
            .rev()
            .filter(|l| level_rank(&l.level) >= want_rank)
            .take(limit)
            .cloned()
            .collect()
    }
}

fn level_rank(level: &str) -> i32 {
    match level.to_ascii_uppercase().as_str() {
        "TRACE" => 1,
        "DEBUG" => 2,
        "INFO" => 3,
        "WARN" => 4,
        "ERROR" => 5,
        _ => 0,
    }
}

/// Capture span-less `tracing::Event`s into a `LogBuffer`. Wired in
/// `main.rs` alongside the fmt layer so logs go both to stdout and to the
/// admin UI.
pub struct LogBufferLayer {
    buffer: LogBuffer,
}

impl LogBufferLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for LogBufferLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let line = LogLine {
            timestamp_ms: chimpflix_common::now_ms(),
            level: event.metadata().level().to_string(),
            target: event.metadata().target().to_string(),
            message: visitor.message,
        };
        self.buffer.push(line);
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        // `message` is `tracing`'s well-known field for the body of an
        // unstructured log; promote it to the line text. Other fields are
        // appended as `key=value` so the captured line is searchable.
        if field.name() == "message" {
            if !self.message.is_empty() {
                self.message.push(' ');
            }
            self.message.push_str(&format!("{value:?}").trim_matches('"'));
        } else {
            if !self.message.is_empty() {
                self.message.push(' ');
            }
            self.message.push_str(&format!("{}={:?}", field.name(), value));
        }
    }
}
