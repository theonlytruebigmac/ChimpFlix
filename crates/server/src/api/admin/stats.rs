//! /admin/stats — Tautulli-lite playback dashboard.
//!
//! Five endpoints, all `AdminAuth` (admin or owner). The `playback_events`
//! table is the source of truth for everything historical; the live
//! "now playing" snapshot comes from the in-memory `TranscodeManager`
//! so a freshly-started stream shows up immediately (no DB round trip).
//!
//! All windowed endpoints accept `?days=30` (1..=365). Default 30 keeps
//! payloads bounded; the UI exposes the dial.

use axum::Json;
use axum::extract::{Query, State};
use chimpflix_library::queries::{
    self, StatsActivityRow, StatsDailyBucket, StatsHourBucket, StatsLibraryBucket,
    StatsOverview, StatsPlatformBucket, StatsTopItemRow, StatsTopUserRow,
};
use chimpflix_transcoder::SessionSnapshot;
use serde::{Deserialize, Serialize};

use crate::api::error::ApiError;
use crate::auth::AdminAuth;
use crate::state::AppState;

/// Convert a `days` query param into an epoch-ms cutoff. Default 30,
/// hard-bounded to keep the indexed range queries snappy.
fn since_ms(days: Option<i64>) -> i64 {
    let d = days.unwrap_or(30).clamp(1, 365);
    chimpflix_common::now_ms() - d * 86_400_000
}

// ─── Overview ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct WindowQuery {
    pub days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct OverviewResponse {
    pub days: i64,
    pub overview: StatsOverview,
    pub now_playing_count: usize,
}

pub async fn overview(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Query(q): Query<WindowQuery>,
) -> Result<Json<OverviewResponse>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let since = since_ms(Some(days));
    let overview = queries::stats_overview(&state.pool, since)
        .await
        .map_err(ApiError::Internal)?;
    let now_playing_count = state.transcoder.list_sessions().len();
    Ok(Json(OverviewResponse {
        days,
        overview,
        now_playing_count,
    }))
}

// ─── Recent activity feed ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub limit: Option<i64>,
    /// Cursor for pagination — pass the smallest `id` from the previous
    /// page to fetch older rows. Newest-first ordering means
    /// `WHERE id < before` is the right cursor predicate.
    pub before: Option<i64>,
    /// Scope to a single user. Used by the per-user drill-in from the
    /// Top Users tile; the global feed leaves it None.
    pub user_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ActivityResponse {
    pub events: Vec<StatsActivityRow>,
}

pub async fn activity(
    State(state): State<AppState>,
    AdminAuth(actor): AdminAuth,
    Query(q): Query<ActivityQuery>,
) -> Result<Json<ActivityResponse>, ApiError> {
    // Guard the per-user drill-in against Admin → Owner spying. The
    // global feed (no user_id) stays accessible to all admins; targeted
    // queries against an Owner's playback history require the caller
    // to also be an Owner.
    if let Some(target_id) = q.user_id {
        if !matches!(actor.role, chimpflix_library::UserRole::Owner) {
            if let Some(target) =
                chimpflix_library::queries::find_user_by_id(&state.pool, target_id)
                    .await
                    .map_err(ApiError::Internal)?
            {
                if matches!(target.role, chimpflix_library::UserRole::Owner) {
                    return Err(ApiError::Forbidden);
                }
            }
        }
    }
    let limit = q.limit.unwrap_or(50);
    let events =
        queries::list_playback_activity(&state.pool, limit, q.before, q.user_id)
            .await
            .map_err(ApiError::Internal)?;
    Ok(Json(ActivityResponse { events }))
}

// ─── Time-series + platform breakdown ──────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DailyResponse {
    pub days: i64,
    pub buckets: Vec<StatsDailyBucket>,
}

pub async fn plays_per_day(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Query(q): Query<WindowQuery>,
) -> Result<Json<DailyResponse>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let buckets = queries::plays_per_day(&state.pool, days)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(DailyResponse { days, buckets }))
}

#[derive(Debug, Serialize)]
pub struct HourlyResponse {
    pub days: i64,
    pub buckets: Vec<StatsHourBucket>,
}

pub async fn plays_per_hour(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Query(q): Query<WindowQuery>,
) -> Result<Json<HourlyResponse>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let buckets = queries::plays_per_hour(&state.pool, days)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(HourlyResponse { days, buckets }))
}

#[derive(Debug, Serialize)]
pub struct LibrariesResponse {
    pub days: i64,
    pub libraries: Vec<StatsLibraryBucket>,
}

pub async fn top_libraries(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Query(q): Query<TopQuery>,
) -> Result<Json<LibrariesResponse>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let limit = q.limit.unwrap_or(10);
    let libraries = queries::top_libraries_by_plays(&state.pool, days, limit)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(LibrariesResponse { days, libraries }))
}

#[derive(Debug, Serialize)]
pub struct PlatformsResponse {
    pub days: i64,
    pub platforms: Vec<StatsPlatformBucket>,
}

pub async fn top_platforms(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Query(q): Query<TopQuery>,
) -> Result<Json<PlatformsResponse>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let limit = q.limit.unwrap_or(10);
    let platforms = queries::top_platforms(&state.pool, days, limit)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(PlatformsResponse { days, platforms }))
}

// ─── Top users / items ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TopQuery {
    pub days: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct TopUsersResponse {
    pub days: i64,
    pub users: Vec<StatsTopUserRow>,
}

pub async fn top_users(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Query(q): Query<TopQuery>,
) -> Result<Json<TopUsersResponse>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let limit = q.limit.unwrap_or(10);
    let users = queries::top_users_by_plays(&state.pool, since_ms(Some(days)), limit)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(TopUsersResponse { days, users }))
}

#[derive(Debug, Serialize)]
pub struct TopItemsResponse {
    pub days: i64,
    pub items: Vec<StatsTopItemRow>,
}

pub async fn top_items(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Query(q): Query<TopQuery>,
) -> Result<Json<TopItemsResponse>, ApiError> {
    let days = q.days.unwrap_or(30).clamp(1, 365);
    let limit = q.limit.unwrap_or(10);
    let items = queries::top_items_by_plays(&state.pool, since_ms(Some(days)), limit)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(TopItemsResponse { days, items }))
}

// ─── Now playing (live from TranscodeManager) ──────────────────────────────

#[derive(Debug, Serialize)]
pub struct NowPlayingResponse {
    pub sessions: Vec<SessionSnapshot>,
}

pub async fn now_playing(
    State(state): State<AppState>,
    _admin: AdminAuth,
) -> Result<Json<NowPlayingResponse>, ApiError> {
    Ok(Json(NowPlayingResponse {
        sessions: state.transcoder.list_sessions(),
    }))
}
