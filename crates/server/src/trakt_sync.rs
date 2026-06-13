//! Trakt sync helpers shared by the HTTP handlers and the scheduled
//! pull task.
//!
//! Two responsibilities:
//!   - `with_user_client` resolves the current per-user access token,
//!     proactively refreshes it when it's within the refresh window,
//!     and yields a closure that takes the access_token. Centralises
//!     the "refresh on 401" pattern so handlers don't repeat it.
//!   - `push_history_event` and `push_rating_event` are the fire-and-
//!     forget hooks called from `play_state` and the ratings API.

use std::sync::Arc;

use anyhow::Result;
use chimpflix_common::now_ms;
use chimpflix_library::queries;
use chimpflix_metadata::{
    HistoryPush, RatingPush, ScrobbleAction, ScrobblePush, TraktClient, TraktIdSet, WatchlistPush,
};
use chrono::{TimeZone, Utc};
use sqlx::Row;
use sqlx::SqlitePool;
use tracing::warn;

use crate::state::AppState;

/// Refresh proactively when the token has less than this much life
/// left. 5 minutes is well above any reasonable request latency.
const REFRESH_LEAD_MS: i64 = 5 * 60 * 1000;

/// Resolve the user's access token, refreshing it through the Trakt
/// app's client_id/secret if necessary, then run `f` with the
/// resulting token string. Returns `None` (and doesn't call `f`) when
/// the user has no Trakt link or the operator hasn't configured the
/// app credentials.
pub async fn with_user_client<F, Fut, T>(state: &AppState, user_id: i64, f: F) -> Result<Option<T>>
where
    F: FnOnce(Arc<TraktClient>, String) -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let Some(client) = state.trakt_snapshot().await else {
        return Ok(None);
    };
    let Some(mut tokens) = queries::get_trakt_tokens(&state.pool, &state.vault, user_id).await?
    else {
        return Ok(None);
    };
    if tokens.expires_at - now_ms() < REFRESH_LEAD_MS {
        // Serialize concurrent refreshes for the same user. Without
        // the per-user mutex, the play_state hook, the scheduled
        // trakt_pull task, and a manual UI ping can all observe an
        // about-to-expire token at the same time and each fire
        // `POST /oauth/token`. Trakt mints fresh pairs for each
        // call; last-writer-wins on our `upsert` can stash an older
        // refresh_token and break subsequent refreshes. Hold the
        // mutex across read-current-tokens → refresh → upsert so
        // only one of the racing callers actually hits Trakt; the
        // rest re-read the freshly-upserted token and skip the
        // network round-trip entirely.
        let lock = state.trakt_refresh_lock(user_id).await;
        let _guard = lock.lock().await;
        // Re-read tokens under the lock. A concurrent caller that
        // got here first will have already refreshed; we'd be
        // wasting a round-trip otherwise.
        if let Some(fresh) = queries::get_trakt_tokens(&state.pool, &state.vault, user_id).await? {
            tokens = fresh;
        }
        if tokens.expires_at - now_ms() < REFRESH_LEAD_MS {
            match client.refresh_token(&tokens.refresh_token).await {
                Ok(pair) => {
                    let expires_at = now_ms() + pair.expires_in * 1000;
                    queries::upsert_trakt_tokens(
                        &state.pool,
                        &state.vault,
                        user_id,
                        &pair.access_token,
                        &pair.refresh_token,
                        pair.scope.as_deref(),
                        expires_at,
                    )
                    .await?;
                    tokens.access_token = pair.access_token;
                }
                Err(e) => {
                    warn!(user_id, error = %format!("{e:#}"), "Trakt refresh failed; using stale token");
                }
            }
        }
    }
    let client_arc = Arc::new(client);
    let access_token_for_call = tokens.access_token.clone();
    let outcome = f(Arc::clone(&client_arc), access_token_for_call).await;
    // Reactive 401 recovery. The proactive expires_at window above
    // catches the common "token aged out" case, but Trakt can also
    // rotate / invalidate a refresh token server-side (security
    // event, account-level revocation). In that case our local
    // `expires_at` still looks fresh but every call comes back 401.
    // Trigger a refresh here so the *next* call uses a freshly-
    // minted token; we don't transparently retry the current call
    // because the closure has already been consumed (FnOnce). The
    // original error still propagates so the user sees an
    // actionable message this time. Subsequent retries succeed
    // without operator intervention.
    if let Err(err) = &outcome {
        if is_unauthorized_error(err) {
            let lock = state.trakt_refresh_lock(user_id).await;
            let _guard = lock.lock().await;
            // Re-read tokens — a concurrent caller may have already
            // refreshed in response to its own 401.
            if let Ok(Some(latest)) =
                queries::get_trakt_tokens(&state.pool, &state.vault, user_id).await
            {
                if latest.access_token == tokens.access_token {
                    match client_arc.refresh_token(&latest.refresh_token).await {
                        Ok(pair) => {
                            let expires_at = now_ms() + pair.expires_in * 1000;
                            if let Err(e) = queries::upsert_trakt_tokens(
                                &state.pool,
                                &state.vault,
                                user_id,
                                &pair.access_token,
                                &pair.refresh_token,
                                pair.scope.as_deref(),
                                expires_at,
                            )
                            .await
                            {
                                warn!(user_id, error = %format!("{e:#}"), "Trakt 401-triggered refresh succeeded but token upsert failed");
                            }
                        }
                        Err(e) => {
                            warn!(
                                user_id,
                                error = %format!("{e:#}"),
                                "Trakt 401 received and refresh attempt also failed; user may need to unlink and relink"
                            );
                        }
                    }
                }
            }
        }
    }
    Ok(Some(outcome?))
}

/// Detect a Trakt 401 in an error chain. The metadata client's
/// `api_error` returns "… returned 401 —" for UNAUTHORIZED responses;
/// match that specific phrase to avoid false-positives on error bodies
/// or IDs that incidentally contain the digit sequence "401".
fn is_unauthorized_error(err: &anyhow::Error) -> bool {
    let chain = format!("{err:#}");
    chain.contains("returned 401")
}

