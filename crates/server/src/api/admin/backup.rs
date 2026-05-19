//! Owner-only admin endpoints.
//!
//! Backups use `VACUUM INTO` against a freshly-locked snapshot, written
//! to `<data_dir>/backups/` (one-shot download path) or
//! `<data_dir>/backups/auto/` (scheduled task; persisted long-term).
//!
//! Restore is **staged**, never in-process:
//!
//! 1. Operator picks an existing backup file (or uploads one).
//! 2. POST `/admin/backups/{filename}/stage-restore` copies the chosen
//!    file to `<data_dir>/chimpflix.db.pending-restore`.
//! 3. Operator restarts the server.
//! 4. On boot, `chimpflix_library::db::open` (or
//!    `chimpflix-server::main`) detects the pending-restore file,
//!    moves the current DB aside to `chimpflix.db.pre-restore-{stamp}`,
//!    renames the pending file into place, and opens normally.
//!
//! We avoid live in-process restore because the scheduler / webhooks /
//! file_watcher all hold long-lived `pool.clone()`s — closing the pool
//! mid-flight would leave them with stale connections and we have no
//! graceful shutdown surface yet. Staging means the swap is a single
//! atomic file move done before any task is alive.
//!
//! Why VACUUM INTO and not `.backup` over the wire: VACUUM INTO is one
//! atomic statement that produces a single self-contained .db file, no
//! WAL/SHM siblings to ship along. The downside is it briefly takes a
//! write lock on the source; acceptable since this is owner-triggered
//! and infrequent.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::http::header::USER_AGENT;
use axum::response::Response;
use chimpflix_library::NewAuditEntry;
use serde::Serialize;
use sqlx::Executor;
use tokio::fs;
use tokio_util::io::ReaderStream;
use tracing::{info, warn};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

/// Sub-directory holding scheduled-task auto backups. Kept distinct
/// from the ad-hoc `<data_dir>/backups/` so the cleanup-after-60s
/// rule on one-shot downloads doesn't accidentally sweep persistent
/// snapshots.
pub const AUTO_BACKUP_SUBDIR: &str = "backups/auto";

/// Filename of the staged restore target. When this file exists at
/// `<data_dir>/<STAGED_RESTORE_FILENAME>`, the next server boot
/// adopts it as the new DB (moving the current one aside first).
pub const STAGED_RESTORE_FILENAME: &str = "chimpflix.db.pending-restore";

