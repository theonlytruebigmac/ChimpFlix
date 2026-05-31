//! `trakt_push_user_history` — push one user's locally-watched history up
//! to Trakt. Payload: `{ "user_id": i64, "since_ms": i64? }`.
//!
//! This is the durable, periodic-sweep form of the Trakt PUSH direction,
//! completing the two-way sync. The per-event hooks in `trakt_sync`
//! (`push_history_event`) are fire-and-forget and lossy on a transient
//! Trakt failure; this daily `trakt_push` sweep enqueues one job per
//! linked user as the durable backstop that catches anything the live
//! hooks missed. `since_ms = None` bulk-pushes the user's full local
//! watch history; Trakt dedupes server-side, so re-pushing is harmless.
//!
//! Unlike the pull job, a failure here returns `Err` so the job queue
//! retries (per `max_attempts` with backoff) — the push is the lossy
//! direction we most want to make durable.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "trakt_push_user_history";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub user_id: i64,
    /// Only push history watched at/after this epoch-ms. `None` pushes the
    /// user's entire local watch history (the periodic sweep uses `None`).
    #[serde(default)]
    pub since_ms: Option<i64>,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { user_id, since_ms } = serde_json::from_value(payload).context("invalid payload")?;

    match crate::trakt_sync::bulk_push_user_history(&state, user_id, since_ms).await {
        Ok((movies, episodes)) => {
            info!(user_id, movies, episodes, since_ms, "trakt_push_user_history ok");
            Ok(())
        }
        Err(e) => {
            warn!(user_id, error = %format!("{e:#}"), "trakt_push_user_history failed; will retry");
            Err(e)
        }
    }
}