/// Detect a Trakt 404 in an error chain. On `/scrobble/*` a 404 means Trakt
/// could not resolve the title from the ids + season/episode we sent. For
/// anime this is an EXPECTED metadata gap — local libraries commonly number
/// seasons TMDB-style, which doesn't line up with Trakt's TVDB-based season
/// structure, so an episode that exists locally has no counterpart on Trakt.
/// It's best-effort and doesn't affect local playback or watched-state, so
/// the caller logs it at DEBUG rather than spamming WARN every start/stop.
fn is_not_found_error(err: &anyhow::Error) -> bool {
    format!("{err:#}").contains("returned 404")
}

/// Push a single watched event to Trakt for every linked user (used by
/// the play_state hooks). Best-effort: per-user failures are warned and
/// don't bubble up to the caller. INFO-logs on success so an operator
/// tailing the log can see pushes hitting Trakt (previously this was
/// completely silent — "did my mark-watched actually push?" had no
/// answer without poking Trakt directly).
pub async fn push_history_event(state: &AppState, user_id: i64, event: HistoryPush) {
    let label = describe_history_event(&event);
    let result = with_user_client(state, user_id, |client, token| async move {
        client.push_history(&token, &[event]).await
    })
    .await;
    match result {
        Ok(Some(())) => {
            tracing::info!(user_id, event = %label, "Trakt push_history ok");
        }
        Ok(None) => {
            tracing::info!(
                user_id,
                event = %label,
                "Trakt push_history skipped: user not linked",
            );
        }
        Err(e) => {
            warn!(user_id, event = %label, error = %format!("{e:#}"), "Trakt push_history failed");
        }
    }
}

/// Mirror of [`push_history_event`] for the un-watch path. Posts to
/// Trakt's `/sync/history/remove` so the user's Trakt history reflects
/// the local un-mark. Best-effort.
pub async fn remove_history_event(state: &AppState, user_id: i64, event: HistoryPush) {
    let label = describe_history_event(&event);
    let result = with_user_client(state, user_id, |client, token| async move {
        client.remove_history(&token, &[event]).await
    })
    .await;
    match result {
        Ok(Some(())) => {
            tracing::info!(user_id, event = %label, "Trakt remove_history ok");
        }
        Ok(None) => {}
        Err(e) => {
            warn!(user_id, event = %label, error = %format!("{e:#}"), "Trakt remove_history failed");
        }
    }
}

/// Fire a single scrobble lifecycle event (start / pause / stop) for
/// the user's current playback session. Resolves the media-file owner
/// to Trakt-keyed (tmdb_id) or (show_tmdb_id, season, episode) coords,
/// reads the user's latest stored position to compute progress, and
/// POSTs the right `/scrobble/{action}` endpoint.
///
/// Best-effort: anything that goes wrong is warn-logged so a Trakt
/// outage can't break local playback. `/scrobble/stop` at progress
/// ≥ 80% additionally writes the entry to Trakt's history, so an
/// uninterrupted watch-through doesn't need a separate `/sync/history`
/// call.
pub async fn scrobble_event(
    state: &AppState,
    user_id: i64,
    action: ScrobbleAction,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    duration_ms: Option<i64>,
) {
    let progress = current_progress_pct(&state.pool, user_id, item_id, episode_id, duration_ms)
        .await
        .unwrap_or(0.0);
    // Trakt's API rejects /scrobble/stop and /scrobble/pause at progress
    // below 1.0% with `422 — Progress should be at least 1.0% to pause.`
    // This guard avoids the cascade of WARN logs (and a useless HTTP
    // round-trip per session) when a user opens a title and immediately
    // closes the player, which is a common interaction. /scrobble/start
    // is allowed at 0% — Trakt uses it to set the "now watching" banner
    // — so the skip only applies to the terminal actions.
    if matches!(action, ScrobbleAction::Stop | ScrobbleAction::Pause) && progress < 1.0 {
        tracing::debug!(
            user_id,
            progress = format!("{progress:.2}"),
            action = ?action,
            "Trakt scrobble skipped — below 1% threshold"
        );
        return;
    }
    let Some(event) = build_scrobble_event(&state.pool, item_id, episode_id, progress).await
    else {
        return;
    };
    let label = describe_scrobble_event(action, &event);
    let result = with_user_client(state, user_id, |client, token| async move {
        client.scrobble(&token, action, event).await
    })
    .await;
    match result {
        Ok(Some(())) => {
            tracing::info!(user_id, event = %label, "Trakt scrobble ok");
        }
        Ok(None) => {}
        Err(e) if is_not_found_error(&e) => {
            // Trakt can't match this title (commonly anime whose local
            // TMDB season/episode numbering doesn't line up with Trakt's
            // TVDB-based structure). Best-effort — playback + watched-state
            // are unaffected — so keep it out of the WARN stream.
            tracing::debug!(
                user_id,
                event = %label,
                "Trakt scrobble skipped — title not found on Trakt (likely anime season/episode numbering mismatch)"
            );
        }
        Err(e) => {
            warn!(user_id, event = %label, error = %format!("{e:#}"), "Trakt scrobble failed");
        }
    }
}

async fn build_scrobble_event(
    pool: &SqlitePool,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    progress: f64,
) -> Option<ScrobblePush> {
    if let Some(id) = item_id {
        let ids = item_trakt_ids(pool, id).await?;
        Some(ScrobblePush::Movie { ids, progress })
    } else if let Some(id) = episode_id {
        let coords = episode_trakt_coords(pool, id).await.ok().flatten()?;
        Some(ScrobblePush::Episode {
            show_ids: coords.show_ids,
            episode_ids: coords.episode_ids,
            season: coords.season,
            episode: coords.episode,
            progress,
        })
    } else {
        None
    }
}