pub async fn backup(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Response, ApiError> {
    let backups_dir = state.data_dir.join("backups");
    fs::create_dir_all(&backups_dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = format!("chimpflix-{stamp}.db");
    let path = backups_dir.join(&filename);

    // Ensure stale snapshots from a half-completed previous run don't
    // collide. Unlink first since VACUUM INTO fails if the target exists.
    if path.exists() {
        let _ = fs::remove_file(&path).await;
    }

    let target = path.to_string_lossy().to_string();
    // VACUUM INTO cannot use bound parameters for the path — we own the
    // string entirely (no user input concatenated) so this is safe.
    let stmt = format!("VACUUM INTO '{}'", target.replace('\'', "''"));
    state
        .pool
        .execute(stmt.as_str())
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let meta = fs::metadata(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let size = meta.len();
    info!(
        bytes = size,
        path = %path.display(),
        "created SQLite backup snapshot"
    );

    let file = fs::File::open(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let stream = ReaderStream::new(file);

    // Best-effort: schedule the snapshot file for unlink after the
    // response finishes. We can't easily do "on close" without wrapping
    // the body, so we spawn a small task that waits a beat and deletes.
    // The directory keeps a one-snapshot-at-a-time invariant via the
    // collision check above; if the delete fails the next backup will
    // overwrite it.
    let cleanup_path = path.clone();
    tokio::spawn(async move {
        // Give the response 60s to drain on slow links.
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        if let Err(e) = fs::remove_file(&cleanup_path).await {
            warn!(error = %e, path = %cleanup_path.display(), "failed to clean up backup snapshot");
        }
    });

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        )
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
        )
        .header(header::CONTENT_LENGTH, size)
        .body(Body::from_stream(stream))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    Ok(response)
}

// ─── Auto-backup management (list / download / delete / restore) ────────────

#[derive(Debug, Serialize)]
pub struct BackupEntry {
    /// Just the filename (e.g. `chimpflix-1715890000.db`). The
    /// frontend uses this as the URL segment for download / delete /
    /// stage-restore — `sanitize_backup_name` rejects anything else.
    pub filename: String,
    pub size_bytes: u64,
    /// Last-modified time as Unix epoch ms.
    pub modified_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct ListBackupsResponse {
    pub backups: Vec<BackupEntry>,
    /// True when a `chimpflix.db.pending-restore` is currently
    /// staged. Surfaced so the admin UI can show a "Restart server
    /// to apply" banner without re-polling.
    pub pending_restore: bool,
    /// Convenience for the UI: total bytes occupied by all
    /// listed snapshots. Operators use this to decide what to prune.
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct StageRestoreResponse {
    pub staged: String,
    /// Plain-language operator instructions: this stays user-facing
    /// because the actual restore only happens on next boot.
    pub message: String,
}

/// `GET /admin/backups` — list the persisted auto-backup snapshots.
/// Sorted newest-first by mtime so the most-likely-useful candidate
/// is at the top.
pub async fn list(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<ListBackupsResponse>, ApiError> {
    let dir = state.data_dir.join(AUTO_BACKUP_SUBDIR);
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let mut entries: Vec<BackupEntry> = Vec::new();
    let mut rd = fs::read_dir(&dir).await.map_err(|e| ApiError::Internal(e.into()))?;
    while let Some(entry) = rd.next_entry().await.map_err(|e| ApiError::Internal(e.into()))? {
        let path = entry.path();
        // Filter to expected naming so unrelated files (a stray
        // chimpflix.db copy, an editor swap file) don't appear.
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_string) else {
            continue;
        };
        if !is_valid_backup_name(&name) {
            continue;
        }
        let meta = entry.metadata().await.map_err(|e| ApiError::Internal(e.into()))?;
        if !meta.is_file() {
            continue;
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        entries.push(BackupEntry {
            filename: name,
            size_bytes: meta.len(),
            modified_ms,
        });
    }
    entries.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));

    let total_bytes = entries.iter().map(|e| e.size_bytes).sum();
    let pending_restore = fs::metadata(state.data_dir.join(STAGED_RESTORE_FILENAME))
        .await
        .is_ok();

    Ok(Json(ListBackupsResponse {
        backups: entries,
        pending_restore,
        total_bytes,
    }))
}

/// `GET /admin/backups/{filename}/download` — stream a specific
/// snapshot. We don't unlink afterwards (unlike the one-shot
/// `backup()` path) because auto-backups are meant to persist.
pub async fn download(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    AxumPath(filename): AxumPath<String>,
) -> Result<Response, ApiError> {
    let path = resolve_backup_path(&state.data_dir, &filename)?;
    let meta = fs::metadata(&path)
        .await
        .map_err(|_| ApiError::NotFound)?;
    let size = meta.len();
    let file = fs::File::open(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let stream = ReaderStream::new(file);
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        )
        .header(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
                .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
        )
        .header(header::CONTENT_LENGTH, size)
        .body(Body::from_stream(stream))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    Ok(response)
}

/// `DELETE /admin/backups/{filename}` — delete a single auto-backup
/// snapshot. Used to prune old backups by hand from the admin UI.
pub async fn delete(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    AxumPath(filename): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let path = resolve_backup_path(&state.data_dir, &filename)?;
    if !path.exists() {
        return Err(ApiError::NotFound);
    }
    fs::remove_file(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "backup.delete".into(),
            target_kind: Some("backup".into()),
            target_id: Some(filename.clone()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /admin/backups/{filename}/stage-restore` — copy the chosen
/// backup file to `<data_dir>/chimpflix.db.pending-restore`. The actual
/// restore happens on next server boot (see
/// `apply_pending_restore_if_present`). Returns plain text instructions
/// so the operator knows to restart.
pub async fn stage_restore(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    AxumPath(filename): AxumPath<String>,
) -> Result<Json<StageRestoreResponse>, ApiError> {
    let src = resolve_backup_path(&state.data_dir, &filename)?;
    if !src.exists() {
        return Err(ApiError::NotFound);
    }

    // Quick sanity check that the file is a SQLite DB by reading its
    // 16-byte header. Cheap: avoids staging a half-downloaded or
    // unrelated file as the next-boot DB.
    let header_bytes = fs::read(&src)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if header_bytes.len() < 16 || !header_bytes.starts_with(b"SQLite format 3\0") {
        return Err(ApiError::validation(format!(
            "`{filename}` does not look like a SQLite database file"
        )));
    }

    let staged = state.data_dir.join(STAGED_RESTORE_FILENAME);
    // Remove any previous staging so the operator can change their
    // mind without an explicit cancel call.
    if staged.exists() {
        let _ = fs::remove_file(&staged).await;
    }
    fs::copy(&src, &staged)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "backup.stage_restore".into(),
            target_kind: Some("backup".into()),
            target_id: Some(filename.clone()),
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;

    info!(staged = %staged.display(), src = %src.display(), "backup staged for restore");

    Ok(Json(StageRestoreResponse {
        staged: filename,
        message: "Backup staged. Restart the server to apply — your current database \
                  will be moved aside as chimpflix.db.pre-restore-<timestamp>.db so you \
                  can roll back if needed."
            .to_string(),
    }))
}

/// `POST /admin/backups/cancel-restore` — remove a previously-staged
/// `pending-restore` file. Useful when the operator stages a backup
/// then decides not to restart.
pub async fn cancel_restore(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let staged = state.data_dir.join(STAGED_RESTORE_FILENAME);
    if !staged.exists() {
        return Err(ApiError::NotFound);
    }
    fs::remove_file(&staged)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "backup.cancel_restore".into(),
            target_kind: Some("backup".into()),
            target_id: None,
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

// ─── Filename validation helpers ───────────────────────────────────────────

/// Reject anything that isn't a valid backup filename. Backup
/// filenames follow `chimpflix-<digits>.db` (matching what we write
/// from both the manual and scheduled paths). Anything else is
/// either a path-traversal attempt or stale junk we don't want to
/// touch.
pub fn is_valid_backup_name(name: &str) -> bool {
    if name.len() > 64 {
        return false;
    }
    // No path separators, no dot-dot, no NUL.
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
        return false;
    }
    // Pattern: chimpflix-<digits>.db
    let Some(rest) = name.strip_prefix("chimpflix-") else {
        return false;
    };
    let Some(stamp) = rest.strip_suffix(".db") else {
        return false;
    };
    !stamp.is_empty() && stamp.chars().all(|c| c.is_ascii_digit())
}

fn resolve_backup_path(data_dir: &Path, filename: &str) -> Result<PathBuf, ApiError> {
    if !is_valid_backup_name(filename) {
        return Err(ApiError::validation(format!(
            "`{filename}` is not a valid backup filename"
        )));
    }
    Ok(data_dir.join(AUTO_BACKUP_SUBDIR).join(filename))
}

/// At startup: check for a `chimpflix.db.pending-restore` file in
/// `data_dir`. When present:
///   1. Move the current `chimpflix.db` to
///      `chimpflix.db.pre-restore-<unix_stamp>.db` (rollback path).
///   2. Rename the pending file into place as `chimpflix.db`.
///   3. Also clean up any leftover `chimpflix.db-shm` / `-wal`
///      siblings of the OLD db — those belong to the connection
///      that's about to be replaced and would crash the new DB
///      open if left behind.
///
/// Idempotent: if the pending file doesn't exist, this is a no-op.
/// Best-effort: any individual step that fails logs + returns Ok so
/// boot still proceeds (operator can recover by hand).
pub async fn apply_pending_restore_if_present(data_dir: &Path) -> anyhow::Result<()> {
    let staged = data_dir.join(STAGED_RESTORE_FILENAME);
    if !staged.exists() {
        return Ok(());
    }
    let current = data_dir.join("chimpflix.db");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let rollback = data_dir.join(format!("chimpflix.db.pre-restore-{stamp}.db"));

    if current.exists() {
        if let Err(e) = fs::rename(&current, &rollback).await {
            warn!(
                error = %e,
                from = %current.display(),
                to = %rollback.display(),
                "pending restore: failed to move current DB to rollback path; aborting restore",
            );
            return Ok(());
        }
        // Sweep WAL/SHM siblings of the moved-aside DB — they're
        // tied to the old file's inode and the new DB will refuse
        // to open if we leave them where SQLite expects them.
        for ext in ["-shm", "-wal", "-journal"] {
            let sib = data_dir.join(format!("chimpflix.db{ext}"));
            if sib.exists() {
                if let Err(e) = fs::remove_file(&sib).await {
                    warn!(error = %e, path = %sib.display(), "pending restore: failed to remove sibling");
                }
            }
        }
    }

    if let Err(e) = fs::rename(&staged, &current).await {
        warn!(
            error = %e,
            from = %staged.display(),
            to = %current.display(),
            "pending restore: failed to rename staged file into place; restoring rollback",
        );
        // Best-effort restore of the rollback so we boot from
        // *something* rather than a missing DB.
        if rollback.exists() {
            let _ = fs::rename(&rollback, &current).await;
        }
        return Ok(());
    }
    info!(
        from = %staged.display(),
        to = %current.display(),
        rollback = %rollback.display(),
        "pending restore applied — old DB preserved at rollback path",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal_names() {
        assert!(!is_valid_backup_name(""));
        assert!(!is_valid_backup_name("../chimpflix.db"));
        assert!(!is_valid_backup_name("..\\chimpflix.db"));
        assert!(!is_valid_backup_name("/etc/passwd"));
        assert!(!is_valid_backup_name("chimpflix-..-1.db"));
        assert!(!is_valid_backup_name("chimpflix-1\0.db"));
        // Wrong prefix.
        assert!(!is_valid_backup_name("something-1.db"));
        // Wrong suffix.
        assert!(!is_valid_backup_name("chimpflix-1.sql"));
        // Non-digit stamp.
        assert!(!is_valid_backup_name("chimpflix-abc.db"));
        // Empty stamp.
        assert!(!is_valid_backup_name("chimpflix-.db"));
    }

    #[test]
    fn accepts_well_formed_names() {
        assert!(is_valid_backup_name("chimpflix-1.db"));
        assert!(is_valid_backup_name("chimpflix-1715890000.db"));
        assert!(is_valid_backup_name(
            "chimpflix-99999999999999999999999.db"
        ));
    }

    #[test]
    fn length_cap_holds() {
        // 64-char cap — anything longer is rejected as a safety net
        // (a real timestamp is ~10-13 chars; the cap exists so a
        // crafted long filename can't cause weird filesystem
        // behavior on path concat).
        let mut over = String::from("chimpflix-");
        while over.len() < 60 {
            over.push('1');
        }
        over.push_str(".db");
        if over.len() <= 64 {
            assert!(is_valid_backup_name(&over));
        }
        let huge: String = std::iter::repeat('1').take(80).collect();
        let huge_name = format!("chimpflix-{huge}.db");
        assert!(!is_valid_backup_name(&huge_name));
    }
}
