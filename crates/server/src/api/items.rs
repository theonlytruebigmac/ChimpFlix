//! /api/v1/items handlers.

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use chimpflix_library::queries;
use chimpflix_library::scanner;
use chimpflix_library::{
    CreditsEditInput, ItemDetail, ItemEdit, ItemFilter, ItemKind, ItemPage, ListedItem, Review,
};
use chimpflix_metadata::{TmdbCandidate, TmdbKind, TmdbPoster};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

const MAX_POSTER_BYTES: usize = 8 * 1024 * 1024; // 8 MiB
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
            Some(req.into_iter().filter(|id| allowed_set.contains(id)).collect())
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(mut filter): Query<ItemFilter>,
) -> Result<Json<ItemPage>, ApiError> {
    let acc = access(&state, &user).await?;
    let effective = restrict_access(acc, filter.library_ids.take());
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
    #[serde(
        default,
        deserialize_with = "chimpflix_library::deserialize_csv_i64s"
    )]
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
    )
    .await?;
    let items = rows
        .into_iter()
        .map(|(rank, item)| TrendingItem { rank, item })
        .collect();
    Ok(Json(TrendingResponse { items }))
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

    let file_ids: Vec<i64> =
        sqlx::query_scalar("SELECT id FROM media_files WHERE episode_id = ?")
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

    // Unlink files + sprites + transcoder caches in the background.
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
                Err(e) => tracing::warn!(path = %path, error = %e, "failed to unlink media artefact"),
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
    let video_id = tmdb
        .lookup_trailer(tmdb_id, is_show)
        .await
        .unwrap_or(None);
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
        return Ok(Json(TmdbPostersResponse { posters: Vec::new() }));
    };
    let Some(tmdb_id) = detail.item.tmdb_id else {
        return Ok(Json(TmdbPostersResponse { posters: Vec::new() }));
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
    if !normalized.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'
    }) {
        return Err(ApiError::validation(
            "path must look like a TMDB file path (alphanumerics, dots, dashes, underscores)",
        ));
    }
    if !(normalized.ends_with(".jpg") || normalized.ends_with(".png") || normalized.ends_with(".webp")) {
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
    let tmdb_snapshot = state.tmdb_snapshot().await;
    let Some(tmdb) = tmdb_snapshot.as_ref() else {
        return Err(ApiError::validation("TMDB enrichment is disabled"));
    };
    let tvdb_snapshot = state.tvdb_snapshot().await;
    let acc = access(&state, &user).await?;
    scanner::refresh_item_metadata(
        &state.pool,
        tmdb,
        tvdb_snapshot.as_ref(),
        state.tvmaze.as_ref(),
        id,
        None,
    )
        .await
        .map_err(ApiError::Internal)?;
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
        .map_err(ApiError::Internal)?;
    Ok(Json(MatchSearchResponse { candidates }))
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
    let tmdb_snapshot = state.tmdb_snapshot().await;
    let Some(tmdb) = tmdb_snapshot.as_ref() else {
        return Err(ApiError::validation("TMDB enrichment is disabled"));
    };
    let tvdb_snapshot = state.tvdb_snapshot().await;
    let acc = access(&state, &user).await?;
    scanner::refresh_item_metadata(
        &state.pool,
        tmdb,
        tvdb_snapshot.as_ref(),
        state.tvmaze.as_ref(),
        id,
        Some(input.tmdb_id),
    )
    .await
    .map_err(ApiError::Internal)?;
    let detail = queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    Ok(Json(detail))
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

#[derive(Debug, Serialize)]
pub struct ReviewsResponse {
    pub reviews: Vec<Review>,
}

pub async fn list_reviews(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ReviewsResponse>, ApiError> {
    let acc = access(&state, &user).await?;
    // Ensure the user can see the item before exposing its reviews.
    queries::get_item_detail(&state.pool, id, user.id, acc.as_deref())
        .await?
        .ok_or(ApiError::NotFound)?;
    let reviews = queries::list_reviews_for_item(&state.pool, id).await?;
    Ok(Json(ReviewsResponse { reviews }))
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

/// Stream the locally-stored image back to the client. No auth required to
/// view; the URL is opaque (`/poster` resolves to whatever was uploaded).
pub async fn get_poster_blob(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    serve_image_blob(&state, id, ImageKind::Poster).await
}

pub async fn get_backdrop_blob(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
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
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        ApiError::validation(format!("multipart error: {e}"))
    })? {
        if field.name() == Some("file") {
            content_type = field.content_type().map(str::to_owned);
            let data = field.bytes().await.map_err(|e| {
                ApiError::validation(format!("read field: {e}"))
            })?;
            if data.len() > MAX_POSTER_BYTES {
                return Err(ApiError::validation(format!(
                    "image must be ≤ {} bytes",
                    MAX_POSTER_BYTES
                )));
            }
            bytes = Some(data.to_vec());
            break;
        }
    }
    let bytes = bytes.ok_or_else(|| ApiError::validation("missing `file` field"))?;
    let ext = match content_type.as_deref() {
        Some("image/jpeg") => "jpg",
        Some("image/png") => "png",
        Some("image/webp") => "webp",
        Some(other) => {
            return Err(ApiError::validation(format!(
                "unsupported content-type `{other}` (use image/jpeg, image/png, or image/webp)"
            )));
        }
        None => return Err(ApiError::validation("missing content-type")),
    };

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
    let path = dir.join(format!("{id}.{ext}"));
    let mut f = tokio::fs::File::create(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    f.write_all(&bytes)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    f.flush().await.ok();

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
    queries::replace_primary_image(&state.pool, id, kind.db_kind(), "local", &url)
        .await
        .map_err(ApiError::Internal)?;

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
