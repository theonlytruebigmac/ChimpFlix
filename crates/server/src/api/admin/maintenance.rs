//! /admin/logs, /admin/alerts, /admin/privacy — Phase 10.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, queries};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::log_buffer::LogLine;
use crate::state::AppState;

// ─── Logs ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LogsParams {
    pub level: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub lines: Vec<LogLine>,
}

pub async fn logs(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(params): Query<LogsParams>,
) -> Result<Json<LogsResponse>, ApiError> {
    let limit = params.limit.unwrap_or(200).min(2_000);
    let lines = state.log_buffer.snapshot(params.level.as_deref(), limit);
    Ok(Json(LogsResponse { lines }))
}

// ─── Alerts ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AlertsParams {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AlertsResponse {
    pub log_alerts: Vec<LogLine>,
    pub audit: Vec<chimpflix_library::AuditLogEntry>,
}

pub async fn alerts(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(params): Query<AlertsParams>,
) -> Result<Json<AlertsResponse>, ApiError> {
    let limit = params.limit.unwrap_or(50);
    // Alerts surface = recent WARN/ERROR log lines + the audit feed.
    let log_alerts = state.log_buffer.snapshot(Some("WARN"), limit as usize);
    let audit = queries::list_audit(&state.pool, None, limit)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AlertsResponse { log_alerts, audit }))
}

// ─── Privacy ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PrivacyResponse {
    pub telemetry_opt_in: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PrivacyUpdate {
    pub telemetry_opt_in: bool,
}

