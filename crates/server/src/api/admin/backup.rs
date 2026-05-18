//! Owner-only admin endpoints.
//!
//! v1 surface: SQLite backup. Backups use `VACUUM INTO` against a
//! freshly-locked snapshot, written to `<data_dir>/backups/`. The file is
//! then streamed back to the caller and unlinked.
//!
//! Why VACUUM INTO and not `.backup` over the wire: VACUUM INTO is one
//! atomic statement that produces a single self-contained .db file, no
//! WAL/SHM siblings to ship along. The downside is it briefly takes a
//! write lock on the source; acceptable since this is owner-triggered
//! and infrequent. For production-grade hot backups we'd switch to
//! sqlite3_backup() through sqlx but that needs more plumbing than the
//! current ergonomic surface warrants.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use sqlx::Executor;
use tokio::fs;
use tokio_util::io::ReaderStream;
use tracing::{info, warn};

use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

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
