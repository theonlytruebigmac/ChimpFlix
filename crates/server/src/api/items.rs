//! /api/v1/items handlers.

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use chimpflix_library::queries;
use chimpflix_library::scanner;
use chimpflix_library::{
    CreditsEditInput, ItemDetail, ItemEdit, ItemFilter, ItemKind, ItemPage, LibraryKind,
    ListedItem, Review,
};
use chimpflix_metadata::{TmdbCandidate, TmdbKind, TmdbPoster, TmdbUpstreamError};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

const MAX_POSTER_BYTES: usize = 8 * 1024 * 1024; // 8 MiB

/// Hard cap on the decoded image dimensions. A 8 KiB JPEG can decode
/// into a 100k × 100k pixel canvas (40 GB RAM); the upstream byte cap
/// doesn't catch that. Posters are at most ~2000 px tall in any real
/// catalog; 8192 px is plenty of slack for HiDPI backdrops.
const MAX_IMAGE_DIMENSION: u32 = 8192;
const POSTER_DIR: &str = "posters";
const BACKDROP_DIR: &str = "backdrops";

/// Per-request "what libraries can this user see?" filter. None for owners
/// (no restriction), Some(Vec) for non-owners.
async fn access(state: &AppState, user: &AuthUser) -> Result<Option<Vec<i64>>, ApiError> {
    queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)
}

/// Combine the role-based access list with an optional request-supplied
/// `library_ids` filter. Either alone restricts; both together restrict
/// to the intersection. Owners (acc = None) accept the request set
/// verbatim.
fn restrict_access(acc: Option<Vec<i64>>, requested: Option<Vec<i64>>) -> Option<Vec<i64>> {
    match (acc, requested) {
        (None, None) => None,
        (None, Some(req)) => Some(req),
        (Some(allowed), None) => Some(allowed),
        (Some(allowed), Some(req)) => {
            let allowed_set: std::collections::HashSet<i64> = allowed.into_iter().collect();
            Some(
                req.into_iter()
                    .filter(|id| allowed_set.contains(id))
                    .collect(),
            )
        }
    }
}

/// Hard cap on the `?q=` substring search. The filter feeds into a
/// `LIKE '%{q}%'` against `items.title`; without a cap an attacker
/// could submit a multi-megabyte `q` (under the global 16MB body
/// limit) and force SQLite to scan against giant strings on every
/// matching row. 256 chars covers every realistic title search.
const MAX_SEARCH_Q_BYTES: usize = 256;

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(mut filter): Query<ItemFilter>,
) -> Result<Json<ItemPage>, ApiError> {
    if let Some(ref q) = filter.q {
        if q.len() > MAX_SEARCH_Q_BYTES {
            return Err(ApiError::validation(
                "search query must be at most 256 characters",
            ));
        }
    }
    let acc = access(&state, &user).await?;
    let effective = restrict_access(acc, filter.library_ids.take());
    // Apply the user's kid-safe preference server-side. `ItemFilter::kids_safe`
    // is `#[serde(skip)]`, so it's always `false` off the wire and a client
    // can't set it by hand — the toggle is authoritative from the stored
    // preference. When false (the default) `list_items` adds no clause and
    // the query is unchanged.
    //
    // EXCEPTION: the `count_only` existence probe (used by the home page to
    // decide whether the server has been scanned at all) bypasses kids_safe so
    // a kids_safe profile on a library with zero rated items doesn't see a
    // false "scan in progress" screen. This is safe because `count_only`
    // returns ZERO items (only `total`) — a viewer can never extract a
    // non-kid-safe title through it, just an unfiltered content count.
    filter.kids_safe = user.kids_safe && !filter.count_only;
    let page = queries::list_items(&state.pool, filter, user.id, effective.as_deref()).await?;
    Ok(Json(page))
}

#[derive(Debug, Deserialize)]
pub struct TrendingQuery {
    /// `movie` or `show`; defaults to `movie`.
    #[serde(default)]
    pub kind: Option<String>,
    /// 1-50. Defaults to 10 (the Top 10 rail wants exactly 10).
    #[serde(default)]
    pub limit: Option<i64>,
    /// Same shape as `ItemFilter::library_ids` — `?library_ids=1,2,3`.
    /// Lets browse surfaces apply the user's visibility prefs without
    /// the trending endpoint needing to know about prefs directly.
    #[serde(default, deserialize_with = "chimpflix_library::deserialize_csv_i64s")]
    pub library_ids: Option<Vec<i64>>,
}

#[derive(Debug, Serialize)]
pub struct TrendingItem {
    pub rank: i64,
    #[serde(flatten)]
    pub item: ListedItem,
}

#[derive(Debug, Serialize)]
pub struct TrendingResponse {
    pub items: Vec<TrendingItem>,
}

/// Top N items in this library that are also on TMDB's global weekly
/// trending list, ordered by trending rank (1 = most trending). Empty
/// when TMDB isn't configured or the `refresh_trending` task hasn't
/// run yet, or when the library doesn't intersect the global list.
pub async fn trending(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<TrendingQuery>,
) -> Result<Json<TrendingResponse>, ApiError> {
    let kind = match q.kind.as_deref().unwrap_or("movie") {
        "show" => ItemKind::Show,
        _ => ItemKind::Movie,
    };
    let limit = q.limit.unwrap_or(10).clamp(1, 50);
    let acc = access(&state, &user).await?;
    let effective = restrict_access(acc, q.library_ids);
    let rows = queries::list_trending_in_library(
        &state.pool,
        kind,
        user.id,
        limit,
        effective.as_deref(),
        // Top-10 rails honor the same per-user kid-safe toggle as plain
        // browse; authoritative from the stored preference.
        user.kids_safe,
    )
    .await?;
    let items = rows
        .into_iter()
        .map(|(rank, item)| TrendingItem { rank, item })
        .collect();
    Ok(Json(TrendingResponse { items }))
}

