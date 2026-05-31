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
//!
//! Before the delta pulls, the job runs an AUTHORITATIVE watched-state
//! seed (`/sync/watched/{movies,shows}` → mirror → reconcile) on the FIRST
//! sync (history cursor still NULL) or when `force` is set (the manual
//! "Sync now" button). `/sync/history` alone is a dated event log that can
//! omit titles marked watched outside ChimpFlix — so a show at "91%
//! watched" on Trakt may carry no history events. The watched-state
//! snapshot is what powers Trakt's own badges, so seeding from it catches
//! everything. The seed is deliberately NOT gated by `check_last_activities`
//! (which compares Trakt-side activity, not whether our mirror is complete).

use anyhow::{Context, Result};
use chimpflix_common::now_ms;
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "trakt_pull_user";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub user_id: i64,
    /// Force a full sync regardless of the `last_activities` rollup: run
    /// the authoritative watched-state seed AND every delta pull. Set by
    /// the manual "Sync now" button; the periodic sweep omits it (defaults
    /// false) so it stays cheap when nothing changed on Trakt.
    #[serde(default)]
    pub force: bool,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { user_id, force } = serde_json::from_value(payload).context("invalid payload")?;

    // Authoritative watched-state seed. Runs on the first sync (history
    // cursor still NULL → we've never captured the full watched state) or
    // on an explicit forced sync. After a successful seed we stamp the
    // history cursor so the periodic sweep doesn't re-seed every hour — the
    // delta `/sync/history` pull then carries new watches from here on.
    let cursor = queries::get_trakt_last_synced(&state.pool, user_id)
        .await
        .unwrap_or(None);
    if force || cursor.is_none() {
        match crate::trakt_sync::pull_user_watched(&state, user_id).await {
            Ok((movies, episodes)) => {
                info!(user_id, movies, episodes, force, "trakt_pull_user: watched-state seed applied");
                if let Err(e) = queries::update_trakt_last_synced(&state.pool, user_id, now_ms()).await {
                    warn!(user_id, error = %format!("{e:#}"), "failed to stamp trakt cursor after watched seed");
                }
            }
            Err(e) => warn!(user_id, error = %format!("{e:#}"), "trakt watched-state seed failed"),
        }
    }

    // Cheap rollup check — skip the three delta pull round-trips when
    // /sync/last_activities reports nothing changed since our last visit.
    // A forced sync bypasses the gate (the user asked for it explicitly).
    let should_pull = if force {
        true
    } else {
        match crate::trakt_sync::check_last_activities(&state, user_id).await {
            Ok(b) => b,
            Err(e) => {
                warn!(user_id, error = %format!("{e:#}"), "trakt last_activities check errored; running full pull");
                true
            }
        }
    };
    if !should_pull {
        info!(user_id, "trakt_pull_user: no change since last sync; skipping delta pulls");
        return Ok(());
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
