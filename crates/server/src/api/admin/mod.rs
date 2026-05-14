//! Owner-only `/admin/*` surface.
//!
//! Phase-1 contents: backup snapshot, server settings, audit log.
//!
//! Every handler in this tree requires the `OwnerAuth` extractor — there is
//! no separate middleware layer; the extractor is type-checked into each
//! handler signature. Mutating handlers append to `audit_log` via the
//! helper in this module.

pub mod agents;
pub mod audit;
pub mod backup;
pub mod dashboard;
pub mod maintenance;
pub mod network;
pub mod optimized;
pub mod settings;
pub mod tasks;
pub mod transcoder;
pub mod users;
pub mod webhooks;

use chimpflix_library::{NewAuditEntry, queries};

use crate::state::AppState;

/// Append an admin audit log entry. Logs but does not fail the parent
/// request on insert errors — audit visibility shouldn't gate the action
/// it records.
pub async fn audit_log(state: &AppState, entry: NewAuditEntry) {
    match queries::append_audit(&state.pool, entry).await {
        Ok(id) => tracing::debug!(audit_id = id, "audit entry"),
        Err(e) => tracing::error!(error = %format!("{e:#}"), "failed to write audit_log"),
    }
}