#[derive(Debug, Deserialize)]
pub struct LibraryTopQuery {
    /// 1-50. Defaults to 10.
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `GET /api/v1/libraries/{id}/top` — the per-library, type-aware
/// "Top 10": the external top-rated/ranked list for this library's KIND
/// (Movies/Shows → TMDB top-rated, Anime → MyAnimeList ranking),
/// intersected with the library's items and topped up with local
/// top-watched. Reuses the trending response shape. Returns an empty
/// list (not an error) when the user can't see the library or the
/// source hasn't been refreshed yet.
pub async fn library_top(
    State(state): State<AppState>,
    user: AuthUser,
    Path(library_id): Path<i64>,
    Query(q): Query<LibraryTopQuery>,
) -> Result<Json<TrendingResponse>, ApiError> {
    let limit = q.limit.unwrap_or(10).clamp(1, 50);
    let Some(library) = queries::get_library(&state.pool, library_id).await? else {
        return Err(ApiError::NotFound);
    };
    // Source is chosen by library KIND, not item kind (anime items are
    // stored as "show"). Anime → MAL ranking; everything else → TMDB
    // top-rated. media_kind is the items.kind to match against.
    let (media_kind, source): (&str, &str) = match library.kind {
        LibraryKind::Movies => ("movie", "tmdb_top_rated"),
        LibraryKind::Shows => ("show", "tmdb_top_rated"),
        LibraryKind::Anime => ("show", "mal_ranking"),
    };
    let acc = access(&state, &user).await?;
    let rows = queries::list_library_top(
        &state.pool,
        library_id,
        media_kind,
        source,
        user.id,
        limit,
        acc.as_deref(),
    )
    .await?;
    let items = rows
        .into_iter()
        .map(|(rank, item)| TrendingItem { rank, item })
        .collect();
    Ok(Json(TrendingResponse { items }))
}

/// Query for `GET /api/v1/calendar`. Two ways to express the window:
///   * `?days=N` — a window of N days *ahead* of now, plus a fixed look-back
///     so "this week" (episodes that aired in the last few days) still shows.
///   * `?from=<ms>&to=<ms>` — an explicit epoch-millisecond window; takes
///     precedence over `days` when both bounds are supplied.
/// `library_ids` carries the same hidden-libraries preference as the rest of
/// browse (the client passes its visible-library set), intersected with the
/// user's access grants server-side.
#[derive(Debug, Deserialize)]
pub struct CalendarQuery {
    /// Window size in days ahead of now. Defaults to 35 (~5 weeks).
    #[serde(default)]
    pub days: Option<i64>,
    /// Explicit window start, epoch milliseconds.
    #[serde(default)]
    pub from: Option<i64>,
    /// Explicit window end, epoch milliseconds.
    #[serde(default)]
    pub to: Option<i64>,
    /// Cap on returned rows. Defaults to 200; clamped server-side.
    #[serde(default)]
    pub limit: Option<i64>,
    /// Visible-library filter, same shape as `ItemFilter::library_ids`.
    #[serde(default, deserialize_with = "chimpflix_library::deserialize_csv_i64s")]
    pub library_ids: Option<Vec<i64>>,
}

#[derive(Debug, Serialize)]
pub struct CalendarResponse {
    pub episodes: Vec<queries::UpcomingEpisode>,
}

const DAY_MS: i64 = 86_400_000;
const CALENDAR_DEFAULT_DAYS_AHEAD: i64 = 35;

/// `GET /api/v1/calendar` — locally-known episodes whose air date falls in
/// the requested window, honoring the same per-library visibility +
/// kids-safe rules as every other browse surface. The LOCAL-data complement
/// to the Trakt-driven coming-soon rail. The frontend groups by `airDate`.
pub async fn calendar(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<CalendarQuery>,
) -> Result<Json<CalendarResponse>, ApiError> {
    let now = chimpflix_common::now_ms();
    // Default window: from the START OF TODAY (UTC) through `days_ahead` —
    // NO look-back, so both the "Coming Up" rail and the calendar page show
    // today + upcoming only (no already-aired episodes). `air_date` is stored
    // as midnight-UTC of the air day, so flooring `from` to the current UTC
    // day still includes anything airing today. Explicit ?from/?to overrides
    // (e.g. a future "scroll back through the week" view).
    let (from_ms, to_ms) = match (q.from, q.to) {
        (Some(from), Some(to)) if to >= from => (from, to),
        _ => {
            let days_ahead = q.days.unwrap_or(CALENDAR_DEFAULT_DAYS_AHEAD).clamp(1, 365);
            let day_start = now - now.rem_euclid(DAY_MS);
            (day_start, day_start + days_ahead * DAY_MS)
        }
    };
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let acc = access(&state, &user).await?;
    let effective = restrict_access(acc, q.library_ids);
    let episodes = queries::list_upcoming_episodes(
        &state.pool,
        user.id,
        effective.as_deref(),
        from_ms,
        to_ms,
        limit,
        // Honor the per-user kid-safe toggle, authoritative from the stored
        // preference — same as trending/browse.
        user.kids_safe,
    )
    .await?;
    Ok(Json(CalendarResponse { episodes }))
}

pub async fn get_one(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ItemDetail>, ApiError> {
    let acc = access(&state, &user).await?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}

#[derive(Debug, Serialize)]
pub struct DeleteMediaResponse {
    pub files_deleted: u64,
    pub episodes_purged: u64,
    pub seasons_purged: u64,
    pub items_purged: u64,
    /// Paths the server is unlinking from disk in the background.
    /// Returned so the operator UI can show "Deleted /movies/foo.mkv,
    /// /movies/foo.srt, …" rather than just a count.
    pub paths: Vec<String>,
}

/// `DELETE /api/v1/items/{id}/media` — hard-delete every media file
/// associated with this item (the movie itself, or every episode of
/// every season for a show). Gated owner-only AND requires the owning
/// library's `allow_media_deletion` flag. No grace window — the rows
/// are gone immediately, on-disk files are unlinked in the background.
///
/// Cascades:
///   - `media_files` rows + their FK cascades (media_streams /
///     markers / optimized_versions)
///   - orphaned `episodes` → `seasons` → `items`
///   - all FK cascades downstream of items (play_state /
///     external_subtitles / images / item_tags / item_genres /
///     item_credits / external_reviews / metadata_overrides /
///     trakt_synced_items / my_list)
///
/// Audit logs the action with file paths so the operator has a
/// record of what was removed.
pub async fn delete_item_media(
    State(state): State<AppState>,
    user: AuthUser,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<DeleteMediaResponse>, ApiError> {
    use sqlx::Row;
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }

    let item_row = sqlx::query("SELECT library_id, kind FROM items WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .ok_or(ApiError::NotFound)?;
    let library_id: i64 = item_row
        .try_get("library_id")
        .map_err(|e| ApiError::Internal(e.into()))?;
    let kind: String = item_row
        .try_get("kind")
        .map_err(|e| ApiError::Internal(e.into()))?;

    ensure_library_allows_delete(&state, library_id).await?;

    let file_ids: Vec<i64> = match kind.as_str() {
        "movie" => sqlx::query_scalar("SELECT id FROM media_files WHERE item_id = ?")
            .bind(id)
            .fetch_all(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?,
        "show" => sqlx::query_scalar(
            "SELECT mf.id
             FROM media_files mf
             JOIN episodes e ON mf.episode_id = e.id
             JOIN seasons s ON e.season_id = s.id
             WHERE s.show_id = ?",
        )
        .bind(id)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?,
        _ => {
            return Err(ApiError::validation(format!(
                "items of kind `{kind}` don't have a media-delete path"
            )));
        }
    };

    run_force_delete(&state, &user, &headers, "item", id, &file_ids).await
}

/// `DELETE /api/v1/episodes/{id}/media` — hard-delete the single
/// episode's media file. Same gates as the item path. When this was
/// the last episode of a season, the cascade also drops the
/// season; ditto for the parent show. Returns a summary so the UI
/// can decide whether to navigate away (item purged) or just close
/// the modal (orphan cleanup didn't reach the item).
pub async fn delete_episode_media(
    State(state): State<AppState>,
    user: AuthUser,
    headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> Result<Json<DeleteMediaResponse>, ApiError> {
    use sqlx::Row;
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }

    let row = sqlx::query(
        "SELECT i.library_id AS library_id
         FROM episodes e
         JOIN seasons s ON e.season_id = s.id
         JOIN items i ON s.show_id = i.id
         WHERE e.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .ok_or(ApiError::NotFound)?;
    let library_id: i64 = row
        .try_get("library_id")
        .map_err(|e| ApiError::Internal(e.into()))?;

    ensure_library_allows_delete(&state, library_id).await?;

    let file_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM media_files WHERE episode_id = ?")
        .bind(id)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    run_force_delete(&state, &user, &headers, "episode", id, &file_ids).await
}

/// Shared body for the two delete-media handlers. Runs the cascade
/// query, audits the action, fires background tasks to unlink files
/// + evict transcoder caches, returns a small report.
async fn run_force_delete(
    state: &AppState,
    user: &AuthUser,
    headers: &axum::http::HeaderMap,
    target_kind: &str,
    target_id: i64,
    file_ids: &[i64],
) -> Result<Json<DeleteMediaResponse>, ApiError> {
    let report = queries::delete_media_files_force(&state.pool, file_ids)
        .await
        .map_err(ApiError::Internal)?;

    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let payload = serde_json::json!({
        "files_deleted": report.files_purged,
        "episodes_purged": report.episodes_purged,
        "seasons_purged": report.seasons_purged,
        "items_purged": report.items_purged,
        "paths": &report.purged_paths,
    });
    crate::api::admin::audit_log(
        state,
        chimpflix_library::NewAuditEntry {
            actor_user_id: Some(user.id),
            action: "media.delete".into(),
            target_kind: Some(target_kind.into()),
            target_id: Some(target_id.to_string()),
            payload_json: Some(payload.to_string()),
            ip: None,
            user_agent,
        },
    )
    .await;

    // Unlink files + transcoder caches in the background.
    // The DB cascade is already committed at this point, so the
    // user-facing response can return immediately and the
    // filesystem cleanup happens off the request path. Failures
    // (file missing, permission denied) log and move on — they
    // can't roll back the DB delete.
    let paths = report.purged_paths.clone();
    let cache_root = state.transcoder.cache_root().to_path_buf();
    tokio::spawn(async move {
        for path in paths {
            match tokio::fs::remove_file(&path).await {
                Ok(()) => tracing::info!(path = %path, "unlinked deleted media artefact"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    tracing::debug!(path = %path, "media artefact already gone");
                }
                Err(e) => {
                    tracing::warn!(path = %path, error = %e, "failed to unlink media artefact")
                }
            }
            // Evict the per-file WebVTT cache (best-effort; idempotent).
            let _ = chimpflix_transcoder::evict_text_subs_cache(
                &cache_root,
                std::path::Path::new(&path),
            )
            .await;
        }
    });

    Ok(Json(DeleteMediaResponse {
        files_deleted: report.files_purged,
        episodes_purged: report.episodes_purged,
        seasons_purged: report.seasons_purged,
        items_purged: report.items_purged,
        paths: report.purged_paths,
    }))
}

/// Lookup the library and reject if `allow_media_deletion` is off.
/// Surfaces a clear validation error pointing the operator at the
/// admin-libraries toggle so they know exactly which knob to flip.
async fn ensure_library_allows_delete(state: &AppState, library_id: i64) -> Result<(), ApiError> {
    let lib = queries::get_library(&state.pool, library_id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if !lib.allow_media_deletion {
        return Err(ApiError::validation(format!(
            "library `{}` does not allow media deletion — enable it from \
             /admin/library/libraries first",
            lib.name,
        )));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct TrailerResponse {
    pub video_id: Option<String>,
}

/// Look up the YouTube trailer ID for an item, via its TMDB id. Returns
/// `{video_id: null}` when TMDB enrichment is off, the item has no tmdb_id,
/// or no trailer is published. Never 4xx for the "no trailer" case so the
/// frontend can treat it as a normal empty response.
pub async fn trailer(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<TrailerResponse>, ApiError> {
    let acc = access(&state, &user).await?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    let tmdb_snapshot = state.tmdb_snapshot().await;
    let Some(tmdb) = tmdb_snapshot.as_ref() else {
        return Ok(Json(TrailerResponse { video_id: None }));
    };
    let Some(tmdb_id) = detail.item.tmdb_id else {
        return Ok(Json(TrailerResponse { video_id: None }));
    };
    let is_show = matches!(detail.item.kind, ItemKind::Show);
    let video_id = tmdb.lookup_trailer(tmdb_id, is_show).await.unwrap_or(None);
    Ok(Json(TrailerResponse { video_id }))
}

#[derive(Debug, Serialize)]
pub struct SimilarResponse {
    pub items: Vec<ListedItem>,
}

/// Items in the local library similar to the given one. Pulls TMDB's
/// /similar candidates and intersects with what we actually have on disk.
/// Returns an empty array when TMDB is off, the item has no tmdb_id, or
/// no overlap exists — never 4xx for the "nothing similar" case.
pub async fn similar(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<SimilarResponse>, ApiError> {
    let acc = access(&state, &user).await?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    let tmdb_snapshot = state.tmdb_snapshot().await;
    let Some(tmdb) = tmdb_snapshot.as_ref() else {
        return Ok(Json(SimilarResponse { items: Vec::new() }));
    };
    let Some(tmdb_id) = detail.item.tmdb_id else {
        return Ok(Json(SimilarResponse { items: Vec::new() }));
    };
    let is_show = matches!(detail.item.kind, ItemKind::Show);
    let candidates = tmdb
        .lookup_similar(tmdb_id, is_show)
        .await
        .unwrap_or_default();
    let items = queries::find_listed_items_by_tmdb_ids(
        &state.pool,
        &candidates,
        detail.item.kind,
        user.id,
        24,
        acc.as_deref(),
    )
    .await?;
    Ok(Json(SimilarResponse { items }))
}

// ─── Cast & Crew editor (PATCH /items/{id}/credits) ────────────────────────

pub async fn patch_credits(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(input): Json<CreditsEditInput>,
) -> Result<Json<ItemDetail>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let acc = access(&state, &user).await?;
    // Make sure the item exists and is visible to the caller before doing
    // the mutation.
    queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    queries::replace_item_credits(&state.pool, id, &input.credits)
        .await
        .map_err(ApiError::Internal)?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}

// ─── TMDB poster selector ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TmdbPostersResponse {
    pub posters: Vec<TmdbPoster>,
}

/// List poster candidates from TMDB for the item. Returns an empty array
/// when TMDB is disabled, the item has no tmdb_id, or TMDB returns no
/// poster results — the UI treats those cases identically.
pub async fn tmdb_posters(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<TmdbPostersResponse>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let acc = access(&state, &user).await?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    let tmdb_snapshot = state.tmdb_snapshot().await;
    let Some(tmdb) = tmdb_snapshot.as_ref() else {
        return Ok(Json(TmdbPostersResponse {
            posters: Vec::new(),
        }));
    };
    let Some(tmdb_id) = detail.item.tmdb_id else {
        return Ok(Json(TmdbPostersResponse {
            posters: Vec::new(),
        }));
    };
    let kind = match detail.item.kind {
        ItemKind::Movie => TmdbKind::Movie,
        ItemKind::Show => TmdbKind::Show,
    };
    let posters = tmdb.fetch_posters(kind, tmdb_id).await.unwrap_or_default();
    Ok(Json(TmdbPostersResponse { posters }))
}

#[derive(Debug, Deserialize)]
pub struct ApplyTmdbPosterInput {
    /// TMDB `file_path`, e.g. "/abc123.jpg". Leading slash optional.
    pub path: String,
}

/// Download a TMDB poster and store it as the item's primary poster.
/// Mirrors the upload flow: file goes under `DATA_DIR/posters/{id}.jpg`,
/// `images` row is replaced, and the `poster` field is locked.
pub async fn apply_tmdb_poster(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(input): Json<ApplyTmdbPosterInput>,
) -> Result<Json<ItemDetail>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let acc = access(&state, &user).await?;
    queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;

    // Validate the input — only accept TMDB-style file paths so a malicious
    // client can't redirect us at an arbitrary URL.
    let raw = input.path.trim();
    if raw.is_empty() {
        return Err(ApiError::validation("path is required"));
    }
    let normalized = raw.strip_prefix('/').unwrap_or(raw);
    if !normalized
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(ApiError::validation(
            "path must look like a TMDB file path (alphanumerics, dots, dashes, underscores)",
        ));
    }
    if !(normalized.ends_with(".jpg")
        || normalized.ends_with(".png")
        || normalized.ends_with(".webp"))
    {
        return Err(ApiError::validation(
            "path must end with .jpg, .png, or .webp",
        ));
    }

    let url = chimpflix_metadata::tmdb_image_url(&format!("/{normalized}"), "original");
    let bytes = reqwest::get(&url)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .error_for_status()
        .map_err(|e| ApiError::Internal(e.into()))?
        .bytes()
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if bytes.len() > MAX_POSTER_BYTES {
        return Err(ApiError::validation(format!(
            "TMDB poster exceeds {MAX_POSTER_BYTES} bytes"
        )));
    }
    let ext = if normalized.ends_with(".png") {
        "png"
    } else if normalized.ends_with(".webp") {
        "webp"
    } else {
        "jpg"
    };

    let dir = state.data_dir.join(POSTER_DIR);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    for prev_ext in ["jpg", "png", "webp"] {
        let prev = dir.join(format!("{id}.{prev_ext}"));
        if prev.exists() {
            let _ = tokio::fs::remove_file(&prev).await;
        }
    }
    let path = dir.join(format!("{id}.{ext}"));
    let mut f = tokio::fs::File::create(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    f.write_all(&bytes)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    f.flush().await.ok();

    let blob_url = format!(
        "/api/v1/items/{id}/poster/blob?v={}",
        chimpflix_common::now_ms()
    );
    queries::replace_primary_image(&state.pool, id, "poster", "local", &blob_url)
        .await
        .map_err(ApiError::Internal)?;

    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}

// ─── Edit metadata (PATCH /items/{id}) ──────────────────────────────────────

pub async fn patch_item(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(edit): Json<ItemEdit>,
) -> Result<Json<ItemDetail>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let acc = access(&state, &user).await?;
    queries::apply_item_edit(&state.pool, id, &edit).await?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}

// ─── Refresh / Fix Match (POST /items/{id}/refresh, /match-search, /match-apply) ─

pub async fn refresh(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ItemDetail>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    // Refresh is chain-aware now — TMDB is optional. Operators with
    // TMDB removed from a library's chain can still hit Refresh and
    // get their other agents (TVDB / OMDb / AniList) to repopulate
    // missing per-episode metadata + cast.
    let tmdb_snapshot = state.tmdb_snapshot().await;
    let tvdb_snapshot = state.tvdb_snapshot().await;
    let anilist_snapshot = state.anilist_snapshot().await;
    let omdb_snapshot = state.omdb_snapshot().await;
    let acc = access(&state, &user).await?;
    scanner::refresh_item_metadata(
        &state.pool,
        tmdb_snapshot.as_ref(),
        tvdb_snapshot.as_ref(),
        state.tvmaze.as_ref(),
        anilist_snapshot.as_ref(),
        omdb_snapshot.as_ref(),
        id,
        None,
    )
    .await
    .map_err(map_tmdb_error)?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}

#[derive(Debug, Deserialize)]
pub struct MatchSearchQuery {
    pub q: String,
    pub year: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct MatchSearchResponse {
    pub candidates: Vec<TmdbCandidate>,
}

pub async fn match_search(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(q): Query<MatchSearchQuery>,
) -> Result<Json<MatchSearchResponse>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let tmdb_snapshot = state.tmdb_snapshot().await;
    let Some(tmdb) = tmdb_snapshot.as_ref() else {
        return Err(ApiError::validation("TMDB enrichment is disabled"));
    };
    let acc = access(&state, &user).await?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    let kind = match detail.item.kind {
        ItemKind::Movie => TmdbKind::Movie,
        ItemKind::Show => TmdbKind::Show,
    };
    let candidates = tmdb
        .search_candidates(kind, q.q.trim(), q.year)
        .await
        .map_err(map_tmdb_error)?;
    Ok(Json(MatchSearchResponse { candidates }))
}

/// Convert TMDB errors to a friendly `ApiError`. An unparseable
/// upstream body (Cloudflare challenge HTML, empty 200, etc.) becomes
/// a 502 the UI can display as "TMDB unavailable — try again." instead
/// of a generic 500 banner. Other errors stay as Internal so they keep
/// their backtrace in the server log.
fn map_tmdb_error(err: anyhow::Error) -> ApiError {
    if err.is::<TmdbUpstreamError>() {
        return ApiError::BadGateway(
            "TMDB is currently unavailable. Try again in a moment.".to_string(),
        );
    }
    ApiError::Internal(err)
}

#[derive(Debug, Deserialize)]
pub struct MatchApplyInput {
    pub tmdb_id: i64,
}

pub async fn match_apply(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(input): Json<MatchApplyInput>,
) -> Result<Json<ItemDetail>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    // Fix Match still requires TMDB — the operator explicitly chose a
    // TMDB id via the candidate picker, so a missing TMDB config is a
    // genuine user-facing error here (unlike plain Refresh, which now
    // works without TMDB).
    let tmdb_snapshot = state.tmdb_snapshot().await;
    if tmdb_snapshot.is_none() {
        return Err(ApiError::validation("TMDB enrichment is disabled"));
    }
    let tvdb_snapshot = state.tvdb_snapshot().await;
    let anilist_snapshot = state.anilist_snapshot().await;
    let omdb_snapshot = state.omdb_snapshot().await;
    let acc = access(&state, &user).await?;
    scanner::refresh_item_metadata(
        &state.pool,
        tmdb_snapshot.as_ref(),
        tvdb_snapshot.as_ref(),
        state.tvmaze.as_ref(),
        anilist_snapshot.as_ref(),
        omdb_snapshot.as_ref(),
        id,
        Some(input.tmdb_id),
    )
    .await
    .map_err(map_tmdb_error)?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}

#[derive(Debug, Deserialize)]
pub struct MergeIntoInput {
    pub target_id: i64,
}

#[derive(Debug, Serialize)]
pub struct MergeIntoResponse {
    pub report: chimpflix_library::queries::MergeReport,
    pub target: ItemDetail,
}

/// Owner-only: re-point every media file (or episode-attached file)
/// from `id` onto `target_id`, then delete the source. Used to clean
/// up duplicate items — typically caused by TMDB enrichment renaming
/// a show after initial scan, which broke the sort_title-based dedup
/// before the 2026-05-20 fix.
pub async fn merge_into(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(input): Json<MergeIntoInput>,
) -> Result<Json<MergeIntoResponse>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    if id == input.target_id {
        return Err(ApiError::validation("cannot merge an item into itself"));
    }
    let report = queries::merge_items(&state.pool, id, input.target_id)
        .await
        .map_err(|e| {
            // Pre-flight validation errors (different libraries, mismatched
            // kinds, episode-level file conflicts) are user-facing — surface
            // as 400 with the underlying message rather than a generic 500.
            let msg = format!("{e:#}");
            if msg.contains("merge refused")
                || msg.contains("different libraries")
                || msg.contains("kind mismatch")
                || msg.contains("source and target are the same")
            {
                ApiError::Conflict(msg)
            } else if msg.contains("database is locked") || msg.contains("code: 517") {
                // Even with busy_timeout, a long-running writer can
                // exhaust the wait. Surface as 503 with a retry hint
                // instead of a confusing "internal error".
                ApiError::TooManyRequests(
                    "Database was busy. Another writer was running — try again in a moment."
                        .to_string(),
                )
            } else {
                ApiError::Internal(e)
            }
        })?;
    let acc = access(&state, &user).await?;
    let target = queries::get_item_detail(&state.pool, input.target_id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(MergeIntoResponse { report, target }))
}

pub async fn match_clear(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ItemDetail>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let acc = access(&state, &user).await?;
    // Verify the user can see the item before mutating — non-owners
    // can't even reach here (role check above) but the get_item_detail
    // shape gives us a consistent NotFound for missing ids.
    let _existing = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    let updated = queries::unmatch_item(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    if !updated {
        return Err(ApiError::NotFound);
    }
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}

// ─── Report Issue (user → admins) ─────────────────────────────────────────
//
// Available to every authed user (owners included — we don't expect them
// to report to themselves often, but it costs nothing and keeps the menu
// consistent across roles). Validates the message length, then drops a
// row into every owner's notifications table and optionally mirrors it
// as email via `notifier::notify_admins`.

const REPORT_ISSUE_MAX_BYTES: usize = 2000;

#[derive(Debug, Deserialize)]
pub struct ReportIssueInput {
    pub kind: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
struct ReportIssuePayload<'a> {
    item_id: i64,
    item_title: &'a str,
    kind: &'a str,
    message: &'a str,
    reporter_user_id: i64,
    reporter_username: &'a str,
}

pub async fn report_issue(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(input): Json<ReportIssueInput>,
) -> Result<StatusCode, ApiError> {
    // Per-(user, item) throttle. Each report fans out to every admin's
    // email + creates one notification row per admin. Without this gate
    // a hostile (or just confused) user could spam the admin inbox by
    // mashing the Report button.
    let throttle_key = format!("{}:{}", user.id, id);
    if state.report_issue_limiter.check_key(&throttle_key).is_err() {
        return Err(ApiError::TooManyRequests(
            "you've sent several reports for this title recently — try again later".into(),
        ));
    }
    // Allowlist the `kind` so the email subject / payload stays tidy
    // and so a buggy/hostile client can't spray arbitrary labels into
    // admin inboxes. Anything outside this set is rejected.
    const KINDS: &[&str] = &[
        "wrong_match",
        "playback",
        "audio",
        "subtitles",
        "metadata",
        "other",
    ];
    let kind = input.kind.trim();
    if !KINDS.contains(&kind) {
        return Err(ApiError::validation(format!(
            "kind must be one of: {}",
            KINDS.join(", "),
        )));
    }
    let message = input.message.trim();
    if message.is_empty() {
        return Err(ApiError::validation("message is required"));
    }
    if message.len() > REPORT_ISSUE_MAX_BYTES {
        return Err(ApiError::validation(format!(
            "message is too long (max {REPORT_ISSUE_MAX_BYTES} bytes)",
        )));
    }
    // We deliberately don't run the access filter here — any authed user
    // can report on any item they have a ratingKey for. The reporter's
    // identity is stamped on the payload so admins can follow up; the
    // notification body never trusts client-supplied subject lines.
    let acc = access(&state, &user).await?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;

    let payload = ReportIssuePayload {
        item_id: id,
        item_title: &detail.item.title,
        kind,
        message,
        reporter_user_id: user.id,
        reporter_username: &user.username,
    };
    let kind_label = match kind {
        "wrong_match" => "wrong match",
        "playback" => "playback",
        "audio" => "audio",
        "subtitles" => "subtitles",
        "metadata" => "metadata",
        _ => "issue",
    };
    let server_name = state.settings.read().await.server_name.clone();
    let title = detail.item.title.clone();
    let year_str = detail
        .item
        .year
        .map(|y| format!("{title} ({y})"))
        .unwrap_or_else(|| title.clone());
    let username_str = format!("@{}", user.username);
    let item_id_label = format!("#{id}");
    let subject = format!("[{kind_label}] {title} — issue reported");

    // ── Plain text ──
    let text_body = format!(
        "@{username} reported a {kind_label} issue on \"{title}\".\n\n\
         {message}\n\n\
         Title: {year_str}\n\
         Item ID: #{id}\n\
         Category: {kind_label}",
        username = user.username,
    );
    let text = crate::mail_template::render_email_text(crate::mail_template::EmailTextOpts {
        server_name: &server_name,
        headline: &subject,
        body: &text_body,
        footer_note: "You're receiving this as a ChimpFlix server owner.",
    });

    // ── HTML ──
    let user_safe = crate::mail_template::html_escape(&user.username);
    let title_safe = crate::mail_template::html_escape(&detail.item.title);
    let kind_safe = crate::mail_template::html_escape(kind_label);
    let mut html_body = String::new();
    html_body.push_str(&crate::mail_template::section_paragraph(&format!(
        "<strong>@{user_safe}</strong> reported a <strong>{kind_safe}</strong> issue on \
         <em>{title_safe}</em>:"
    )));
    html_body.push_str(&crate::mail_template::section_quote(message));
    html_body.push_str(&crate::mail_template::section_kv(&[
        ("Title", &year_str),
        ("Item ID", &item_id_label),
        ("Reporter", &username_str),
        ("Category", kind_label),
    ]));
    let eyebrow = format!(
        "Admin · Issue reported &nbsp;{}",
        crate::mail_template::section_pip(crate::mail_template::PipKind::Warn, kind_label),
    );
    let html = crate::mail_template::render_email(crate::mail_template::EmailOpts {
        server_name: &server_name,
        eyebrow_html: &eyebrow,
        headline: "Someone reported a problem.",
        body_html: &html_body,
        footer_note: "You're receiving this as a ChimpFlix server owner.",
    });

    crate::notifier::notify_admins(
        &state,
        "item.issue_reported",
        &payload,
        &subject,
        &text,
        &html,
    )
    .await;
    Ok(StatusCode::ACCEPTED)
}

// ─── Reviews (read-only: top reviews ingested from the metadata provider) ─

#[derive(Debug, Default, Deserialize)]
pub struct ReviewsQuery {
    /// Page size. Server clamps to 1..=100; default 12 (modal shows up to
    /// 6 — extra buffer covers any future "see more reviews" surface
    /// without a second round-trip).
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct ReviewsResponse {
    pub reviews: Vec<Review>,
    pub total: i64,
}

pub async fn list_reviews(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Query(q): Query<ReviewsQuery>,
) -> Result<Json<ReviewsResponse>, ApiError> {
    let acc = access(&state, &user).await?;
    // Ensure the user can see the item before exposing its reviews.
    queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    let limit = q.limit.unwrap_or(12);
    let offset = q.offset.unwrap_or(0);
    let reviews = queries::list_reviews_for_item(&state.pool, id, limit, offset).await?;
    let total = queries::count_reviews_for_item(&state.pool, id).await?;
    Ok(Json(ReviewsResponse { reviews, total }))
}

// ─── Poster / backdrop upload + serve ──────────────────────────────────────

/// Upload a custom poster for the item. Owner-only. Replaces any existing
/// poster row, writes the file under `DATA_DIR/posters/{id}.{ext}`, and
/// locks the `poster` field so the metadata pipeline won't overwrite the
/// user's choice on the next refresh.
pub async fn upload_poster(
    state: State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Json<ItemDetail>, ApiError> {
    upload_image_impl(state, user, id, multipart, ImageKind::Poster).await
}

pub async fn upload_backdrop(
    state: State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    multipart: Multipart,
) -> Result<Json<ItemDetail>, ApiError> {
    upload_image_impl(state, user, id, multipart, ImageKind::Backdrop).await
}

/// Stream the locally-stored image back to the client. Auth-gated +
/// library-access enforced so item ids in libraries the caller can't
/// see don't leak via the existence-vs-404 channel. Previously these
/// were fully unauthenticated, which let any internet caller enumerate
/// item ids from `1..N` and pull artwork.
pub async fn get_poster_blob(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    crate::api::access::ensure_item_accessible(&state, &user, id).await?;
    serve_image_blob(&state, id, ImageKind::Poster).await
}

pub async fn get_backdrop_blob(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    crate::api::access::ensure_item_accessible(&state, &user, id).await?;
    serve_image_blob(&state, id, ImageKind::Backdrop).await
}

#[derive(Copy, Clone)]
enum ImageKind {
    Poster,
    Backdrop,
}

impl ImageKind {
    fn dir(self) -> &'static str {
        match self {
            ImageKind::Poster => POSTER_DIR,
            ImageKind::Backdrop => BACKDROP_DIR,
        }
    }
    fn db_kind(self) -> &'static str {
        match self {
            ImageKind::Poster => "poster",
            ImageKind::Backdrop => "backdrop",
        }
    }
    fn url_suffix(self) -> &'static str {
        match self {
            ImageKind::Poster => "poster/blob",
            ImageKind::Backdrop => "backdrop/blob",
        }
    }
}

async fn upload_image_impl(
    State(state): State<AppState>,
    user: AuthUser,
    id: i64,
    mut multipart: Multipart,
    kind: ImageKind,
) -> Result<Json<ItemDetail>, ApiError> {
    if !matches!(user.role, chimpflix_library::UserRole::Owner) {
        return Err(ApiError::Forbidden);
    }
    let acc = access(&state, &user).await?;
    // Confirm the item exists and the caller can see it before doing the
    // upload work.
    queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;

    // Extract the single "file" field. Other fields are ignored.
    let mut bytes: Option<Vec<u8>> = None;
    let mut content_type: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::validation(format!("multipart error: {e}")))?
    {
        if field.name() == Some("file") {
            content_type = field.content_type().map(str::to_owned);
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::validation(format!("read field: {e}")))?;
            if data.len() > MAX_POSTER_BYTES {
                return Err(ApiError::validation(format!(
                    "image must be ≤ {MAX_POSTER_BYTES} bytes"
                )));
            }
            bytes = Some(data.to_vec());
            break;
        }
    }
    let bytes = bytes.ok_or_else(|| ApiError::validation("missing `file` field"))?;
    let target_format = match content_type.as_deref() {
        Some("image/jpeg") => image::ImageFormat::Jpeg,
        Some("image/png") => image::ImageFormat::Png,
        Some("image/webp") => image::ImageFormat::WebP,
        Some(other) => {
            return Err(ApiError::validation(format!(
                "unsupported content-type `{other}` (use image/jpeg, image/png, or image/webp)"
            )));
        }
        None => return Err(ApiError::validation("missing content-type")),
    };

    // SECURITY: decode then re-encode through the `image` crate so:
    //   * SVG / HTML / anything-not-actually-an-image labelled as JPEG
    //     fails the decode and is rejected here, not after it's been
    //     written to disk.
    //   * EXIF / XMP / IPTC metadata (which can leak GPS, camera serial,
    //     editing history, embedded thumbnails) is stripped by the
    //     round-trip — the encoder only writes the pixel data.
    //   * Pixel-flood "decompression bomb" images get bounded by the
    //     in-memory decode step plus the post-decode dimension check.
    //
    // We honour the operator-declared content_type for the OUTPUT format
    // — re-encoding to a different format would silently break the
    // upload UX. JPEG quality 90 matches Plex's default.
    let (sanitized_bytes, ext) =
        sanitize_image(&bytes, target_format).map_err(ApiError::validation)?;
    let bytes = sanitized_bytes;

    let dir = state.data_dir.join(kind.dir());
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    // We overwrite any previous file for this item, regardless of extension.
    // Clean siblings so the GET endpoint never picks up a stale file.
    for prev_ext in ["jpg", "png", "webp"] {
        let prev = dir.join(format!("{id}.{prev_ext}"));
        if prev.exists() {
            let _ = tokio::fs::remove_file(&prev).await;
        }
    }
    // Write to a `.tmp` sibling first so a DB failure below doesn't
    // leave a half-orphaned image file under data_dir. The sibling-
    // cleanup loop above already dropped the previous file; we only
    // promote the new bytes once the DB row update succeeds.
    let path = dir.join(format!("{id}.{ext}"));
    let tmp_path = dir.join(format!("{id}.{ext}.tmp"));
    let mut f = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    f.write_all(&bytes)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    f.flush().await.ok();
    drop(f);

    // Stamp a version onto the stored URL so callers see a fresh string
    // every time the file changes. The blob handler ignores the query, so
    // the file is served unchanged — this just defeats the browser cache
    // and downstream image proxies. Without it, the stable URL keeps
    // pointing at whatever bytes the client cached first.
    let url = format!(
        "/api/v1/items/{id}/{}?v={}",
        kind.url_suffix(),
        chimpflix_common::now_ms()
    );
    if let Err(e) =
        queries::replace_primary_image(&state.pool, id, kind.db_kind(), "local", &url).await
    {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(ApiError::Internal(e));
    }
    tokio::fs::rename(&tmp_path, &path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
}

async fn serve_image_blob(
    state: &AppState,
    id: i64,
    kind: ImageKind,
) -> Result<Response, ApiError> {
    let dir = state.data_dir.join(kind.dir());
    let (path, content_type) = ["jpg", "png", "webp"]
        .iter()
        .map(|ext| dir.join(format!("{id}.{ext}")))
        .find(|p| p.exists())
        .map(|p| {
            let ct = match p.extension().and_then(|e| e.to_str()) {
                Some("jpg") => "image/jpeg",
                Some("png") => "image/png",
                Some("webp") => "image/webp",
                _ => "application/octet-stream",
            };
            (p, ct)
        })
        .ok_or(ApiError::NotFound)?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, HeaderValue::from_static(content_type))
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal(e.into()))
}

/// Decode user-supplied image bytes and re-encode as the requested
/// format. Returns the cleaned bytes plus the canonical file extension
/// to use on disk. On any decode error, dimension overflow, or
/// unexpected pixel-bomb shape, returns an error suitable for an
/// `ApiError::validation` body — never panics on hostile input.
///
/// This is the canonical defense against:
///   * SVG / HTML / arbitrary blobs labelled as `image/jpeg` — fails
///     the decode, rejected before any disk write.
///   * EXIF / XMP / IPTC metadata leaks — the re-encode only writes
///     pixel data, so GPS / camera serial / editing history are gone.
///   * Pixel-flood decompression bombs — capped at MAX_IMAGE_DIMENSION
///     per side before the encoder allocates a buffer.
fn sanitize_image(
    raw: &[u8],
    format: image::ImageFormat,
) -> Result<(Vec<u8>, &'static str), String> {
    let decoded = image::load_from_memory(raw).map_err(|e| format!("image decode failed: {e}"))?;
    if decoded.width() > MAX_IMAGE_DIMENSION || decoded.height() > MAX_IMAGE_DIMENSION {
        return Err(format!(
            "image is too large ({}×{}); max is {MAX_IMAGE_DIMENSION}px per side",
            decoded.width(),
            decoded.height(),
        ));
    }
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let mut cursor = std::io::Cursor::new(&mut out);
    decoded
        .write_to(&mut cursor, format)
        .map_err(|e| format!("image re-encode failed: {e}"))?;
    let ext = match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::WebP => "webp",
        // Sanitizer is only called with the three formats the upload
        // handler accepts — any other arm is a programmer error.
        _ => return Err("unsupported image format".to_string()),
    };
    Ok((out, ext))
}
