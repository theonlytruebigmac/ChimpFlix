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

pub async fn list(
    State(state): State<AppState>,
    user: AuthUser,
    Query(filter): Query<ItemFilter>,
) -> Result<Json<ItemPage>, ApiError> {
    let acc = access(&state, &user).await?;
    let page = queries::list_items(&state.pool, filter, user.id, acc.as_deref()).await?;
    Ok(Json(page))
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
    let Some(tmdb) = &state.tmdb else {
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
    let Some(tmdb) = &state.tmdb else {
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
    let Some(tmdb) = &state.tmdb else {
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
    let Some(tmdb) = &state.tmdb else {
        return Err(ApiError::validation("TMDB enrichment is disabled"));
    };
    let acc = access(&state, &user).await?;
    scanner::refresh_item_metadata(&state.pool, tmdb, state.tvmaze.as_ref(), id, None)
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
    let Some(tmdb) = &state.tmdb else {
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
    let Some(tmdb) = &state.tmdb else {
        return Err(ApiError::validation("TMDB enrichment is disabled"));
    };
    let acc = access(&state, &user).await?;
    scanner::refresh_item_metadata(
        &state.pool,
        tmdb,
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
