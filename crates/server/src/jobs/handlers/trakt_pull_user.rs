//! `trakt_pull_user` — pull one user's Trakt watch state into the local
//! library. Payload: `{ "user_id": i64 }`.
//!
//! This is the durable, per-user form of the Trakt PULL direction. The
//! hourly `trakt_pull` scheduled sweep enqueues one of these jobs per
//! linked user (see `scheduler::trakt_pull_task`) instead of doing the
//! work inline on the scheduler tick. Benefits: each user's pull retries
//! independently, a slow/offline Trakt for one user doesn't block the
//! others, and the work rides the normal worker-pool concurrency cap.
//!
//! Sub-steps (history → playback → watchlist) are best-effort: a failure
//! in one is logged and the others still run, and the job returns `Ok` so
//! it isn't retried in a tight loop — the next hourly sweep re-enqueues.
//! Idempotency is handled downstream (Trakt dedupes; local upserts are
//! by-id), and `check_last_activities` short-circuits users with nothing
//! new since the last visit.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "trakt_pull_user";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub user_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { user_id } = serde_json::from_value(payload).context("invalid payload")?;

    // Cheap rollup check — skip the three pull round-trips when
    // /sync/last_activities reports nothing changed since our last visit.
    match crate::trakt_sync::check_last_activities(&state, user_id).await {
        Ok(false) => {
            info!(user_id, "trakt_pull_user: no change since last sync; skipping");
            return Ok(());
        }
        Ok(true) => {}
        Err(e) => {
            warn!(user_id, error = %format!("{e:#}"), "trakt last_activities check errored; running full pull");
        }
    }

    match crate::trakt_sync::pull_user_history(&state, user_id).await {
        Ok((movies, episodes)) => {
            info!(user_id, movies, episodes, "trakt_pull_user: history applied")
        }
        Err(e) => warn!(user_id, error = %format!("{e:#}"), "trakt pull history failed"),
    }
    match crate::trakt_sync::pull_user_playback(&state, user_id).await {
        Ok(applied) => info!(user_id, applied, "trakt_pull_user: playback applied"),
        Err(e) => warn!(user_id, error = %format!("{e:#}"), "trakt pull playback failed"),
    }
    match crate::trakt_sync::pull_user_watchlist(&state, user_id).await {
        Ok((added, removed)) => {
            info!(user_id, added, removed, "trakt_pull_user: watchlist reconciled")
        }
        Err(e) => warn!(user_id, error = %format!("{e:#}"), "trakt pull watchlist failed"),
    }

    Ok(())
}
