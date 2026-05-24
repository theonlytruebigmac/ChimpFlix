//! `/trakt/*` — per-user Trakt linking + manual sync.
//!
//! `link/start` initiates the device-code flow and returns the code
//! and verification URL for the UI. `link/poll` is called by the UI
//! every few seconds with the device_code until either tokens come
//! back (success) or Trakt says expired/denied. `status` reports the
//! current link state; `unlink` clears the stored tokens; `sync-now`
//! triggers an immediate history + playback pull.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use chimpflix_common::now_ms;
use chimpflix_library::queries;
use chimpflix_metadata::{DeviceCodeResponse, DevicePollResult};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::api::access;
use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;
use crate::trakt_sync;

/// Short-lived per-user device-code cache. The Trakt poll endpoint
/// requires us to remember the device_code we received in `link/start`
/// (the UI never sees it); the user_code shown to the user is bound
/// to it server-side. Entries expire whenever Trakt expires them; we
/// also evict on success/expiry so the map stays small.
///
/// Uses `tokio::sync::Mutex` so a future widening of the critical
/// section (e.g. lookup-then-await for HTTP) doesn't accidentally
/// block a Tokio worker. Today's critical sections are HashMap-only
/// (microseconds) but a blocking mutex on an async worker is a
/// latent foot-gun we don't want to leave behind.
type DeviceCache = Arc<Mutex<HashMap<i64, CachedDevice>>>;

struct CachedDevice {
    device_code: String,
    expires_at: Instant,
}

fn device_cache() -> &'static DeviceCache {
    use std::sync::OnceLock;
    static CACHE: OnceLock<DeviceCache> = OnceLock::new();
    CACHE.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Drop entries whose `expires_at` is in the past. Called at every
/// `link_start` / `link_poll` access so a user who abandons the link
/// flow doesn't pin a `CachedDevice` in memory until process restart.
/// The map is small (≤ concurrent linking users) so a full sweep is
/// cheaper than threading a timer in.
fn sweep_expired(map: &mut HashMap<i64, CachedDevice>) {
    let now = Instant::now();
    map.retain(|_, entry| entry.expires_at > now);
}

#[derive(Debug, Serialize)]
pub struct LinkStartResponse {
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: i64,
    pub interval: i64,
}

