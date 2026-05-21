//! `/admin/library-health` — read-only report of common library
//! pathologies the owner should know about.
//!
//! Each section is a separate query so a single bad row doesn't poison
//! the whole report. Pure SELECTs against the existing schema — no new
//! tables, no background work, computed on demand.
//!
//! Sibling endpoint `/admin/library-health/items?category=…` returns the
//! ACTUAL ROWS behind each counter so admins can act on the findings
//! (re-match metadata, upload artwork, delete orphans) instead of just
//! staring at a number.

use axum::Json;
use axum::extract::{Query, State};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct LibraryHealthResponse {
    pub items_without_files: i64,
    pub items_without_metadata: i64,
    pub items_without_poster: i64,
    pub items_without_backdrop: i64,
    pub orphan_episodes: i64,
    pub orphan_media_files: i64,
    pub missing_files: Vec<MissingFileRow>,
    pub libraries_without_paths: Vec<LibraryNoPathRow>,
}

#[derive(Debug, Serialize)]
pub struct MissingFileRow {
    pub id: i64,
    pub path: String,
    pub item_title: Option<String>,
    pub episode_title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LibraryNoPathRow {
    pub id: i64,
    pub name: String,
}

pub async fn get(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<LibraryHealthResponse>, ApiError> {
    let pool = &state.pool;

    let items_without_files: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM items i
         WHERE i.kind = 'movie'
           AND NOT EXISTS (SELECT 1 FROM media_files mf WHERE mf.item_id = i.id)",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let items_without_metadata: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM items
         WHERE tmdb_id IS NULL
           AND imdb_id IS NULL
           AND tvdb_id IS NULL
           AND anilist_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let items_without_poster: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM items i
         WHERE NOT EXISTS (
            SELECT 1 FROM images img
            WHERE img.item_id = i.id AND img.kind = 'poster'
         )",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let items_without_backdrop: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM items i
         WHERE NOT EXISTS (
            SELECT 1 FROM images img
            WHERE img.item_id = i.id AND img.kind = 'backdrop'
         )",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let orphan_episodes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM episodes e
         WHERE NOT EXISTS (SELECT 1 FROM media_files mf WHERE mf.episode_id = e.id)",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let orphan_media_files: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM media_files
         WHERE item_id IS NULL AND episode_id IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    // "Missing files" requires touching the filesystem so we cap the
    // sample to avoid hammering disk on a giant library. The full
    // scrub-and-fix workflow belongs to a Tier-2 cleanup task; this
    // is just a preview.
    let candidate_rows = sqlx::query(
        "SELECT mf.id AS id, mf.path AS path, i.title AS item_title, e.title AS episode_title
         FROM media_files mf
         LEFT JOIN items i ON i.id = mf.item_id
         LEFT JOIN episodes e ON e.id = mf.episode_id
         ORDER BY mf.id DESC
         LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let mut missing_files = Vec::new();
    for row in candidate_rows {
        let path: String = row.try_get("path").unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        if !std::path::Path::new(&path).exists() {
            missing_files.push(MissingFileRow {
                id: row.try_get("id").unwrap_or(0),
                path,
                item_title: row.try_get("item_title").ok().flatten(),
                episode_title: row.try_get("episode_title").ok().flatten(),
            });
            if missing_files.len() >= 50 {
                break;
            }
        }
    }

    let libs_no_paths_rows = sqlx::query(
        "SELECT l.id AS id, l.name AS name
         FROM libraries l
         WHERE NOT EXISTS (SELECT 1 FROM library_paths lp WHERE lp.library_id = l.id)",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let libraries_without_paths = libs_no_paths_rows
        .iter()
        .map(|row| LibraryNoPathRow {
            id: row.try_get("id").unwrap_or(0),
            name: row.try_get("name").unwrap_or_default(),
        })
        .collect();

    // Silence the unused queries import (kept for symmetry with siblings
    // that do use it; trivially removable once this module needs a real
    // query helper).
    let _ = queries::vault_list_metadata;

    Ok(Json(LibraryHealthResponse {
        items_without_files,
        items_without_metadata,
        items_without_poster,
        items_without_backdrop,
        orphan_episodes,
        orphan_media_files,
        missing_files,
        libraries_without_paths,
    }))
}

// ─── Per-category drill-in ────────────────────────────────────────────────
//
// One uniform row shape across every category so the admin UI renders
// the same table regardless of which tile the operator clicked. The
// shape tells the frontend whether it can open the title modal
// (`item_id_for_modal`) — orphan media files and orphan episodes have
// no parent item so the link gracefully degrades to "no action".

#[derive(Debug, Serialize)]
pub struct HealthItemRow {
    /// Row id in the underlying table. Disambiguated by `kind`:
    /// `item`, `episode`, or `media_file`.
    pub id: i64,
    pub kind: &'static str,
    pub title: String,
    /// Year (movies) or `Season N · Episode N` (episodes) — caller-friendly
    /// extra context that doesn't fit in the title.
    pub subtitle: Option<String>,
    /// Library this row belongs to. None when the row IS the orphan
    /// (no item/episode to anchor a library).
    pub library_name: Option<String>,
    /// If the row maps to a clickable title in the main app, this is
    /// the items.id to pass to `?modal=<id>`. None for media-file-only
    /// rows.
    pub item_id_for_modal: Option<i64>,
    /// Filesystem path — only populated for `media_file` rows; lets the
    /// admin spot-check what they're about to delete.
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct HealthItemsQuery {
    pub category: String,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct HealthItemsResponse {
    pub category: String,
    pub total: i64,
    pub rows: Vec<HealthItemRow>,
}

/// Drill-in for the Library Health counters. `category` is one of:
///   - `no_files`         — movie items with no media_files
///   - `no_metadata`      — items lacking every external metadata id
///   - `no_poster`        — items with no poster image
///   - `no_backdrop`      — items with no backdrop image
///   - `orphan_episodes`  — episodes with no media_file (the show
///                          exists; just no file)
///   - `orphan_media_files` — media_files with neither item nor
///                            episode (scanner rejected the match)
///
/// Bounded result set (max 500 rows per call); the `total` count gives
/// the operator a "what you're seeing is N of M" indicator without
/// having to paginate through the whole set just to know the scope.
pub async fn items(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(q): Query<HealthItemsQuery>,
) -> Result<Json<HealthItemsResponse>, ApiError> {
    let pool = &state.pool;
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let offset = q.offset.unwrap_or(0).max(0);
    let category = q.category.as_str();

    // Each category gets its own SELECT — uniformising would mean
    // unsightly UNION-with-fillers. The branch is short; readability
    // wins over DRY here.
    let (rows, total): (Vec<HealthItemRow>, i64) = match category {
        "no_files" => {
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM items i
                 WHERE i.kind = 'movie'
                   AND NOT EXISTS (SELECT 1 FROM media_files mf WHERE mf.item_id = i.id)",
            )
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rs = sqlx::query(
                "SELECT i.id, i.title, i.year, l.name AS library_name
                 FROM items i
                 LEFT JOIN libraries l ON l.id = i.library_id
                 WHERE i.kind = 'movie'
                   AND NOT EXISTS (SELECT 1 FROM media_files mf WHERE mf.item_id = i.id)
                 ORDER BY i.title COLLATE NOCASE ASC
                 LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rows = rs
                .into_iter()
                .map(|r| {
                    let id: i64 = r.try_get("id").unwrap_or(0);
                    HealthItemRow {
                        id,
                        kind: "item",
                        title: r.try_get("title").unwrap_or_default(),
                        subtitle: r
                            .try_get::<Option<i32>, _>("year")
                            .ok()
                            .flatten()
                            .map(|y| y.to_string()),
                        library_name: r
                            .try_get::<Option<String>, _>("library_name")
                            .ok()
                            .flatten(),
                        item_id_for_modal: Some(id),
                        path: None,
                    }
                })
                .collect();
            (rows, total)
        }
        "no_metadata" => {
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM items
                 WHERE tmdb_id IS NULL
                   AND imdb_id IS NULL
                   AND tvdb_id IS NULL
                   AND anilist_id IS NULL",
            )
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rs = sqlx::query(
                "SELECT i.id, i.title, i.year, i.kind, l.name AS library_name
                 FROM items i
                 LEFT JOIN libraries l ON l.id = i.library_id
                 WHERE i.tmdb_id IS NULL
                   AND i.imdb_id IS NULL
                   AND i.tvdb_id IS NULL
                   AND i.anilist_id IS NULL
                 ORDER BY i.title COLLATE NOCASE ASC
                 LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rows = rs
                .into_iter()
                .map(|r| {
                    let id: i64 = r.try_get("id").unwrap_or(0);
                    let year: Option<i32> =
                        r.try_get::<Option<i32>, _>("year").ok().flatten();
                    let kind: String = r.try_get("kind").unwrap_or_default();
                    let kind_label = if kind == "show" { "Series" } else { "Film" };
                    let subtitle = match year {
                        Some(y) => Some(format!("{kind_label} · {y}")),
                        None => Some(kind_label.to_string()),
                    };
                    HealthItemRow {
                        id,
                        kind: "item",
                        title: r.try_get("title").unwrap_or_default(),
                        subtitle,
                        library_name: r
                            .try_get::<Option<String>, _>("library_name")
                            .ok()
                            .flatten(),
                        item_id_for_modal: Some(id),
                        path: None,
                    }
                })
                .collect();
            (rows, total)
        }
        "no_poster" | "no_backdrop" => {
            let img_kind = if category == "no_poster" { "poster" } else { "backdrop" };
            let count_sql =
                "SELECT COUNT(*) FROM items i
                 WHERE NOT EXISTS (
                    SELECT 1 FROM images img
                    WHERE img.item_id = i.id AND img.kind = ?
                 )";
            let total: i64 = sqlx::query_scalar(count_sql)
                .bind(img_kind)
                .fetch_one(pool)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
            let rs = sqlx::query(
                "SELECT i.id, i.title, i.year, i.kind, l.name AS library_name
                 FROM items i
                 LEFT JOIN libraries l ON l.id = i.library_id
                 WHERE NOT EXISTS (
                    SELECT 1 FROM images img
                    WHERE img.item_id = i.id AND img.kind = ?
                 )
                 ORDER BY i.title COLLATE NOCASE ASC
                 LIMIT ? OFFSET ?",
            )
            .bind(img_kind)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rows = rs
                .into_iter()
                .map(|r| {
                    let id: i64 = r.try_get("id").unwrap_or(0);
                    let year: Option<i32> =
                        r.try_get::<Option<i32>, _>("year").ok().flatten();
                    let kind: String = r.try_get("kind").unwrap_or_default();
                    let kind_label = if kind == "show" { "Series" } else { "Film" };
                    let subtitle = match year {
                        Some(y) => Some(format!("{kind_label} · {y}")),
                        None => Some(kind_label.to_string()),
                    };
                    HealthItemRow {
                        id,
                        kind: "item",
                        title: r.try_get("title").unwrap_or_default(),
                        subtitle,
                        library_name: r
                            .try_get::<Option<String>, _>("library_name")
                            .ok()
                            .flatten(),
                        item_id_for_modal: Some(id),
                        path: None,
                    }
                })
                .collect();
            (rows, total)
        }
        "orphan_episodes" => {
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM episodes e
                 WHERE NOT EXISTS (SELECT 1 FROM media_files mf WHERE mf.episode_id = e.id)",
            )
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rs = sqlx::query(
                "SELECT e.id, e.title AS ep_title, e.season_number, e.episode_number,
                        show.id AS show_id, show.title AS show_title,
                        l.name AS library_name
                 FROM episodes e
                 JOIN seasons s ON s.id = e.season_id
                 JOIN items show ON show.id = s.show_id
                 LEFT JOIN libraries l ON l.id = show.library_id
                 WHERE NOT EXISTS (SELECT 1 FROM media_files mf WHERE mf.episode_id = e.id)
                 ORDER BY show.title COLLATE NOCASE ASC,
                          e.season_number ASC, e.episode_number ASC
                 LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rows = rs
                .into_iter()
                .map(|r| {
                    let id: i64 = r.try_get("id").unwrap_or(0);
                    let ep_title: String = r.try_get("ep_title").unwrap_or_default();
                    let show_title: String =
                        r.try_get("show_title").unwrap_or_default();
                    let s: i64 = r.try_get("season_number").unwrap_or(0);
                    let e: i64 = r.try_get("episode_number").unwrap_or(0);
                    let show_id: Option<i64> = r.try_get("show_id").ok();
                    HealthItemRow {
                        id,
                        kind: "episode",
                        title: format!("{show_title} — {ep_title}"),
                        subtitle: Some(format!(
                            "Season {s} · Episode {e}",
                        )),
                        library_name: r
                            .try_get::<Option<String>, _>("library_name")
                            .ok()
                            .flatten(),
                        // Drill-in opens the parent show's modal so the
                        // admin lands on a season/episode list they can act on.
                        item_id_for_modal: show_id,
                        path: None,
                    }
                })
                .collect();
            (rows, total)
        }
        "orphan_media_files" => {
            let total: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM media_files
                 WHERE item_id IS NULL AND episode_id IS NULL",
            )
            .fetch_one(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rs = sqlx::query(
                "SELECT mf.id, mf.path
                 FROM media_files mf
                 WHERE mf.item_id IS NULL AND mf.episode_id IS NULL
                 ORDER BY mf.id DESC
                 LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let rows = rs
                .into_iter()
                .map(|r| {
                    let path: String = r.try_get("path").unwrap_or_default();
                    // Filename only in the title; full path on the
                    // subtitle. Admins recognise titles, not paths.
                    let title = std::path::Path::new(&path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(str::to_owned)
                        .unwrap_or_else(|| path.clone());
                    HealthItemRow {
                        id: r.try_get("id").unwrap_or(0),
                        kind: "media_file",
                        title,
                        subtitle: Some(path.clone()),
                        library_name: None,
                        item_id_for_modal: None,
                        path: Some(path),
                    }
                })
                .collect();
            (rows, total)
        }
        _ => {
            return Err(ApiError::validation(format!(
                "unknown category '{category}'; one of: no_files, no_metadata, no_poster, no_backdrop, orphan_episodes, orphan_media_files",
            )));
        }
    };

    Ok(Json(HealthItemsResponse {
        category: category.to_string(),
        total,
        rows,
    }))
}
