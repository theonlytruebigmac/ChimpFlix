//! `GET /admin/dashboard` — aggregated server status for the Admin UI.
//!
//! Today the dashboard is polled (5s on the client). The composite shape
//! is intentionally one round-trip: it returns server identity + library
//! stats + active transcodes + recent scans + disk usage. WS push for
//! near-real-time updates can layer on later without changing the shape.

use std::collections::HashSet;
use std::ffi::CString;
use std::path::Path as StdPath;

use axum::Json;
use axum::extract::State;
use chimpflix_common::now_ms;
use chimpflix_library::{LibraryStats, ScanJob, queries};
use chimpflix_transcoder::SessionSnapshot;
use serde::Serialize;

use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct DashboardResponse {
    pub server: ServerStatus,
    pub library_stats: Vec<LibraryStats>,
    pub active_transcodes: Vec<SessionSnapshot>,
    pub recent_scans: Vec<ScanJob>,
    pub disks: Vec<DiskUsage>,
}

#[derive(Debug, Serialize)]
pub struct ServerStatus {
    pub version: String,
    pub started_at_ms: i64,
    pub uptime_s: i64,
    pub now_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct DiskUsage {
    pub path: String,
    pub label: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
}

pub async fn get(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<DashboardResponse>, ApiError> {
    let now = now_ms();
    let uptime_s = ((now - state.started_at_ms) / 1000).max(0);

    let library_stats = queries::library_stats(&state.pool)
        .await
        .map_err(ApiError::Internal)?;

    let active_transcodes = state.transcoder.list_sessions();

    let recent_scans = queries::recent_scan_jobs(&state.pool, 10)
        .await
        .map_err(ApiError::Internal)?;

    // Disk usage: DATA_DIR + every distinct library path. Skip paths that
    // statvfs can't resolve (missing mount, permissions); the dashboard
    // surfaces what it can rather than failing the whole request.
    let mut paths: Vec<(String, String)> = Vec::new();
    paths.push((
        "Data".to_string(),
        state.data_dir.to_string_lossy().into_owned(),
    ));
    let lib_paths = sqlx::query("SELECT DISTINCT library_id, path FROM library_paths")
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    use sqlx::Row;
    for r in &lib_paths {
        let lib_id: i64 = r.try_get("library_id").unwrap_or(0);
        let path: String = r.try_get("path").unwrap_or_default();
        let label = library_stats
            .iter()
            .find(|s| s.library_id == lib_id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| format!("Library {lib_id}"));
        paths.push((label, path));
    }
    // Deduplicate by underlying mountpoint so two libraries on the same
    // filesystem don't double-count disk space. The cheap proxy is the
    // statvfs result tuple (total, free) — we only emit one row per such
    // pair, picking the friendliest label.
    let mut seen: HashSet<(u64, u64)> = HashSet::new();
    let mut disks = Vec::with_capacity(paths.len());
    for (label, path) in paths {
        if let Some((total, used)) = statvfs_usage(&path) {
            let key = (total, used);
            if seen.insert(key) {
                disks.push(DiskUsage {
                    path,
                    label,
                    total_bytes: total,
                    used_bytes: used,
                });
            }
        }
    }

    Ok(Json(DashboardResponse {
        server: ServerStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            started_at_ms: state.started_at_ms,
            uptime_s,
            now_ms: now,
        },
        library_stats,
        active_transcodes,
        recent_scans,
        disks,
    }))
}

/// Returns `(total_bytes, used_bytes)` for the filesystem hosting `path`,
/// or `None` if the path is not statvfs-able. Implemented with a direct
/// libc call to avoid pulling in `nix` / `sysinfo` for one syscall.
// `libc::statvfs`'s fields are `c_ulong`, which is already u64 on
// x86_64 Linux/macOS but u32 on 32-bit targets. The `as u64` casts
// are intentional cross-platform widening — clippy reads them as
// redundant on the CI host arch.
#[allow(clippy::unnecessary_cast)]
pub fn statvfs_usage(path: &str) -> Option<(u64, u64)> {
    if !StdPath::new(path).exists() {
        return None;
    }
    let c = CString::new(path).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid C string and `stat` is properly sized.
    let rc = unsafe { libc::statvfs(c.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    let block_size = stat.f_frsize as u64;
    let total = stat.f_blocks as u64 * block_size;
    let free = stat.f_bavail as u64 * block_size;
    Some((total, total.saturating_sub(free)))
}
