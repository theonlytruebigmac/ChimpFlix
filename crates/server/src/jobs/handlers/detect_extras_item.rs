//! `detect_extras_item` — discover trailers / featurettes / behind-
//! the-scenes / deleted scenes under one item's media directory.
//! Payload: `{ "item_id": i64 }`.
//!
//! Plex / Jellyfin convention places extras in either a sibling
//! `Extras/`, `Featurettes/`, `Behind The Scenes/`, … directory or as
//! files with a `-trailer.{ext}` suffix. The handler walks the item's
//! media directory once, classifies each match, and upserts into
//! `item_extras` (uniqued on `(item_id, path)`).
//!
//! Idempotency: the parent directory's mtime is stamped on the item
//! row after a successful scan; the handler skips if the mtime
//! hasn't advanced since the last run. A Sonarr/Radarr drop that
//! touches the dir (typical pattern) re-triggers the scan; a no-op
//! sweep against an unchanged dir is fast.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use tracing::info;

use crate::state::AppState;

pub const KIND: &str = "detect_extras_item";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub item_id: i64,
}

/// Video file extensions we treat as extras candidates. Matches the
/// scanner's accepted set so an extras file we'd accept here is also
/// one that ffprobe / the player can play.
const EXTRA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "m4v", "webm", "wmv", "ts", "mpg", "mpeg", "flv",
];

/// Subdirectory names → kind mapping. Case-insensitive match. Plex
/// and Jellyfin both ship this convention. Values align with the
/// `item_extras.kind` column set already used by the TMDB-fetched
/// (YouTube) extras path so the UI sees a single list per item.
const EXTRA_DIRS: &[(&str, &str)] = &[
    ("extras",            "clip"),
    ("featurettes",       "featurette"),
    ("behind the scenes", "behind_the_scenes"),
    ("deleted scenes",    "deleted_scene"),
    ("interviews",        "clip"),
    ("scenes",            "clip"),
    ("shorts",            "clip"),
    ("trailers",          "trailer"),
];

