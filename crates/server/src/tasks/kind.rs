//! Kind metadata — the static description of every task kind known
//! to the binary.
//!
//! Some fields here are intentionally read only by future phases
//! (admin API surface for the activity / detail screens). Keeping
//! them on the struct now means the registry stays a single edit
//! point when those screens land.

#![allow(dead_code)]

/// How a kind behaves relative to admin control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskMode {
    /// Always runs on the on-add path (assuming dependencies hold).
    /// Admin can't disable; the only switch is removing the
    /// dependency it relies on (e.g. unsetting TMDB for the logos
    /// kind).
    Automatic,
    /// Gated by a `server_settings` boolean. Off by default. Both
    /// the on-add discovery pipeline and the safety-net sweep
    /// consult the gate before dispatching.
    Gated,
    /// Periodic kinds aren't per-file/per-item — they're scheduled
    /// refreshes (refresh_metadata, refresh_trending, backup_db).
    /// The `scheduled_tasks.enabled` column on the row gates these;
    /// the registry treats them as always-allowed at the gate layer
    /// since the scheduler already won't tick a disabled row.
    Periodic,
}

/// Shape of the work — how the kind is fanned out from upstream
/// events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskScope {
    /// One job per `media_files` row.
    PerFile,
    /// One job per `items` row (movie / show / season).
    PerItem,
    /// Server-wide; no fan-out target.
    Global,
}

/// Compile-time description of one task kind.
///
/// Same kind often has *two* names: the per-file/item job kind (what
/// gets written to `job_queue.kind`) and the scheduler sweep kind
/// (what gets written to `scheduled_tasks.kind`). The registry maps
/// either name back to the same metadata so gates apply consistently.
#[derive(Debug, Clone, Copy)]
pub struct KindMetadata {
    /// The name written to `job_queue.kind` for individual fan-out
    /// jobs. Always present.
    pub job_kind: &'static str,
    /// The name written to `scheduled_tasks.kind` for the safety-net
    /// sweep that fans out into per-file/item jobs. `None` for
    /// per-file kinds that have no sweep counterpart (rare —
    /// every pipeline kind ships with a sweep today).
    pub sweep_kind: Option<&'static str>,
    /// User-facing label. Surfaced in the admin tasks list.
    pub display_name: &'static str,
    pub mode: TaskMode,
    pub scope: TaskScope,
    /// The `server_settings` boolean that controls the gate. Must
    /// be `Some(...)` iff `mode == TaskMode::Gated`. Read by
    /// [`crate::tasks::gates::is_kind_allowed`].
    pub gate_setting_key: Option<&'static str>,
    /// Hard cap on simultaneous workers for this kind. The worker
    /// pool's global limit (number of tokio workers) bounds total
    /// concurrency; this prevents one kind from saturating the
    /// pool when its backlog sits at the queue head.
    ///
    /// Rules of thumb:
    ///   1 → CPU-bound (ffmpeg, FFT, OCR). One outstanding job at
    ///       a time keeps it from competing with live transcodes.
    ///   2 → Filesystem-bound, lightly serial (read_dir + ffprobe).
    ///   4 → Network-bound (TMDB, OpenSubtitles, OMDb).
    ///
    /// `0` means "no per-kind cap, fall back to worker-pool limit".
    pub concurrency: u32,
}

impl KindMetadata {
    /// True if either the job-kind or sweep-kind name matches `name`.
    /// Used by [`registry::find_kind`] so a caller doesn't need to
    /// know which side it's looking up from.
    pub fn matches(&self, name: &str) -> bool {
        self.job_kind == name || self.sweep_kind == Some(name)
    }
}