pub async fn get_privacy(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<PrivacyResponse>, ApiError> {
    let s = state.settings.read().await.clone();
    Ok(Json(PrivacyResponse {
        telemetry_opt_in: s.telemetry_opt_in,
    }))
}

pub async fn patch_privacy(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<PrivacyUpdate>,
) -> Result<Json<PrivacyResponse>, ApiError> {
    let updated = queries::update_server_settings(
        &state.pool,
        Some(actor.id),
        chimpflix_library::ServerSettingsUpdate {
            telemetry_opt_in: Some(input.telemetry_opt_in),
            ..Default::default()
        },
    )
    .await
    .map_err(ApiError::Internal)?;
    {
        let mut g = state.settings.write().await;
        *g = updated.clone();
    }

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "privacy.update".into(),
            target_kind: Some("settings".into()),
            target_id: Some("1".into()),
            payload_json: serde_json::to_string(&input).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok(Json(PrivacyResponse {
        telemetry_opt_in: updated.telemetry_opt_in,
    }))
}

// ---------------------------------------------------------------------------
// One-click instance-wide maintenance actions
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct VerifyAllResponse {
    pub libraries_checked: usize,
    pub files_checked: usize,
    pub files_missing: usize,
    pub newly_marked_removed: u64,
    pub returned_files: usize,
}

/// Run verify across every library in one go. Mirrors the scheduled
/// `verify_libraries` task but returns a structured report instead
/// of writing to the task-run log. Slow on cold-cache file systems
/// because every media_file gets stat()'d — surface a spinner.
pub async fn verify_all(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<VerifyAllResponse>, ApiError> {
    let libs = queries::list_libraries(&state.pool, None)
        .await
        .map_err(ApiError::Internal)?;
    let mut out = VerifyAllResponse {
        libraries_checked: 0,
        files_checked: 0,
        files_missing: 0,
        newly_marked_removed: 0,
        returned_files: 0,
    };
    for lib in libs {
        match queries::verify_library(&state.pool, lib.id).await {
            Ok(r) => {
                out.libraries_checked += 1;
                out.files_checked += r.files_checked;
                out.files_missing += r.files_missing;
                out.newly_marked_removed += r.newly_marked_removed;
                out.returned_files += r.returned_files;
            }
            Err(e) => {
                tracing::warn!(
                    library_id = lib.id,
                    error = %format!("{e:#}"),
                    "verify_all: per-library failure"
                );
            }
        }
    }
    Ok(Json(out))
}

#[derive(Debug, Deserialize, Default)]
pub struct PurgeAllQuery {
    /// 0 means "purge every soft-deleted file regardless of age".
    /// Default is 7 days so a casual click of the button doesn't
    /// accidentally nuke a temp-unmounted drive.
    #[serde(default)]
    pub grace_days: Option<i64>,
}

#[derive(Serialize)]
pub struct PurgeAllResponse {
    pub files_purged: u64,
    pub episodes_purged: u64,
    pub seasons_purged: u64,
    pub items_purged: u64,
}

/// Hard-delete every soft-deleted file row past the grace window
/// across the entire instance + cascade through episodes / seasons /
/// items. Pairs with the scheduled `purge_removed_files` task — same
/// underlying query, just on-demand.
pub async fn purge_all(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(q): Query<PurgeAllQuery>,
) -> Result<Json<PurgeAllResponse>, ApiError> {
    let grace_days = q.grace_days.unwrap_or(7).max(0);
    let cutoff_ms = chimpflix_common::now_ms() - grace_days * 86_400_000;
    let report = queries::purge_removed_media_files(&state.pool, cutoff_ms)
        .await
        .map_err(ApiError::Internal)?;
    // Best-effort cache eviction for the just-purged files. Spawned
    // so a bulk purge doesn't block the HTTP response on hundreds
    // of small filesystem deletes.
    if !report.purged_paths.is_empty() {
        let cache_root = state.transcoder.cache_root().to_path_buf();
        let paths = report.purged_paths.clone();
        tokio::spawn(async move {
            for p in paths {
                let _ = chimpflix_transcoder::evict_text_subs_cache(
                    &cache_root,
                    std::path::Path::new(&p),
                )
                .await;
            }
        });
    }
    Ok(Json(PurgeAllResponse {
        files_purged: report.files_purged,
        episodes_purged: report.episodes_purged,
        seasons_purged: report.seasons_purged,
        items_purged: report.items_purged,
    }))
}

#[derive(Serialize)]
pub struct VacuumResponse {
    pub bytes_reclaimed: i64,
    pub before_bytes: i64,
    pub after_bytes: i64,
    pub duration_ms: i64,
}

/// Run `VACUUM` on the SQLite database. Rebuilds the file from
/// scratch, defragmenting pages and shrinking the on-disk size.
/// Blocking — SQLite holds an exclusive lock for the duration; on
/// our scale (< 1 GB DB) it takes a couple of seconds.
pub async fn vacuum_database(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<VacuumResponse>, ApiError> {
    let db_path = state.data_dir.join("chimpflix.db");
    let before = tokio::fs::metadata(&db_path)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    let started = chimpflix_common::now_ms();
    sqlx::query("VACUUM")
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let after = tokio::fs::metadata(&db_path)
        .await
        .map(|m| m.len() as i64)
        .unwrap_or(0);
    let duration_ms = chimpflix_common::now_ms() - started;
    Ok(Json(VacuumResponse {
        bytes_reclaimed: before - after,
        before_bytes: before,
        after_bytes: after,
        duration_ms,
    }))
}

#[derive(Serialize)]
pub struct ClearTranscodeCacheResponse {
    pub sessions_removed: usize,
    pub bytes_freed: i64,
}

/// Remove every transcoder session directory currently on disk.
/// Active sessions are NOT killed (the running ffmpeg keeps writing
/// to its dir); only orphan dirs left behind by previous server
/// crashes or unclean shutdowns get reaped. Operator escape hatch
/// for "the cache is full of stale segments and I want them gone".
pub async fn clear_transcode_cache(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<ClearTranscodeCacheResponse>, ApiError> {
    let cache_root = state.data_dir.join("cache/sessions");
    // Set of currently-live session ids — directories matching one
    // of these are skipped (the encoder is still writing to them).
    let live: std::collections::HashSet<String> = state
        .transcoder
        .list_sessions()
        .into_iter()
        .map(|s| s.id)
        .collect();
    let mut removed = 0usize;
    let mut bytes = 0i64;
    let mut entries = match tokio::fs::read_dir(&cache_root).await {
        Ok(e) => e,
        Err(_) => {
            return Ok(Json(ClearTranscodeCacheResponse {
                sessions_removed: 0,
                bytes_freed: 0,
            }));
        }
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        if live.contains(&name) {
            continue;
        }
        // Sum bytes before removing so we can report savings.
        if let Ok(dir_bytes) = dir_size_recursive(&entry.path()).await {
            bytes += dir_bytes;
        }
        if tokio::fs::remove_dir_all(entry.path()).await.is_ok() {
            removed += 1;
        }
    }
    Ok(Json(ClearTranscodeCacheResponse {
        sessions_removed: removed,
        bytes_freed: bytes,
    }))
}

/// Recursive total size of a directory in bytes. Returns 0 on any
/// traversal error — this powers a "bytes reclaimed" stat, so an
/// undercount is OK; we'd rather not 500 on a transient FS error.
async fn dir_size_recursive(path: &std::path::Path) -> std::io::Result<i64> {
    let mut total: i64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&p).await?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let meta = entry.metadata().await?;
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len() as i64;
            }
        }
    }
    Ok(total)
}