/// Filename suffixes (before the extension) → kind. Matches patterns
/// like `Movie Title-trailer.mp4`.
const EXTRA_SUFFIXES: &[(&str, &str)] = &[
    ("-trailer",         "trailer"),
    ("-teaser",          "teaser"),
    ("-behindthescenes", "behind_the_scenes"),
    ("-deleted",         "deleted_scene"),
    ("-featurette",      "featurette"),
    ("-interview",       "clip"),
    ("-scene",           "clip"),
    ("-short",           "clip"),
];

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { item_id } =
        serde_json::from_value(payload).context("invalid payload")?;

    let Some(item_dir) = resolve_item_directory(&state, item_id).await? else {
        // Item has no media files we can locate — nothing to scan.
        return Ok(());
    };

    let current_mtime = read_dir_mtime(&item_dir).await;
    let row = sqlx::query("SELECT extras_dir_mtime FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(&state.pool)
        .await
        .context("items extras_dir_mtime lookup")?;
    let last_mtime: Option<i64> = row
        .and_then(|r| r.try_get::<Option<i64>, _>("extras_dir_mtime").ok().flatten());

    if let (Some(cur), Some(prev)) = (current_mtime, last_mtime) {
        if cur <= prev {
            // Directory hasn't changed since last successful scan.
            return Ok(());
        }
    }

    let extras = walk_for_extras(&item_dir).await?;
    if !extras.is_empty() {
        upsert_extras(&state, item_id, &extras).await?;
    }

    let now = chimpflix_common::now_ms();
    sqlx::query(
        "UPDATE items
         SET extras_scanned_at = ?, extras_dir_mtime = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(now)
    .bind(current_mtime)
    .bind(now)
    .bind(item_id)
    .execute(&state.pool)
    .await
    .context("items extras watermark update")?;

    info!(item_id, count = extras.len(), "extras scan complete");
    Ok(())
}

/// Resolve the item's media directory. Movies: parent of any
/// `media_files.path` for the item. Shows: typically two parents up
/// from an episode file (show root, parent of season folders). We
/// take the shallowest directory shared by all of the item's files —
/// `PathBuf::ancestors()` of the first file works for the movie case;
/// for shows we use the heuristic of "walk up until we leave the
/// library_root prefix or we hit a directory whose name doesn't match
/// `Season *` / `Specials`".
async fn resolve_item_directory(state: &AppState, item_id: i64) -> Result<Option<PathBuf>> {
    let row = sqlx::query(
        "SELECT mf.path AS path, i.kind AS kind, l.root_path AS root
         FROM items i
         JOIN media_files mf ON mf.item_id = i.id
         JOIN libraries l ON l.id = i.library_id
         WHERE i.id = ? AND mf.removed_at IS NULL
         LIMIT 1",
    )
    .bind(item_id)
    .fetch_optional(&state.pool)
    .await
    .context("items + media_files lookup")?;
    let Some(row) = row else {
        return Ok(None);
    };
    let path: String = row.try_get("path")?;
    let kind: String = row.try_get("kind").unwrap_or_default();
    let root: String = row.try_get("root").unwrap_or_default();

    let file = PathBuf::from(&path);
    let mut dir = file.parent().map(Path::to_path_buf);

    // For shows, walk up out of the season folder so the extras scan
    // looks at the show root. Heuristic: keep walking up while the
    // current directory name starts with "season" (case-insensitive)
    // or equals "specials", and we're still under the library root.
    if kind == "show" {
        while let Some(d) = dir.as_ref() {
            let name = d
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let is_season_like = name.starts_with("season") || name == "specials";
            let still_in_library = d.starts_with(&root);
            if !(is_season_like && still_in_library) {
                break;
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
    Ok(dir)
}

/// Read the directory's mtime as epoch-millis. None if the path
/// doesn't exist or stat fails.
async fn read_dir_mtime(dir: &Path) -> Option<i64> {
    let meta = tokio::fs::metadata(dir).await.ok()?;
    let mtime = meta.modified().ok()?;
    Some(
        mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_millis() as i64,
    )
}

/// One discovered extra prior to DB insert.
struct DiscoveredExtra {
    kind: &'static str,
    path: PathBuf,
    title: String,
}

/// Walk the item directory looking for extras matches. Synchronous
/// std::fs inside a `spawn_blocking` so we don't tie up the runtime
/// on a large series root with many extras subfolders.
async fn walk_for_extras(item_dir: &Path) -> Result<Vec<DiscoveredExtra>> {
    let item_dir = item_dir.to_path_buf();
    let extras = tokio::task::spawn_blocking(move || walk_sync(&item_dir))
        .await
        .context("walk_for_extras join")??;
    Ok(extras)
}

fn walk_sync(item_dir: &Path) -> Result<Vec<DiscoveredExtra>> {
    let mut out: Vec<DiscoveredExtra> = Vec::new();

    // Pass 1: direct children of item_dir — pick up `*-trailer.mp4`
    // and friends, plus the named subdirectories.
    let entries = match std::fs::read_dir(item_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            let name_lc = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if let Some((_, kind)) =
                EXTRA_DIRS.iter().find(|(dirname, _)| *dirname == name_lc)
            {
                collect_dir_videos(&path, kind, &mut out)?;
            }
        } else if file_type.is_file() {
            if let Some((kind, title)) = classify_file_suffix(&path) {
                out.push(DiscoveredExtra {
                    kind,
                    path,
                    title,
                });
            }
        }
    }
    Ok(out)
}

fn collect_dir_videos(
    dir: &Path,
    kind: &'static str,
    out: &mut Vec<DiscoveredExtra>,
) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_video_file(&path) {
            continue;
        }
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        out.push(DiscoveredExtra {
            kind,
            path,
            title,
        });
    }
    Ok(())
}

fn is_video_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let ext_lc = ext.to_ascii_lowercase();
    EXTRA_EXTENSIONS.iter().any(|e| *e == ext_lc)
}

