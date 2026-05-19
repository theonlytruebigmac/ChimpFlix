//! `/admin/items/bulk/*` — multi-item operator actions.
//!
//! Each endpoint takes a JSON `{item_ids: [...], ...}` body and applies
//! the operation per-item, returning a structured report with per-id
//! success/failure. Failures don't abort the batch — the operator
//! sees exactly which ids failed and why.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, queries, scanner};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

const MAX_BULK_ITEMS: usize = 500;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BulkRefreshRequest {
    pub item_ids: Vec<i64>,
}

#[derive(Debug, Serialize)]
pub struct BulkReport {
    pub ok: usize,
    pub failed: usize,
    pub errors: Vec<BulkError>,
}

#[derive(Debug, Serialize)]
pub struct BulkError {
    pub item_id: i64,
    pub error: String,
}

pub async fn refresh_metadata(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(req): Json<BulkRefreshRequest>,
) -> Result<Json<BulkReport>, ApiError> {
    cap_check(req.item_ids.len())?;
    let tmdb_snapshot = state.tmdb_snapshot().await;
    let Some(tmdb) = tmdb_snapshot.as_ref() else {
        return Err(ApiError::validation("TMDB enrichment is disabled"));
    };
    let tvdb_snapshot = state.tvdb_snapshot().await;
    let mut ok = 0usize;
    let mut errors: Vec<BulkError> = Vec::new();
    for id in &req.item_ids {
        match scanner::refresh_item_metadata(
            &state.pool,
            tmdb,
            tvdb_snapshot.as_ref(),
            state.tvmaze.as_ref(),
            *id,
            None,
        )
        .await
        {
            Ok(()) => ok += 1,
            Err(e) => errors.push(BulkError {
                item_id: *id,
                error: format!("{e:#}"),
            }),
        }
    }
    let failed = errors.len();
    audit_with(
        &state,
        actor.id,
        &headers,
        "items.bulk.refresh_metadata",
        &req,
    )
    .await;
    Ok(Json(BulkReport { ok, failed, errors }))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BulkTagRequest {
    pub item_ids: Vec<i64>,
    pub tag_name: String,
}

pub async fn add_tag(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(req): Json<BulkTagRequest>,
) -> Result<Json<BulkReport>, ApiError> {
    cap_check(req.item_ids.len())?;
    let trimmed = req.tag_name.trim().to_string();
    if trimmed.is_empty() {
        return Err(ApiError::validation("tag_name is empty"));
    }
    let mut ok = 0usize;
    let mut errors: Vec<BulkError> = Vec::new();
    for id in &req.item_ids {
        match queries::add_tag_to_item(&state.pool, *id, &trimmed).await {
            Ok(_) => ok += 1,
            Err(e) => errors.push(BulkError {
                item_id: *id,
                error: format!("{e:#}"),
            }),
        }
    }
    let failed = errors.len();
    audit_with(&state, actor.id, &headers, "items.bulk.add_tag", &req).await;
    Ok(Json(BulkReport { ok, failed, errors }))
}

pub async fn remove_tag(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(req): Json<BulkTagRequest>,
) -> Result<Json<BulkReport>, ApiError> {
    cap_check(req.item_ids.len())?;
    let trimmed = req.tag_name.trim().to_string();
    if trimmed.is_empty() {
        return Err(ApiError::validation("tag_name is empty"));
    }
    let mut ok = 0usize;
    let mut errors: Vec<BulkError> = Vec::new();
    for id in &req.item_ids {
        match queries::remove_tag_from_item_by_name(&state.pool, *id, &trimmed).await {
            Ok(_) => ok += 1,
            Err(e) => errors.push(BulkError {
                item_id: *id,
                error: format!("{e:#}"),
            }),
        }
    }
    let failed = errors.len();
    audit_with(&state, actor.id, &headers, "items.bulk.remove_tag", &req).await;
    Ok(Json(BulkReport { ok, failed, errors }))
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BulkDetectMarkersRequest {
    pub item_ids: Vec<i64>,
}

pub async fn detect_markers(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(req): Json<BulkDetectMarkersRequest>,
) -> Result<(StatusCode, Json<BulkReport>), ApiError> {
    cap_check(req.item_ids.len())?;
    let mut total_files: Vec<(i64, String, Option<i64>)> = Vec::new();
    let mut errors: Vec<BulkError> = Vec::new();
    let mut ok = 0usize;
    for id in &req.item_ids {
        match collect_files_for_item(&state, *id, actor.id).await {
            Ok(files) => {
                ok += 1;
                total_files.extend(files);
            }
            Err(e) => errors.push(BulkError {
                item_id: *id,
                error: format!("{e:#}"),
            }),
        }
    }
    if !total_files.is_empty() {
        crate::api::markers::spawn_detection(&state, total_files);
    }
    let failed = errors.len();
    audit_with(&state, actor.id, &headers, "items.bulk.detect_markers", &req).await;
    Ok((StatusCode::ACCEPTED, Json(BulkReport { ok, failed, errors })))
}

async fn collect_files_for_item(
    state: &AppState,
    item_id: i64,
    user_id: i64,
) -> anyhow::Result<Vec<(i64, String, Option<i64>)>> {
    let detail = queries::get_item_detail(&state.pool, item_id, user_id, None)
        .await?
        .ok_or_else(|| anyhow::anyhow!("item not found"))?;
    let mut out: Vec<(i64, String, Option<i64>)> = Vec::new();
    match detail.item.kind {
        chimpflix_library::ItemKind::Movie => {
            for f in &detail.files {
                let path = sqlx::query_scalar::<_, String>(
                    "SELECT path FROM media_files WHERE id = ?",
                )
                .bind(f.id)
                .fetch_one(&state.pool)
                .await?;
                out.push((f.id, path, f.duration_ms));
            }
        }
        chimpflix_library::ItemKind::Show => {
            let rows = sqlx::query_as::<_, (i64, String, Option<i64>)>(
                "SELECT mf.id, mf.path, mf.duration_ms
                 FROM media_files mf
                 JOIN episodes e ON e.id = mf.episode_id
                 JOIN seasons s ON s.id = e.season_id
                 WHERE s.show_id = ?",
            )
            .bind(item_id)
            .fetch_all(&state.pool)
            .await?;
            out.extend(rows);
        }
    }
    Ok(out)
}

fn cap_check(n: usize) -> Result<(), ApiError> {
    if n == 0 {
        return Err(ApiError::validation("item_ids is empty"));
    }
    if n > MAX_BULK_ITEMS {
        return Err(ApiError::validation(format!(
            "item_ids exceeds {MAX_BULK_ITEMS} cap"
        )));
    }
    Ok(())
}

async fn audit_with<T: Serialize>(
    state: &AppState,
    actor_id: i64,
    headers: &HeaderMap,
    action: &str,
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
            target_kind: Some("items_bulk".into()),
            target_id: None,
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}
