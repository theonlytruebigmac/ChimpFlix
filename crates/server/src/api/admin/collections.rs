//! `/admin/collections*` — admin-only CRUD for manual collections.
//!
//! Auto collections (TMDB franchises) are read-only here; only `manual`
//! kind rows are mutable. Every handler audits.

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::Response;
use chimpflix_library::{
    NewAuditEntry, queries,
    queries::{ManualCollectionUpdate, NewManualCollection, NewSmartCollection},
    smart_rule::{SmartRule, compile_to_sql},
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::{AuthUser, OwnerAuth};
use crate::state::AppState;

const NAME_MAX: usize = 200;
const DESC_MAX: usize = 4000;
const MAX_ART_BYTES: usize = 8 * 1024 * 1024; // 8 MiB
const POSTER_DIR: &str = "collection_posters";
const BACKDROP_DIR: &str = "collection_backdrops";

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    pub id: i64,
}

pub async fn create(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<NewManualCollection>,
) -> Result<(StatusCode, Json<CreateResponse>), ApiError> {
    let trimmed_name = input.name.trim().to_string();
    validate_name(&trimmed_name)?;
    if let Some(desc) = &input.description {
        validate_description(desc)?;
    }

    let normalized = NewManualCollection {
        name: trimmed_name,
        sort_title: input
            .sort_title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        description: input.description.clone(),
    };
    let id = queries::create_manual_collection(&state.pool, normalized.clone(), actor.id)
        .await
        .map_err(ApiError::Internal)?;

    audit(
        &state,
        actor.id,
        &headers,
        "collection.create",
        id,
        &normalized,
    )
    .await;
    Ok((StatusCode::CREATED, Json(CreateResponse { id })))
}