/// If a file's stem (before extension) ends in one of the recognized
/// suffixes, classify it and return the cleaned title. e.g.
/// `"Inception-trailer.mp4"` → `("trailer", "Inception")`.
fn classify_file_suffix(path: &Path) -> Option<(&'static str, String)> {
    if !is_video_file(path) {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let stem_lc = stem.to_ascii_lowercase();
    for (suffix, kind) in EXTRA_SUFFIXES {
        if stem_lc.ends_with(suffix) {
            let cleaned = stem[..stem.len() - suffix.len()].trim_end_matches(['.', '_', ' ']);
            return Some((kind, cleaned.to_string()));
        }
    }
    None
}

async fn upsert_extras(
    state: &AppState,
    item_id: i64,
    extras: &[DiscoveredExtra],
) -> Result<()> {
    // The `item_extras` table is shared with the TMDB-fetched
    // (YouTube) path. Local discoveries write rows with
    // `source = 'local'` and `source_id = <absolute path>`; uniquely
    // keyed by `(item_id, source, source_id)`. ON CONFLICT DO NOTHING
    // means first-discovery wins — manual title edits stick because
    // we don't update_set after the initial insert.
    let mut tx = state.pool.begin().await?;
    for e in extras {
        let path_str = e.path.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO item_extras
                (item_id, kind, title, source, source_id)
             VALUES (?, ?, ?, 'local', ?)
             ON CONFLICT(item_id, source, source_id) DO NOTHING",
        )
        .bind(item_id)
        .bind(e.kind)
        .bind(&e.title)
        .bind(&path_str)
        .execute(&mut *tx)
        .await
        .context("item_extras insert")?;
    }
    tx.commit().await?;
    Ok(())
}

/// Enqueue one `detect_extras_item` job per item id. Deduped on
/// item_id so a re-trigger while jobs are in flight is safe.
pub async fn enqueue_for_items(
    pool: &sqlx::SqlitePool,
    item_ids: &[i64],
) -> Result<usize> {
    let mut queued = 0usize;
    for &item_id in item_ids {
        let payload = serde_json::json!({ "item_id": item_id });
        let res = queries::enqueue_job_unique(
            pool,
            queries::JobInput::new(KIND, payload),
            "item_id",
            item_id,
        )
        .await?;
        if res.is_some() {
            queued += 1;
        }
    }
    Ok(queued)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn classify_trailer_suffix() {
        let p = PathBuf::from("/movies/Inception-trailer.mp4");
        let (k, t) = classify_file_suffix(&p).unwrap();
        assert_eq!(k, "trailer");
        assert_eq!(t, "Inception");
    }

    #[test]
    fn classify_strips_separator_runs() {
        let p = PathBuf::from("/movies/Some Title  -trailer.mkv");
        let (_, t) = classify_file_suffix(&p).unwrap();
        assert_eq!(t, "Some Title");
    }

    #[test]
    fn classify_non_matching_filename_returns_none() {
        let p = PathBuf::from("/movies/Inception.mp4");
        assert!(classify_file_suffix(&p).is_none());
    }

    #[test]
    fn classify_rejects_non_video_extension() {
        let p = PathBuf::from("/movies/Inception-trailer.txt");
        assert!(classify_file_suffix(&p).is_none());
    }

    #[test]
    fn classify_featurette_suffix() {
        let p = PathBuf::from("/movies/Movie-featurette.mp4");
        let (k, _) = classify_file_suffix(&p).unwrap();
        assert_eq!(k, "featurette");
    }

    #[test]
    fn is_video_file_matches_common_extensions() {
        assert!(is_video_file(Path::new("/a.mp4")));
        assert!(is_video_file(Path::new("/a.MKV")));
        assert!(!is_video_file(Path::new("/a.srt")));
        assert!(!is_video_file(Path::new("/a")));
    }
}