fn describe_scrobble_event(action: ScrobbleAction, event: &ScrobblePush) -> String {
    let action_label = match action {
        ScrobbleAction::Start => "start",
        ScrobbleAction::Pause => "pause",
        ScrobbleAction::Stop => "stop",
    };
    match event {
        ScrobblePush::Movie { ids, progress } => {
            format!("{action_label} movie {} @ {progress:.1}%", describe_ids(ids))
        }
        ScrobblePush::Episode {
            show_ids,
            episode_ids,
            season,
            episode,
            progress,
        } => {
            // Surface the id we actually resolve by so a 404 in the log is
            // diagnosable (episode-id path vs show+season/number fallback).
            let target = if episode_ids.is_empty() {
                describe_ids(show_ids)
            } else {
                format!("ep {}", describe_ids(episode_ids))
            };
            format!("{action_label} show {target} S{season:02}E{episode:02} @ {progress:.1}%")
        }
    }
}

fn describe_ids(ids: &TraktIdSet) -> String {
    if let Some(t) = ids.tmdb {
        format!("tmdb={t}")
    } else if let Some(t) = ids.tvdb {
        format!("tvdb={t}")
    } else if let Some(i) = ids.imdb.as_deref() {
        format!("imdb={i}")
    } else {
        "ids=?".into()
    }
}

/// Read the user's most recent stored position for the given item or
/// episode and express it as a 0–100 percentage against `duration_ms`.
/// Returns `None` when duration is unknown or zero — the caller falls
/// back to 0% (a fresh play hasn't accrued progress yet anyway).
async fn current_progress_pct(
    pool: &SqlitePool,
    user_id: i64,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    duration_ms: Option<i64>,
) -> Option<f64> {
    let dur = duration_ms.filter(|d| *d > 0)?;
    let position_ms: i64 = if let Some(id) = item_id {
        sqlx::query_scalar(
            "SELECT COALESCE(position_ms, 0) FROM play_state
             WHERE user_id = ? AND item_id = ? LIMIT 1",
        )
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
    } else if let Some(id) = episode_id {
        sqlx::query_scalar(
            "SELECT COALESCE(position_ms, 0) FROM play_state
             WHERE user_id = ? AND episode_id = ? LIMIT 1",
        )
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or(0)
    } else {
        return None;
    };
    Some((position_ms as f64 / dur as f64 * 100.0).clamp(0.0, 100.0))
}

fn describe_history_event(event: &HistoryPush) -> String {
    match event {
        HistoryPush::Movie { ids, .. } => format!("movie {}", describe_ids(ids)),
        HistoryPush::Episode {
            show_ids,
            season,
            episode,
            ..
        } => format!(
            "show {} S{season:02}E{episode:02}",
            describe_ids(show_ids)
        ),
    }
}

/// Push a single My-List add to the user's Trakt watchlist.
/// Best-effort: a missing tmdb_id or Trakt outage just leaves the
/// local list ahead of the remote until the next sync_now reconcile.
pub async fn push_watchlist_event(state: &AppState, user_id: i64, item_id: i64) {
    let Some(event) = build_watchlist_event(&state.pool, item_id).await else {
        return;
    };
    let label = describe_watchlist_event(&event);
    let result = with_user_client(state, user_id, |client, token| async move {
        client.push_watchlist(&token, &[event]).await
    })
    .await;
    match result {
        Ok(Some(())) => {
            tracing::info!(user_id, event = %label, "Trakt push_watchlist ok");
        }
        Ok(None) => {}
        Err(e) => {
            warn!(user_id, event = %label, error = %format!("{e:#}"), "Trakt push_watchlist failed");
        }
    }
}

/// Mirror of [`push_watchlist_event`] for the My-List remove path.
pub async fn remove_watchlist_event(state: &AppState, user_id: i64, item_id: i64) {
    let Some(event) = build_watchlist_event(&state.pool, item_id).await else {
        return;
    };
    let label = describe_watchlist_event(&event);
    let result = with_user_client(state, user_id, |client, token| async move {
        client.remove_watchlist(&token, &[event]).await
    })
    .await;
    match result {
        Ok(Some(())) => {
            tracing::info!(user_id, event = %label, "Trakt remove_watchlist ok");
        }
        Ok(None) => {}
        Err(e) => {
            warn!(user_id, event = %label, error = %format!("{e:#}"), "Trakt remove_watchlist failed");
        }
    }
}

