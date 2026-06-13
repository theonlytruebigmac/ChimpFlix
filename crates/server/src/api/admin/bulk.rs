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
const TAG_NAME_MAX: usize = 100;

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
    if trimmed.chars().count() > TAG_NAME_MAX {
        return Err(ApiError::validation(format!(
            "tag_name exceeds {TAG_NAME_MAX} characters"
        )));
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
    if trimmed.chars().count() > TAG_NAME_MAX {
        return Err(ApiError::validation(format!(
            "tag_name exceeds {TAG_NAME_MAX} characters"
        )));
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

// ---------------------------------------------------------------------------
// Whole-library bulk operations
//
// Unlike the per-item ops above (which act on a hand-selected page of
// rows), these act on an ENTIRE library in one pass — the model the
// maintenance redesign reframes around. Mark watched/unwatched applies
// to the ACTING operator only (Plex semantics), re-scan reuses the
// per-library scan trigger, and delete is destructive behind a typed
// confirmation contract.
// ---------------------------------------------------------------------------

/// Op selector for [`library_op`]. Serialized as a lowercase tag so the
/// frontend sends `{"op": "mark_watched", ...}`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryBulkOp {
    MarkWatched,
    MarkUnwatched,
    Rescan,
    Delete,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LibraryBulkRequest {
    pub library_id: i64,
    pub op: LibraryBulkOp,
    /// Required ONLY for `delete`. The operator must echo the library
    /// id here AND type the exact library name into `confirm_name` —
    /// either missing or mismatched rejects the request with a 400.
    #[serde(default)]
    pub confirm_library_id: Option<i64>,
    #[serde(default)]
    pub confirm_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LibraryBulkResponse {
    pub library_id: i64,
    pub op: LibraryBulkOp,
    /// Items (movie library) or episodes (show/anime) whose play-state
    /// changed, for mark watched/unwatched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affected: Option<u64>,
    /// Top-level items deleted, for the delete op (children cascade).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_items: Option<u64>,
    /// Queued scan job id, for the re-scan op.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_job_id: Option<i64>,
    /// Human-readable one-liner the UI drops straight into a cf-banner.
    pub message: String,
}

/// `POST /admin/libraries/bulk` — apply one operation to an entire
/// library. Owner-gated (the whole module is). The destructive delete
/// path additionally requires the typed-confirmation contract; the
/// other ops run immediately.
pub async fn library_op(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(req): Json<LibraryBulkRequest>,
) -> Result<Json<LibraryBulkResponse>, ApiError> {
    // 404 if the library doesn't exist — do this before any mutation so
    // a typo'd id can't, e.g., silently affect zero rows and look "ok".
    let library = queries::get_library(&state.pool, req.library_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    match req.op {
        LibraryBulkOp::MarkWatched | LibraryBulkOp::MarkUnwatched => {
            let watched = req.op == LibraryBulkOp::MarkWatched;
            let affected =
                queries::mark_library_watched(&state.pool, actor.id, req.library_id, watched)
                    .await
                    .map_err(ApiError::Internal)?;
            let verb = if watched { "watched" } else { "unwatched" };
            audit_library(
                &state,
                actor.id,
                &headers,
                if watched {
                    "library.bulk.mark_watched"
                } else {
                    "library.bulk.mark_unwatched"
                },
                req.library_id,
                &json!({
                    "library_id": req.library_id,
                    "library_name": &library.name,
                    "affected": affected,
                }),
            )
            .await;
            Ok(Json(LibraryBulkResponse {
                library_id: req.library_id,
                op: req.op,
                affected: Some(affected),
                deleted_items: None,
                scan_job_id: None,
                message: format!("Marked {affected} title(s) {verb} for you in “{}”.", library.name),
            }))
        }
        LibraryBulkOp::Rescan => {
            // Reuse the exact on-demand scan trigger (per-library lock,
            // exclusivity gate, pipeline emitter, 409-if-already-running).
            let job = crate::api::libraries::spawn_library_scan(&state, req.library_id).await?;
            audit_library(
                &state,
                actor.id,
                &headers,
                "library.bulk.rescan",
                req.library_id,
                &json!({
                    "library_id": req.library_id,
                    "library_name": &library.name,
                    "scan_job_id": job.id,
                }),
            )
            .await;
            Ok(Json(LibraryBulkResponse {
                library_id: req.library_id,
                op: req.op,
                affected: None,
                deleted_items: None,
                scan_job_id: Some(job.id),
                message: format!("Queued a re-scan of “{}”.", library.name),
            }))
        }
        LibraryBulkOp::Delete => {
            // Typed-confirmation contract — BOTH must hold or we reject:
            //   1. confirm_library_id must equal library_id
            //   2. confirm_name must equal the library's exact name
            // This makes a delete impossible to fire by replaying a
            // wrong-id payload or by a single fat-fingered click.
            if req.confirm_library_id != Some(req.library_id) {
                return Err(ApiError::validation(
                    "confirm_library_id must echo library_id to confirm deletion",
                ));
            }
            match req.confirm_name.as_deref() {
                Some(name) if name == library.name => {}
                _ => {
                    return Err(ApiError::validation(
                        "confirm_name must exactly match the library name to confirm deletion",
                    ));
                }
            }

            // Snapshot the count first so the audit row records what was
            // actually removed (post-delete the rows are gone).
            let item_count = queries::count_library_items(&state.pool, req.library_id)
                .await
                .map_err(ApiError::Internal)?;
            let deleted = queries::delete_library_content(&state.pool, req.library_id)
                .await
                .map_err(ApiError::Internal)?;
            audit_library(
                &state,
                actor.id,
                &headers,
                "library.bulk.delete_content",
                req.library_id,
                &json!({
                    "library_id": req.library_id,
                    "library_name": &library.name,
                    "items_before": item_count,
                    "deleted_items": deleted,
                }),
            )
            .await;
            Ok(Json(LibraryBulkResponse {
                library_id: req.library_id,
                op: req.op,
                affected: None,
                deleted_items: Some(deleted),
                scan_job_id: None,
                message: format!(
                    "Deleted {deleted} item(s) and all their files/episodes from “{}”. The library itself remains.",
                    library.name
                ),
            }))
        }
    }
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

/// Same as [`audit_with`] but tags the row to the affected library
/// (`target_kind = "library_bulk"`, `target_id = library_id`) so a
/// forensic query can filter destructive whole-library actions by the
/// library they hit.
async fn audit_library<T: Serialize>(
    state: &AppState,
    actor_id: i64,
    headers: &HeaderMap,
    action: &str,
    library_id: i64,
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
            target_kind: Some("library_bulk".into()),
            target_id: Some(library_id.to_string()),
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}