pub async fn link_start(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<LinkStartResponse>, ApiError> {
    let Some(client) = state.trakt_snapshot().await else {
        return Err(ApiError::validation(
            "Trakt is not configured on the server — set client_id/client_secret in /admin/server/credentials first",
        ));
    };
    let resp: DeviceCodeResponse = client.device_code().await.map_err(ApiError::Internal)?;
    let expires_at = Instant::now() + Duration::from_secs(resp.expires_in.max(0) as u64);
    {
        let mut guard = device_cache().lock().await;
        sweep_expired(&mut guard);
        guard.insert(
            user.id,
            CachedDevice {
                device_code: resp.device_code.clone(),
                expires_at,
            },
        );
    }
    Ok(Json(LinkStartResponse {
        user_code: resp.user_code,
        verification_url: resp.verification_url,
        expires_in: resp.expires_in,
        interval: resp.interval,
    }))
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LinkPollResponse {
    Pending,
    Ready,
    Expired,
    Denied,
    SlowDown,
}

pub async fn link_poll(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<LinkPollResponse>, ApiError> {
    let Some(client) = state.trakt_snapshot().await else {
        return Err(ApiError::validation("Trakt is not configured"));
    };
    let entry = {
        let mut guard = device_cache().lock().await;
        sweep_expired(&mut guard);
        guard.remove(&user.id)
    };
    let Some(entry) = entry else {
        return Err(ApiError::validation(
            "no pending link — call /trakt/link/start first",
        ));
    };
    if entry.expires_at <= Instant::now() {
        return Ok(Json(LinkPollResponse::Expired));
    }
    let result = client
        .poll_device_token(&entry.device_code)
        .await
        .map_err(ApiError::Internal)?;
    match result {
        DevicePollResult::Ready(pair) => {
            let expires_at = now_ms() + pair.expires_in * 1000;
            queries::upsert_trakt_tokens(
                &state.pool,
                &state.vault,
                user.id,
                &pair.access_token,
                &pair.refresh_token,
                pair.scope.as_deref(),
                expires_at,
            )
            .await
            .map_err(ApiError::Internal)?;
            Ok(Json(LinkPollResponse::Ready))
        }
        DevicePollResult::Pending | DevicePollResult::SlowDown => {
            // Put it back so the next poll uses the same code.
            device_cache().lock().await.insert(user.id, entry);
            Ok(Json(if matches!(result, DevicePollResult::SlowDown) {
                LinkPollResponse::SlowDown
            } else {
                LinkPollResponse::Pending
            }))
        }
        DevicePollResult::Expired | DevicePollResult::AlreadyApproved => {
            Ok(Json(LinkPollResponse::Expired))
        }
        DevicePollResult::Denied => Ok(Json(LinkPollResponse::Denied)),
    }
}

#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub linked: bool,
    pub linked_at: Option<i64>,
    pub last_synced_at: Option<i64>,
    pub scope: Option<String>,
    pub app_configured: bool,
    /// True when the access token is within ~10 days of its
    /// `expires_at` AND no sync has refreshed it recently. Trakt
    /// refresh tokens are valid 60 days from last use; after 50
    /// days of no sync, the link is about to silently expire.
    /// MONTH 1 in `docs/PUBLIC_RELEASE_HARDENING.md`.
    pub expiring_soon: bool,
    /// True when the access token's `expires_at` is already in the
    /// past. The next sync will attempt a refresh; if the refresh
    /// token has also expired (60+ days since last use) the link
    /// is dead and the user has to re-link.
    pub expired: bool,
}

pub async fn status(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<StatusResponse>, ApiError> {
    let app_configured = state.trakt_snapshot().await.is_some();
    let tokens = queries::get_trakt_tokens(&state.pool, &state.vault, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(match tokens {
        Some(t) => {
            let now = chimpflix_common::now_ms();
            // 10 days in milliseconds.
            const WARN_WINDOW_MS: i64 = 10 * 24 * 60 * 60 * 1000;
            let expired = t.expires_at < now;
            let expiring_soon = !expired && (t.expires_at - now) < WARN_WINDOW_MS;
            StatusResponse {
                linked: true,
                linked_at: Some(t.linked_at),
                last_synced_at: t.last_synced_at,
                scope: t.scope,
                app_configured,
                expiring_soon,
                expired,
            }
        }
        None => StatusResponse {
            linked: false,
            linked_at: None,
            last_synced_at: None,
            scope: None,
            app_configured,
            expiring_soon: false,
            expired: false,
        },
    }))
}

pub async fn unlink(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    let removed = queries::delete_trakt_tokens(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(serde_json::json!({ "removed": removed })))
}

#[derive(Debug, Serialize)]
pub struct SyncNowResponse {
    pub movies_marked: usize,
    pub episodes_marked: usize,
    pub playback_applied: usize,
    /// Number of locally-watched movies pushed up to Trakt during this
    /// sync. Includes anything since `last_synced_at` — typically zero
    /// when the fire-and-forget mark-watched hook ran cleanly, non-zero
    /// after a token expiry, network blip, or for items matched only
    /// after the hook fired.
    pub movies_pushed: usize,
    pub episodes_pushed: usize,
    /// Count of Trakt watchlist entries newly added to local My List
    /// during this sync. Movies + shows combined.
    pub watchlist_added: usize,
    /// Count of My List entries removed because they were removed from
    /// the user's Trakt watchlist since the previous sync. Movies +
    /// shows combined. First-sync for a freshly-linked user always
    /// reports 0 (the diff baseline is empty so we only emit adds).
    pub watchlist_removed: usize,
}

pub async fn sync_now(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<SyncNowResponse>, ApiError> {
    // Capture the cursor *before* pulling so the push step doesn't
    // miss rows the pull just upserted.
    let since = chimpflix_library::queries::get_trakt_last_synced(&state.pool, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let (movies_pushed, episodes_pushed) =
        trakt_sync::bulk_push_user_history(&state, user.id, since)
            .await
            .map_err(ApiError::Internal)?;
    // Short-circuit the pull half when /sync/last_activities reports
    // nothing has changed since the previous sync. The manual "Sync
    // now" UI button still fires the push above so a local-side
    // backlog flushes, but we don't burn three round-trips on a
    // user pressing the button twice in a minute.
    let should_pull = trakt_sync::check_last_activities(&state, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let (movies, episodes, playback, watchlist_added, watchlist_removed) = if should_pull {
        let (m, e) = trakt_sync::pull_user_history(&state, user.id)
            .await
            .map_err(ApiError::Internal)?;
        let p = trakt_sync::pull_user_playback(&state, user.id)
            .await
            .map_err(ApiError::Internal)?;
        let (wa, wr) = trakt_sync::pull_user_watchlist(&state, user.id)
            .await
            .map_err(ApiError::Internal)?;
        (m, e, p, wa, wr)
    } else {
        (0, 0, 0, 0, 0)
    };
    Ok(Json(SyncNowResponse {
        movies_marked: movies,
        episodes_marked: episodes,
        playback_applied: playback,
        movies_pushed,
        episodes_pushed,
        watchlist_added,
        watchlist_removed,
    }))
}

// ─── Calendar ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct UpcomingEpisode {
    /// Local items.id for the parent show, present only when the
    /// show is in this user's library (and accessible to them).
    /// Drives the click-through into the local title modal.
    pub show_item_id: Option<i64>,
    pub show_title: String,
    pub season: i32,
    pub episode: i32,
    pub episode_title: Option<String>,
    /// ISO-8601 air time.
    pub first_aired: String,
    pub show_tmdb_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct UpcomingResponse {
    pub items: Vec<UpcomingEpisode>,
}

#[derive(Debug, Deserialize)]
pub struct UpcomingQuery {
    /// Number of days to look ahead. Clamped 1..=31 by Trakt.
    pub days: Option<u32>,
    /// Variant selector. Defaults to "shows" (every upcoming episode
    /// of every tracked show). Other values:
    ///   - "premieres" → season premieres only (S(N+1)E1)
    ///   - "new"       → series premieres (E1 of brand-new shows)
    pub variant: Option<String>,
}

/// `GET /v1/trakt/calendars/shows` — return upcoming episodes for
/// shows the user has watched on Trakt, optionally enhanced with the
/// local items.id so the UI can deep-link into a show that's already
/// in our library. No Trakt link → empty list (not an error).
pub async fn calendar_shows(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Query(q): axum::extract::Query<UpcomingQuery>,
) -> Result<Json<UpcomingResponse>, ApiError> {
    let days = q.days.unwrap_or(14).clamp(1, 31);
    let kind = match q.variant.as_deref().unwrap_or("shows") {
        "shows" => chimpflix_metadata::ShowCalendarKind::Shows,
        "new" => chimpflix_metadata::ShowCalendarKind::NewShows,
        "premieres" => chimpflix_metadata::ShowCalendarKind::SeasonPremieres,
        _ => return Err(ApiError::validation("variant must be: shows, new, premieres")),
    };
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let result = trakt_sync::with_user_client(&state, user.id, |client, token| async move {
        client.pull_calendar_shows(&token, kind, &today, days).await
    })
    .await
    .map_err(ApiError::Internal)?;
    let entries = match result {
        Some(v) => v,
        None => return Ok(Json(UpcomingResponse { items: vec![] })),
    };
    // Look up local item ids by show tmdb so the rail can link into
    // /show/[id]. Use access-filtered lookup so a user without access
    // to the show's library sees the entry without a local link
    // (rather than missing the entry entirely — they still want to
    // know when their tracked show airs).
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let show_tmdb_id = entry.show.ids.tmdb;
        let show_item_id = match show_tmdb_id {
            Some(t) => find_show_item_by_tmdb(&state, user.id, acc.as_deref(), t).await,
            None => None,
        };
        items.push(UpcomingEpisode {
            show_item_id,
            show_title: entry.show.title,
            season: entry.episode.season,
            episode: entry.episode.number,
            episode_title: entry.episode.title,
            first_aired: entry.first_aired,
            show_tmdb_id,
        });
    }
    Ok(Json(UpcomingResponse { items }))
}

#[derive(Debug, Serialize)]
pub struct UpcomingMovie {
    pub movie_item_id: Option<i64>,
    pub title: String,
    pub year: Option<i32>,
    /// `YYYY-MM-DD` (no time-of-day per Trakt's contract).
    pub released: String,
    pub tmdb_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct UpcomingMoviesResponse {
    pub items: Vec<UpcomingMovie>,
}

/// `GET /v1/trakt/calendars/movies` — upcoming movie releases for
/// movies the user has on their watchlist or in their collection.
/// Trakt restricts the universe server-side; we just intersect with
/// our local catalogue for click-through.
pub async fn calendar_movies(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Query(q): axum::extract::Query<UpcomingQuery>,
) -> Result<Json<UpcomingMoviesResponse>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 31);
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let result = trakt_sync::with_user_client(&state, user.id, |client, token| async move {
        client.pull_calendar_movies(&token, &today, days).await
    })
    .await
    .map_err(ApiError::Internal)?;
    let entries = match result {
        Some(v) => v,
        None => return Ok(Json(UpcomingMoviesResponse { items: vec![] })),
    };
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let tmdb_id = entry.movie.ids.tmdb;
        let movie_item_id = match tmdb_id {
            Some(t) => find_movie_item_by_tmdb(&state, user.id, acc.as_deref(), t).await,
            None => None,
        };
        items.push(UpcomingMovie {
            movie_item_id,
            title: entry.movie.title,
            year: entry.movie.year,
            released: entry.released,
            tmdb_id,
        });
    }
    Ok(Json(UpcomingMoviesResponse { items }))
}

async fn find_movie_item_by_tmdb(
    state: &AppState,
    user_id: i64,
    accessible: Option<&[i64]>,
    tmdb_id: i64,
) -> Option<i64> {
    let item_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM items WHERE tmdb_id = ? AND kind = 'movie' LIMIT 1",
    )
    .bind(tmdb_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()?;
    let visible = queries::get_item(&state.pool, item_id, user_id, accessible)
        .await
        .ok()
        .flatten()
        .is_some();
    visible.then_some(item_id)
}

async fn find_show_item_by_tmdb(
    state: &AppState,
    user_id: i64,
    accessible: Option<&[i64]>,
    tmdb_id: i64,
) -> Option<i64> {
    let item_id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM items WHERE tmdb_id = ? AND kind = 'tv' LIMIT 1",
    )
    .bind(tmdb_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()?;
    // Ensure the user can actually see the show. `get_item` already
    // applies the access filter, but we just want the id back rather
    // than the full struct, so re-check directly.
    let visible = queries::get_item(&state.pool, item_id, user_id, accessible)
        .await
        .ok()
        .flatten()
        .is_some();
    visible.then_some(item_id)
}

// ─── Recommendations ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RecommendationsQuery {
    /// "movie" or "show". Anything else → 400.
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct RecommendationsResponse {
    pub items: Vec<chimpflix_library::ListedItem>,
}

/// `GET /v1/trakt/recommendations?kind=movie|show` — proxy Trakt's
/// personalized recs and intersect with the user's accessible local
/// library. Tiles not in our library get dropped (no poster to show
/// and no destination to link to). Trakt-side dismissals can be
/// surfaced via a future DELETE route; for now the rail just hides
/// what's already locally watched on subsequent renders since the
/// items are real library entries with `play_state`.
pub async fn recommendations(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Query(q): axum::extract::Query<RecommendationsQuery>,
) -> Result<Json<RecommendationsResponse>, ApiError> {
    let kind = match q.kind.as_str() {
        "movie" => chimpflix_metadata::RecommendationKind::Movies,
        "show" => chimpflix_metadata::RecommendationKind::Shows,
        _ => return Err(ApiError::validation("kind must be one of: movie, show")),
    };
    let local_kind = if matches!(kind, chimpflix_metadata::RecommendationKind::Movies) {
        "movie"
    } else {
        "tv"
    };
    let result = trakt_sync::with_user_client(&state, user.id, |client, token| async move {
        // Two GETs in one closure share a single token refresh. Hidden
        // list is a belt-and-suspenders filter on top of the algo's
        // server-side respect for hides — covers the brief window
        // between user hides X on mobile and Trakt's recommender
        // re-rank picking that up.
        let recs = client.pull_recommendations(&token, kind).await?;
        let hidden = client
            .pull_hidden_recommendations(&token)
            .await
            .unwrap_or_default();
        Ok::<_, anyhow::Error>((recs, hidden))
    })
    .await
    .map_err(ApiError::Internal)?;
    let (entries, hidden) = match result {
        Some(v) => v,
        None => return Ok(Json(RecommendationsResponse { items: vec![] })),
    };
    let hidden_tmdb: std::collections::HashSet<i64> = hidden
        .into_iter()
        .filter_map(|h| match h.kind.as_str() {
            "movie" => h.movie.and_then(|m| m.ids.tmdb),
            "show" => h.show.and_then(|s| s.ids.tmdb),
            _ => None,
        })
        .collect();
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let mut items = Vec::new();
    for entry in entries {
        let Some(tmdb_id) = entry.ids.tmdb else {
            continue;
        };
        // Skip items the user explicitly hid on Trakt.
        if hidden_tmdb.contains(&tmdb_id) {
            continue;
        }
        let Some(local_id) = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM items WHERE tmdb_id = ? AND kind = ? LIMIT 1",
        )
        .bind(tmdb_id)
        .bind(local_kind)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        else {
            continue;
        };
        // Pull the full ListedItem with play_state so the Card can
        // show "watched" badges and the rail can drop items the
        // user already finished.
        let Some(listed) = queries::list_items_by_ids(
            &state.pool,
            &[local_id],
            user.id,
            acc.as_deref(),
        )
        .await
        .map_err(ApiError::Internal)?
        .into_iter()
        .next()
        else {
            continue;
        };
        items.push(listed);
    }
    Ok(Json(RecommendationsResponse { items }))
}

// ─── Favorites ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct FavoritesResponse {
    pub items: Vec<chimpflix_library::ListedItem>,
}

/// `GET /v1/trakt/favorites` — return the user's Trakt favorites
/// (their hand-curated "desert island" subset) intersected with the
/// local library. Read-only; ChimpFlix has no local Favorites concept
/// distinct from My List, so this just surfaces what's on Trakt for
/// rail rendering.
pub async fn favorites(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<FavoritesResponse>, ApiError> {
    let result = trakt_sync::with_user_client(&state, user.id, |client, token| async move {
        client.pull_favorites(&token).await
    })
    .await
    .map_err(ApiError::Internal)?;
    let entries = match result {
        Some(v) => v,
        None => return Ok(Json(FavoritesResponse { items: vec![] })),
    };
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let mut ids: Vec<i64> = Vec::new();
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for entry in entries {
        let (kind, tmdb_id) = match entry.kind.as_str() {
            "movie" => {
                let Some(m) = entry.movie else { continue };
                let Some(t) = m.ids.tmdb else { continue };
                ("movie", t)
            }
            "show" => {
                let Some(s) = entry.show else { continue };
                let Some(t) = s.ids.tmdb else { continue };
                ("tv", t)
            }
            _ => continue,
        };
        let Some(local_id) = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM items WHERE tmdb_id = ? AND kind = ? LIMIT 1",
        )
        .bind(tmdb_id)
        .bind(kind)
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten()
        else {
            continue;
        };
        if seen.insert(local_id) {
            ids.push(local_id);
        }
    }
    let items = queries::list_items_by_ids(&state.pool, &ids, user.id, acc.as_deref())
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(FavoritesResponse { items }))
}

