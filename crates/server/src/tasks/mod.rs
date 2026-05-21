//! Scheduled-task registry, gating, and metadata.
//!
//! This module is the source of truth for what background kinds the
//! binary knows how to run, how they classify (automatic vs gated vs
//! periodic), and whether they're allowed to run *right now* given
//! current server settings.
//!
//! Two callers consume the registry today:
//!
//!  1. [`crate::jobs::pipeline::enqueue_pipeline`] — when the scanner
//!     emits `FileAdded`, the discovery fan-out asks the registry
//!     which kinds to skip for this file.
//!
//!  2. [`crate::scheduler`] — when a `scheduled_tasks` row's
//!     `next_run_at` lapses, the dispatch match asks the registry
//!     before running the sweep handler.
//!
//! Both paths consult the *same* gate function, fixing the legacy bug
//! where toggling `analyze_loudness` in admin would stop the weekly
//! sweep but not the on-add path. See `docs/pipelines/backend-plan.md`
//! §2 for the design.

pub mod gates;
pub mod kind;
pub mod metrics;
pub mod registry;

// Re-exports for the common gate-checking call site. Other consumers
// (admin API, future activity surface) reach for the submodules
// directly.
pub use gates::{GateState, is_kind_allowed};
#[allow(unused_imports)]
pub use kind::{KindMetadata, TaskMode, TaskScope};
#[allow(unused_imports)]
pub use registry::find_kind;
