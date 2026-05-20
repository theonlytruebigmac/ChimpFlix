//! /api/v1/play-state handlers.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use chimpflix_library::queries;
use chimpflix_library::{ListedItem, OnDeckResponse, PlayStateBatch, ScrobbleRequest};
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;
use crate::trakt_sync;

pub async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    Json(batch): Json<PlayStateBatch>,
) -> Result<StatusCode, ApiError> {
    if batch.updates.is_empty() {
        return Err(ApiError::validation("updates must not be empty"));
    }
    for (i, u) in batch.updates.iter().enumerate() {
        match (u.item_id, u.episode_id) {
            (Some(_), Some(_)) => {
                return Err(ApiError::validation(format!(
                    "update #{i}: only one of item_id or episode_id may be set",
                )));
            }
            (None, None) => {
                return Err(ApiError::validation(format!(
                    "update #{i}: one of item_id or episode_id is required",
                )));
            }
            _ => {}
        }
    }
    queries::apply_play_state_batch(&state.pool, user.id, batch).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct WatchedInput {
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    /// When set, marks every episode of the given show id. Atomic
    /// upsert across all `play_state` rows for the show. Mutually
    /// exclusive with `item_id` / `episode_id`.
    pub show_id: Option<i64>,
    pub watched: bool,
}

/// Explicit toggle for the Plex-style "Mark as watched / unwatched"
/// modal action. Distinct from scrobble (which is the implicit threshold
/// crossing) and from update (which writes a specific position).
pub async fn set_watched(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<WatchedInput>,
) -> Result<StatusCode, ApiError> {
    let n_set = [req.item_id, req.episode_id, req.show_id]
        .iter()
        .filter(|o| o.is_some())
        .count();
    if n_set == 0 {
        return Err(ApiError::validation(
            "one of item_id, episode_id, or show_id is required",
        ));
    }
    if n_set > 1 {
        return Err(ApiError::validation(
            "only one of item_id, episode_id, or show_id may be set",
        ));
    }

    if let Some(show_id) = req.show_id {
        let episode_ids = queries::set_all_episodes_watched_for_show(
            &state.pool,
            user.id,
            show_id,
            req.watched,
        )
        .await?;
        // Fan out Trakt history pushes — one per episode. Each call
        // is already fire-and-forget; spawning N concurrent tasks is
        // fine for typical season/show sizes. Skip on unwatched
        // (symmetry with the single-item path).
        if req.watched {
            for ep_id in episode_ids {
                push_watched_to_trakt(state.clone(), user.id, None, Some(ep_id));
            }
        }
        return Ok(StatusCode::NO_CONTENT);
    }

    queries::set_watched(&state.pool, user.id, req.item_id, req.episode_id, req.watched)
        .await?;
    // Fire-and-forget Trakt push when marking watched. Unwatch sync is
    // intentionally one-way for now — the symmetric remove endpoint
    // exists in the Trakt client but isn't wired here yet to avoid
    // accidentally clobbering history during testing.
    if req.watched {
        push_watched_to_trakt(state.clone(), user.id, req.item_id, req.episode_id);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn scrobble(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<ScrobbleRequest>,
) -> Result<StatusCode, ApiError> {
    if req.item_id.is_none() && req.episode_id.is_none() {
        return Err(ApiError::validation(
            "scrobble requires item_id or episode_id",
        ));
    }
    if req.item_id.is_some() && req.episode_id.is_some() {
        return Err(ApiError::validation(
            "scrobble must not have both item_id and episode_id",
        ));
    }
    queries::scrobble(&state.pool, user.id, req.item_id, req.episode_id).await?;
    // Same fire-and-forget Trakt push as set_watched — scrobble is
    // the threshold-crossing event the player emits at 90% playback.
    push_watched_to_trakt(state.clone(), user.id, req.item_id, req.episode_id);
    // Record a `complete` event for the admin Stats page. Paired with
    // the `start` event from POST /stream/sessions, this gives the
    // dashboard a started-vs-completed ratio per user / item without
    // any new client wiring. Fire-and-forget — stats can't gate the
    // scrobble write that drives the user's own Continue Watching.
    spawn_event(
        state.clone(),
        user.id,
        "complete",
        req.item_id,
        req.episode_id,
        None,
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct PlaybackEventInputDto {
    pub kind: String,
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub position_ms: Option<i64>,
}

/// `POST /v1/play-state/event` — accept fine-grained player events
/// (pause, resume) so the admin Stats page can show engagement
/// signals beyond just "started" and "completed". Whitelisted to
/// `pause` / `resume` only — `start` and `complete` are written
/// server-side from their authoritative paths (stream-session POST,
/// scrobble) so they can't be spoofed by a hostile client.
///
/// Fire-and-forget: any signed-in user can emit, the response is 204
/// regardless of insert success, so a flaky stats DB never disrupts
/// playback.
pub async fn event(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<PlaybackEventInputDto>,
) -> Result<StatusCode, ApiError> {
    let kind = req.kind.trim();
    if !matches!(kind, "pause" | "resume") {
        return Err(ApiError::validation(
            "kind must be one of: pause, resume",
        ));
    }
    if req.item_id.is_none() && req.episode_id.is_none() {
        return Err(ApiError::validation(
            "event requires item_id or episode_id",
        ));
    }
    if req.item_id.is_some() && req.episode_id.is_some() {
        return Err(ApiError::validation(
            "event must not have both item_id and episode_id",
        ));
    }
    // Use Box::leak alternative — the spawn helper accepts &str so we
    // copy into a leaked-friendly &'static. Simpler: clone the kind
    // and pass owned string into the spawned task via a closure that
    // borrows from the captured String. Implemented inline:
    let pool = state.pool.clone();
    let user_id = user.id;
    let item_id = req.item_id;
    let episode_id = req.episode_id;
    let position_ms = req.position_ms;
    let kind_owned = kind.to_string();
    tokio::spawn(async move {
        let ev = queries::PlaybackEventInput {
            item_id,
            episode_id,
            position_ms,
            ..queries::PlaybackEventInput::new(user_id, kind_owned.as_str())
        };
        if let Err(e) = queries::record_playback_event(&pool, ev).await {
            tracing::warn!(
                error = %format!("{e:#}"),
                kind = %kind_owned,
                "record playback event",
            );
        }
    });
    Ok(StatusCode::NO_CONTENT)
}

/// Shared helper that fires `record_playback_event` from a spawned
/// task. Used by `scrobble` (complete) and could be reused by other
/// server-emitted event sites — keeps the spawn + warn pattern in one
/// place.
fn spawn_event(
    state: AppState,
    user_id: i64,
    event_type: &'static str,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    position_ms: Option<i64>,
) {
    tokio::spawn(async move {
        let ev = queries::PlaybackEventInput {
            item_id,
            episode_id,
            position_ms,
            ..queries::PlaybackEventInput::new(user_id, event_type)
        };
        if let Err(e) = queries::record_playback_event(&state.pool, ev).await {
            tracing::warn!(
                error = %format!("{e:#}"),
                event_type,
                "record playback event",
            );
        }
    });
}

fn push_watched_to_trakt(
    state: AppState,
    user_id: i64,
    item_id: Option<i64>,
    episode_id: Option<i64>,
) {
    tokio::spawn(async move {
        let now_iso = trakt_sync::epoch_ms_to_iso(chimpflix_common::now_ms());
        let event = if let Some(id) = item_id {
            let Some(tmdb_id) = trakt_sync::item_tmdb_id(&state.pool, id).await else {
                return;
            };
            chimpflix_metadata::HistoryPush::Movie {
                tmdb_id,
                watched_at: now_iso,
            }
        } else if let Some(id) = episode_id {
            let coords = trakt_sync::episode_trakt_coords(&state.pool, id).await.ok().flatten();
            let Some((tmdb_show_id, season, episode)) = coords else {
                return;
            };
            chimpflix_metadata::HistoryPush::Episode {
                tmdb_show_id,
                season,
                episode,
                watched_at: now_iso,
            }
        } else {
            return;
        };
        trakt_sync::push_history_event(&state, user_id, event).await;
    });
}

#[derive(Debug, Serialize)]
pub struct PlayStateConfigResponse {
    /// Threshold (1–99) at which the player auto-scrobbles a session
    /// as watched. Mirrors the server's source of truth so the player
    /// stays in sync after the operator changes it without needing a
    /// rebuild. Same value gates the Continue Watching rail's upper
    /// bound — see `queries::on_deck`.
    pub played_threshold_pct: i64,
    /// One of `threshold_pct` / `first_credits_marker` /
    /// `earliest_of_both`. Drives the player's auto-scrobble decision
    /// alongside `played_threshold_pct` — when `first_credits_marker`
    /// is selected, the player scrobbles when the first credits
    /// marker's start_ms is reached. `earliest_of_both` scrobbles at
    /// whichever lands first.
    pub completion_behaviour: String,
    /// Days an item stays badged as "Recently Added" on Card. 0 means
    /// "never badge". Card.tsx reads this via `useRecentlyAddedDays()`
    /// so an admin change takes effect on the next config-poll without
    /// a client rebuild.
    pub recently_added_days: i64,
}

/// `GET /v1/play-state/config` — return the small subset of playback
/// settings the player needs. Auth-required (any signed-in user) but
/// not admin — these aren't secret, just need to be tied to a session
/// so anonymous probes don't fingerprint deployments.
pub async fn config(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<PlayStateConfigResponse>, ApiError> {
    let s = state.settings.read().await;
    Ok(Json(PlayStateConfigResponse {
        played_threshold_pct: s.video_played_threshold_pct,
        completion_behaviour: s.video_completion_behaviour.clone(),
        recently_added_days: s.recently_added_days,
    }))
}

pub async fn on_deck(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<OnDeckResponse>, ApiError> {
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    // Pull the operator's CW dials from the settings cache. The
    // cache is the canonical reader-side source (kept in lockstep
    // with the DB by the /admin/settings PATCH handler), so no
    // round-trip is needed.
    let options = {
        let s = state.settings.read().await;
        // Note: the on-deck filter uses the percentage threshold only.
        // When completion_behaviour is `first_credits_marker`, the
        // player scrobbles earlier (at the credits start), which
        // already drops the tile off the rail via `watched=true`. The
        // pct fallback here is the defensive backstop; we don't need
        // to thread the marker-based filter into the SQL.
        chimpflix_library::OnDeckOptions {
            max_items: s.continue_watching_max_items,
            played_threshold_pct: s.video_played_threshold_pct,
            max_age_weeks: s.continue_watching_max_age_weeks,
            include_premieres: s.continue_watching_include_premieres,
        }
    };
    let resp = queries::on_deck(&state.pool, user.id, acc.as_deref(), options).await?;
    Ok(Json(resp))
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct HistoryResponse {
    pub items: Vec<ListedItem>,
}

pub async fn history(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, ApiError> {
    let limit = q.limit.unwrap_or(60).clamp(1, 200);
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let items = queries::list_watch_history(&state.pool, user.id, limit, acc.as_deref())
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(HistoryResponse { items }))
}