/// Resolve an `items.id` into a [`WatchlistPush`] variant by reading
/// the row's `kind` ('movie' or 'tv') + `tmdb_id`. Anime-only matches
/// (no tmdb_id) skip with a single info log rather than an error
/// because the same item won't ever sync regardless of how often the
/// user clicks add/remove.
async fn build_watchlist_event(pool: &SqlitePool, item_id: i64) -> Option<WatchlistPush> {
    let row = sqlx::query(
        "SELECT kind, tmdb_id FROM items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()?;
    let kind: String = row.try_get("kind").ok()?;
    let tmdb_id: Option<i64> = row.try_get("tmdb_id").ok().flatten();
    let Some(tmdb_id) = tmdb_id else {
        tracing::info!(item_id, "Trakt watchlist skipped: item has no tmdb_id");
        return None;
    };
    match kind.as_str() {
        "movie" => Some(WatchlistPush::Movie { tmdb_id }),
        "tv" => Some(WatchlistPush::Show { tmdb_id }),
        _ => None,
    }
}

fn describe_watchlist_event(event: &WatchlistPush) -> String {
    match event {
        WatchlistPush::Movie { tmdb_id } => format!("movie tmdb={tmdb_id}"),
        WatchlistPush::Show { tmdb_id } => format!("show tmdb={tmdb_id}"),
    }
}

/// Reconcile the user's Trakt watchlist with their local My List.
/// Two-way diff against the previously-seen snapshot in
/// `user_trakt_watchlist_state`:
///
///   - In Trakt now, not in snapshot → user added on Trakt → add to
///     local My List (idempotent against existing rows).
///   - In snapshot, not in Trakt now → user removed on Trakt → remove
///     from local My List.
///
/// Critical: a fresh-link user starts with an empty snapshot, so the
/// first pull is all-adds, no removes. This matches the spirit of the
/// previous additive-only behaviour for first-time syncs while still
/// honouring Trakt-side removes on subsequent pulls.
///
/// Returns `(added, removed)`.
pub async fn pull_user_watchlist(state: &AppState, user_id: i64) -> Result<(usize, usize)> {
    let Some((current_movies, current_shows)) =
        with_user_client(state, user_id, |client, token| async move {
            let entries = client.pull_watchlist(&token).await?;
            let mut movies = Vec::new();
            let mut shows = Vec::new();
            for entry in entries {
                match entry.kind.as_str() {
                    "movie" => {
                        let Some(m) = entry.movie else { continue };
                        if let Some(id) = m.ids.tmdb {
                            movies.push(id);
                        }
                    }
                    "show" => {
                        let Some(s) = entry.show else { continue };
                        if let Some(id) = s.ids.tmdb {
                            shows.push(id);
                        }
                    }
                    // season / episode entries don't map to My List
                    _ => {}
                }
            }
            Ok::<(Vec<i64>, Vec<i64>), anyhow::Error>((movies, shows))
        })
        .await?
    else {
        return Ok((0, 0));
    };

    let current_movie_set: std::collections::HashSet<i64> =
        current_movies.iter().copied().collect();
    let current_show_set: std::collections::HashSet<i64> =
        current_shows.iter().copied().collect();
    let (prev_movies, prev_shows) =
        queries::list_trakt_watchlist_state(&state.pool, user_id).await?;

    let mut added = 0usize;
    let mut removed = 0usize;

    // Adds: in current Trakt set but not in our snapshot.
    for &tmdb_id in current_movie_set.difference(&prev_movies) {
        if let Some(item_id) = find_local_item(&state.pool, tmdb_id, "movie").await {
            if !is_in_my_list(&state.pool, user_id, item_id).await
                && queries::add_to_my_list(&state.pool, user_id, item_id)
                    .await
                    .is_ok()
            {
                added += 1;
            }
        }
    }
    for &tmdb_id in current_show_set.difference(&prev_shows) {
        if let Some(item_id) = find_local_item(&state.pool, tmdb_id, "tv").await {
            if !is_in_my_list(&state.pool, user_id, item_id).await
                && queries::add_to_my_list(&state.pool, user_id, item_id)
                    .await
                    .is_ok()
            {
                added += 1;
            }
        }
    }

    // Removes: in our snapshot but not in current Trakt set. Only
    // touch items that are *currently* in the user's My List —
    // anything they've already manually removed locally stays gone
    // (no need to re-fire the local-remove path).
    for &tmdb_id in prev_movies.difference(&current_movie_set) {
        if let Some(item_id) = find_local_item(&state.pool, tmdb_id, "movie").await {
            if is_in_my_list(&state.pool, user_id, item_id).await
                && queries::remove_from_my_list(&state.pool, user_id, item_id)
                    .await
                    .is_ok()
            {
                removed += 1;
            }
        }
    }
    for &tmdb_id in prev_shows.difference(&current_show_set) {
        if let Some(item_id) = find_local_item(&state.pool, tmdb_id, "tv").await {
            if is_in_my_list(&state.pool, user_id, item_id).await
                && queries::remove_from_my_list(&state.pool, user_id, item_id)
                    .await
                    .is_ok()
            {
                removed += 1;
            }
        }
    }

    queries::replace_trakt_watchlist_state(
        &state.pool,
        user_id,
        &current_movies,
        &current_shows,
    )
    .await?;

    Ok((added, removed))
}

async fn find_local_item(pool: &SqlitePool, tmdb_id: i64, kind: &str) -> Option<i64> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM items WHERE tmdb_id = ? AND kind = ? LIMIT 1")
        .bind(tmdb_id)
        .bind(kind)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn is_in_my_list(pool: &SqlitePool, user_id: i64, item_id: i64) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT 1 FROM user_my_list WHERE user_id = ? AND item_id = ? LIMIT 1",
    )
    .bind(user_id)
    .bind(item_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .is_some()
}

/// Cheap cursor check: fetch the user's `/sync/last_activities` and
/// compare the rollup `all` timestamp against the previously-seen
/// value. Returns `Ok(true)` when the pull workflow should continue
/// (something changed since last sync) and `Ok(false)` when there's
/// nothing new to pull — the caller is expected to skip every other
/// pull endpoint in that case.
///
/// On a "continue" outcome we also persist the new `all` value so the
/// next sync compares against the latest known state. Failures don't
/// short-circuit anything (they return `true` so the rest of the
/// pull still runs — a flaky last_activities endpoint shouldn't
/// silently stall the sync).
pub async fn check_last_activities(state: &AppState, user_id: i64) -> Result<bool> {
    let previous = queries::get_trakt_last_activities_seen(&state.pool, user_id).await?;
    let result = with_user_client(state, user_id, |client, token| async move {
        client.pull_last_activities(&token).await
    })
    .await;
    let activities = match result {
        Ok(Some(a)) => a,
        Ok(None) => return Ok(false),
        Err(e) => {
            warn!(user_id, error = %format!("{e:#}"), "Trakt last_activities check failed; falling back to full pull");
            return Ok(true);
        }
    };
    if previous.as_deref() == Some(activities.all.as_str()) {
        // Skip the rest — nothing new on Trakt since our last visit.
        tracing::info!(user_id, "Trakt last_activities unchanged; skipping pulls");
        return Ok(false);
    }
    queries::update_trakt_last_activities_seen(&state.pool, user_id, &activities.all).await?;
    Ok(true)
}

