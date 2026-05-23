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
        Some(t) => StatusResponse {
            linked: true,
            linked_at: Some(t.linked_at),
            last_synced_at: t.last_synced_at,
            scope: t.scope,
            app_configured,
        },
        None => StatusResponse {
            linked: false,
            linked_at: None,
            last_synced_at: None,
            scope: None,
            app_configured,
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
    let (movies, episodes) = trakt_sync::pull_user_history(&state, user.id)
        .await
        .map_err(ApiError::Internal)?;
    let playback = trakt_sync::pull_user_playback(&state, user.id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(SyncNowResponse {
        movies_marked: movies,
        episodes_marked: episodes,
        playback_applied: playback,
        movies_pushed,
        episodes_pushed,
    }))
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
    if let Some(tmdb_id) = trakt_sync::item_tmdb_id(&state.pool, id).await {
        tokio::spawn(async move {
            trakt_sync::push_rating_event(
                &state_clone,
                user.id,
                chimpflix_metadata::RatingPush::Movie {
                    tmdb_id,
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
    if let Some(tmdb_id) = trakt_sync::item_tmdb_id(&state.pool, id).await {
        let state_clone = state.clone();
        tokio::spawn(async move {
            trakt_sync::push_rating_remove(
                &state_clone,
                user.id,
                chimpflix_metadata::RatingPush::Movie {
                    tmdb_id,
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
    if let Ok(Some((show_tmdb_id, season, episode))) =
        trakt_sync::episode_trakt_coords(&state.pool, id).await
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            trakt_sync::push_rating_event(
                &state_clone,
                user.id,
                chimpflix_metadata::RatingPush::Episode {
                    tmdb_show_id: show_tmdb_id,
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
    if let Ok(Some((show_tmdb_id, season, episode))) =
        trakt_sync::episode_trakt_coords(&state.pool, id).await
    {
        let state_clone = state.clone();
        tokio::spawn(async move {
            trakt_sync::push_rating_remove(
                &state_clone,
                user.id,
                chimpflix_metadata::RatingPush::Episode {
                    tmdb_show_id: show_tmdb_id,
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
