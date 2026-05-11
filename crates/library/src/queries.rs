//! SQL data access for libraries, scan jobs, items, and the upserts the
//! scanner needs.
//!
//! Plain `sqlx::query` / `query_as` — no `query!` macros, so we don't need
//! `DATABASE_URL` at build time. Trade-off: no compile-time SQL checks.
//! Acceptable for v0.1; revisit if we start landing nontrivial SQL bugs.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chimpflix_common::now_ms;
use chimpflix_metadata::{TmdbEpisode, TmdbMovie, TmdbShow, tmdb_image_url};
use chimpflix_transcoder::ProbeStream;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::models::{
    Episode, EpisodeDetail, EpisodeListed, Invite, Item, ItemDetail, ItemFilter, ItemKind,
    ItemPage, Library, LibraryUpdate, ListedItem, MediaFileLocator, MediaFileSummary,
    MediaStreamSummary, NewLibrary, OnDeckEntry, OnDeckResponse, PlayStateBatch, PlayStateForItem,
    ScanJob, Season, SeasonDetail, SeasonSummary, SessionRow, User, UserRole, UserWithSecret,
    make_sort_title,
};

// ---------------------------------------------------------------------------
// Libraries
// ---------------------------------------------------------------------------

pub async fn create_library(pool: &SqlitePool, input: NewLibrary) -> Result<Library> {
    if input.name.trim().is_empty() {
        anyhow::bail!("library name is required");
    }
    if input.paths.is_empty() {
        anyhow::bail!("library must have at least one path");
    }

    let now = now_ms();
    let scan_interval = input.scan_interval_s.unwrap_or(3600);

    let mut tx = pool.begin().await?;

    let lib_id: i64 = sqlx::query(
        "INSERT INTO libraries (name, kind, scan_interval_s, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&input.name)
    .bind(input.kind.as_str())
    .bind(scan_interval)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?
    .try_get("id")?;

    for path in &input.paths {
        sqlx::query("INSERT INTO library_paths (library_id, path) VALUES (?, ?)")
            .bind(lib_id)
            .bind(path)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    get_library(pool, lib_id)
        .await?
        .context("library disappeared after insert")
}

pub async fn list_libraries(pool: &SqlitePool) -> Result<Vec<Library>> {
    let rows = sqlx::query("SELECT * FROM libraries ORDER BY created_at ASC")
        .fetch_all(pool)
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: i64 = row.try_get("id")?;
        let paths = library_paths(pool, id).await?;
        out.push(Library::from_row(row, paths)?);
    }
    Ok(out)
}

pub async fn get_library(pool: &SqlitePool, id: i64) -> Result<Option<Library>> {
    let Some(row) = sqlx::query("SELECT * FROM libraries WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let paths = library_paths(pool, id).await?;
    Ok(Some(Library::from_row(&row, paths)?))
}

pub async fn update_library(
    pool: &SqlitePool,
    id: i64,
    update: LibraryUpdate,
) -> Result<Option<Library>> {
    let mut tx = pool.begin().await?;
    let now = now_ms();

    if let Some(name) = &update.name {
        sqlx::query("UPDATE libraries SET name = ?, updated_at = ? WHERE id = ?")
            .bind(name)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(interval) = update.scan_interval_s {
        sqlx::query("UPDATE libraries SET scan_interval_s = ?, updated_at = ? WHERE id = ?")
            .bind(interval)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(paths) = &update.paths {
        if paths.is_empty() {
            anyhow::bail!("library must have at least one path");
        }
        sqlx::query("DELETE FROM library_paths WHERE library_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        for p in paths {
            sqlx::query("INSERT INTO library_paths (library_id, path) VALUES (?, ?)")
                .bind(id)
                .bind(p)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("UPDATE libraries SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    get_library(pool, id).await
}

pub async fn delete_library(pool: &SqlitePool, id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM libraries WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn library_paths(pool: &SqlitePool, library_id: i64) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT path FROM library_paths WHERE library_id = ? ORDER BY path ASC")
        .bind(library_id)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|r| r.try_get::<String, _>("path").map_err(Into::into))
        .collect()
}

pub async fn touch_library_last_scan(pool: &SqlitePool, library_id: i64) -> Result<()> {
    let now = now_ms();
    sqlx::query("UPDATE libraries SET last_scan_at = ? WHERE id = ?")
        .bind(now)
        .bind(library_id)
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Scan jobs
// ---------------------------------------------------------------------------

pub async fn create_scan_job(pool: &SqlitePool, library_id: i64) -> Result<ScanJob> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO scan_jobs (library_id, status, created_at)
         VALUES (?, 'queued', ?)
         RETURNING *",
    )
    .bind(library_id)
    .bind(now)
    .fetch_one(pool)
    .await?;
    ScanJob::from_row(&row)
}

pub async fn get_scan_job(pool: &SqlitePool, id: i64) -> Result<Option<ScanJob>> {
    let Some(row) = sqlx::query("SELECT * FROM scan_jobs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(ScanJob::from_row(&row)?))
}

pub async fn list_scan_jobs(
    pool: &SqlitePool,
    library_id: i64,
    limit: u32,
) -> Result<Vec<ScanJob>> {
    let rows = sqlx::query(
        "SELECT * FROM scan_jobs WHERE library_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(library_id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;
    rows.iter().map(ScanJob::from_row).collect()
}

pub async fn mark_scan_running(pool: &SqlitePool, id: i64) -> Result<()> {
    let now = now_ms();
    sqlx::query("UPDATE scan_jobs SET status = 'running', started_at = ? WHERE id = ?")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_scan_counters(
    pool: &SqlitePool,
    id: i64,
    files_seen: i64,
    files_added: i64,
    files_updated: i64,
    files_removed: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE scan_jobs
         SET files_seen = ?, files_added = ?, files_updated = ?, files_removed = ?
         WHERE id = ?",
    )
    .bind(files_seen)
    .bind(files_added)
    .bind(files_updated)
    .bind(files_removed)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_scan_completed(
    pool: &SqlitePool,
    id: i64,
    files_seen: i64,
    files_added: i64,
    files_updated: i64,
    files_removed: i64,
) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE scan_jobs
         SET status = 'completed', finished_at = ?,
             files_seen = ?, files_added = ?, files_updated = ?, files_removed = ?
         WHERE id = ?",
    )
    .bind(now)
    .bind(files_seen)
    .bind(files_added)
    .bind(files_updated)
    .bind(files_removed)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_scan_failed(pool: &SqlitePool, id: i64, error: &str) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE scan_jobs SET status = 'failed', finished_at = ?, error_message = ? WHERE id = ?",
    )
    .bind(now)
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark any scan job left in `running` state from a previous process as
/// failed (`server restart while running`). Call once at startup.
pub async fn mark_interrupted_scans(pool: &SqlitePool) -> Result<u64> {
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE scan_jobs
         SET status = 'failed', finished_at = ?, error_message = 'interrupted by server restart'
         WHERE status IN ('running', 'queued')",
    )
    .bind(now)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

// ---------------------------------------------------------------------------
// Items (read)
// ---------------------------------------------------------------------------

/// SELECT clause that produces every Item column plus poster/backdrop and
/// the current user's play_state (NULL columns when no play_state row
/// exists). Use it with a trailing `FROM items i ... WHERE ...`.
const ITEM_SELECT: &str = "
    SELECT i.*,
        (SELECT source_url FROM images
            WHERE item_id = i.id AND kind = 'poster'
            ORDER BY is_primary DESC, id ASC LIMIT 1) AS poster_path,
        (SELECT source_url FROM images
            WHERE item_id = i.id AND kind = 'backdrop'
            ORDER BY is_primary DESC, id ASC LIMIT 1) AS backdrop_path,
        ps.position_ms     AS ps_position_ms,
        ps.duration_ms     AS ps_duration_ms,
        ps.watched         AS ps_watched,
        ps.view_count      AS ps_view_count,
        ps.last_played_at  AS ps_last_played_at
    FROM items i
    LEFT JOIN play_state ps
        ON ps.item_id = i.id AND ps.user_id = ?
";

pub async fn list_items(pool: &SqlitePool, filter: ItemFilter, user_id: i64) -> Result<ItemPage> {
    let page = filter.page.unwrap_or(1).max(1);
    let page_size = filter.page_size.unwrap_or(50).clamp(1, 200);
    let offset = ((page - 1) * page_size) as i64;

    let mut where_clauses: Vec<&str> = Vec::new();
    if filter.library_id.is_some() {
        where_clauses.push("i.library_id = ?");
    }
    if filter.kind.is_some() {
        where_clauses.push("i.kind = ?");
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) AS n FROM items i {where_sql}");
    let list_sql = format!("{ITEM_SELECT} {where_sql} ORDER BY i.added_at DESC LIMIT ? OFFSET ?");

    let mut count_q = sqlx::query(&count_sql);
    if let Some(lib) = filter.library_id {
        count_q = count_q.bind(lib);
    }
    if let Some(k) = filter.kind {
        count_q = count_q.bind(k.as_str());
    }
    let total: i64 = count_q.fetch_one(pool).await?.try_get("n")?;

    let mut list_q = sqlx::query(&list_sql).bind(user_id);
    if let Some(lib) = filter.library_id {
        list_q = list_q.bind(lib);
    }
    if let Some(k) = filter.kind {
        list_q = list_q.bind(k.as_str());
    }
    list_q = list_q.bind(page_size as i64).bind(offset);
    let rows = list_q.fetch_all(pool).await?;

    let items = rows
        .iter()
        .map(|row| -> Result<ListedItem> {
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(ItemPage {
        items,
        total,
        page,
        page_size,
    })
}

pub async fn get_item(pool: &SqlitePool, id: i64, user_id: i64) -> Result<Option<Item>> {
    let sql = format!("{ITEM_SELECT} WHERE i.id = ?");
    let Some(row) = sqlx::query(&sql)
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(Item::from_row(&row)?))
}

/// Full item detail: base item + genres + play state, plus either
/// `files` (for movies) or `seasons` (for shows). At most one of the
/// child arrays is populated.
pub async fn get_item_detail(
    pool: &SqlitePool,
    id: i64,
    user_id: i64,
) -> Result<Option<ItemDetail>> {
    let sql = format!("{ITEM_SELECT} WHERE i.id = ?");
    let Some(row) = sqlx::query(&sql)
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let item = Item::from_row(&row)?;
    let play_state = PlayStateForItem::from_columns(&row)?;
    let genres = list_item_genres(pool, id).await?;

    let (files, seasons) = match item.kind {
        ItemKind::Movie => (list_files_for_item(pool, id).await?, Vec::new()),
        ItemKind::Show => (Vec::new(), list_seasons_for_show(pool, id).await?),
    };

    Ok(Some(ItemDetail {
        item,
        genres,
        play_state,
        files,
        seasons,
    }))
}

async fn list_item_genres(pool: &SqlitePool, item_id: i64) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "SELECT g.name FROM genres g
         JOIN item_genres ig ON ig.genre_id = g.id
         WHERE ig.item_id = ?
         ORDER BY g.name ASC",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| r.try_get::<String, _>("name").map_err(Into::into))
        .collect()
}

async fn list_seasons_for_show(pool: &SqlitePool, show_id: i64) -> Result<Vec<SeasonSummary>> {
    let rows = sqlx::query(
        "SELECT s.id, s.season_number, s.title,
                (SELECT COUNT(*) FROM episodes WHERE season_id = s.id) AS episode_count
         FROM seasons s
         WHERE s.show_id = ?
         ORDER BY s.season_number ASC",
    )
    .bind(show_id)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(SeasonSummary {
            id: r.try_get("id")?,
            season_number: r.try_get("season_number")?,
            title: r.try_get::<Option<String>, _>("title").ok().flatten(),
            episode_count: r.try_get("episode_count")?,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Seasons & episodes (read)
// ---------------------------------------------------------------------------

pub async fn get_season_detail(
    pool: &SqlitePool,
    id: i64,
    user_id: i64,
) -> Result<Option<SeasonDetail>> {
    let Some(row) = sqlx::query("SELECT * FROM seasons WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let season = Season {
        id: row.try_get("id")?,
        show_id: row.try_get("show_id")?,
        season_number: row.try_get("season_number")?,
        title: row.try_get::<Option<String>, _>("title").ok().flatten(),
        summary: row.try_get::<Option<String>, _>("summary").ok().flatten(),
    };

    let ep_rows = sqlx::query(
        "SELECT e.*,
                (SELECT source_url FROM images
                    WHERE episode_id = e.id AND kind = 'thumb'
                    ORDER BY is_primary DESC, id ASC LIMIT 1) AS thumb_path,
                ps.position_ms    AS ps_position_ms,
                ps.duration_ms    AS ps_duration_ms,
                ps.watched        AS ps_watched,
                ps.view_count     AS ps_view_count,
                ps.last_played_at AS ps_last_played_at
         FROM episodes e
         LEFT JOIN play_state ps
             ON ps.episode_id = e.id AND ps.user_id = ?
         WHERE e.season_id = ?
         ORDER BY e.episode_number ASC",
    )
    .bind(user_id)
    .bind(id)
    .fetch_all(pool)
    .await?;

    let mut episodes = Vec::with_capacity(ep_rows.len());
    for r in ep_rows {
        let episode = episode_from_row(&r, season.show_id, season.season_number)?;
        let play_state = PlayStateForItem::from_columns(&r)?;
        episodes.push(EpisodeListed {
            episode,
            play_state,
        });
    }

    Ok(Some(SeasonDetail { season, episodes }))
}

pub async fn get_episode_detail(
    pool: &SqlitePool,
    id: i64,
    user_id: i64,
) -> Result<Option<EpisodeDetail>> {
    let Some(r) = sqlx::query(
        "SELECT e.*,
                s.show_id, s.season_number,
                (SELECT source_url FROM images
                    WHERE episode_id = e.id AND kind = 'thumb'
                    ORDER BY is_primary DESC, id ASC LIMIT 1) AS thumb_path,
                ps.position_ms    AS ps_position_ms,
                ps.duration_ms    AS ps_duration_ms,
                ps.watched        AS ps_watched,
                ps.view_count     AS ps_view_count,
                ps.last_played_at AS ps_last_played_at
         FROM episodes e
         JOIN seasons s ON s.id = e.season_id
         LEFT JOIN play_state ps
             ON ps.episode_id = e.id AND ps.user_id = ?
         WHERE e.id = ?",
    )
    .bind(user_id)
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let show_id: i64 = r.try_get("show_id")?;
    let season_number: i32 = r.try_get("season_number")?;
    let episode = episode_from_row(&r, show_id, season_number)?;
    let play_state = PlayStateForItem::from_columns(&r)?;
    let files = list_files_for_episode(pool, id).await?;

    Ok(Some(EpisodeDetail {
        episode,
        play_state,
        files,
    }))
}

fn episode_from_row(row: &SqliteRow, show_id: i64, season_number: i32) -> Result<Episode> {
    Ok(Episode {
        id: row.try_get("id")?,
        season_id: row.try_get("season_id")?,
        show_id,
        season_number,
        episode_number: row.try_get("episode_number")?,
        title: row.try_get("title")?,
        summary: row.try_get::<Option<String>, _>("summary").ok().flatten(),
        air_date: row.try_get::<Option<i64>, _>("air_date").ok().flatten(),
        duration_ms: row.try_get::<Option<i64>, _>("duration_ms").ok().flatten(),
        thumb_path: row
            .try_get::<Option<String>, _>("thumb_path")
            .ok()
            .flatten()
            .filter(|s| !s.is_empty()),
        added_at: row.try_get("added_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

// ---------------------------------------------------------------------------
// Media files & streams (for detail responses and the stream endpoint)
// ---------------------------------------------------------------------------

const MEDIA_FILE_SUMMARY_COLS: &str = "
    id, container, duration_ms, bit_rate, width, height, hdr_format, size_bytes
";

pub async fn list_files_for_item(pool: &SqlitePool, item_id: i64) -> Result<Vec<MediaFileSummary>> {
    let sql = format!(
        "SELECT {MEDIA_FILE_SUMMARY_COLS} FROM media_files
         WHERE item_id = ? ORDER BY id ASC"
    );
    let rows = sqlx::query(&sql).bind(item_id).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(media_file_summary(pool, &r).await?);
    }
    Ok(out)
}

pub async fn list_files_for_episode(
    pool: &SqlitePool,
    episode_id: i64,
) -> Result<Vec<MediaFileSummary>> {
    let sql = format!(
        "SELECT {MEDIA_FILE_SUMMARY_COLS} FROM media_files
         WHERE episode_id = ? ORDER BY id ASC"
    );
    let rows = sqlx::query(&sql).bind(episode_id).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(media_file_summary(pool, &r).await?);
    }
    Ok(out)
}

async fn media_file_summary(pool: &SqlitePool, row: &SqliteRow) -> Result<MediaFileSummary> {
    let id: i64 = row.try_get("id")?;
    let streams = list_streams_for_file(pool, id).await?;
    Ok(MediaFileSummary {
        id,
        container: row.try_get::<Option<String>, _>("container").ok().flatten(),
        duration_ms: row.try_get::<Option<i64>, _>("duration_ms").ok().flatten(),
        bit_rate: row.try_get::<Option<i64>, _>("bit_rate").ok().flatten(),
        width: row.try_get::<Option<i32>, _>("width").ok().flatten(),
        height: row.try_get::<Option<i32>, _>("height").ok().flatten(),
        hdr_format: row
            .try_get::<Option<String>, _>("hdr_format")
            .ok()
            .flatten(),
        size_bytes: row.try_get("size_bytes")?,
        streams,
    })
}

async fn list_streams_for_file(
    pool: &SqlitePool,
    media_file_id: i64,
) -> Result<Vec<MediaStreamSummary>> {
    let rows = sqlx::query(
        "SELECT stream_index, kind, codec, language, channels, is_default, is_forced
         FROM media_streams
         WHERE media_file_id = ?
         ORDER BY stream_index ASC",
    )
    .bind(media_file_id)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(MediaStreamSummary {
            index: r.try_get("stream_index")?,
            kind: r.try_get("kind")?,
            codec: r.try_get::<Option<String>, _>("codec").ok().flatten(),
            language: r.try_get::<Option<String>, _>("language").ok().flatten(),
            channels: r.try_get::<Option<i32>, _>("channels").ok().flatten(),
            is_default: r.try_get::<i64, _>("is_default")? != 0,
            is_forced: r.try_get::<i64, _>("is_forced")? != 0,
        });
    }
    Ok(out)
}

pub async fn get_media_file_locator(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<MediaFileLocator>> {
    let Some(r) =
        sqlx::query("SELECT id, path, size_bytes, container FROM media_files WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
    else {
        return Ok(None);
    };
    Ok(Some(MediaFileLocator {
        id: r.try_get("id")?,
        path: r.try_get("path")?,
        size_bytes: r.try_get("size_bytes")?,
        container: r.try_get::<Option<String>, _>("container").ok().flatten(),
    }))
}

// ---------------------------------------------------------------------------
// Play state (write)
// ---------------------------------------------------------------------------

pub async fn ensure_default_user(pool: &SqlitePool) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT OR IGNORE INTO users (id, username, password_hash, role, created_at, updated_at)
         VALUES (1, '_default', '!disabled', 'owner', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Users (auth)
// ---------------------------------------------------------------------------

/// In setup mode iff user 1 still has the placeholder username `_default`.
pub async fn is_in_setup_mode(pool: &SqlitePool) -> Result<bool> {
    let row = sqlx::query("SELECT 1 AS x FROM users WHERE id = 1 AND username = '_default'")
        .fetch_optional(pool)
        .await?;
    Ok(row.is_some())
}

/// Transform the placeholder user 1 into the real owner. Reuses id=1 so any
/// pre-setup `play_state` rows (none in practice, but defensive) remain
/// valid.
pub async fn complete_setup(
    pool: &SqlitePool,
    username: &str,
    password_hash: &str,
    display_name: Option<&str>,
) -> Result<User> {
    let now = now_ms();
    let row = sqlx::query(
        "UPDATE users
           SET username      = ?,
               password_hash = ?,
               role          = 'owner',
               display_name  = ?,
               updated_at    = ?
         WHERE id = 1 AND username = '_default'
         RETURNING *",
    )
    .bind(username)
    .bind(password_hash)
    .bind(display_name)
    .bind(now)
    .fetch_optional(pool)
    .await?
    .context("setup already completed or placeholder user is missing")?;
    User::from_row(&row)
}

pub async fn find_user_by_id(pool: &SqlitePool, id: i64) -> Result<Option<User>> {
    let Some(row) = sqlx::query("SELECT * FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(User::from_row(&row)?))
}

pub async fn find_user_with_secret_by_username(
    pool: &SqlitePool,
    username: &str,
) -> Result<Option<UserWithSecret>> {
    let Some(row) = sqlx::query("SELECT * FROM users WHERE username = ? COLLATE NOCASE")
        .bind(username)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(UserWithSecret {
        user: User::from_row(&row)?,
        password_hash: row.try_get("password_hash")?,
    }))
}

pub async fn create_user(
    pool: &SqlitePool,
    username: &str,
    password_hash: &str,
    role: UserRole,
    display_name: Option<&str>,
) -> Result<User> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO users (username, password_hash, role, display_name, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING *",
    )
    .bind(username)
    .bind(password_hash)
    .bind(role.as_str())
    .bind(display_name)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    User::from_row(&row)
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

pub async fn create_session(
    pool: &SqlitePool,
    user_id: i64,
    nonce: &[u8; 32],
    expires_at: i64,
    user_agent: Option<&str>,
    ip: Option<&str>,
) -> Result<i64> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO sessions
            (user_id, nonce, user_agent, ip, last_seen_at, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(user_id)
    .bind(&nonce[..])
    .bind(user_agent)
    .bind(ip)
    .bind(now)
    .bind(expires_at)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

pub async fn find_session(pool: &SqlitePool, id: i64) -> Result<Option<SessionRow>> {
    let Some(row) = sqlx::query("SELECT * FROM sessions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    let nonce_blob: Vec<u8> = row.try_get("nonce")?;
    if nonce_blob.len() != 32 {
        anyhow::bail!("corrupt session nonce length: {}", nonce_blob.len());
    }
    let mut nonce = [0u8; 32];
    nonce.copy_from_slice(&nonce_blob);
    Ok(Some(SessionRow {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        nonce,
        expires_at: row.try_get("expires_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
        created_at: row.try_get("created_at")?,
    }))
}

pub async fn touch_session(pool: &SqlitePool, id: i64) -> Result<()> {
    let now = now_ms();
    sqlx::query("UPDATE sessions SET last_seen_at = ? WHERE id = ?")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_session(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn cleanup_expired_sessions(pool: &SqlitePool) -> Result<u64> {
    let now = now_ms();
    let res = sqlx::query("DELETE FROM sessions WHERE expires_at < ?")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// ---------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------

pub async fn create_invite(
    pool: &SqlitePool,
    code: &str,
    created_by: i64,
    expires_at: Option<i64>,
) -> Result<Invite> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO invites (code, created_by, expires_at, created_at)
         VALUES (?, ?, ?, ?)
         RETURNING *",
    )
    .bind(code)
    .bind(created_by)
    .bind(expires_at)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Invite::from_row(&row)
}

pub async fn list_invites(pool: &SqlitePool) -> Result<Vec<Invite>> {
    let rows = sqlx::query("SELECT * FROM invites ORDER BY created_at DESC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(Invite::from_row).collect()
}

pub async fn find_invite_by_code(pool: &SqlitePool, code: &str) -> Result<Option<Invite>> {
    let Some(row) = sqlx::query("SELECT * FROM invites WHERE code = ?")
        .bind(code)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(Invite::from_row(&row)?))
}

pub async fn consume_invite(pool: &SqlitePool, code: &str, user_id: i64) -> Result<()> {
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE invites
            SET consumed_by = ?, consumed_at = ?
          WHERE code = ?
            AND consumed_by IS NULL
            AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(user_id)
    .bind(now)
    .bind(code)
    .bind(now)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        anyhow::bail!("invite is invalid, expired, or already consumed");
    }
    Ok(())
}

pub async fn revoke_invite(pool: &SqlitePool, code: &str) -> Result<bool> {
    let res = sqlx::query("DELETE FROM invites WHERE code = ? AND consumed_by IS NULL")
        .bind(code)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn apply_play_state_batch(
    pool: &SqlitePool,
    user_id: i64,
    batch: PlayStateBatch,
) -> Result<()> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    for update in batch.updates {
        match (update.item_id, update.episode_id) {
            (Some(item_id), None) => {
                upsert_play_state_movie_tx(
                    &mut tx,
                    user_id,
                    item_id,
                    update.position_ms,
                    update.duration_ms,
                    update.watched.unwrap_or(false),
                    now,
                )
                .await?;
            }
            (None, Some(episode_id)) => {
                upsert_play_state_episode_tx(
                    &mut tx,
                    user_id,
                    episode_id,
                    update.position_ms,
                    update.duration_ms,
                    update.watched.unwrap_or(false),
                    now,
                )
                .await?;
            }
            _ => anyhow::bail!("play state update must have exactly one of item_id or episode_id"),
        }
    }
    tx.commit().await?;
    Ok(())
}

async fn upsert_play_state_movie_tx<'a>(
    tx: &mut sqlx::Transaction<'a, sqlx::Sqlite>,
    user_id: i64,
    item_id: i64,
    position_ms: i64,
    duration_ms: Option<i64>,
    watched: bool,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO play_state
            (user_id, item_id, position_ms, duration_ms, watched, view_count, last_played_at)
         VALUES (?, ?, ?, ?, ?, 0, ?)
         ON CONFLICT (user_id, item_id) WHERE item_id IS NOT NULL DO UPDATE SET
            position_ms     = excluded.position_ms,
            duration_ms     = COALESCE(excluded.duration_ms, play_state.duration_ms),
            watched         = excluded.watched,
            last_played_at  = excluded.last_played_at",
    )
    .bind(user_id)
    .bind(item_id)
    .bind(position_ms)
    .bind(duration_ms)
    .bind(watched as i64)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn upsert_play_state_episode_tx<'a>(
    tx: &mut sqlx::Transaction<'a, sqlx::Sqlite>,
    user_id: i64,
    episode_id: i64,
    position_ms: i64,
    duration_ms: Option<i64>,
    watched: bool,
    now: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO play_state
            (user_id, episode_id, position_ms, duration_ms, watched, view_count, last_played_at)
         VALUES (?, ?, ?, ?, ?, 0, ?)
         ON CONFLICT (user_id, episode_id) WHERE episode_id IS NOT NULL DO UPDATE SET
            position_ms     = excluded.position_ms,
            duration_ms     = COALESCE(excluded.duration_ms, play_state.duration_ms),
            watched         = excluded.watched,
            last_played_at  = excluded.last_played_at",
    )
    .bind(user_id)
    .bind(episode_id)
    .bind(position_ms)
    .bind(duration_ms)
    .bind(watched as i64)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn scrobble(
    pool: &SqlitePool,
    user_id: i64,
    item_id: Option<i64>,
    episode_id: Option<i64>,
) -> Result<()> {
    let now = now_ms();
    match (item_id, episode_id) {
        (Some(iid), None) => {
            sqlx::query(
                "INSERT INTO play_state
                    (user_id, item_id, position_ms, watched, view_count, last_played_at)
                 VALUES (?, ?, 0, 1, 1, ?)
                 ON CONFLICT (user_id, item_id) WHERE item_id IS NOT NULL DO UPDATE SET
                    watched         = 1,
                    view_count      = play_state.view_count + 1,
                    last_played_at  = excluded.last_played_at",
            )
            .bind(user_id)
            .bind(iid)
            .bind(now)
            .execute(pool)
            .await?;
        }
        (None, Some(eid)) => {
            sqlx::query(
                "INSERT INTO play_state
                    (user_id, episode_id, position_ms, watched, view_count, last_played_at)
                 VALUES (?, ?, 0, 1, 1, ?)
                 ON CONFLICT (user_id, episode_id) WHERE episode_id IS NOT NULL DO UPDATE SET
                    watched         = 1,
                    view_count      = play_state.view_count + 1,
                    last_played_at  = excluded.last_played_at",
            )
            .bind(user_id)
            .bind(eid)
            .bind(now)
            .execute(pool)
            .await?;
        }
        _ => anyhow::bail!("scrobble requires exactly one of item_id or episode_id"),
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// On-deck
// ---------------------------------------------------------------------------

const ON_DECK_LIMIT: i64 = 20;

pub async fn on_deck(pool: &SqlitePool, user_id: i64) -> Result<OnDeckResponse> {
    // Anything actively watching (5% — 95%) AND not flagged watched yet.
    // Ordered by most recently played.
    let rows = sqlx::query(
        "SELECT * FROM play_state
         WHERE user_id = ?
           AND watched = 0
           AND position_ms > 0
           AND (duration_ms IS NULL OR position_ms < duration_ms * 95 / 100)
         ORDER BY last_played_at DESC
         LIMIT ?",
    )
    .bind(user_id)
    .bind(ON_DECK_LIMIT)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::new();
    for r in rows {
        let item_id: Option<i64> = r.try_get("item_id")?;
        let episode_id: Option<i64> = r.try_get("episode_id")?;

        let play_state = PlayStateForItem {
            position_ms: r.try_get("position_ms")?,
            duration_ms: r.try_get::<Option<i64>, _>("duration_ms").ok().flatten(),
            watched: r.try_get::<i64, _>("watched")? != 0,
            view_count: r.try_get("view_count")?,
            last_played_at: r.try_get("last_played_at")?,
        };

        if let Some(iid) = item_id {
            if let Some(item) = get_item(pool, iid, user_id).await? {
                out.push(OnDeckEntry::Movie { item, play_state });
            }
        } else if let Some(eid) = episode_id {
            if let Some(detail) = get_episode_detail(pool, eid, user_id).await? {
                let show_id = detail.episode.show_id;
                if let Some(show) = get_item(pool, show_id, user_id).await? {
                    out.push(OnDeckEntry::Episode {
                        episode: detail.episode,
                        show,
                        play_state,
                    });
                }
            }
        }
    }

    Ok(OnDeckResponse { items: out })
}

// ---------------------------------------------------------------------------
// Scanner upserts
// ---------------------------------------------------------------------------

/// Map of path → last-known `mtime_ms` for every media file currently
/// associated with this library (via either `item_id` or
/// `episode_id → season → show_id → items.library_id`). Used by the
/// scanner to skip unchanged files.
pub async fn existing_media_files(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<HashMap<String, i64>> {
    let rows = sqlx::query(
        "SELECT mf.path, mf.mtime_ms
         FROM media_files mf
         LEFT JOIN items i_movie ON mf.item_id = i_movie.id
         LEFT JOIN episodes ep ON mf.episode_id = ep.id
         LEFT JOIN seasons s ON ep.season_id = s.id
         LEFT JOIN items i_show ON s.show_id = i_show.id
         WHERE i_movie.library_id = ?1 OR i_show.library_id = ?1",
    )
    .bind(library_id)
    .fetch_all(pool)
    .await?;

    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        let path: String = row.try_get("path")?;
        let mtime_ms: i64 = row.try_get("mtime_ms")?;
        out.insert(path, mtime_ms);
    }
    Ok(out)
}

pub async fn upsert_item(
    pool: &SqlitePool,
    library_id: i64,
    kind: ItemKind,
    title: &str,
    sort_title: &str,
    year: Option<i32>,
) -> Result<i64> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO items (library_id, kind, title, sort_title, year, added_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(library_id, kind, sort_title) DO UPDATE SET
             title = excluded.title,
             year = COALESCE(items.year, excluded.year),
             updated_at = excluded.updated_at
         RETURNING id",
    )
    .bind(library_id)
    .bind(kind.as_str())
    .bind(title)
    .bind(sort_title)
    .bind(year)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

pub async fn upsert_season(pool: &SqlitePool, show_id: i64, season_number: i32) -> Result<i64> {
    let row = sqlx::query(
        "INSERT INTO seasons (show_id, season_number) VALUES (?, ?)
         ON CONFLICT(show_id, season_number) DO UPDATE SET season_number = excluded.season_number
         RETURNING id",
    )
    .bind(show_id)
    .bind(season_number)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

pub async fn upsert_episode(
    pool: &SqlitePool,
    season_id: i64,
    episode_number: i32,
    title: &str,
) -> Result<i64> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO episodes (season_id, episode_number, title, added_at, updated_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(season_id, episode_number) DO UPDATE SET
             title = CASE
                WHEN length(episodes.title) = 0 OR episodes.title LIKE 'Episode %'
                THEN excluded.title
                ELSE episodes.title
             END,
             updated_at = excluded.updated_at
         RETURNING id",
    )
    .bind(season_id)
    .bind(episode_number)
    .bind(title)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    Added,
    Updated,
    Unchanged,
}

#[derive(Debug, Clone)]
pub struct MediaFileInput<'a> {
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub path: &'a str,
    pub size_bytes: i64,
    pub mtime_ms: i64,
    pub container: Option<&'a str>,
    pub duration_ms: Option<i64>,
    pub bit_rate: Option<i64>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub hdr_format: Option<&'a str>,
}

pub async fn upsert_media_file(
    pool: &SqlitePool,
    input: MediaFileInput<'_>,
    existing_mtime_ms: Option<i64>,
) -> Result<(i64, FileOutcome)> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO media_files
            (item_id, episode_id, path, size_bytes, mtime_ms, container,
             duration_ms, bit_rate, width, height, hdr_format, scanned_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(path) DO UPDATE SET
            item_id    = excluded.item_id,
            episode_id = excluded.episode_id,
            size_bytes = excluded.size_bytes,
            mtime_ms   = excluded.mtime_ms,
            container  = excluded.container,
            duration_ms = excluded.duration_ms,
            bit_rate   = excluded.bit_rate,
            width      = excluded.width,
            height     = excluded.height,
            hdr_format = excluded.hdr_format,
            scanned_at = excluded.scanned_at
         RETURNING id",
    )
    .bind(input.item_id)
    .bind(input.episode_id)
    .bind(input.path)
    .bind(input.size_bytes)
    .bind(input.mtime_ms)
    .bind(input.container)
    .bind(input.duration_ms)
    .bind(input.bit_rate)
    .bind(input.width)
    .bind(input.height)
    .bind(input.hdr_format)
    .bind(now)
    .fetch_one(pool)
    .await?;
    let id: i64 = row.try_get("id")?;

    let outcome = match existing_mtime_ms {
        None => FileOutcome::Added,
        Some(mt) if mt != input.mtime_ms => FileOutcome::Updated,
        Some(_) => FileOutcome::Unchanged,
    };
    Ok((id, outcome))
}

pub async fn replace_media_streams(
    pool: &SqlitePool,
    media_file_id: i64,
    streams: &[ProbeStream],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM media_streams WHERE media_file_id = ?")
        .bind(media_file_id)
        .execute(&mut *tx)
        .await?;
    for s in streams {
        sqlx::query(
            "INSERT INTO media_streams
                (media_file_id, stream_index, kind, codec, profile, language, title,
                 pix_fmt, frame_rate, channels, channel_layout, sample_rate,
                 is_default, is_forced)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(media_file_id)
        .bind(s.index)
        .bind(stream_kind_str(s.kind))
        .bind(s.codec.as_deref())
        .bind(s.profile.as_deref())
        .bind(s.language.as_deref())
        .bind(s.title.as_deref())
        .bind(s.pix_fmt.as_deref())
        .bind(s.frame_rate)
        .bind(s.channels)
        .bind(s.channel_layout.as_deref())
        .bind(s.sample_rate)
        .bind(s.is_default as i64)
        .bind(s.is_forced as i64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn stream_kind_str(k: chimpflix_transcoder::StreamKind) -> &'static str {
    use chimpflix_transcoder::StreamKind::*;
    match k {
        Video => "video",
        Audio => "audio",
        Subtitle => "subtitle",
        Other => "other",
    }
}

pub async fn set_item_duration_if_null(
    pool: &SqlitePool,
    item_id: i64,
    duration_ms: i64,
) -> Result<()> {
    sqlx::query("UPDATE items SET duration_ms = ? WHERE id = ? AND duration_ms IS NULL")
        .bind(duration_ms)
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn apply_movie_metadata(pool: &SqlitePool, item_id: i64, meta: &TmdbMovie) -> Result<()> {
    let now = now_ms();
    let sort_title = make_sort_title(&meta.title);
    sqlx::query(
        "UPDATE items SET
            title = ?,
            sort_title = ?,
            original_title = ?,
            summary = ?,
            tagline = ?,
            year = COALESCE(?, year),
            duration_ms = COALESCE(duration_ms, ?),
            rating_audience = ?,
            tmdb_id = ?,
            imdb_id = ?,
            refreshed_at = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&meta.title)
    .bind(sort_title)
    .bind(meta.original_title.as_deref())
    .bind(meta.summary.as_deref())
    .bind(meta.tagline.as_deref())
    .bind(meta.year)
    .bind(meta.runtime_min.map(|m| (m as i64) * 60_000))
    .bind(meta.rating_audience)
    .bind(meta.tmdb_id)
    .bind(meta.imdb_id.as_deref())
    .bind(now)
    .bind(now)
    .bind(item_id)
    .execute(pool)
    .await?;

    apply_genres(pool, item_id, &meta.genres).await?;
    if let Some(p) = &meta.poster_path {
        store_image(
            pool,
            Some(item_id),
            None,
            "poster",
            "tmdb",
            &tmdb_image_url(p, "w500"),
        )
        .await?;
    }
    if let Some(p) = &meta.backdrop_path {
        store_image(
            pool,
            Some(item_id),
            None,
            "backdrop",
            "tmdb",
            &tmdb_image_url(p, "w1280"),
        )
        .await?;
    }

    Ok(())
}

pub async fn apply_show_metadata(pool: &SqlitePool, item_id: i64, meta: &TmdbShow) -> Result<()> {
    let now = now_ms();
    let sort_title = make_sort_title(&meta.title);
    sqlx::query(
        "UPDATE items SET
            title = ?,
            sort_title = ?,
            original_title = ?,
            summary = ?,
            year = COALESCE(?, year),
            tmdb_id = ?,
            imdb_id = ?,
            refreshed_at = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&meta.title)
    .bind(sort_title)
    .bind(meta.original_title.as_deref())
    .bind(meta.summary.as_deref())
    .bind(meta.year)
    .bind(meta.tmdb_id)
    .bind(meta.imdb_id.as_deref())
    .bind(now)
    .bind(now)
    .bind(item_id)
    .execute(pool)
    .await?;

    apply_genres(pool, item_id, &meta.genres).await?;
    if let Some(p) = &meta.poster_path {
        store_image(
            pool,
            Some(item_id),
            None,
            "poster",
            "tmdb",
            &tmdb_image_url(p, "w500"),
        )
        .await?;
    }
    if let Some(p) = &meta.backdrop_path {
        store_image(
            pool,
            Some(item_id),
            None,
            "backdrop",
            "tmdb",
            &tmdb_image_url(p, "w1280"),
        )
        .await?;
    }

    Ok(())
}

pub async fn apply_episode_metadata(
    pool: &SqlitePool,
    episode_id: i64,
    meta: &TmdbEpisode,
) -> Result<()> {
    let now = now_ms();
    let air_date_ms = meta.air_date.as_deref().and_then(parse_air_date_to_ms);
    sqlx::query(
        "UPDATE episodes SET
            title = ?,
            summary = ?,
            duration_ms = COALESCE(duration_ms, ?),
            air_date = COALESCE(air_date, ?),
            tmdb_id = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&meta.title)
    .bind(meta.summary.as_deref())
    .bind(meta.runtime_min.map(|m| (m as i64) * 60_000))
    .bind(air_date_ms)
    .bind(meta.episode_number as i64)
    .bind(now)
    .bind(episode_id)
    .execute(pool)
    .await?;

    if let Some(p) = &meta.still_path {
        store_image(
            pool,
            None,
            Some(episode_id),
            "thumb",
            "tmdb",
            &tmdb_image_url(p, "w300"),
        )
        .await?;
    }
    Ok(())
}

async fn apply_genres(pool: &SqlitePool, item_id: i64, names: &[String]) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM item_genres WHERE item_id = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    for name in names {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            continue;
        }
        let row = sqlx::query(
            "INSERT INTO genres (name) VALUES (?)
             ON CONFLICT(name) DO UPDATE SET name = excluded.name
             RETURNING id",
        )
        .bind(trimmed)
        .fetch_one(&mut *tx)
        .await?;
        let gid: i64 = row.try_get("id")?;
        sqlx::query("INSERT OR IGNORE INTO item_genres (item_id, genre_id) VALUES (?, ?)")
            .bind(item_id)
            .bind(gid)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn store_image(
    pool: &SqlitePool,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    kind: &str,
    source: &str,
    url: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    if let Some(iid) = item_id {
        sqlx::query("DELETE FROM images WHERE item_id = ? AND kind = ?")
            .bind(iid)
            .bind(kind)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(eid) = episode_id {
        sqlx::query("DELETE FROM images WHERE episode_id = ? AND kind = ?")
            .bind(eid)
            .bind(kind)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "INSERT INTO images (item_id, episode_id, kind, source, source_url, is_primary)
         VALUES (?, ?, ?, ?, ?, 1)",
    )
    .bind(item_id)
    .bind(episode_id)
    .bind(kind)
    .bind(source)
    .bind(url)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

fn parse_air_date_to_ms(s: &str) -> Option<i64> {
    // ISO date "YYYY-MM-DD" → epoch ms (UTC midnight). Manual parse to
    // avoid pulling in chrono.
    let mut parts = s.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;

    // Days since 1970-01-01 using Gregorian arithmetic. Reference:
    // Hatcher / Howard-Hinnant date algorithm.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_date_epoch() {
        // 1970-01-01 = 0
        assert_eq!(parse_air_date_to_ms("1970-01-01"), Some(0));
        // 2024-01-19 = 1705622400000 (UTC midnight)
        assert_eq!(parse_air_date_to_ms("2024-01-19"), Some(1_705_622_400_000));
    }
}