// ─── Personal Lists ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TraktListView {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    /// Items in this list intersected with the user's accessible
    /// local library, in Trakt's stored list order. Lists where
    /// nothing matches locally are dropped server-side so the rail
    /// component doesn't have to filter empty rails out.
    pub items: Vec<chimpflix_library::ListedItem>,
}

#[derive(Debug, Serialize)]
pub struct TraktListsResponse {
    pub lists: Vec<TraktListView>,
}

/// `GET /v1/trakt/lists` — return the user's personal Trakt lists
/// (the ones they created), each hydrated with the subset of items
/// that's actually in their accessible local library. Empty lists +
/// lists with no local intersection are omitted. Unlinked users get
/// an empty array (not an error) so the page can render gracefully.
pub async fn user_lists(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<TraktListsResponse>, ApiError> {
    let result = trakt_sync::with_user_client(&state, user.id, |client, token| async move {
        let lists = client.pull_my_lists(&token).await?;
        // Per-list items in parallel — Trakt's per-request rate limit
        // is generous enough that a small fan-out is fine, and most
        // users have <10 lists. `try_join_all` cancels in-flight
        // requests if any errors so we don't hammer Trakt on a 401.
        let list_ids: Vec<String> = lists.iter().map(|l| l.ids.trakt.to_string()).collect();
        let items_per_list = futures::future::try_join_all(
            list_ids
                .iter()
                .map(|id| client.pull_my_list_items(&token, id)),
        )
        .await?;
        Ok::<_, anyhow::Error>(
            lists
                .into_iter()
                .zip(items_per_list)
                .collect::<Vec<(_, _)>>(),
        )
    })
    .await
    .map_err(ApiError::Internal)?;
    let pairs = match result {
        Some(v) => v,
        None => return Ok(Json(TraktListsResponse { lists: vec![] })),
    };
    let acc = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let mut out = Vec::with_capacity(pairs.len());
    for (list, items) in pairs {
        // Walk in Trakt's list order — for many users, the manual
        // ordering of a personal list is the whole point (e.g.
        // "watch order for franchise").
        let mut local_ids: Vec<i64> = Vec::new();
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for entry in items {
            let (kind, tmdb_id) = match entry.kind.as_str() {
                "movie" => {
                    let Some(m) = entry.movie else { continue };
                    let Some(t) = m.ids.tmdb else { continue };
                    ("movie", t)
                }
                "show" => {
                    let Some(s) = entry.show else { continue };
                    let Some(t) = s.ids.tmdb else { continue };
                    ("tv", t)
                }
                _ => continue,
            };
            let Some(local_id) = sqlx::query_scalar::<_, i64>(
                "SELECT id FROM items WHERE tmdb_id = ? AND kind = ? LIMIT 1",
            )
            .bind(tmdb_id)
            .bind(kind)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten()
            else {
                continue;
            };
            if seen.insert(local_id) {
                local_ids.push(local_id);
            }
        }
        if local_ids.is_empty() {
            continue;
        }
        let listed =
            queries::list_items_by_ids(&state.pool, &local_ids, user.id, acc.as_deref())
                .await
                .map_err(ApiError::Internal)?;
        if listed.is_empty() {
            continue;
        }
        out.push(TraktListView {
            id: list.ids.trakt,
            slug: list.ids.slug,
            name: list.name,
            description: list.description,
            items: listed,
        });
    }
    Ok(Json(TraktListsResponse { lists: out }))
}

// ─── Stats (lifetime watch totals) ─────────────────────────────────────────

/// `GET /v1/trakt/stats` — proxy the user's `/users/me/stats` response.
/// Trakt's payload is verbose; we forward the watch + collection
/// counts directly and drop the rest. Returns `null` when the user
/// hasn't linked Trakt rather than erroring, so the settings card can
/// render conditionally without a separate "linked?" check.
pub async fn stats(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Option<chimpflix_metadata::UserStats>>, ApiError> {
    let result = trakt_sync::with_user_client(&state, user.id, |client, token| async move {
        client.pull_user_stats(&token).await
    })
    .await
    .map_err(ApiError::Internal)?;
    Ok(Json(result))
}

// ─── Ratings (Phase 15) ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RatingInput {
    pub rating: i32,
}

#[derive(Debug, Serialize)]
pub struct RatingResponse {
    pub rating: Option<i32>,
}

pub async fn get_item_rating(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<RatingResponse>, ApiError> {
    access::ensure_item_accessible(&state, &user, id).await?;
    let rating = queries::get_user_rating_for_item(&state.pool, user.id, id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(RatingResponse { rating }))
}

pub async fn put_item_rating(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(input): Json<RatingInput>,
) -> Result<Json<RatingResponse>, ApiError> {
    access::ensure_item_accessible(&state, &user, id).await?;
    let row = queries::set_user_rating(&state.pool, user.id, Some(id), None, input.rating)
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    // Best-effort Trakt push.
    let state_clone = state.clone();
    if let Some(ids) = trakt_sync::item_trakt_ids(&state.pool, id).await {
        tokio::spawn(async move {
            trakt_sync::push_rating_event(
                &state_clone,
                user.id,
                chimpflix_metadata::RatingPush::Movie {
                    ids,
                    rating: input.rating,
                    rated_at: trakt_sync::epoch_ms_to_iso(row.rated_at),
                },
            )
            .await;
        });
    }
    Ok(Json(RatingResponse {
        rating: Some(row.rating),
    }))
}

pub async fn delete_item_rating(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<RatingResponse>, ApiError> {
    access::ensure_item_accessible(&state, &user, id).await?;
    let _ = queries::delete_user_rating(&state.pool, user.id, Some(id), None)
        .await
        .map_err(ApiError::Internal)?;
    if let Some(ids) = trakt_sync::item_trakt_ids(&state.pool, id).await {
        let state_clone = state.clone();
        tokio::spawn(async move {
            trakt_sync::push_rating_remove(
                &state_clone,
                user.id,
                chimpflix_metadata::RatingPush::Movie {
                    ids,
                    rating: 0,
                    rated_at: trakt_sync::epoch_ms_to_iso(now_ms()),
                },
            )
            .await;
        });
    }
    Ok(Json(RatingResponse { rating: None }))
}

pub async fn get_episode_rating(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<RatingResponse>, ApiError> {
    access::ensure_episode_accessible(&state, &user, id).await?;
    let rating = queries::get_user_rating_for_episode(&state.pool, user.id, id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(RatingResponse { rating }))
}

pub async fn put_episode_rating(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(input): Json<RatingInput>,
) -> Result<Json<RatingResponse>, ApiError> {
    access::ensure_episode_accessible(&state, &user, id).await?;
    let row = queries::set_user_rating(&state.pool, user.id, None, Some(id), input.rating)
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    if let Ok(Some((show_ids, season, episode))) =
        trakt_sync::episode_trakt_coords(&state.pool, id).await
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            trakt_sync::push_rating_event(
                &state_clone,
                user.id,
                chimpflix_metadata::RatingPush::Episode {
                    show_ids,
                    season,
                    episode,
                    rating: input.rating,
                    rated_at: trakt_sync::epoch_ms_to_iso(row.rated_at),
                },
            )
            .await;
        });
    }
    Ok(Json(RatingResponse {
        rating: Some(row.rating),
    }))
}

pub async fn delete_episode_rating(
    State(state): State<AppState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<RatingResponse>, ApiError> {
    access::ensure_episode_accessible(&state, &user, id).await?;
    let _ = queries::delete_user_rating(&state.pool, user.id, None, Some(id))
        .await
        .map_err(ApiError::Internal)?;
    if let Ok(Some((show_ids, season, episode))) =
        trakt_sync::episode_trakt_coords(&state.pool, id).await
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            trakt_sync::push_rating_remove(
                &state_clone,
                user.id,
                chimpflix_metadata::RatingPush::Episode {
                    show_ids,
                    season,
                    episode,
                    rating: 0,
                    rated_at: trakt_sync::epoch_ms_to_iso(now_ms()),
                },
            )
            .await;
        });
    }
    Ok(Json(RatingResponse { rating: None }))
}