/// Reconcile the user's Trakt collection with the local catalogue.
/// Diffs the previously-pushed snapshot (in
/// `user_trakt_collection_state`) against what's currently on disk
/// and pushes only the delta: newly-added files via `/sync/collection`
/// and deletions via `/sync/collection/remove`. Returns
/// `(movies_added, episodes_added, movies_removed, episodes_removed)`.
///
/// The state-table contract is what lets us safely call `/remove`
/// without nuking items the user collected via another media server
/// or directly on Trakt — we only ever remove rows *we* previously
/// inserted. First-run for a freshly-linked user starts with an empty
/// state row → everything currently on disk is treated as an add,
/// nothing is removed.
///
/// Owners push the whole library; restricted users push only items in
/// their accessible libraries — so a guest user's Trakt collection
/// won't leak the existence of restricted-library content.
pub async fn bulk_push_user_collection(
    state: &AppState,
    user_id: i64,
) -> Result<(usize, usize, usize, usize)> {
    let role = queries::find_user_by_id(&state.pool, user_id)
        .await?
        .map(|u| u.role)
        .unwrap_or(chimpflix_library::UserRole::User);
    let accessible = queries::user_library_filter(&state.pool, user_id, role).await?;
    let current_movies =
        queries::list_collected_movies_for_user(&state.pool, accessible.as_deref()).await?;
    let current_episodes =
        queries::list_collected_episodes_for_user(&state.pool, accessible.as_deref()).await?;
    let (prev_movies, prev_episodes) =
        queries::list_trakt_collection_state(&state.pool, user_id).await?;

    let current_movie_set: std::collections::HashSet<i64> =
        current_movies.iter().copied().collect();
    let current_episode_set: std::collections::HashSet<(i64, i32, i32)> =
        current_episodes.iter().copied().collect();

    let movies_to_add: Vec<i64> = current_movie_set
        .difference(&prev_movies)
        .copied()
        .collect();
    let movies_to_remove: Vec<i64> = prev_movies
        .difference(&current_movie_set)
        .copied()
        .collect();
    let episodes_to_add: Vec<(i64, i32, i32)> = current_episode_set
        .difference(&prev_episodes)
        .copied()
        .collect();
    let episodes_to_remove: Vec<(i64, i32, i32)> = prev_episodes
        .difference(&current_episode_set)
        .copied()
        .collect();

    let (added_m, added_e, removed_m, removed_e) = (
        movies_to_add.len(),
        episodes_to_add.len(),
        movies_to_remove.len(),
        episodes_to_remove.len(),
    );

    if added_m + added_e + removed_m + removed_e == 0 {
        return Ok((0, 0, 0, 0));
    }

    // We call Trakt twice in the same `with_user_client` so a single
    // token refresh covers both legs. The closure does add → remove
    // sequentially; failure of either bubbles up and we skip the
    // state-table update so the next run retries the full delta.
    let movies_add_clone = movies_to_add.clone();
    let episodes_add_clone = episodes_to_add.clone();
    let movies_remove_clone = movies_to_remove.clone();
    let episodes_remove_clone = episodes_to_remove.clone();
    let pushed = with_user_client(state, user_id, |client, token| async move {
        if !movies_add_clone.is_empty() || !episodes_add_clone.is_empty() {
            client
                .push_collection(&token, &movies_add_clone, &episodes_add_clone)
                .await?;
        }
        if !movies_remove_clone.is_empty() || !episodes_remove_clone.is_empty() {
            client
                .remove_collection(&token, &movies_remove_clone, &episodes_remove_clone)
                .await?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await?;
    if pushed.is_some() {
        // Snapshot the post-push set so the next nightly diff is
        // computed against what we just told Trakt — not the union of
        // everything we've ever sent.
        queries::replace_trakt_collection_state(
            &state.pool,
            user_id,
            &current_movies,
            &current_episodes,
        )
        .await?;
        tracing::info!(
            user_id,
            added_movies = added_m,
            added_episodes = added_e,
            removed_movies = removed_m,
            removed_episodes = removed_e,
            "Trakt collection reconcile ok"
        );
        Ok((added_m, added_e, removed_m, removed_e))
    } else {
        Ok((0, 0, 0, 0))
    }
}

pub async fn push_rating_event(state: &AppState, user_id: i64, event: RatingPush) {
    let result = with_user_client(state, user_id, |client, token| async move {
        client.push_rating(&token, event).await
    })
    .await;
    if let Err(e) = result {
        warn!(user_id, error = %format!("{e:#}"), "Trakt push_rating hook failed");
    }
}

pub async fn push_rating_remove(state: &AppState, user_id: i64, event: RatingPush) {
    let result = with_user_client(state, user_id, |client, token| async move {
        client.remove_rating(&token, event).await
    })
    .await;
    if let Err(e) = result {
        warn!(user_id, error = %format!("{e:#}"), "Trakt remove_rating hook failed");
    }
}

/// Bulk-push every locally-watched item that has accrued since
/// `since_ms` (or all of them when `since_ms` is None) in a single
/// `/sync/history` POST. Used by `sync_now` so an operator pressing
/// "Sync now" actually closes the loop — without this, items the
/// fire-and-forget hook missed (network blip, token expiry, item
/// matched after first hook fired) stayed local forever.
///
/// Trakt deduplicates by tmdb_id + watched_at-within-window, so
/// re-pushing items that already exist on Trakt is harmless. We
/// still cap the response at the actual number of rows we sent,
/// not what Trakt accepted, so the UI's "Pushed N" matches the
/// operator's intent rather than Trakt's dedup math.
pub async fn bulk_push_user_history(
    state: &AppState,
    user_id: i64,
    since_ms: Option<i64>,
) -> Result<(usize, usize)> {
    let movies =
        queries::list_watched_movies_for_push(&state.pool, user_id, since_ms).await?;
    let episodes =
        queries::list_watched_episodes_for_push(&state.pool, user_id, since_ms).await?;
    if movies.is_empty() && episodes.is_empty() {
        return Ok((0, 0));
    }
    let mut events: Vec<HistoryPush> = Vec::with_capacity(movies.len() + episodes.len());
    for m in &movies {
        events.push(HistoryPush::Movie {
            ids: TraktIdSet {
                tmdb: m.tmdb_id,
                imdb: m.imdb_id.clone(),
                tvdb: m.tvdb_id,
            },
            watched_at: epoch_ms_to_iso(m.watched_at),
        });
    }
    for e in &episodes {
        events.push(HistoryPush::Episode {
            show_ids: TraktIdSet {
                tmdb: e.show_tmdb_id,
                imdb: e.show_imdb_id.clone(),
                tvdb: e.show_tvdb_id,
            },
            episode_ids: TraktIdSet {
                tmdb: e.episode_tmdb_id,
                imdb: None,
                tvdb: e.episode_tvdb_id,
            },
            season: e.season,
            episode: e.episode,
            watched_at: epoch_ms_to_iso(e.watched_at),
        });
    }
    let pushed = with_user_client(state, user_id, |client, token| async move {
        client.push_history(&token, &events).await
    })
    .await?;
    if pushed.is_some() {
        tracing::info!(
            user_id,
            movies = movies.len(),
            episodes = episodes.len(),
            since_ms,
            "Trakt bulk push ok",
        );
        Ok((movies.len(), episodes.len()))
    } else {
        Ok((0, 0))
    }
}

/// Pull Trakt history since the last successful sync and mark matching
/// items watched locally. Returns (movies_marked, episodes_marked).
pub async fn pull_user_history(state: &AppState, user_id: i64) -> Result<(usize, usize)> {
    let Some((movies, episodes)) = with_user_client(state, user_id, |client, token| {
        let pool = state.pool.clone();
        let vault = state.vault.clone();
        async move {
            let tokens = queries::get_trakt_tokens(&pool, &vault, user_id).await?;
            let since_iso = tokens
                .as_ref()
                .and_then(|t| t.last_synced_at)
                .map(epoch_ms_to_iso);
            // Pull the delta since our cursor (or the full, paginated history
            // on first sync). Mirror EVERY entry locally — matched or not —
            // keyed on Trakt's per-event id so re-pulls dedupe and a library
            // added later reconciles against it WITHOUT re-pulling. Then
            // reconcile the mirror against the current library to mark
            // watched (multi-id match → also covers anime that has no tmdb).
            let entries = client.pull_history(&token, since_iso.as_deref()).await?;
            let rows: Vec<queries::TraktHistoryRow> =
                entries.iter().filter_map(history_entry_to_row).collect();
            queries::store_trakt_history(&pool, user_id, &rows).await?;
            let (m, e) = queries::reconcile_trakt_history(&pool, user_id).await?;
            Ok::<_, anyhow::Error>((m as usize, e as usize))
        }
    })
    .await?
    else {
        return Ok((0, 0));
    };
    queries::update_trakt_last_synced(&state.pool, user_id, now_ms()).await?;
    Ok((movies, episodes))
}

/// Pull the user's COMPLETE watched-state snapshot from Trakt
/// (`/sync/watched/{movies,shows}`) into the local mirror, then reconcile.
///
/// This is the AUTHORITATIVE seed for watched status. [`pull_user_history`]
/// reads `/sync/history`, a dated event log that can omit titles marked
/// watched outside ChimpFlix (or before linking) and is pruned/paginated —
/// so a show sitting at "91% watched" on Trakt may have no history events
/// for us to mirror. `/sync/watched` is the snapshot that powers Trakt's
/// own "watched" badges, so it catches everything. One pull per direction,
/// no pagination.
///
/// Rows synthesised here get a deterministic NEGATIVE `trakt_event_id`
/// (see [`synthetic_watched_id`]) so they coexist in `user_trakt_history`
/// with real (positive) history event ids without colliding, and re-pulls
/// upsert rather than duplicate. Returns the (movies, episodes) rows
/// reconciled to watched.
pub async fn pull_user_watched(state: &AppState, user_id: i64) -> Result<(usize, usize)> {
    let Some((movies, episodes)) = with_user_client(state, user_id, |client, token| {
        let pool = state.pool.clone();
        async move {
            let watched_movies = client.pull_watched_movies(&token).await?;
            let watched_shows = client.pull_watched_shows(&token).await?;
            let mut rows: Vec<queries::TraktHistoryRow> =
                Vec::with_capacity(watched_movies.len() + watched_shows.len() * 12);
            for m in &watched_movies {
                let watched_at_ms = m
                    .last_watched_at
                    .as_deref()
                    .and_then(iso_to_epoch_ms)
                    .unwrap_or_else(now_ms);
                let key = format!(
                    "mv:{:?}:{:?}:{:?}",
                    m.movie.ids.tmdb, m.movie.ids.tvdb, m.movie.ids.imdb
                );
                rows.push(queries::TraktHistoryRow {
                    trakt_event_id: synthetic_watched_id(&key),
                    media_type: "movie",
                    tmdb_id: m.movie.ids.tmdb,
                    tvdb_id: m.movie.ids.tvdb,
                    imdb_id: m.movie.ids.imdb.clone(),
                    season: None,
                    episode: None,
                    watched_at_ms,
                });
            }
            for s in &watched_shows {
                // Fall back to the show-level last_watched_at when an
                // episode row omits its own timestamp (rare, but Trakt
                // allows it for very old plays).
                let show_fallback_ms = s
                    .last_watched_at
                    .as_deref()
                    .and_then(iso_to_epoch_ms)
                    .unwrap_or_else(now_ms);
                for season in &s.seasons {
                    for ep in &season.episodes {
                        let watched_at_ms = ep
                            .last_watched_at
                            .as_deref()
                            .and_then(iso_to_epoch_ms)
                            .unwrap_or(show_fallback_ms);
                        let key = format!(
                            "ep:{:?}:{:?}:{:?}:{}:{}",
                            s.show.ids.tmdb,
                            s.show.ids.tvdb,
                            s.show.ids.imdb,
                            season.number,
                            ep.number
                        );
                        rows.push(queries::TraktHistoryRow {
                            trakt_event_id: synthetic_watched_id(&key),
                            media_type: "episode",
                            tmdb_id: s.show.ids.tmdb,
                            tvdb_id: s.show.ids.tvdb,
                            imdb_id: s.show.ids.imdb.clone(),
                            season: Some(season.number),
                            episode: Some(ep.number),
                            watched_at_ms,
                        });
                    }
                }
            }
            queries::store_trakt_history(&pool, user_id, &rows).await?;
            let (m, e) = queries::reconcile_trakt_history(&pool, user_id).await?;
            Ok::<_, anyhow::Error>((m as usize, e as usize))
        }
    })
    .await?
    else {
        return Ok((0, 0));
    };
    Ok((movies, episodes))
}

/// Deterministic synthetic mirror id for a watched-state row. Trakt's
/// `/sync/watched` entries carry no per-event id (unlike `/sync/history`),
/// but `user_trakt_history` is keyed on `(user_id, trakt_event_id)`. We
/// FNV-1a hash the title's natural key and map it into the NEGATIVE i64
/// space so it can never collide with a real (positive) history event id
/// in the shared table, and so re-pulling the same watched-state upserts
/// the same row instead of duplicating it.
fn synthetic_watched_id(key: &str) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // Drop the sign bit, then map to the strictly-negative range so the
    // id is always < 0 (Trakt history event ids are positive).
    -((hash >> 1) as i64) - 1
}

/// Normalize a Trakt `/sync/history` entry into a mirror row. Movies carry
/// the movie's ids; episodes carry the SHOW's ids + season/episode. Returns
/// `None` for entries missing their movie/show/episode payload.
fn history_entry_to_row(
    entry: &chimpflix_metadata::HistoryEntry,
) -> Option<queries::TraktHistoryRow> {
    let watched_at_ms = iso_to_epoch_ms(&entry.watched_at).unwrap_or_else(now_ms);
    match entry.kind.as_str() {
        "movie" => {
            let m = entry.movie.as_ref()?;
            Some(queries::TraktHistoryRow {
                trakt_event_id: entry.id,
                media_type: "movie",
                tmdb_id: m.ids.tmdb,
                tvdb_id: m.ids.tvdb,
                imdb_id: m.ids.imdb.clone(),
                season: None,
                episode: None,
                watched_at_ms,
            })
        }
        "episode" => {
            let show = entry.show.as_ref()?;
            let ep = entry.episode.as_ref()?;
            Some(queries::TraktHistoryRow {
                trakt_event_id: entry.id,
                media_type: "episode",
                tmdb_id: show.ids.tmdb,
                tvdb_id: show.ids.tvdb,
                imdb_id: show.ids.imdb.clone(),
                season: Some(ep.season),
                episode: Some(ep.number),
                watched_at_ms,
            })
        }
        _ => None,
    }
}

/// Parse a Trakt RFC3339 `watched_at` timestamp to epoch ms.
fn iso_to_epoch_ms(iso: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Reconcile the local Trakt-history mirror against the library for EVERY
/// linked user. Called on scan completion so newly-added items pick up their
/// watched status from already-pulled history — no Trakt API calls.
pub async fn reconcile_all_linked_users(state: &AppState) {
    let user_ids = match queries::list_trakt_linked_user_ids(&state.pool).await {
        Ok(ids) => ids,
        Err(e) => {
            warn!(error = %format!("{e:#}"), "trakt reconcile: list linked users failed");
            return;
        }
    };
    for uid in user_ids {
        match queries::reconcile_trakt_history(&state.pool, uid).await {
            Ok((m, e)) if m + e > 0 => {
                tracing::info!(user_id = uid, movies = m, episodes = e, "trakt history reconciled after scan")
            }
            Ok(_) => {}
            Err(e) => {
                warn!(user_id = uid, error = %format!("{e:#}"), "trakt history reconcile failed")
            }
        }
    }
}

/// Pull Trakt's `/sync/playback` and write any progress entry that's
/// newer than ours into local `play_state`. Best-effort.
pub async fn pull_user_playback(state: &AppState, user_id: i64) -> Result<usize> {
    let Some(applied) = with_user_client(state, user_id, |client, token| {
        let pool = state.pool.clone();
        async move {
            let entries = client.pull_playback(&token).await?;
            let mut applied = 0usize;
            for e in entries {
                match e.kind.as_str() {
                    "movie" => {
                        let Some(m) = e.movie else { continue };
                        let Some(tmdb_id) = m.ids.tmdb else { continue };
                        if let Some(item_id) =
                            find_local_item_by_tmdb(&pool, tmdb_id, "movie").await
                        {
                            // Trakt progress is a percentage. Convert
                            // against our stored duration_ms when we
                            // have one — fall back to 0 (no harm).
                            let duration =
                                lookup_item_duration_ms(&pool, item_id).await.unwrap_or(0);
                            let position_ms = ((e.progress / 100.0) * duration as f64) as i64;
                            let _ =
                                apply_position(&pool, user_id, Some(item_id), None, position_ms)
                                    .await;
                            applied += 1;
                        }
                    }
                    "episode" => {
                        let Some(show) = e.show else { continue };
                        let Some(ep) = e.episode else { continue };
                        let Some(show_tmdb) = show.ids.tmdb else {
                            continue;
                        };
                        if let Some(episode_id) =
                            find_local_episode(&pool, show_tmdb, ep.season, ep.number).await
                        {
                            let duration = lookup_episode_duration_ms(&pool, episode_id)
                                .await
                                .unwrap_or(0);
                            let position_ms = ((e.progress / 100.0) * duration as f64) as i64;
                            let _ =
                                apply_position(&pool, user_id, None, Some(episode_id), position_ms)
                                    .await;
                            applied += 1;
                        }
                    }
                    _ => {}
                }
            }
            Ok::<usize, anyhow::Error>(applied)
        }
    })
    .await?
    else {
        return Ok(0);
    };
    Ok(applied)
}

async fn apply_position(
    pool: &SqlitePool,
    user_id: i64,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    position_ms: i64,
) -> Result<()> {
    // Use the watched-preserving upsert. The previous code reused the
    // live-player batch path, whose `watched.unwrap_or(false)` default
    // un-watched any row Trakt's /sync/playback reported as in-progress
    // — exactly the "sync makes things unwatched or partially watched"
    // symptom users hit.
    queries::upsert_external_position(pool, user_id, item_id, episode_id, position_ms).await
}

async fn find_local_item_by_tmdb(pool: &SqlitePool, tmdb_id: i64, kind: &str) -> Option<i64> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM items WHERE tmdb_id = ? AND kind = ? LIMIT 1")
        .bind(tmdb_id)
        .bind(kind)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn find_local_episode(
    pool: &SqlitePool,
    show_tmdb_id: i64,
    season: i32,
    episode: i32,
) -> Option<i64> {
    sqlx::query_scalar::<_, i64>(
        // Match only a DOWNLOADED episode. A Trakt resume position is
        // meaningless on a placeholder (no media_files, materialized to
        // complete a season) and would leak undownloaded content into
        // play_state / continue-watching, so require a live file.
        "SELECT e.id FROM episodes e
         JOIN seasons s ON s.id = e.season_id
         JOIN items i ON i.id = s.show_id
         WHERE i.tmdb_id = ? AND s.season_number = ? AND e.episode_number = ?
           AND EXISTS (SELECT 1 FROM media_files mf
                       WHERE mf.episode_id = e.id AND mf.removed_at IS NULL)
         LIMIT 1",
    )
    .bind(show_tmdb_id)
    .bind(season)
    .bind(episode)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

async fn lookup_item_duration_ms(pool: &SqlitePool, item_id: i64) -> Option<i64> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT duration_ms FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
}

async fn lookup_episode_duration_ms(pool: &SqlitePool, episode_id: i64) -> Option<i64> {
    sqlx::query_scalar::<_, Option<i64>>("SELECT duration_ms FROM episodes WHERE id = ?")
        .bind(episode_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
}

pub fn epoch_ms_to_iso(epoch_ms: i64) -> String {
    let secs = epoch_ms / 1000;
    let nanos = ((epoch_ms % 1000) * 1_000_000) as u32;
    Utc.timestamp_opt(secs, nanos)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).unwrap())
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Look up every Trakt-compatible id we have for a movie. Returns
/// `None` only when *no* id is populated — the typical anime-only-via-
/// AniList row has neither tmdb nor tvdb nor imdb, in which case
/// there's nothing Trakt can match against and we skip the push.
pub async fn item_trakt_ids(pool: &SqlitePool, item_id: i64) -> Option<TraktIdSet> {
    let row = sqlx::query("SELECT tmdb_id, imdb_id, tvdb_id FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()?;
    let ids = TraktIdSet {
        tmdb: row.try_get::<Option<i64>, _>("tmdb_id").ok().flatten(),
        imdb: row.try_get::<Option<String>, _>("imdb_id").ok().flatten(),
        tvdb: row.try_get::<Option<i64>, _>("tvdb_id").ok().flatten(),
    };
    if ids.is_empty() {
        None
    } else {
        Some(ids)
    }
}

/// For an episode, look up the show's id set + season + episode
/// number in a single query. Trakt's APIs always reference episodes
/// through their parent show + season/episode coordinates, so we
/// return the show's ids (not the episode's own ids — those aren't
/// guaranteed to be populated in our schema).
pub async fn episode_trakt_coords(
    pool: &SqlitePool,
    episode_id: i64,
) -> Result<Option<EpisodeCoords>> {
    let row = sqlx::query(
        "SELECT i.tmdb_id AS show_tmdb, i.imdb_id AS show_imdb, i.tvdb_id AS show_tvdb,
                e.tmdb_id AS ep_tmdb, e.tvdb_id AS ep_tvdb,
                s.season_number AS season,
                e.episode_number AS episode
         FROM episodes e
         JOIN seasons s ON s.id = e.season_id
         JOIN items i ON i.id = s.show_id
         WHERE e.id = ?",
    )
    .bind(episode_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let show_ids = TraktIdSet {
        tmdb: row.try_get::<Option<i64>, _>("show_tmdb").ok().flatten(),
        imdb: row.try_get::<Option<String>, _>("show_imdb").ok().flatten(),
        tvdb: row.try_get::<Option<i64>, _>("show_tvdb").ok().flatten(),
    };
    // The episode's OWN ids. Episodes carry no imdb column, only tmdb/tvdb.
    let episode_ids = TraktIdSet {
        tmdb: row.try_get::<Option<i64>, _>("ep_tmdb").ok().flatten(),
        imdb: None,
        tvdb: row.try_get::<Option<i64>, _>("ep_tvdb").ok().flatten(),
    };
    let season: i32 = row.try_get("season")?;
    let episode: i32 = row.try_get("episode")?;
    // Nothing Trakt can match against if we have neither a show id nor an
    // episode id — skip. An episode id alone is enough (Trakt infers the
    // show), so don't gate on show ids being present.
    if show_ids.is_empty() && episode_ids.is_empty() {
        Ok(None)
    } else {
        Ok(Some(EpisodeCoords {
            show_ids,
            episode_ids,
            season,
            episode,
        }))
    }
}

/// Trakt addressing coordinates for a local episode: the parent show's id
/// set, the episode's OWN id set (preferred when present — see
/// [`ScrobblePush::Episode`]), and the local season/episode numbers used as
/// the fallback when no episode-level id exists.
pub struct EpisodeCoords {
    pub show_ids: TraktIdSet,
    pub episode_ids: TraktIdSet,
    pub season: i32,
    pub episode: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_watched_id_is_deterministic_and_negative() {
        // Stable across calls — re-pulling the same watched-state must
        // upsert the same mirror row, not duplicate it.
        let a = synthetic_watched_id("ep:Some(366924):None:Some(\"tt9288030\"):1:1");
        let b = synthetic_watched_id("ep:Some(366924):None:Some(\"tt9288030\"):1:1");
        assert_eq!(a, b);
        // Always strictly negative so it can NEVER collide with a real
        // (positive) Trakt /sync/history event id in the shared table.
        assert!(a < 0, "synthetic id must be negative, got {a}");
    }

    #[test]
    fn synthetic_watched_id_separates_distinct_keys() {
        // Different episodes / movies → different ids (no accidental merge).
        let s1e1 = synthetic_watched_id("ep:Some(1):None:None:1:1");
        let s1e2 = synthetic_watched_id("ep:Some(1):None:None:1:2");
        let s2e1 = synthetic_watched_id("ep:Some(1):None:None:2:1");
        let movie = synthetic_watched_id("mv:Some(1):None:None");
        let ids = [s1e1, s1e2, s2e1, movie];
        for (i, x) in ids.iter().enumerate() {
            for y in &ids[i + 1..] {
                assert_ne!(x, y, "distinct keys must not collide");
            }
            assert!(*x < 0);
        }
    }
}