pub async fn update(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ManualCollectionUpdate>,
) -> Result<StatusCode, ApiError> {
    if let Some(name) = &input.name {
        validate_name(name.trim())?;
    }
    if let Some(Some(desc)) = &input.description {
        validate_description(desc)?;
    }

    let ok = queries::update_manual_collection(&state.pool, id, input.clone())
        .await
        .map_err(ApiError::Internal)?;
    if !ok {
        return Err(ApiError::NotFound);
    }
    audit(&state, actor.id, &headers, "collection.update", id, &input).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn delete(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    // Try manual first; if not found, fall through to smart. Auto
    // collections aren't deletable from here — the scanner owns those.
    let removed_manual = queries::delete_manual_collection(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    if removed_manual {
        audit(&state, actor.id, &headers, "collection.delete", id, &()).await;
        return Ok(StatusCode::NO_CONTENT);
    }
    let removed_smart = queries::delete_smart_collection(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed_smart {
        return Err(ApiError::NotFound);
    }
    audit(&state, actor.id, &headers, "collection.delete", id, &()).await;
    Ok(StatusCode::NO_CONTENT)
}

// ─── Smart collections ─────────────────────────────────────────────────

pub async fn create_smart(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<NewSmartCollection>,
) -> Result<(StatusCode, Json<CreateResponse>), ApiError> {
    let trimmed_name = input.name.trim().to_string();
    validate_name(&trimmed_name)?;
    if let Some(desc) = &input.description {
        validate_description(desc)?;
    }
    // Validate the rule by attempting compilation. We discard the
    // compiled output here — `list_items_in_collection` re-compiles
    // at read time. The point is to reject malformed rules at create
    // time rather than at the first browse-of-collection request.
    let parsed: SmartRule = serde_json::from_str(&input.rule_json)
        .map_err(|e| ApiError::validation(format!("invalid rule JSON: {e}")))?;
    compile_to_sql(&parsed).map_err(|e| ApiError::validation(format!("rule rejected: {e:#}")))?;

    let normalized = NewSmartCollection {
        name: trimmed_name,
        sort_title: input
            .sort_title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        description: input.description.clone(),
        rule_json: input.rule_json.clone(),
    };
    let id = queries::create_smart_collection(&state.pool, normalized.clone(), actor.id)
        .await
        .map_err(ApiError::Internal)?;
    audit(
        &state,
        actor.id,
        &headers,
        "collection.create_smart",
        id,
        &normalized,
    )
    .await;
    Ok((StatusCode::CREATED, Json(CreateResponse { id })))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UpdateSmartRuleRequest {
    pub rule_json: String,
}

pub async fn update_smart_rule(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<UpdateSmartRuleRequest>,
) -> Result<StatusCode, ApiError> {
    let parsed: SmartRule = serde_json::from_str(&input.rule_json)
        .map_err(|e| ApiError::validation(format!("invalid rule JSON: {e}")))?;
    compile_to_sql(&parsed).map_err(|e| ApiError::validation(format!("rule rejected: {e:#}")))?;
    let ok = queries::update_smart_collection_rule(&state.pool, id, &input.rule_json)
        .await
        .map_err(ApiError::Internal)?;
    if !ok {
        return Err(ApiError::NotFound);
    }
    audit(
        &state,
        actor.id,
        &headers,
        "collection.update_smart_rule",
        id,
        &input,
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AddItemsRequest {
    pub item_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct AddItemsResponse {
    pub inserted: u64,
}

pub async fn add_items(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<AddItemsRequest>,
) -> Result<Json<AddItemsResponse>, ApiError> {
    // Caller-facing protection against unbounded bulk-add — the SQL is
    // bounded by item table size but we still want a clear cap.
    if input.item_ids.len() > 1000 {
        return Err(ApiError::validation("item_ids may not exceed 1000 entries"));
    }
    // Confirm the target exists and is manual before touching the junction.
    let row = queries::get_collection(&state.pool, id, None)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if row.kind != "manual" {
        return Err(ApiError::validation(
            "auto-generated collections (TMDB franchises) are read-only",
        ));
    }
    let inserted = queries::add_items_to_manual_collection(&state.pool, id, &input.item_ids)
        .await
        .map_err(ApiError::Internal)?;
    audit(
        &state,
        actor.id,
        &headers,
        "collection.add_items",
        id,
        &input,
    )
    .await;
    Ok(Json(AddItemsResponse { inserted }))
}

pub async fn remove_item(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path((id, item_id)): Path<(i64, i64)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let row = queries::get_collection(&state.pool, id, None)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if row.kind != "manual" {
        return Err(ApiError::validation(
            "auto-generated collections (TMDB franchises) are read-only",
        ));
    }
    let removed = queries::remove_item_from_manual_collection(&state.pool, id, item_id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    audit(
        &state,
        actor.id,
        &headers,
        "collection.remove_item",
        id,
        &serde_json::json!({ "item_id": item_id }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReorderRequest {
    pub item_ids: Vec<i64>,
}

pub async fn reorder(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<ReorderRequest>,
) -> Result<StatusCode, ApiError> {
    if input.item_ids.len() > 5000 {
        return Err(ApiError::validation("item_ids may not exceed 5000 entries"));
    }
    let row = queries::get_collection(&state.pool, id, None)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if row.kind != "manual" {
        return Err(ApiError::validation(
            "auto-generated collections (TMDB franchises) are read-only",
        ));
    }
    queries::replace_manual_collection_items(&state.pool, id, &input.item_ids)
        .await
        .map_err(ApiError::Internal)?;
    audit(&state, actor.id, &headers, "collection.reorder", id, &input).await;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_name(name: &str) -> Result<(), ApiError> {
    if name.is_empty() {
        return Err(ApiError::validation("name must not be empty"));
    }
    if name.chars().count() > NAME_MAX {
        return Err(ApiError::validation(format!(
            "name exceeds {NAME_MAX} characters"
        )));
    }
    if name.chars().any(|c| c.is_control() && c != '\t') {
        return Err(ApiError::validation("name contains control characters"));
    }
    Ok(())
}

fn validate_description(desc: &str) -> Result<(), ApiError> {
    if desc.chars().count() > DESC_MAX {
        return Err(ApiError::validation(format!(
            "description exceeds {DESC_MAX} characters"
        )));
    }
    Ok(())
}

async fn audit<T: Serialize>(
    state: &AppState,
    actor_id: i64,
    headers: &HeaderMap,
    action: &str,
    target_id: i64,
    payload: &T,
) {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(actor_id),
            action: action.to_string(),
            target_kind: Some("collection".into()),
            target_id: Some(target_id.to_string()),
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}

// ─── Poster / backdrop upload + serve ─────────────────────────────────

#[derive(Copy, Clone)]
enum ArtKind {
    Poster,
    Backdrop,
}

impl ArtKind {
    fn dir(self) -> &'static str {
        match self {
            ArtKind::Poster => POSTER_DIR,
            ArtKind::Backdrop => BACKDROP_DIR,
        }
    }
    fn url_suffix(self) -> &'static str {
        match self {
            ArtKind::Poster => "poster/blob",
            ArtKind::Backdrop => "backdrop/blob",
        }
    }
    fn action(self) -> &'static str {
        match self {
            ArtKind::Poster => "collection.upload_poster",
            ArtKind::Backdrop => "collection.upload_backdrop",
        }
    }
}

pub async fn upload_poster(
    state: State<AppState>,
    auth: OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<StatusCode, ApiError> {
    upload_art(state, auth, id, headers, multipart, ArtKind::Poster).await
}

pub async fn upload_backdrop(
    state: State<AppState>,
    auth: OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<StatusCode, ApiError> {
    upload_art(state, auth, id, headers, multipart, ArtKind::Backdrop).await
}

async fn upload_art(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    id: i64,
    headers: HeaderMap,
    mut multipart: Multipart,
    kind: ArtKind,
) -> Result<StatusCode, ApiError> {
    // Confirm the target is a manual collection before any I/O — auto
    // collection art comes from TMDB and we don't want operators to
    // shadow it with local uploads (the next metadata refresh would
    // wipe the local URL anyway via the same code path).
    let row = queries::get_collection(&state.pool, id, None)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    if row.kind != "manual" {
        return Err(ApiError::validation(
            "auto-generated collections use TMDB art; local uploads are not supported",
        ));
    }

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
            if data.len() > MAX_ART_BYTES {
                return Err(ApiError::validation(format!(
                    "image must be ≤ {MAX_ART_BYTES} bytes"
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
    // Wipe sibling extensions so the GET handler doesn't serve a stale
    // file when the operator switches format mid-upload (jpg → png).
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

    // Version stamp on the URL so browser/CDN caches refetch the new
    // bytes after an overwrite. Blob handler ignores the query.
    let url = format!(
        "/api/v1/collections/{id}/{}?v={}",
        kind.url_suffix(),
        chimpflix_common::now_ms()
    );
    let patch = match kind {
        ArtKind::Poster => ManualCollectionUpdate {
            poster_path: Some(Some(url)),
            ..Default::default()
        },
        ArtKind::Backdrop => ManualCollectionUpdate {
            backdrop_path: Some(Some(url)),
            ..Default::default()
        },
    };
    queries::update_manual_collection(&state.pool, id, patch)
        .await
        .map_err(ApiError::Internal)?;

    audit(&state, actor.id, &headers, kind.action(), id, &()).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_poster_blob(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    serve_art_blob(&state, id, ArtKind::Poster).await
}

pub async fn get_backdrop_blob(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    serve_art_blob(&state, id, ArtKind::Backdrop).await
}

async fn serve_art_blob(state: &AppState, id: i64, kind: ArtKind) -> Result<Response, ApiError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn validate_name_rejects_control() {
        assert!(validate_name("hello\nworld").is_err());
    }

    #[test]
    fn validate_name_accepts_tab() {
        assert!(validate_name("hello\tworld").is_ok());
    }

    #[test]
    fn validate_name_rejects_too_long() {
        let long = "a".repeat(NAME_MAX + 1);
        assert!(validate_name(&long).is_err());
    }

    #[test]
    fn validate_name_accepts_typical() {
        assert!(validate_name("Sunday Night Sci-Fi").is_ok());
    }
}
