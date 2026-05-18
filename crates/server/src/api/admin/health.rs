//! `/admin/library-health` — read-only report of common library
//! pathologies the owner should know about.
//!
//! Each section is a separate query so a single bad row doesn't poison
//! the whole report. Pure SELECTs against the existing schema — no new
//! tables, no background work, computed on demand.

use axum::Json;
use axum::extract::State;
use chimpflix_library::queries;
use serde::Serialize;
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
