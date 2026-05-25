//! `/admin/items/bulk/*` — multi-item operator actions.
//!
//! Each endpoint takes a JSON `{item_ids: [...], ...}` body and applies
//! the operation per-item, returning a structured report with per-id
//! success/failure. Failures don't abort the batch — the operator
//! sees exactly which ids failed and why.

use axum::Json;
use axum::extract::State;
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, StatusCode};
use chimpflix_library::{NewAuditEntry, queries, scanner};
use serde::{Deserialize, Serialize};
use serde_json::json;

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
    let tvdb_snapshot = state.tvdb_snapshot().await;
    let anilist_snapshot = state.anilist_snapshot().await;
    let omdb_snapshot = state.omdb_snapshot().await;
    let mut ok = 0usize;
    let mut errors: Vec<BulkError> = Vec::new();
    for id in &req.item_ids {
        match scanner::refresh_item_metadata(
            &state.pool,
            tmdb_snapshot.as_ref(),
            tvdb_snapshot.as_ref(),
            state.tvmaze.as_ref(),
            anilist_snapshot.as_ref(),
            omdb_snapshot.as_ref(),
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
        &with_counts(&req, req.item_ids.len(), ok, failed),
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
    audit_with(
        &state,
        actor.id,
        &headers,
        "items.bulk.add_tag",
        &with_counts(&req, req.item_ids.len(), ok, failed),
    )
    .await;
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
    audit_with(
        &state,
        actor.id,
        &headers,
        "items.bulk.remove_tag",
        &with_counts(&req, req.item_ids.len(), ok, failed),
    )
    .await;
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
    let mut errors: Vec<BulkError> = Vec::new();
    let mut ok = 0usize;
    // Fan out per-file (not per-item) so each media_file_id gets its
    // own queue row with independent retry/dedup. A bulk-detect on
    // 50 shows ends up as N×episodes jobs rather than 50 — fine for
    // the queue (SQLite handles tens of thousands of rows trivially)
    // and gives the worker pool finer interleaving.
    for id in &req.item_ids {
        let detail = match queries::get_item_detail(&state.pool, *id, actor.id, None).await {
            Ok(Some(d)) => d,
            Ok(None) => {
                errors.push(BulkError {
                    item_id: *id,
                    error: "item not found".into(),
                });
                continue;
            }
            Err(e) => {
                errors.push(BulkError {
                    item_id: *id,
                    error: format!("{e:#}"),
                });
                continue;
            }
        };
        let file_ids: Vec<i64> = match detail.item.kind {
            chimpflix_library::ItemKind::Movie => detail.files.iter().map(|f| f.id).collect(),
            chimpflix_library::ItemKind::Show => {
                match sqlx::query_scalar::<_, i64>(
                    "SELECT mf.id
                     FROM media_files mf
                     JOIN episodes e ON e.id = mf.episode_id
                     JOIN seasons s ON s.id = e.season_id
                     WHERE s.show_id = ? AND mf.removed_at IS NULL",
                )
                .bind(*id)
                .fetch_all(&state.pool)
                .await
                {
                    Ok(rows) => rows,
                    Err(e) => {
                        errors.push(BulkError {
                            item_id: *id,
                            error: format!("{e:#}"),
                        });
                        continue;
                    }
                }
            }
        };
        match crate::jobs::handlers::detect_markers_file::enqueue_for_files(&state.pool, &file_ids)
            .await
        {
            Ok(_) => ok += 1,
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
        "items.bulk.detect_markers",
        &with_counts(&req, req.item_ids.len(), ok, failed),
    )
    .await;
    Ok((
        StatusCode::ACCEPTED,
        Json(BulkReport { ok, failed, errors }),
    ))
}

/// Wrap a request payload with the post-execution outcome so the
/// audit row shows "operator ran X against 50 items, 47 succeeded, 3
/// failed" instead of just "operator ran X against [some ids]".
/// Forensics for bulk operations need the counts; the bare request
/// body buries them in `item_ids.len()` and reveals nothing about
/// what actually happened.
fn with_counts<T: Serialize>(
    req: &T,
    affected_item_count: usize,
    ok: usize,
    failed: usize,
) -> serde_json::Value {
    json!({
        "request": req,
        "affected_item_count": affected_item_count,
        "ok": ok,
        "failed": failed,
    })
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
