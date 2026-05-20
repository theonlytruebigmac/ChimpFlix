//! Scanner event types and emitter alias.

use std::sync::Arc;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScanEvent {
    Started {
        job_id: i64,
        library_id: i64,
    },
    Progress {
        job_id: i64,
        library_id: i64,
        files_seen: i64,
        files_added: i64,
        files_updated: i64,
        files_removed: i64,
    },
    Completed {
        job_id: i64,
        library_id: i64,
        files_seen: i64,
        files_added: i64,
        files_updated: i64,
        files_removed: i64,
    },
    Failed {
        job_id: i64,
        library_id: i64,
        error: String,
    },
    /// Fired the moment a new media_files row is created. Consumers
    /// (the server's scan emitter) use this to enqueue per-file
    /// discovery-pipeline jobs (detect markers, generate preview,
    /// analyze loudness, build chapter thumbs) so processing starts
    /// as soon as the file lands rather than waiting for the next
    /// scheduled detection / preview tick. Not emitted for
    /// `Updated` or `Unchanged` outcomes — the pipeline only fires
    /// on freshly-discovered files.
    FileAdded {
        job_id: i64,
        library_id: i64,
        media_file_id: i64,
    },
}

/// The scanner is event-source-agnostic: it takes a closure and the caller
/// (the server's event hub) decides what to do with each emission. Using
/// `Arc<dyn Fn>` keeps the scanner free of dependencies on the WS hub.
pub type ScanEmitter = Arc<dyn Fn(ScanEvent) + Send + Sync>;

/// No-op emitter — useful in tests and for callers who don't need events.
pub fn noop_emitter() -> ScanEmitter {
    Arc::new(|_| {})
}
