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
use chimpflix_metadata::{HistoryPush, RatingPush, TraktClient};
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
    let client = Arc::new(client);
    Ok(Some(f(client, tokens.access_token).await?))
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

fn describe_history_event(event: &HistoryPush) -> String {
    match event {
        HistoryPush::Movie { tmdb_id, .. } => format!("movie tmdb={tmdb_id}"),
        HistoryPush::Episode {
            tmdb_show_id,
            season,
            episode,
            ..
        } => format!("show tmdb={tmdb_show_id} S{season:02}E{episode:02}"),
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
    for (_item_id, tmdb_id, watched_at) in &movies {
        events.push(HistoryPush::Movie {
            tmdb_id: *tmdb_id,
            watched_at: epoch_ms_to_iso(*watched_at),
        });
    }
    for (_ep_id, show_tmdb, season, episode, watched_at) in &episodes {
        events.push(HistoryPush::Episode {
            tmdb_show_id: *show_tmdb,
            season: *season,
            episode: *episode,
            watched_at: epoch_ms_to_iso(*watched_at),
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
    let Some((mut movie_count, mut episode_count)) =
        with_user_client(state, user_id, |client, token| {
            let pool = state.pool.clone();
            let vault = state.vault.clone();
            async move {
                let tokens = queries::get_trakt_tokens(&pool, &vault, user_id).await?;
                let since_iso = tokens
                    .as_ref()
                    .and_then(|t| t.last_synced_at)
                    .map(epoch_ms_to_iso);
                let entries = client.pull_history(&token, since_iso.as_deref()).await?;
                let mut movies = 0usize;
                let mut episodes = 0usize;
                for entry in entries {
                    match entry.kind.as_str() {
                        "movie" => {
                            let Some(m) = entry.movie else { continue };
                            let Some(tmdb_id) = m.ids.tmdb else { continue };
                            if let Some(item_id) =
                                find_local_item_by_tmdb(&pool, tmdb_id, "movie").await
                            {
                                let _ =
                                    queries::set_watched(&pool, user_id, Some(item_id), None, true)
                                        .await;
                                movies += 1;
                            }
                        }
                        "episode" => {
                            let Some(show) = entry.show else { continue };
                            let Some(ep) = entry.episode else { continue };
                            let Some(show_tmdb) = show.ids.tmdb else {
                                continue;
                            };
                            if let Some(episode_id) =
                                find_local_episode(&pool, show_tmdb, ep.season, ep.number).await
                            {
                                let _ = queries::set_watched(
                                    &pool,
                                    user_id,
                                    None,
                                    Some(episode_id),
                                    true,
                                )
                                .await;
                                episodes += 1;
                            }
                        }
                        _ => {}
                    }
                }
                Ok::<_, anyhow::Error>((movies, episodes))
            }
        })
        .await?
    else {
        return Ok((0, 0));
    };
    queries::update_trakt_last_synced(&state.pool, user_id, now_ms()).await?;
    if movie_count == 0 && episode_count == 0 {
        // No-op call still ran; nothing else to report.
        movie_count = 0;
        episode_count = 0;
    }
    Ok((movie_count, episode_count))
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
        "SELECT e.id FROM episodes e
         JOIN seasons s ON s.id = e.season_id
         JOIN items i ON i.id = s.show_id
         WHERE i.tmdb_id = ? AND s.season_number = ? AND e.episode_number = ?
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

/// Look up a movie's tmdb_id by local item_id; returns None if missing.
pub async fn item_tmdb_id(pool: &SqlitePool, item_id: i64) -> Option<i64> {
    sqlx::query("SELECT tmdb_id FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.try_get::<Option<i64>, _>("tmdb_id").ok().flatten())
}

/// For an episode, look up (show_tmdb_id, season_number, episode_number)
/// in a single query — Trakt's APIs always reference episodes through
/// their parent show + season/episode coordinates.
pub async fn episode_trakt_coords(
    pool: &SqlitePool,
    episode_id: i64,
) -> Result<Option<(i64, i32, i32)>> {
    let row = sqlx::query(
        "SELECT i.tmdb_id AS show_tmdb, s.season_number AS season,
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
    let show_tmdb: Option<i64> = row.try_get("show_tmdb").ok().flatten();
    let season: i32 = row.try_get("season")?;
    let episode: i32 = row.try_get("episode")?;
    Ok(show_tmdb.map(|t| (t, season, episode)))
}
