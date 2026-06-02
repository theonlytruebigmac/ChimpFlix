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
    /// Global count of movie items across every library. Lets the Library
    /// hero tile read "N · M movies · K eps" without the client having to
    /// know which library kind a row belongs to.
    pub movie_count: i64,
    /// Global count of episodes across every library (one row per episode
    /// in the `episodes` table).
    pub episode_count: i64,
    pub active_transcodes: Vec<DashboardSession>,
    pub recent_scans: Vec<ScanJob>,
    pub disks: Vec<DiskUsage>,
}

/// A live transcode session enriched with human-resolved fields. Wraps
/// the in-memory [`SessionSnapshot`] (flattened so the wire shape is
/// unchanged for existing fields) and adds `username` / `title` /
/// `subtitle`, resolved via a batched DB lookup so the admin dashboard
/// can render names instead of raw ids. Resolution is best-effort: a
/// missing user / media row leaves the field `None` and the client
/// falls back to the id.
#[derive(Debug, Serialize)]
pub struct DashboardSession {
    #[serde(flatten)]
    pub snapshot: SessionSnapshot,
    /// Preferred display name for `user_id` (`display_name` ?? `username`).
    pub username: Option<String>,
    /// Human title for `media_file_id` — movie title, or show name for
    /// an episode (with the episode descriptor in `subtitle`).
    pub title: Option<String>,
    /// "S{n}E{n} — Episode title" for episode sessions; `None` for movies.
    pub subtitle: Option<String>,
}

/// Resolve a batch of live sessions into [`DashboardSession`] DTOs.
/// Collects the distinct user + media-file ids across all sessions and
/// runs one lookup each (no N+1), then stitches the resolved strings
/// back onto each session. Shared by the dashboard and the stats
/// now-playing endpoint.
pub async fn enrich_sessions(
    state: &AppState,
    sessions: Vec<SessionSnapshot>,
) -> Vec<DashboardSession> {
    let user_ids: Vec<i64> = {
        let mut s: Vec<i64> = sessions.iter().map(|s| s.user_id).collect();
        s.sort_unstable();
        s.dedup();
        s
    };
    let media_file_ids: Vec<i64> = {
        let mut s: Vec<i64> = sessions.iter().map(|s| s.media_file_id).collect();
        s.sort_unstable();
        s.dedup();
        s
    };
    // Best-effort: a lookup failure leaves the map empty and every
    // field falls back to the id on the client. Never sink the
    // dashboard over a name resolution miss.
    let names = queries::resolve_user_display_names(&state.pool, &user_ids)
        .await
        .unwrap_or_default();
    let titles = queries::resolve_media_file_titles(&state.pool, &media_file_ids)
        .await
        .unwrap_or_default();
    sessions
        .into_iter()
        .map(|snapshot| {
            let username = names.get(&snapshot.user_id).cloned();
            let resolved = titles.get(&snapshot.media_file_id);
            let title = resolved.map(|t| t.title.clone());
            let subtitle = resolved.and_then(|t| t.subtitle.clone());
            DashboardSession {
                snapshot,
                username,
                title,
                subtitle,
            }
        })
        .collect()
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

    // Global movies-vs-episodes split for the Library hero tile. Movies are
    // `items` rows with kind 'movie'; episodes are every DOWNLOADED episode
    // (one with a live media_files row). Placeholder rows — materialized to
    // complete in-progress / future seasons for the finale flag + calendar —
    // have no file and are NOT content the operator has, so they're excluded
    // here. Both are best-effort — a count failure shouldn't sink the whole
    // dashboard, so fall back to 0 like the disk-usage rows do.
    let movie_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE kind = 'movie'")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);
    let episode_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM episodes e
         WHERE EXISTS (SELECT 1 FROM media_files mf
                       WHERE mf.episode_id = e.id AND mf.removed_at IS NULL)",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap_or(0);

    let active_transcodes = enrich_sessions(&state, state.transcoder.list_sessions()).await;

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
        movie_count,
        episode_count,
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
