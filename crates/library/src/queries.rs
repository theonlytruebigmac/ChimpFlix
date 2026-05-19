//! SQL data access for libraries, scan jobs, items, and the upserts the
//! scanner needs.
//!
//! Plain `sqlx::query` / `query_as` — no `query!` macros, so we don't need
//! `DATABASE_URL` at build time. Trade-off: no compile-time SQL checks.
//! Acceptable for v0.1; revisit if we start landing nontrivial SQL bugs.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chimpflix_common::now_ms;
use chimpflix_metadata::{
    AniListShow, TmdbCastMember, TmdbCollection, TmdbCollectionStub, TmdbCredits, TmdbCrewMember,
    TmdbEpisode, TmdbMovie, TmdbShow, TmdbVideo, TvMazeShow, TvdbMovie, TvdbShow, tmdb_image_url,
};
use chimpflix_transcoder::ProbeStream;
use serde::Serialize;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::models::{
    AccessGroup, AccessGroupDetail, AccessGroupUpdate, AuditLogEntry, Credit, Episode,
    EpisodeDetail, EpisodeListed, ExternalSubtitle, Extra, Invite, Item, ItemDetail, ItemEdit,
    ItemFilter, ItemKind, ItemPage, Library, LibraryAgent, LibraryUpdate, ListedItem, Marker,
    MediaFileLocator, MediaFileSummary, MediaStreamSummary, NewAccessGroup, NewAuditEntry,
    NewExternalSubtitle, NewLibrary, NewOptimizedVersion, NewScheduledTask, NewTranscoderPreset,
    NewWebhook, Notification, OnDeckEntry, OnDeckResponse, OptimizedVersion, Person,
    PlayStateBatch, PlayStateForItem, Review, ReviewsSummary, ScanJob, ScheduledTask,
    ScheduledTaskUpdate, Season, SeasonDetail, SeasonSummary, SecretMetadata, ServerSettings,
    ServerSettingsUpdate, SessionRow, TaskRun, TranscoderPreset, TranscoderPresetUpdate, User,
    UserRole, UserWithSecret, Webhook, WebhookDelivery, WebhookUpdate, make_sort_title,
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

    // Auto-grant all current owners access to the new library. Regular
    // users get nothing by default — owners promote individuals via the
    // access mgmt endpoint.
    sqlx::query(
        "INSERT INTO library_access (user_id, library_id)
         SELECT id, ? FROM users WHERE role = 'owner'
         ON CONFLICT DO NOTHING",
    )
    .bind(lib_id)
    .execute(&mut *tx)
    .await?;

    // Seed the default metadata agent chain. Movies: TMDB + TVDB.
    // Shows: TMDB + TVMaze + TVDB. Anime: TMDB + TVDB (TVMaze isn't an
    // anime catalogue, and the AniList agent will join the chain once it
    // ships). Owners can reorder/disable later via
    // /admin/libraries/{id}/agents.
    sqlx::query(
        "INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
         VALUES (?, 'tmdb', 0, 1, '{}')",
    )
    .bind(lib_id)
    .execute(&mut *tx)
    .await?;
    if matches!(input.kind, crate::models::LibraryKind::Shows) {
        sqlx::query(
            "INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
             VALUES (?, 'tvmaze', 1, 1, '{}')",
        )
        .bind(lib_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
         VALUES (?, 'tvdb', 2, 1, '{}')",
    )
    .bind(lib_id)
    .execute(&mut *tx)
    .await?;
    if matches!(input.kind, crate::models::LibraryKind::Anime) {
        // AniList is the primary metadata source for anime; priority 0
        // puts it ahead of TMDB so the per-library agent picker reflects
        // the actual scan order.
        sqlx::query(
            "INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
             VALUES (?, 'anilist', 0, 1, '{}')",
        )
        .bind(lib_id)
        .execute(&mut *tx)
        .await?;
        // Drop the TMDB priority below AniList so the chain in the UI
        // matches what the scanner actually does (AniList primary,
        // TMDB+TVDB backfill).
        sqlx::query(
            "UPDATE library_agents SET priority = 1
             WHERE library_id = ? AND agent_name = 'tmdb'",
        )
        .bind(lib_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    get_library(pool, lib_id)
        .await?
        .context("library disappeared after insert")
}

pub async fn list_libraries(
    pool: &SqlitePool,
    accessible: Option<&[i64]>,
) -> Result<Vec<Library>> {
    let filter = library_filter_sql("id", accessible);
    let sql = format!(
        "SELECT * FROM libraries WHERE {filter} ORDER BY created_at ASC",
    );
    let rows = sqlx::query(&sql).fetch_all(pool).await?;

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
    if let Some(v) = &update.episode_sort_order {
        sqlx::query("UPDATE libraries SET episode_sort_order = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = &update.episode_naming {
        sqlx::query("UPDATE libraries SET episode_naming = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = &update.certification_country {
        sqlx::query("UPDATE libraries SET certification_country = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = &update.visibility {
        sqlx::query("UPDATE libraries SET visibility = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = update.allow_media_deletion {
        sqlx::query("UPDATE libraries SET allow_media_deletion = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(v))
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    get_library(pool, id).await
}

// ─── Library agents ────────────────────────────────────────────────────────

pub async fn list_library_agents(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<Vec<LibraryAgent>> {
    let rows = sqlx::query(
        "SELECT agent_name, priority, enabled, config_json
         FROM library_agents WHERE library_id = ?
         ORDER BY priority ASC, agent_name ASC",
    )
    .bind(library_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(LibraryAgent::from_row).collect()
}

/// Replace the entire agent chain for a library. Input order determines
/// priority (first = priority 0). Agents not present in the input are
/// removed; the table effectively becomes the new list.
pub async fn set_library_agents(
    pool: &SqlitePool,
    library_id: i64,
    agents: &[LibraryAgent],
) -> Result<Vec<LibraryAgent>> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM library_agents WHERE library_id = ?")
        .bind(library_id)
        .execute(&mut *tx)
        .await?;
    for (idx, a) in agents.iter().enumerate() {
        sqlx::query(
            "INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(library_id)
        .bind(&a.agent_name)
        .bind(idx as i64)
        .bind(i64::from(a.enabled))
        .bind(&a.config_json)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    list_library_agents(pool, library_id).await
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

/// Turn a free-text query into a safe FTS5 MATCH expression: each whitespace-
/// separated token becomes a double-quoted phrase with a trailing `*` for
/// prefix matching. Returns None when nothing useful remains after
/// stripping the FTS5 quote character.
fn fts_match_query(q: &str) -> Option<String> {
    let parts: Vec<String> = q
        .split_whitespace()
        .map(|tok| tok.replace('"', "")) // FTS5's only quote-escape is to remove the char
        .filter(|s| !s.is_empty())
        .map(|s| format!("\"{s}\"*"))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

pub async fn list_items(
    pool: &SqlitePool,
    filter: ItemFilter,
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<ItemPage> {
    let page = filter.page.unwrap_or(1).max(1);
    let page_size = filter.page_size.unwrap_or(50).clamp(1, 200);
    let offset = ((page - 1) * page_size) as i64;

    let mut where_clauses: Vec<String> = Vec::new();
    if filter.library_id.is_some() {
        where_clauses.push("i.library_id = ?".to_string());
    }
    if filter.kind.is_some() {
        where_clauses.push("i.kind = ?".to_string());
    }
    if filter.genre.is_some() {
        // EXISTS subquery is faster than a JOIN here because items can
        // have many genres and we don't want duplicate rows.
        where_clauses.push(
            "EXISTS (SELECT 1 FROM item_genres ig \
             JOIN genres g ON g.id = ig.genre_id \
             WHERE ig.item_id = i.id AND g.name = ?)"
                .to_string(),
        );
    }
    if filter.q.is_some() {
        // Full-text via items_fts. The MATCH query is built below from the
        // user input with each token quoted to defang FTS5 operators.
        where_clauses.push(
            "i.id IN (SELECT rowid FROM items_fts WHERE items_fts MATCH ?)".to_string(),
        );
    }
    where_clauses.push(library_filter_sql("i.library_id", accessible));
    let where_sql = format!("WHERE {}", where_clauses.join(" AND "));

    let order_by = filter.sort.unwrap_or_default().order_by();
    let count_sql = format!("SELECT COUNT(*) AS n FROM items i {where_sql}");
    let list_sql =
        format!("{ITEM_SELECT} {where_sql} ORDER BY {order_by} LIMIT ? OFFSET ?");

    let fts_query = filter.q.as_ref().and_then(|s| fts_match_query(s));

    let mut count_q = sqlx::query(&count_sql);
    if let Some(lib) = filter.library_id {
        count_q = count_q.bind(lib);
    }
    if let Some(k) = filter.kind {
        count_q = count_q.bind(k.as_str());
    }
    if let Some(ref g) = filter.genre {
        count_q = count_q.bind(g.as_str());
    }
    if let Some(ref fts) = fts_query {
        count_q = count_q.bind(fts);
    }
    let total: i64 = count_q.fetch_one(pool).await?.try_get("n")?;

    let mut list_q = sqlx::query(&list_sql).bind(user_id);
    if let Some(lib) = filter.library_id {
        list_q = list_q.bind(lib);
    }
    if let Some(k) = filter.kind {
        list_q = list_q.bind(k.as_str());
    }
    if let Some(ref g) = filter.genre {
        list_q = list_q.bind(g.as_str());
    }
    if let Some(ref fts) = fts_query {
        list_q = list_q.bind(fts);
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

/// Items the user has played, newest-first. Includes finished + in-progress.
/// Differs from on-deck which surfaces just resume-able items.
pub async fn list_watch_history(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<ListedItem>> {
    let filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "{ITEM_SELECT} \
         WHERE ps.user_id = ? AND ps.last_played_at IS NOT NULL AND {filter} \
         ORDER BY ps.last_played_at DESC \
         LIMIT ?",
    );
    let rows = sqlx::query(&sql)
        .bind(user_id)
        .bind(user_id)
        .bind(limit)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| -> Result<ListedItem> {
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Library access
// ---------------------------------------------------------------------------

/// Build a per-request filter describing which libraries this user can see.
/// Owners get `None` (no filter, full access). Non-owners get a `Some(Vec<i64>)`
/// of the library IDs accessible to them — the UNION of direct
/// `library_access` rows and group-derived rows (via `user_access_groups`
/// → `access_group_libraries`). An empty Vec means the user is locked
/// out of everything.
pub async fn user_library_filter(
    pool: &SqlitePool,
    user_id: i64,
    role: UserRole,
) -> Result<Option<Vec<i64>>> {
    if matches!(role, UserRole::Owner) {
        return Ok(None);
    }
    let rows = sqlx::query(
        "SELECT library_id FROM library_access WHERE user_id = ?
         UNION
         SELECT agl.library_id
           FROM access_group_libraries agl
           JOIN user_access_groups uag ON uag.group_id = agl.group_id
          WHERE uag.user_id = ?
         ORDER BY library_id",
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let ids: Result<Vec<i64>> = rows
        .iter()
        .map(|r| Ok(r.try_get::<i64, _>("library_id")?))
        .collect();
    Ok(Some(ids?))
}

/// Render an access filter as a SQL fragment for the given `column`.
/// - `None` → `"1=1"` (no restriction; owners take this path).
/// - `Some(&[])` → `"<col> = 0"` (matches nothing; locked-out user).
/// - `Some(ids)` → `"<col> IN (1,2,3)"`.
///
/// IDs are integers from trusted sources (DB rows or AuthUser), so direct
/// string interpolation here is safe.
fn library_filter_sql(column: &str, accessible: Option<&[i64]>) -> String {
    match accessible {
        None => "1=1".to_string(),
        Some(ids) if ids.is_empty() => format!("{column} = 0"),
        Some(ids) => {
            let list = ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");
            format!("{column} IN ({list})")
        }
    }
}

/// Resolve which library a media_file belongs to. Walks the
/// movie path (item_id → items.library_id) and the episode path
/// (episode_id → seasons → items(show).library_id). Returns None if
/// the file id doesn't exist.
pub async fn media_file_library_id(
    pool: &SqlitePool,
    file_id: i64,
) -> Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT COALESCE(
             (SELECT library_id FROM items WHERE id = mf.item_id),
             (SELECT i.library_id FROM seasons s
                JOIN items i ON i.id = s.show_id
                JOIN episodes e ON e.season_id = s.id
                WHERE e.id = mf.episode_id)
         ) AS library_id
         FROM media_files mf WHERE mf.id = ?",
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.try_get("library_id").ok()))
}

/// User IDs that currently have access to the given library.
pub async fn list_library_user_ids(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT user_id FROM library_access WHERE library_id = ? ORDER BY user_id",
    )
    .bind(library_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| Ok(r.try_get::<i64, _>("user_id")?))
        .collect()
}

/// Replace the access set for a library. Owners are always granted access,
/// regardless of whether they're in the request body — the UI shows them
/// as locked-on but a malformed request shouldn't be able to revoke them.
pub async fn set_library_user_ids(
    pool: &SqlitePool,
    library_id: i64,
    user_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM library_access WHERE library_id = ?")
        .bind(library_id)
        .execute(&mut *tx)
        .await?;
    // Always include all owners. Compute the union.
    let owners: Vec<i64> =
        sqlx::query("SELECT id FROM users WHERE role = 'owner'")
            .fetch_all(&mut *tx)
            .await?
            .iter()
            .map(|r| r.try_get::<i64, _>("id"))
            .collect::<std::result::Result<_, _>>()?;
    let mut seen = std::collections::HashSet::new();
    for uid in user_ids.iter().chain(owners.iter()) {
        if !seen.insert(*uid) {
            continue;
        }
        sqlx::query(
            "INSERT INTO library_access (user_id, library_id) VALUES (?, ?) \
             ON CONFLICT DO NOTHING",
        )
        .bind(uid)
        .bind(library_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// User prefs: hidden libraries
// ---------------------------------------------------------------------------

pub async fn list_hidden_libraries(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT library_id FROM user_hidden_libraries WHERE user_id = ? ORDER BY library_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| Ok(r.try_get::<i64, _>("library_id")?))
        .collect()
}

pub async fn set_hidden_libraries(
    pool: &SqlitePool,
    user_id: i64,
    library_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_hidden_libraries WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for lib_id in library_ids {
        sqlx::query(
            "INSERT INTO user_hidden_libraries (user_id, library_id) VALUES (?, ?) \
             ON CONFLICT DO NOTHING",
        )
        .bind(user_id)
        .bind(lib_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// My List
// ---------------------------------------------------------------------------

/// Items the user has saved, newest first. Uses the same ITEM_SELECT as
/// list_items so cards get full play_state + artwork.
pub async fn list_my_list(
    pool: &SqlitePool,
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<ListedItem>> {
    let filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "{ITEM_SELECT} \
         JOIN user_my_list uml ON uml.item_id = i.id AND uml.user_id = ? \
         WHERE {filter} \
         ORDER BY uml.added_at DESC"
    );
    let rows = sqlx::query(&sql)
        .bind(user_id)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| -> Result<ListedItem> {
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
            })
        })
        .collect()
}

pub async fn add_to_my_list(
    pool: &SqlitePool,
    user_id: i64,
    item_id: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO user_my_list (user_id, item_id, added_at) VALUES (?, ?, ?) \
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(item_id)
    .bind(chimpflix_common::now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_from_my_list(
    pool: &SqlitePool,
    user_id: i64,
    item_id: i64,
) -> Result<bool> {
    let res =
        sqlx::query("DELETE FROM user_my_list WHERE user_id = ? AND item_id = ?")
            .bind(user_id)
            .bind(item_id)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

/// Local items matching any of the given TMDB ids (and the given kind).
/// Caller-controlled limit; used by the "Similar" rail in the modal.
pub async fn find_listed_items_by_tmdb_ids(
    pool: &SqlitePool,
    tmdb_ids: &[i64],
    kind: ItemKind,
    user_id: i64,
    limit: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<ListedItem>> {
    if tmdb_ids.is_empty() {
        return Ok(Vec::new());
    }
    // Build (?, ?, ...) for the IN clause. sqlx-sqlite doesn't bind slices
    // natively, so we render placeholders and bind one-by-one.
    let placeholders = std::iter::repeat("?")
        .take(tmdb_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let lib_filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "{ITEM_SELECT} WHERE i.kind = ? AND i.tmdb_id IN ({placeholders}) AND {lib_filter} LIMIT ?",
    );
    let mut q = sqlx::query(&sql).bind(user_id).bind(kind.as_str());
    for id in tmdb_ids {
        q = q.bind(id);
    }
    q = q.bind(limit);
    let rows = q.fetch_all(pool).await?;
    rows.iter()
        .map(|row| -> Result<ListedItem> {
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
            })
        })
        .collect()
}

/// Overwrite the cached trending list for one (source, media_kind)
/// slice. Wrapped in a transaction so the rail can't observe a torn
/// half-old half-new state mid-refresh. Returns the number of entries
/// written so the caller can log it.
pub async fn replace_trending(
    pool: &SqlitePool,
    source: &str,
    media_kind: &str,
    entries: &[crate::TrendingEntry],
) -> Result<usize> {
    let now = chimpflix_common::now_ms();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM trending_cache WHERE source = ? AND media_kind = ?")
        .bind(source)
        .bind(media_kind)
        .execute(&mut *tx)
        .await?;
    for entry in entries {
        sqlx::query(
            "INSERT INTO trending_cache \
             (source, media_kind, rank, tmdb_id, title, poster_path, fetched_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(source)
        .bind(media_kind)
        .bind(entry.rank)
        .bind(entry.tmdb_id)
        .bind(&entry.title)
        .bind(&entry.poster_path)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(entries.len())
}

/// Trending entries that exist in this server's library, ordered by
/// trending rank. Source is hard-coded to TMDB for now — when a second
/// provider is added the query will take a source parameter and pick
/// the freshest one. `kind` is matched as a string ("movie" / "show")
/// against `items.kind`.
pub async fn list_trending_in_library(
    pool: &SqlitePool,
    kind: ItemKind,
    user_id: i64,
    limit: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<(i64, ListedItem)>> {
    let lib_filter = library_filter_sql("i.library_id", accessible);
    // Mirrors ITEM_SELECT but adds `tc.rank` to the projection so the
    // caller can render rank badges (and we keep the natural ORDER BY
    // ranking). The shared ITEM_SELECT constant is geometry-locked for
    // Item::from_row, so we re-spell it here rather than mutate it.
    let sql = format!(
        "SELECT i.*, \
            (SELECT source_url FROM images \
                WHERE item_id = i.id AND kind = 'poster' \
                ORDER BY is_primary DESC, id ASC LIMIT 1) AS poster_path, \
            (SELECT source_url FROM images \
                WHERE item_id = i.id AND kind = 'backdrop' \
                ORDER BY is_primary DESC, id ASC LIMIT 1) AS backdrop_path, \
            ps.position_ms     AS ps_position_ms, \
            ps.duration_ms     AS ps_duration_ms, \
            ps.watched         AS ps_watched, \
            ps.view_count      AS ps_view_count, \
            ps.last_played_at  AS ps_last_played_at, \
            tc.rank            AS trending_rank \
         FROM items i \
         INNER JOIN trending_cache tc \
           ON tc.tmdb_id = i.tmdb_id \
          AND tc.source = 'tmdb' \
          AND tc.media_kind = ? \
         LEFT JOIN play_state ps \
           ON ps.item_id = i.id AND ps.user_id = ? \
         WHERE i.kind = ? AND {lib_filter} \
         ORDER BY tc.rank ASC \
         LIMIT ?",
    );
    let rows = sqlx::query(&sql)
        .bind(kind.as_str())   // tc.media_kind
        .bind(user_id)         // ps.user_id
        .bind(kind.as_str())   // i.kind
        .bind(limit)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| -> Result<(i64, ListedItem)> {
            use sqlx::Row;
            let rank: i64 = row.try_get("trending_rank")?;
            Ok((
                rank,
                ListedItem {
                    item: Item::from_row(row)?,
                    play_state: PlayStateForItem::from_columns(row)?,
                },
            ))
        })
        .collect()
}

pub async fn get_item(
    pool: &SqlitePool,
    id: i64,
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<Option<Item>> {
    let filter = library_filter_sql("i.library_id", accessible);
    let sql = format!("{ITEM_SELECT} WHERE i.id = ? AND {filter}");
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
    accessible: Option<&[i64]>,
) -> Result<Option<ItemDetail>> {
    let filter = library_filter_sql("i.library_id", accessible);
    let sql = format!("{ITEM_SELECT} WHERE i.id = ? AND {filter}");
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
    let credits = list_credits_for_item(pool, id).await.unwrap_or_default();
    let extras = list_extras_for_item(pool, id).await.unwrap_or_default();
    let reviews = reviews_summary_for_item(pool, id)
        .await
        .unwrap_or_default();

    Ok(Some(ItemDetail {
        item,
        genres,
        play_state,
        files,
        seasons,
        credits,
        extras,
        reviews,
    }))
}

async fn list_credits_for_item(pool: &SqlitePool, item_id: i64) -> Result<Vec<Credit>> {
    let rows = sqlx::query(
        "SELECT c.id, c.role_kind, c.role, c.character_name, c.sort_order,
                p.id AS p_id, p.name, p.tmdb_id, p.imdb_id, p.photo_url,
                p.biography, p.birthday, p.deathday, p.place_of_birth,
                p.known_for_department
         FROM item_credits c
         JOIN people p ON p.id = c.person_id
         WHERE c.item_id = ?
         ORDER BY c.role_kind, c.sort_order, c.id",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(Credit {
                id: r.try_get("id")?,
                role_kind: r.try_get("role_kind")?,
                role: r.try_get("role")?,
                character_name: r.try_get("character_name")?,
                sort_order: r.try_get("sort_order")?,
                person: Person {
                    id: r.try_get("p_id")?,
                    name: r.try_get("name")?,
                    tmdb_id: r.try_get("tmdb_id")?,
                    imdb_id: r.try_get("imdb_id")?,
                    photo_url: r.try_get("photo_url")?,
                    biography: r.try_get("biography")?,
                    birthday: r.try_get("birthday")?,
                    deathday: r.try_get("deathday")?,
                    place_of_birth: r.try_get("place_of_birth")?,
                    known_for_department: r.try_get("known_for_department")?,
                },
            })
        })
        .collect()
}

async fn list_extras_for_item(pool: &SqlitePool, item_id: i64) -> Result<Vec<Extra>> {
    let rows = sqlx::query(
        "SELECT id, kind, title, source, source_id, thumb_url, duration_ms, published_at
         FROM item_extras
         WHERE item_id = ?
         ORDER BY sort_order, id",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(Extra {
                id: r.try_get("id")?,
                kind: r.try_get("kind")?,
                title: r.try_get("title")?,
                source: r.try_get("source")?,
                source_id: r.try_get("source_id")?,
                thumb_url: r.try_get("thumb_url")?,
                duration_ms: r.try_get("duration_ms")?,
                published_at: r.try_get("published_at")?,
            })
        })
        .collect()
}

async fn reviews_summary_for_item(pool: &SqlitePool, item_id: i64) -> Result<ReviewsSummary> {
    // Count covers all reviews (including those without a rating, since they
    // still appear in the list). Average is computed only across rated rows.
    let row = sqlx::query(
        "SELECT
            (SELECT COUNT(*) FROM item_reviews WHERE item_id = ?) AS n,
            (SELECT AVG(rating) FROM item_reviews
                WHERE item_id = ? AND rating IS NOT NULL) AS avg_rating",
    )
    .bind(item_id)
    .bind(item_id)
    .fetch_one(pool)
    .await?;
    Ok(ReviewsSummary {
        count: row.try_get("n")?,
        average: row.try_get("avg_rating")?,
    })
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
    accessible: Option<&[i64]>,
) -> Result<Option<SeasonDetail>> {
    // Resolve the season + its show's library so we can apply access
    // control. Owners bypass via accessible=None.
    let filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "SELECT s.* FROM seasons s \
         JOIN items i ON i.id = s.show_id \
         WHERE s.id = ? AND {filter}",
    );
    let Some(row) = sqlx::query(&sql)
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
    accessible: Option<&[i64]>,
) -> Result<Option<EpisodeDetail>> {
    let filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
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
         JOIN items i   ON i.id = s.show_id
         LEFT JOIN play_state ps
             ON ps.episode_id = e.id AND ps.user_id = ?
         WHERE e.id = ? AND {filter}",
    );
    let Some(r) = sqlx::query(&sql)
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
         WHERE item_id = ? AND removed_at IS NULL ORDER BY id ASC"
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
         WHERE episode_id = ? AND removed_at IS NULL ORDER BY id ASC"
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
    let markers = list_markers_for_file(pool, id).await?;
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
        markers,
    })
}

pub async fn list_markers_for_file(
    pool: &SqlitePool,
    media_file_id: i64,
) -> Result<Vec<Marker>> {
    let rows = sqlx::query(
        "SELECT kind, start_ms, end_ms, label FROM markers \
         WHERE media_file_id = ? ORDER BY start_ms ASC",
    )
    .bind(media_file_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(Marker {
                kind: r.try_get("kind")?,
                start_ms: r.try_get("start_ms")?,
                end_ms: r.try_get("end_ms")?,
                label: r.try_get::<Option<String>, _>("label").ok().flatten(),
            })
        })
        .collect()
}

/// Replace previously auto-detected markers for this file with `new_markers`.
/// Manually-edited markers (source != 'auto') are preserved.
pub async fn replace_auto_markers(
    pool: &SqlitePool,
    media_file_id: i64,
    new_markers: &[(String, i64, i64)],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM markers WHERE media_file_id = ? AND source = 'auto'",
    )
    .bind(media_file_id)
    .execute(&mut *tx)
    .await?;
    for (kind, start_ms, end_ms) in new_markers {
        sqlx::query(
            "INSERT INTO markers (media_file_id, kind, start_ms, end_ms, source) \
             VALUES (?, ?, ?, ?, 'auto')",
        )
        .bind(media_file_id)
        .bind(kind)
        .bind(start_ms)
        .bind(end_ms)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// All media files in a library — used by the bulk marker detection
/// endpoint. Returns (file_id, path, duration_ms).
pub async fn list_media_files_in_library(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<Vec<(i64, String, Option<i64>)>> {
    let rows = sqlx::query(
        "SELECT mf.id, mf.path, mf.duration_ms FROM media_files mf
         LEFT JOIN items i ON i.id = mf.item_id
         LEFT JOIN episodes e ON e.id = mf.episode_id
         LEFT JOIN seasons s ON s.id = e.season_id
         LEFT JOIN items shows ON shows.id = s.show_id
         WHERE i.library_id = ? OR shows.library_id = ?",
    )
    .bind(library_id)
    .bind(library_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok((
                r.try_get::<i64, _>("id")?,
                r.try_get::<String, _>("path")?,
                r.try_get::<Option<i64>, _>("duration_ms").ok().flatten(),
            ))
        })
        .collect()
}

/// Files in `library_id` that don't already have any auto-detected
/// markers. Used by the scheduled `detect_markers` task to skip
/// previously-processed files — keeps the maintenance-window run
/// idempotent on subsequent runs (only new files get the expensive
/// blackdetect pass). Operator-triggered re-detection still uses
/// `list_media_files_in_library` and overwrites via
/// `replace_auto_markers`.
pub async fn list_media_files_needing_markers(
    pool: &SqlitePool,
    library_id: i64,
    batch_size: i64,
) -> Result<Vec<(i64, String, Option<i64>)>> {
    let limit = batch_size.clamp(1, 1000);
    let rows = sqlx::query(
        "SELECT mf.id, mf.path, mf.duration_ms FROM media_files mf
         LEFT JOIN items i ON i.id = mf.item_id
         LEFT JOIN episodes e ON e.id = mf.episode_id
         LEFT JOIN seasons s ON s.id = e.season_id
         LEFT JOIN items shows ON shows.id = s.show_id
         WHERE (i.library_id = ? OR shows.library_id = ?)
           AND mf.removed_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM markers WHERE media_file_id = mf.id AND source = 'auto'
           )
         ORDER BY mf.id
         LIMIT ?",
    )
    .bind(library_id)
    .bind(library_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok((
                r.try_get::<i64, _>("id")?,
                r.try_get::<String, _>("path")?,
                r.try_get::<Option<i64>, _>("duration_ms").ok().flatten(),
            ))
        })
        .collect()
}

async fn list_streams_for_file(
    pool: &SqlitePool,
    media_file_id: i64,
) -> Result<Vec<MediaStreamSummary>> {
    let rows = sqlx::query(
        "SELECT stream_index, kind, codec, language, title, channels, is_default, is_forced
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
            title: r.try_get::<Option<String>, _>("title").ok().flatten(),
            channels: r.try_get::<Option<i32>, _>("channels").ok().flatten(),
            is_default: r.try_get::<i64, _>("is_default")? != 0,
            is_forced: r.try_get::<i64, _>("is_forced")? != 0,
        });
    }
    Ok(out)
}

/// Resolve a media_file row to `(item_id, episode_id)` — exactly one
/// is non-None for a healthy row. Used by the playback-events
/// recorder so the start event can be tagged with the right owning
/// id (movies → item_id, episodes → episode_id) and rolled up at
/// aggregation time.
pub async fn media_file_owner(
    pool: &SqlitePool,
    file_id: i64,
) -> Result<(Option<i64>, Option<i64>)> {
    let row = sqlx::query("SELECT item_id, episode_id FROM media_files WHERE id = ?")
        .bind(file_id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some(r) => Ok((
            r.try_get::<Option<i64>, _>("item_id").ok().flatten(),
            r.try_get::<Option<i64>, _>("episode_id").ok().flatten(),
        )),
        None => Ok((None, None)),
    }
}

pub async fn get_media_file_locator(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<MediaFileLocator>> {
    // Soft-deleted rows (removed_at IS NOT NULL) intentionally return
    // None here. The stream handler treats that as 404 — the row
    // hangs around for the grace period purely for "this user
    // already watched it" history reasons.
    let Some(r) = sqlx::query(
        "SELECT id, path, size_bytes, container FROM media_files
         WHERE id = ? AND removed_at IS NULL",
    )
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
    email: Option<&str>,
) -> Result<User> {
    let now = now_ms();
    let row = sqlx::query(
        "UPDATE users
           SET username      = ?,
               password_hash = ?,
               role          = 'owner',
               display_name  = ?,
               email         = ?,
               updated_at    = ?
         WHERE id = 1 AND username = '_default'
         RETURNING *",
    )
    .bind(username)
    .bind(password_hash)
    .bind(display_name)
    .bind(email)
    .bind(now)
    .fetch_optional(pool)
    .await?
    .context("setup already completed or placeholder user is missing")?;
    User::from_row(&row)
}

/// All registered users, ordered by id (owner first since they're #1).
pub async fn list_users(pool: &SqlitePool) -> Result<Vec<User>> {
    let rows = sqlx::query("SELECT * FROM users ORDER BY id ASC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(User::from_row).collect()
}

/// Removes a user. ON DELETE CASCADE wipes the user's sessions; play_state
/// and similar per-user tables behave the same in this schema.
pub async fn delete_user(pool: &SqlitePool, id: i64) -> Result<bool> {
    // Last-owner guard: the system must always have at least one owner
    // who can manage everything. Otherwise an admin could end up
    // unable to mint a fresh owner and the install would be stuck.
    let target_role = current_user_role(pool, id).await?;
    if matches!(target_role, Some(UserRole::Owner)) && count_owners(pool).await? <= 1 {
        anyhow::bail!("cannot delete the last owner — promote another user to owner first");
    }
    let res = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn set_user_role(
    pool: &SqlitePool,
    id: i64,
    role: UserRole,
) -> Result<Option<User>> {
    // Same last-owner guard for demotion. Promoting to owner has no
    // such constraint — extra owners are fine.
    if !matches!(role, UserRole::Owner) {
        let current = current_user_role(pool, id).await?;
        if matches!(current, Some(UserRole::Owner)) && count_owners(pool).await? <= 1 {
            anyhow::bail!(
                "cannot demote the last owner — promote another user to owner first"
            );
        }
    }
    let res = sqlx::query(
        "UPDATE users SET role = ?, updated_at = ? WHERE id = ? RETURNING *",
    )
    .bind(role.as_str())
    .bind(chimpflix_common::now_ms())
    .bind(id)
    .fetch_optional(pool)
    .await?;
    res.as_ref().map(User::from_row).transpose()
}

/// Fetch the current role of a user by id, or `None` if not found.
/// Internal helper for the last-owner guards.
async fn current_user_role(pool: &SqlitePool, id: i64) -> Result<Option<UserRole>> {
    let row = sqlx::query("SELECT role FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some(r) => {
            let s: String = r.try_get("role")?;
            Ok(Some(UserRole::from_db(&s)?))
        }
        None => Ok(None),
    }
}

/// Live count of owner-role users. The two callers above use this to
/// prevent demote/delete actions that would leave the system orphaned.
pub async fn count_owners(pool: &SqlitePool) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM users WHERE role = 'owner'")
        .fetch_one(pool)
        .await?;
    Ok(row.try_get("c")?)
}

#[derive(Debug, Default)]
pub struct UserSelfUpdate {
    pub display_name: Option<Option<String>>,
    pub avatar_url: Option<Option<String>>,
    pub email: Option<Option<String>>,
    pub default_audio_lang: Option<Option<String>>,
    pub default_subtitle_lang: Option<Option<String>>,
    pub notify_via_email: Option<bool>,
}

/// Patch the caller's own profile/prefs. Each field is double-Option: `None`
/// means "leave as-is", `Some(None)` means "clear this field", `Some(Some(v))`
/// means "set to v".
pub async fn update_user_self(
    pool: &SqlitePool,
    user_id: i64,
    patch: UserSelfUpdate,
) -> Result<Option<User>> {
    let mut sets: Vec<&str> = Vec::new();
    if patch.display_name.is_some() {
        sets.push("display_name = ?");
    }
    if patch.avatar_url.is_some() {
        sets.push("avatar_path = ?");
    }
    if patch.email.is_some() {
        sets.push("email = ?");
    }
    if patch.default_audio_lang.is_some() {
        sets.push("default_audio_lang = ?");
    }
    if patch.default_subtitle_lang.is_some() {
        sets.push("default_subtitle_lang = ?");
    }
    if patch.notify_via_email.is_some() {
        sets.push("notify_via_email = ?");
    }
    if sets.is_empty() {
        return find_user_by_id(pool, user_id).await;
    }
    sets.push("updated_at = ?");
    let sql = format!(
        "UPDATE users SET {} WHERE id = ? RETURNING *",
        sets.join(", "),
    );
    let mut q = sqlx::query(&sql);
    if let Some(v) = patch.display_name {
        q = q.bind(v);
    }
    if let Some(v) = patch.avatar_url {
        q = q.bind(v);
    }
    if let Some(v) = patch.email {
        q = q.bind(v);
    }
    if let Some(v) = patch.default_audio_lang {
        q = q.bind(v);
    }
    if let Some(v) = patch.default_subtitle_lang {
        q = q.bind(v);
    }
    if let Some(v) = patch.notify_via_email {
        q = q.bind(i64::from(v));
    }
    q = q.bind(chimpflix_common::now_ms()).bind(user_id);
    let res = q.fetch_optional(pool).await?;
    res.as_ref().map(User::from_row).transpose()
}

// ─── Email-change tokens (Phase 28) ────────────────────────────────────────

pub async fn create_email_change_token(
    pool: &SqlitePool,
    user_id: i64,
    new_email: &str,
    code_hash: &str,
    expires_at: i64,
) -> Result<i64> {
    let now = now_ms();
    // Wipe any in-flight token for this user so they only have one
    // outstanding at a time — avoids "which link do I click" confusion.
    sqlx::query("DELETE FROM email_change_tokens WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    let row = sqlx::query(
        "INSERT INTO email_change_tokens
            (user_id, new_email, code_hash, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(user_id)
    .bind(new_email)
    .bind(code_hash)
    .bind(now)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

/// Look up an active token. Returns (token_id, user_id, new_email).
pub async fn find_active_email_change_token(
    pool: &SqlitePool,
    code_hash: &str,
) -> Result<Option<(i64, i64, String)>> {
    let now = now_ms();
    let Some(row) = sqlx::query(
        "SELECT id, user_id, new_email FROM email_change_tokens
          WHERE code_hash = ?
            AND consumed_at IS NULL
            AND expires_at > ?",
    )
    .bind(code_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some((
        row.try_get("id")?,
        row.try_get("user_id")?,
        row.try_get("new_email")?,
    )))
}

/// Apply the change: mark token consumed + write the new email to the
/// user row. One transaction so a unique-index collision on email
/// rolls back the token consumption too.
pub async fn consume_email_change(
    pool: &SqlitePool,
    token_id: i64,
    user_id: i64,
    new_email: &str,
) -> Result<()> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        "UPDATE email_change_tokens
            SET consumed_at = ?
          WHERE id = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(token_id)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        anyhow::bail!("email-change token already consumed");
    }
    sqlx::query("UPDATE users SET email = ?, updated_at = ? WHERE id = ?")
        .bind(new_email)
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Replace the user's password hash. Caller is responsible for hashing
/// the plaintext (via the argon2 helper) and for any session-rotation
/// that should follow. We bump `updated_at` so the touch_audit pattern
/// still works.
pub async fn update_user_password(
    pool: &SqlitePool,
    user_id: i64,
    new_hash: &str,
) -> Result<bool> {
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
    )
    .bind(new_hash)
    .bind(now)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Lookup by email (case-insensitive). Used by password-reset request to
/// resolve the user before issuing a token. Returns None for unknown
/// emails — the caller MUST treat that identically to "user found,
/// token issued" to avoid leaking which addresses are registered.
pub async fn find_user_by_email(
    pool: &SqlitePool,
    email: &str,
) -> Result<Option<User>> {
    let Some(row) = sqlx::query("SELECT * FROM users WHERE email = ? COLLATE NOCASE")
        .bind(email)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(User::from_row(&row)?))
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

/// Look up the first (oldest) user with `role = 'owner'`. Used by the
/// network auth-bypass path to map a trusted IP to a concrete user.
/// `None` is essentially impossible on a healthy deployment — the
/// setup flow guarantees an owner — but callers should still handle
/// it gracefully rather than panic.
pub async fn find_first_owner(pool: &SqlitePool) -> Result<Option<User>> {
    let Some(row) =
        sqlx::query("SELECT * FROM users WHERE role = 'owner' ORDER BY id ASC LIMIT 1")
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
    email: Option<&str>,
) -> Result<User> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO users (username, password_hash, role, display_name, email, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING *",
    )
    .bind(username)
    .bind(password_hash)
    .bind(role.as_str())
    .bind(display_name)
    .bind(email)
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

/// Wipe every session for a user — used by password reset (so a recovered
/// account boots all other devices) and by admin "sign out everywhere".
pub async fn delete_sessions_for_user(pool: &SqlitePool, user_id: i64) -> Result<u64> {
    let res = sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Wipe every session for a user EXCEPT the one identified by `keep_id`.
/// Used by the user-facing "sign out of all other devices" action and
/// by 2FA enroll/disable rotation (keep the request's session alive).
pub async fn delete_sessions_for_user_except(
    pool: &SqlitePool,
    user_id: i64,
    keep_id: i64,
) -> Result<u64> {
    let res = sqlx::query("DELETE FROM sessions WHERE user_id = ? AND id != ?")
        .bind(user_id)
        .bind(keep_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// ─── Password reset tokens (Phase 23) ──────────────────────────────────────
//
// Same design as invites: plaintext token shown once via email, only the
// SHA-256 hash persisted. Single-use, short-lived (default 1h), and any
// successful redemption wipes all existing sessions for the user via
// `delete_sessions_for_user`.

pub async fn create_password_reset_token(
    pool: &SqlitePool,
    user_id: i64,
    code_hash: &str,
    expires_at: i64,
    requested_ip: Option<&str>,
    user_agent: Option<&str>,
) -> Result<i64> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO password_reset_tokens
            (user_id, code_hash, requested_ip, user_agent, created_at, expires_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(user_id)
    .bind(code_hash)
    .bind(requested_ip)
    .bind(user_agent)
    .bind(now)
    .bind(expires_at)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

/// Look up an active token by hash. Returns None for expired, consumed,
/// or unknown tokens — callers MUST surface identical "invalid or
/// expired" errors for all three to avoid leaking which case matched.
pub async fn find_active_password_reset_token(
    pool: &SqlitePool,
    code_hash: &str,
) -> Result<Option<(i64, i64)>> {
    let now = now_ms();
    let Some(row) = sqlx::query(
        "SELECT id, user_id FROM password_reset_tokens
          WHERE code_hash = ?
            AND consumed_at IS NULL
            AND expires_at > ?",
    )
    .bind(code_hash)
    .bind(now)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    Ok(Some((row.try_get("id")?, row.try_get("user_id")?)))
}

/// Mark a token consumed + update the user's password + wipe sessions in
/// a single transaction so a partial failure can't leave the account
/// half-reset.
pub async fn consume_password_reset(
    pool: &SqlitePool,
    token_id: i64,
    user_id: i64,
    new_password_hash: &str,
) -> Result<u64> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        "UPDATE password_reset_tokens
            SET consumed_at = ?
          WHERE id = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(token_id)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        anyhow::bail!("reset token already consumed");
    }
    sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(new_password_hash)
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let sessions = sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(sessions.rows_affected())
}

/// House-cleaning. The scheduler can call this periodically; not
/// strictly required for correctness since `find_active_*` filters by
/// expires_at, but keeps the table from growing unbounded.
pub async fn cleanup_expired_password_reset_tokens(pool: &SqlitePool) -> Result<u64> {
    let now = now_ms();
    let res = sqlx::query("DELETE FROM password_reset_tokens WHERE expires_at < ?")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Trim audit_log to a retention window. Defaults applied at the
/// caller (currently 90 days). Returns the row count removed.
pub async fn cleanup_old_audit_log(
    pool: &SqlitePool,
    older_than_ms: i64,
) -> Result<u64> {
    let res = sqlx::query("DELETE FROM audit_log WHERE created_at < ?")
        .bind(older_than_ms)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// ─── TOTP / 2FA (Phase 24) ─────────────────────────────────────────────────
//
// Storage shape mirrors the vault: encrypted secret BLOB + nonce, with
// NULL nonce meaning plaintext mode. Callers always supply the secret
// plaintext + let the vault encrypt it before persisting. Recovery
// codes use the same SHA-256-hashed-at-rest pattern as invites.

#[derive(Debug, Clone)]
pub struct UserTotpRecord {
    pub user_id: i64,
    pub secret_enc: Vec<u8>,
    pub secret_nonce: Option<Vec<u8>>,
    pub verified_at: Option<i64>,
    pub created_at: i64,
}

/// Upsert the user's TOTP secret. Always resets `verified_at` to NULL
/// because the new secret hasn't been proven to belong to a real
/// authenticator yet — the user must POST a code through the verify
/// endpoint to flip the row to "active".
pub async fn upsert_user_totp(
    pool: &SqlitePool,
    user_id: i64,
    secret_enc: &[u8],
    secret_nonce: Option<&[u8]>,
) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO user_totp (user_id, secret_enc, secret_nonce, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET
             secret_enc   = excluded.secret_enc,
             secret_nonce = excluded.secret_nonce,
             verified_at  = NULL,
             created_at   = excluded.created_at",
    )
    .bind(user_id)
    .bind(secret_enc)
    .bind(secret_nonce)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark the user's TOTP enrollment as verified. Idempotent; calling
/// twice just refreshes the timestamp.
pub async fn mark_user_totp_verified(pool: &SqlitePool, user_id: i64) -> Result<()> {
    let now = now_ms();
    sqlx::query("UPDATE user_totp SET verified_at = ? WHERE user_id = ?")
        .bind(now)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_user_totp(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Option<UserTotpRecord>> {
    let Some(row) = sqlx::query("SELECT * FROM user_totp WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(UserTotpRecord {
        user_id: row.try_get("user_id")?,
        secret_enc: row.try_get("secret_enc")?,
        secret_nonce: row.try_get::<Option<Vec<u8>>, _>("secret_nonce").ok().flatten(),
        verified_at: row.try_get::<Option<i64>, _>("verified_at").ok().flatten(),
        created_at: row.try_get("created_at")?,
    }))
}

/// Remove the user's TOTP enrollment + all recovery codes. Used by the
/// user-initiated disable flow and by the admin "reset 2FA" action.
pub async fn delete_user_totp(pool: &SqlitePool, user_id: i64) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let res = sqlx::query("DELETE FROM user_totp WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Replace any existing recovery codes with a fresh set. Called as the
/// last step of enrollment + by the "regenerate recovery codes" action.
pub async fn replace_recovery_codes(
    pool: &SqlitePool,
    user_id: i64,
    code_hashes: &[String],
) -> Result<()> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for h in code_hashes {
        sqlx::query(
            "INSERT INTO user_recovery_codes (user_id, code_hash, created_at)
             VALUES (?, ?, ?)",
        )
        .bind(user_id)
        .bind(h)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Atomically mark a recovery code consumed. Returns true if a row was
/// updated (a valid, unused code); false otherwise. We never reveal
/// which case applied — caller surfaces a generic "invalid code".
pub async fn consume_recovery_code(
    pool: &SqlitePool,
    user_id: i64,
    code_hash: &str,
) -> Result<bool> {
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE user_recovery_codes
            SET consumed_at = ?
          WHERE user_id = ? AND code_hash = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .bind(code_hash)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn count_unused_recovery_codes(pool: &SqlitePool, user_id: i64) -> Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM user_recovery_codes
          WHERE user_id = ? AND consumed_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("n")?)
}

// ─── Notifications (Phase 25) ──────────────────────────────────────────────

/// Users with `role = 'owner'`. Used by `fan_out_notification` so any
/// "all admins" event lands in every owner's inbox.
pub async fn list_owner_ids(pool: &SqlitePool) -> Result<Vec<i64>> {
    let rows = sqlx::query("SELECT id FROM users WHERE role = 'owner'")
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|r| r.try_get::<i64, _>("id").map_err(Into::into))
        .collect()
}

pub async fn insert_notification(
    pool: &SqlitePool,
    user_id: i64,
    kind: &str,
    payload_json: &str,
) -> Result<i64> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO notifications (user_id, kind, payload_json, created_at)
         VALUES (?, ?, ?, ?)
         RETURNING id",
    )
    .bind(user_id)
    .bind(kind)
    .bind(payload_json)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

/// Recent notifications for the user, newest first. Combine read + unread
/// up to `limit` rows so the UI can render a single chronological list.
pub async fn list_notifications(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<Notification>> {
    let rows = sqlx::query(
        "SELECT * FROM notifications
          WHERE user_id = ?
          ORDER BY created_at DESC
          LIMIT ?",
    )
    .bind(user_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?;
    rows.iter().map(Notification::from_row).collect()
}

pub async fn count_unread_notifications(pool: &SqlitePool, user_id: i64) -> Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM notifications
          WHERE user_id = ? AND read_at IS NULL",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("n")?)
}

pub async fn mark_notification_read(
    pool: &SqlitePool,
    user_id: i64,
    notification_id: i64,
) -> Result<bool> {
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE notifications
            SET read_at = ?
          WHERE id = ? AND user_id = ? AND read_at IS NULL",
    )
    .bind(now)
    .bind(notification_id)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Record a successful login. Shifts `last_login_*` into
/// `previous_login_*` and overwrites with the new value, so the
/// post-login screen can show "Last signed in <previous> from <ip>".
/// Best-effort: callers log on failure but never fail the login.
pub async fn record_user_login(
    pool: &SqlitePool,
    user_id: i64,
    ip: Option<&str>,
) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE users
            SET previous_login_at = last_login_at,
                previous_login_ip = last_login_ip,
                last_login_at     = ?,
                last_login_ip     = ?
          WHERE id = ?",
    )
    .bind(now)
    .bind(ip)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_all_notifications_read(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<u64> {
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE notifications SET read_at = ? WHERE user_id = ? AND read_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

// ─── Admin session summaries (Phase 8) ─────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionSummary {
    pub id: i64,
    pub user_id: i64,
    pub username: String,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub last_seen_at: i64,
    pub expires_at: i64,
    pub created_at: i64,
}

pub async fn list_all_sessions(pool: &SqlitePool) -> Result<Vec<SessionSummary>> {
    let now = now_ms();
    let rows = sqlx::query(
        "SELECT s.id, s.user_id, u.username,
                s.user_agent, s.ip,
                s.last_seen_at, s.expires_at, s.created_at
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.expires_at >= ?
         ORDER BY s.last_seen_at DESC",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(SessionSummary {
            id: r.try_get("id")?,
            user_id: r.try_get("user_id")?,
            username: r.try_get("username")?,
            user_agent: r.try_get::<Option<String>, _>("user_agent").ok().flatten(),
            ip: r.try_get::<Option<String>, _>("ip").ok().flatten(),
            last_seen_at: r.try_get("last_seen_at")?,
            expires_at: r.try_get("expires_at")?,
            created_at: r.try_get("created_at")?,
        });
    }
    Ok(out)
}

pub async fn list_user_sessions(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<SessionSummary>> {
    let now = now_ms();
    let rows = sqlx::query(
        "SELECT s.id, s.user_id, u.username,
                s.user_agent, s.ip,
                s.last_seen_at, s.expires_at, s.created_at
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.user_id = ? AND s.expires_at >= ?
         ORDER BY s.last_seen_at DESC",
    )
    .bind(user_id)
    .bind(now)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(SessionSummary {
            id: r.try_get("id")?,
            user_id: r.try_get("user_id")?,
            username: r.try_get("username")?,
            user_agent: r.try_get::<Option<String>, _>("user_agent").ok().flatten(),
            ip: r.try_get::<Option<String>, _>("ip").ok().flatten(),
            last_seen_at: r.try_get("last_seen_at")?,
            expires_at: r.try_get("expires_at")?,
            created_at: r.try_get("created_at")?,
        });
    }
    Ok(out)
}

pub async fn delete_user_sessions(pool: &SqlitePool, user_id: i64) -> Result<u64> {
    let res = sqlx::query("DELETE FROM sessions WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

// Access matrix: every user × library pair, with `allowed` indicating
// whether the access exists. Used by the admin Access page.
//
// `allowed` reflects ONLY direct `library_access` rows — those are what
// the matrix's checkbox edits. `via_groups` lists access-group names
// that ALSO grant this user this library; those grants live in
// `access_group_libraries` × `user_access_groups` and aren't editable
// from the matrix (the admin manages them under Settings → Users →
// Groups). The UI surfaces both so admins can see effective access at
// a glance — without `via_groups` an invite-via-group looked locked
// out here even though the user could browse fine.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AccessMatrixEntry {
    pub user_id: i64,
    pub username: String,
    pub library_id: i64,
    pub library_name: String,
    pub allowed: bool,
    /// Names of access-groups granting this user access to this
    /// library. Empty when none. Sorted alphabetically.
    pub via_groups: Vec<String>,
}

pub async fn access_matrix(pool: &SqlitePool) -> Result<Vec<AccessMatrixEntry>> {
    // The correlated subquery is per (user, library) pair and emits a
    // comma-separated string for transport — Vec<String> deserialises
    // post-fetch. Sorting inside GROUP_CONCAT keeps the order stable so
    // the rendered "via Friends, Family" string doesn't flicker between
    // refreshes.
    let rows = sqlx::query(
        "SELECT u.id AS user_id, u.username,
                l.id AS library_id, l.name AS library_name,
                CASE WHEN la.user_id IS NULL THEN 0 ELSE 1 END AS allowed,
                (SELECT GROUP_CONCAT(g.name, '\u{1f}')
                   FROM user_access_groups uag
                   JOIN access_group_libraries agl
                        ON agl.group_id = uag.group_id AND agl.library_id = l.id
                   JOIN access_groups g ON g.id = uag.group_id
                  WHERE uag.user_id = u.id
                  ORDER BY g.name COLLATE NOCASE) AS via_groups
         FROM users u
         CROSS JOIN libraries l
         LEFT JOIN library_access la
                ON la.user_id = u.id AND la.library_id = l.id
         WHERE u.username <> '_default'
         ORDER BY u.username, l.name",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        // U+001F (ASCII unit separator) is the GROUP_CONCAT delimiter —
        // safe because access-group names can't contain control chars.
        // Falls back to comma split if the value somehow lacks any
        // separator (single-group case has no delimiter to split on).
        let via_raw: Option<String> = r.try_get("via_groups").ok().flatten();
        let via_groups: Vec<String> = via_raw
            .map(|s| s.split('\u{1f}').map(str::to_owned).collect())
            .unwrap_or_default();
        out.push(AccessMatrixEntry {
            user_id: r.try_get("user_id")?,
            username: r.try_get("username")?,
            library_id: r.try_get("library_id")?,
            library_name: r.try_get("library_name")?,
            allowed: r.try_get::<i64, _>("allowed")? != 0,
            via_groups,
        });
    }
    Ok(out)
}

// ─── Access groups (Phase 27) ──────────────────────────────────────────────

const ACCESS_GROUP_LIST_SQL: &str = "
SELECT g.*,
       (SELECT COUNT(*) FROM user_access_groups WHERE group_id = g.id)
           AS member_count,
       (SELECT COUNT(*) FROM access_group_libraries WHERE group_id = g.id)
           AS library_count
  FROM access_groups g";

pub async fn list_access_groups(pool: &SqlitePool) -> Result<Vec<AccessGroup>> {
    let sql = format!("{ACCESS_GROUP_LIST_SQL} ORDER BY g.name COLLATE NOCASE");
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter().map(AccessGroup::from_row_with_counts).collect()
}

pub async fn create_access_group(
    pool: &SqlitePool,
    input: NewAccessGroup,
) -> Result<AccessGroup> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        anyhow::bail!("name must not be empty");
    }
    let description = input.description.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let now = now_ms();
    sqlx::query(
        "INSERT INTO access_groups (name, description, created_at, updated_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&name)
    .bind(description.as_deref())
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    let sql = format!("{ACCESS_GROUP_LIST_SQL} WHERE g.name = ?");
    let row = sqlx::query(&sql).bind(&name).fetch_one(pool).await?;
    AccessGroup::from_row_with_counts(&row)
}

pub async fn get_access_group_detail(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<AccessGroupDetail>> {
    let sql = format!("{ACCESS_GROUP_LIST_SQL} WHERE g.id = ?");
    let Some(row) = sqlx::query(&sql).bind(id).fetch_optional(pool).await? else {
        return Ok(None);
    };
    let group = AccessGroup::from_row_with_counts(&row)?;

    let member_rows = sqlx::query(
        "SELECT user_id FROM user_access_groups WHERE group_id = ? ORDER BY user_id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let member_ids: Result<Vec<i64>> = member_rows
        .iter()
        .map(|r| Ok(r.try_get::<i64, _>("user_id")?))
        .collect();

    let lib_rows = sqlx::query(
        "SELECT library_id FROM access_group_libraries WHERE group_id = ? ORDER BY library_id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let library_ids: Result<Vec<i64>> = lib_rows
        .iter()
        .map(|r| Ok(r.try_get::<i64, _>("library_id")?))
        .collect();

    Ok(Some(AccessGroupDetail {
        group,
        member_ids: member_ids?,
        library_ids: library_ids?,
    }))
}

pub async fn update_access_group(
    pool: &SqlitePool,
    id: i64,
    patch: AccessGroupUpdate,
) -> Result<Option<AccessGroup>> {
    let mut sets: Vec<&str> = Vec::new();
    if patch.name.is_some() {
        sets.push("name = ?");
    }
    if patch.description.is_some() {
        sets.push("description = ?");
    }
    if sets.is_empty() {
        let sql = format!("{ACCESS_GROUP_LIST_SQL} WHERE g.id = ?");
        let row = sqlx::query(&sql).bind(id).fetch_optional(pool).await?;
        return row.as_ref().map(AccessGroup::from_row_with_counts).transpose();
    }
    sets.push("updated_at = ?");
    let sql = format!("UPDATE access_groups SET {} WHERE id = ?", sets.join(", "));
    let mut q = sqlx::query(&sql);
    if let Some(v) = patch.name {
        q = q.bind(v.trim().to_string());
    }
    if let Some(v) = patch.description {
        q = q.bind(v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
    }
    q = q.bind(now_ms()).bind(id);
    let res = q.execute(pool).await?;
    if res.rows_affected() == 0 {
        return Ok(None);
    }
    let sql = format!("{ACCESS_GROUP_LIST_SQL} WHERE g.id = ?");
    let row = sqlx::query(&sql).bind(id).fetch_one(pool).await?;
    Ok(Some(AccessGroup::from_row_with_counts(&row)?))
}

pub async fn delete_access_group(pool: &SqlitePool, id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM access_groups WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Replace the group's library set atomically.
pub async fn set_access_group_libraries(
    pool: &SqlitePool,
    group_id: i64,
    library_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM access_group_libraries WHERE group_id = ?")
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    for lib in library_ids {
        sqlx::query(
            "INSERT INTO access_group_libraries (group_id, library_id) VALUES (?, ?)",
        )
        .bind(group_id)
        .bind(lib)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE access_groups SET updated_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Replace the group's member set atomically.
pub async fn set_access_group_members(
    pool: &SqlitePool,
    group_id: i64,
    user_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_access_groups WHERE group_id = ?")
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    for uid in user_ids {
        sqlx::query(
            "INSERT INTO user_access_groups (user_id, group_id) VALUES (?, ?)",
        )
        .bind(uid)
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE access_groups SET updated_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Groups this user is a member of. Useful for the user-detail view in
/// the admin UI and for displaying "you're in groups: X, Y" to the user.
pub async fn list_user_group_ids(pool: &SqlitePool, user_id: i64) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT group_id FROM user_access_groups WHERE user_id = ? ORDER BY group_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| Ok(r.try_get::<i64, _>("group_id")?))
        .collect()
}

/// Replace the user's group memberships atomically. Mirror of
/// [`set_access_group_members`] from the other direction so the admin
/// can manage from either side.
pub async fn set_user_groups(
    pool: &SqlitePool,
    user_id: i64,
    group_ids: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_access_groups WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for gid in group_ids {
        sqlx::query(
            "INSERT INTO user_access_groups (user_id, group_id) VALUES (?, ?)",
        )
        .bind(user_id)
        .bind(gid)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Library IDs this user has access to through groups only (excluding
/// direct `library_access` grants). Used by the admin user-detail view
/// to distinguish "granted directly" vs "via group X".
pub async fn list_user_group_library_ids(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT DISTINCT agl.library_id
           FROM access_group_libraries agl
           JOIN user_access_groups uag ON uag.group_id = agl.group_id
          WHERE uag.user_id = ?
          ORDER BY agl.library_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| Ok(r.try_get::<i64, _>("library_id")?))
        .collect()
}

// ---------------------------------------------------------------------------
// Invites
// ---------------------------------------------------------------------------

/// Create an invite. The caller passes the SHA-256 hash of the plaintext
/// token — the plaintext never reaches the DB. Library + group pre-binding
/// is inserted as part of the same transaction so an orphan invite (with
/// libraries/groups that no longer exist) is impossible.
pub async fn create_invite(
    pool: &SqlitePool,
    code_hash: &str,
    created_by: i64,
    expires_at: Option<i64>,
    email: Option<&str>,
    library_ids: &[i64],
    group_ids: &[i64],
) -> Result<Invite> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO invites (code_hash, created_by, expires_at, email, created_at)
         VALUES (?, ?, ?, ?, ?)
         RETURNING *",
    )
    .bind(code_hash)
    .bind(created_by)
    .bind(expires_at)
    .bind(email)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let invite = Invite::from_row(&row)?;
    for lib_id in library_ids {
        sqlx::query("INSERT INTO invite_libraries (invite_id, library_id) VALUES (?, ?)")
            .bind(invite.id)
            .bind(lib_id)
            .execute(&mut *tx)
            .await?;
    }
    for group_id in group_ids {
        sqlx::query("INSERT INTO invite_groups (invite_id, group_id) VALUES (?, ?)")
            .bind(invite.id)
            .bind(group_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(invite)
}

/// Group IDs pre-bound to the invite. Mirror of [`invite_library_ids`].
pub async fn invite_group_ids(pool: &SqlitePool, invite_id: i64) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT group_id FROM invite_groups WHERE invite_id = ? ORDER BY group_id",
    )
    .bind(invite_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| Ok(r.try_get::<i64, _>("group_id")?))
        .collect()
}

pub async fn list_invites(pool: &SqlitePool) -> Result<Vec<Invite>> {
    let rows = sqlx::query("SELECT * FROM invites ORDER BY created_at DESC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(Invite::from_row).collect()
}

/// Pre-bound library IDs for the given invite. Order matches library_id ASC.
pub async fn invite_library_ids(pool: &SqlitePool, invite_id: i64) -> Result<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT library_id FROM invite_libraries WHERE invite_id = ? ORDER BY library_id",
    )
    .bind(invite_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| r.try_get::<i64, _>("library_id").map_err(Into::into))
        .collect()
}

pub async fn find_invite_by_code_hash(
    pool: &SqlitePool,
    code_hash: &str,
) -> Result<Option<Invite>> {
    let Some(row) = sqlx::query("SELECT * FROM invites WHERE code_hash = ?")
        .bind(code_hash)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(Invite::from_row(&row)?))
}

/// Consume the invite + grant any pre-bound library access in a single
/// transaction. The invite is identified by its `code_hash`; the plain-
/// text token is hashed at the API edge and never reaches this layer.
pub async fn consume_invite(
    pool: &SqlitePool,
    code_hash: &str,
    user_id: i64,
) -> Result<()> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        "UPDATE invites
            SET consumed_by = ?, consumed_at = ?
          WHERE code_hash = ?
            AND consumed_by IS NULL
            AND (expires_at IS NULL OR expires_at > ?)",
    )
    .bind(user_id)
    .bind(now)
    .bind(code_hash)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        anyhow::bail!("invite is invalid, expired, or already consumed");
    }
    // Grant any pre-bound libraries directly. ON CONFLICT IGNORE keeps
    // the call idempotent if a library row already exists (e.g. retry
    // after a partial transaction in an earlier version).
    sqlx::query(
        "INSERT INTO library_access (user_id, library_id)
         SELECT ?, il.library_id
           FROM invite_libraries il
           JOIN invites i ON i.id = il.invite_id
          WHERE i.code_hash = ?
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(code_hash)
    .execute(&mut *tx)
    .await?;
    // Materialise each pre-bound group's libraries as direct
    // `library_access` rows too, so "invite via Friends group" yields
    // the same end-state as "invite with Movies + Shows checked
    // individually". Without this, the user could only see those
    // libraries via the `user_library_filter` UNION through their
    // group membership — technically functional, but the admin Access
    // matrix (which renders direct grants) showed them as locked out
    // and the behaviour diverged from the manual-checkbox path.
    sqlx::query(
        "INSERT INTO library_access (user_id, library_id)
         SELECT ?, agl.library_id
           FROM invite_groups ig
           JOIN invites i ON i.id = ig.invite_id
           JOIN access_group_libraries agl ON agl.group_id = ig.group_id
          WHERE i.code_hash = ?
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(code_hash)
    .execute(&mut *tx)
    .await?;
    // Preserve the group membership too. Future additions to the group
    // (admin adds Anime to Friends three months from now) automatically
    // propagate to existing members via `user_library_filter`'s UNION
    // — we'd lose that ongoing-template behaviour if we only
    // materialised the snapshot above. Admins can detach a user from a
    // group later under Settings → Users → Groups.
    sqlx::query(
        "INSERT INTO user_access_groups (user_id, group_id)
         SELECT ?, ig.group_id
           FROM invite_groups ig
           JOIN invites i ON i.id = ig.invite_id
          WHERE i.code_hash = ?
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(code_hash)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Mark the invite as sent. Called after the SMTP relay accepts the
/// invite email — purely informational so the admin UI can show whether
/// each invite was emailed vs. only copy-linked.
pub async fn mark_invite_sent(pool: &SqlitePool, invite_id: i64) -> Result<()> {
    sqlx::query("UPDATE invites SET sent_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(invite_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn revoke_invite_by_hash(pool: &SqlitePool, code_hash: &str) -> Result<bool> {
    let res = sqlx::query("DELETE FROM invites WHERE code_hash = ? AND consumed_by IS NULL")
        .bind(code_hash)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Revoke by `invite_id`. The admin UI uses this because the plaintext
/// code isn't recoverable from the list endpoint.
pub async fn revoke_invite_by_id(pool: &SqlitePool, invite_id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM invites WHERE id = ? AND consumed_by IS NULL")
        .bind(invite_id)
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

/// Operator-tunable knobs for the Continue Watching rail. Threaded
/// through from `server_settings` rather than carried as free args
/// because the same set of values affects several places (the query
/// limit + threshold here, plus the dedup post-processing).
#[derive(Debug, Clone, Copy)]
pub struct OnDeckOptions {
    /// Max rows the query returns *before* dedup. We over-fetch so a
    /// user binging one show doesn't end up with a near-empty rail
    /// after dedup collapses 19 episodes into 1 tile. The final list
    /// is also capped by `max_items` after dedup.
    pub max_items: i64,
    /// "Watched" threshold (0-100). Items past this percentage drop
    /// out of the rail because the scrobbler is about to mark them
    /// watched anyway. Matches the client-side scrobble threshold so
    /// the tile vanishes the moment we scrobble (no phantom entries).
    pub played_threshold_pct: i64,
    /// Drop items whose `last_played_at` is older than this many
    /// weeks. 0 disables the time-window filter entirely.
    pub max_age_weeks: i64,
    /// When true, augment the in-progress list with S(N+1)E01 of
    /// any show the user has watched at least one episode of (but
    /// hasn't yet started any episode in season N+1). Plex parity.
    pub include_premieres: bool,
}

impl Default for OnDeckOptions {
    fn default() -> Self {
        Self {
            max_items: 40,
            played_threshold_pct: 90,
            max_age_weeks: 16,
            include_premieres: true,
        }
    }
}

pub async fn on_deck(
    pool: &SqlitePool,
    user_id: i64,
    accessible: Option<&[i64]>,
    options: OnDeckOptions,
) -> Result<OnDeckResponse> {
    // Anything actively watching (started but past neither the
    // "watched" threshold nor the age window) AND not flagged
    // watched yet. Ordered by most recently played.
    //
    // Over-fetch ~2x the user-visible cap so the dedup post-step
    // (collapse multiple in-progress episodes of one show to one
    // tile) still has enough rows to fill the rail after collapsing.
    // SQLite's planner with the index on (user_id, last_played_at)
    // makes this cheap.
    let threshold = options.played_threshold_pct.clamp(50, 99);
    let fetch_limit = (options.max_items * 2).clamp(10, 400);
    let max_age_weeks = options.max_age_weeks.max(0);
    let cutoff_ms = if max_age_weeks == 0 {
        // 0 = disabled — use a value old enough that the filter is a
        // no-op (Unix epoch start).
        0
    } else {
        now_ms() - max_age_weeks * 7 * 86_400_000
    };
    let rows = sqlx::query(
        "SELECT * FROM play_state
         WHERE user_id = ?
           AND watched = 0
           AND position_ms > 0
           AND last_played_at >= ?
           AND (duration_ms IS NULL OR position_ms < duration_ms * ? / 100)
         ORDER BY last_played_at DESC
         LIMIT ?",
    )
    .bind(user_id)
    .bind(cutoff_ms)
    .bind(threshold)
    .bind(fetch_limit)
    .fetch_all(pool)
    .await?;

    // Dedupe show entries: a user partway through a show should see ONE
    // Continue Watching tile for that show — the most recently played
    // episode — not one tile per in-progress episode. Netflix does the
    // same. Rows are already ordered by last_played_at DESC, so the
    // first episode we see for a given show_id is the right one to
    // surface. Movies have no analogous dedup (one play_state per movie
    // already; multiple play_states for one movie isn't a concept here).
    let mut out = Vec::new();
    let mut seen_show_ids = std::collections::HashSet::<i64>::new();
    let cap = options.max_items.max(1) as usize;
    for r in rows {
        if out.len() >= cap {
            break;
        }
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
            if let Some(item) = get_item(pool, iid, user_id, accessible).await? {
                out.push(OnDeckEntry::Movie { item, play_state });
            }
        } else if let Some(eid) = episode_id {
            if let Some(detail) = get_episode_detail(pool, eid, user_id, accessible).await? {
                let show_id = detail.episode.show_id;
                if !seen_show_ids.insert(show_id) {
                    continue;
                }
                if let Some(show) = get_item(pool, show_id, user_id, accessible).await? {
                    out.push(OnDeckEntry::Episode {
                        episode: detail.episode,
                        show,
                        play_state,
                    });
                }
            }
        }
    }

    // Season-premiere augmentation. For shows the user has watched
    // any episode of but isn't actively in-progress on right now,
    // surface the first episode of the next-up season if one exists
    // and they haven't started it. Run AFTER the in-progress dedup
    // so an actively-watched show always shows its current episode,
    // not the upcoming premiere (which is rarer + later).
    if options.include_premieres && out.len() < cap {
        let premieres = list_user_show_premieres(pool, user_id).await?;
        for (show_id, episode_id) in premieres {
            if out.len() >= cap {
                break;
            }
            if !seen_show_ids.insert(show_id) {
                continue;
            }
            let Some(detail) = get_episode_detail(pool, episode_id, user_id, accessible).await?
            else {
                continue;
            };
            let Some(show) = get_item(pool, show_id, user_id, accessible).await? else {
                continue;
            };
            // Stub play_state for an unstarted episode — the UI
            // surface uses these fields to render thumbnails /
            // progress bars; zeros render as "ready to play"
            // without any progress indicator.
            let play_state = PlayStateForItem {
                position_ms: 0,
                duration_ms: detail.episode.duration_ms,
                watched: false,
                view_count: 0,
                last_played_at: now_ms(),
            };
            out.push(OnDeckEntry::Episode {
                episode: detail.episode,
                show,
                play_state,
            });
        }
    }

    Ok(OnDeckResponse { items: out })
}

/// For each show the user has started or finished any episode of,
/// find the lowest S(N+1)E01 they haven't yet started. Returns
/// `(show_id, episode_id)` pairs in show_id order. Used by `on_deck`
/// when `include_premieres` is true.
///
/// Definition of "next-up season": the smallest `season_number`
/// strictly greater than the maximum season the user has any
/// play_state row for. We pick the smallest so a user who watched
/// S1 and skipped S2 + S3 sees S2E1 as their next premiere — not
/// S4E1 — matching the "missed it, here's where you left off
/// chronologically" intent.
async fn list_user_show_premieres(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<(i64, i64)>> {
    // Two CTEs because SQLite rejects aggregate functions inside a
    // correlated subquery's WHERE — `... AND s2.season_number = MIN(s.season_number)`
    // surfaces as "misuse of aggregate function". We compute the
    // per-show next_season in `next_seasons`, then the outer SELECT's
    // subquery references it as a plain column.
    let rows = sqlx::query(
        "WITH user_show_max AS (
             SELECT s.show_id AS show_id, MAX(s.season_number) AS max_season
             FROM play_state ps
             JOIN episodes e ON ps.episode_id = e.id
             JOIN seasons s ON e.season_id = s.id
             WHERE ps.user_id = ?
               AND (ps.watched = 1 OR ps.position_ms > 0)
             GROUP BY s.show_id
         ),
         next_seasons AS (
             SELECT usm.show_id AS show_id, MIN(s.season_number) AS next_season
             FROM user_show_max usm
             JOIN seasons s ON s.show_id = usm.show_id
             WHERE s.season_number > usm.max_season
               AND NOT EXISTS (
                   SELECT 1
                   FROM episodes e3
                   JOIN play_state ps3 ON ps3.episode_id = e3.id
                   WHERE e3.season_id = s.id AND ps3.user_id = ?
               )
             GROUP BY usm.show_id
         )
         SELECT
             ns.show_id AS show_id,
             ns.next_season AS next_season,
             (SELECT e2.id
                FROM episodes e2
                JOIN seasons s2 ON e2.season_id = s2.id
                WHERE s2.show_id = ns.show_id
                  AND s2.season_number = ns.next_season
                  AND e2.episode_number = 1
                LIMIT 1) AS episode_id
         FROM next_seasons ns
         ORDER BY ns.show_id ASC",
    )
    .bind(user_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let show_id: i64 = r.try_get("show_id")?;
        let episode_id: Option<i64> = r.try_get("episode_id").ok().flatten();
        if let Some(eid) = episode_id {
            out.push((show_id, eid));
        }
    }
    Ok(out)
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

// ---------------------------------------------------------------------------
// Library verify / orphan cleanup
// ---------------------------------------------------------------------------

/// One media_file row that the verify pass needs to consider —
/// path on disk so we can stat() it, plus the current removed_at
/// state so we know whether marking-missing is a state change.
#[derive(Debug, Clone)]
pub struct MediaFileVerifyRow {
    pub id: i64,
    pub path: String,
    pub removed_at: Option<i64>,
}

/// Pull every media_file row that belongs to the given library —
/// including soft-deleted ones, so the verify pass can re-mark them
/// (cheap idempotent update) and so a re-appeared file gets its
/// removed_at cleared by the same write path the scanner uses.
pub async fn list_media_files_for_verify(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<Vec<MediaFileVerifyRow>> {
    let rows = sqlx::query(
        "SELECT mf.id, mf.path, mf.removed_at
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
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(MediaFileVerifyRow {
            id: row.try_get("id")?,
            path: row.try_get("path")?,
            removed_at: row.try_get::<Option<i64>, _>("removed_at").ok().flatten(),
        });
    }
    Ok(out)
}

/// Soft-delete the given media_file ids by stamping `removed_at`.
/// Idempotent: re-marking an already-removed row is a no-op (we
/// preserve the *first* removal timestamp so the purge grace
/// window doesn't reset on every verify run).
pub async fn mark_media_files_removed(pool: &SqlitePool, ids: &[i64]) -> Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let now = now_ms();
    let placeholders = std::iter::repeat("?")
        .take(ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "UPDATE media_files SET removed_at = ?
         WHERE id IN ({placeholders}) AND removed_at IS NULL"
    );
    let mut q = sqlx::query(&sql).bind(now);
    for id in ids {
        q = q.bind(id);
    }
    let r = q.execute(pool).await?;
    Ok(r.rows_affected())
}

/// Result of a verify pass against a single library. Surfaced to
/// the admin UI so the operator can see what happened without
/// trawling through the scheduler log.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VerifyReport {
    pub library_id: i64,
    pub files_checked: usize,
    pub files_missing: usize,
    pub newly_marked_removed: u64,
    /// Files we expected to still be missing (already had
    /// removed_at set on entry) that still aren't on disk. Useful
    /// for "did anything come back?" inspection.
    pub still_missing: usize,
    /// Files that had removed_at set but were on disk this pass —
    /// the scanner is what actually clears removed_at; verify just
    /// counts so the operator sees there's pending resurrection
    /// work for the next scan to do.
    pub returned_files: usize,
}

/// Stat each media_file in the library and soft-delete the ones
/// whose underlying path no longer exists. Does NOT hard-delete —
/// the row stays around with `removed_at` set so a purge pass can
/// reap it after a grace window (defends against transient mount
/// failures eating an entire library).
pub async fn verify_library(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<VerifyReport> {
    let rows = list_media_files_for_verify(pool, library_id).await?;
    let mut report = VerifyReport {
        library_id,
        files_checked: rows.len(),
        ..Default::default()
    };
    let mut missing_ids: Vec<i64> = Vec::new();
    for row in &rows {
        // tokio::fs::metadata is async + per-file slow on cold
        // caches; for libraries with tens of thousands of files
        // this loop dominates verify runtime. Acceptable for now
        // — verify is a background task, not a hot path.
        let exists = tokio::fs::metadata(&row.path).await.is_ok();
        match (exists, row.removed_at) {
            (false, None) => {
                report.files_missing += 1;
                missing_ids.push(row.id);
            }
            (false, Some(_)) => {
                report.files_missing += 1;
                report.still_missing += 1;
            }
            (true, Some(_)) => {
                report.returned_files += 1;
                // Don't clear removed_at here — let the next
                // scanner pass do it via the upsert path so the
                // file gets a fresh probe + size/mtime refresh.
                // Listing the file as "returned" is enough signal.
            }
            (true, None) => {}
        }
    }
    report.newly_marked_removed = mark_media_files_removed(pool, &missing_ids).await?;
    Ok(report)
}

/// Hard-delete media_files whose `removed_at` is older than the
/// grace window. Cascades via the existing FK chains:
/// `media_streams`, `markers`, `preview_sprites` rows attached to
/// these files vanish automatically (`ON DELETE CASCADE`).
///
/// After the file delete, sweep parent rows that have been left
/// childless:
///   * Episodes with zero media_files
///   * Seasons with zero episodes
///   * Items (movies) with zero media_files
///   * Items (shows) with zero seasons
/// Each parent delete also cascades to its own children.
///
/// Returns the count of each tier removed so the admin UI can
/// surface what the cleanup did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct PurgeReport {
    pub files_purged: u64,
    pub episodes_purged: u64,
    pub seasons_purged: u64,
    pub items_purged: u64,
    /// Paths of the hard-deleted files. Caller uses these to evict
    /// per-file caches (WebVTT subtitle cache, future thumbnail
    /// caches) that won't be cleaned by the DB cascade. Not
    /// serialized — internal-only.
    #[serde(skip)]
    pub purged_paths: Vec<String>,
}

pub async fn purge_removed_media_files(
    pool: &SqlitePool,
    older_than_ms: i64,
) -> Result<PurgeReport> {
    let mut report = PurgeReport::default();

    // Collect paths first so we can hand them back for cache eviction.
    // The DELETE below removes the rows but cascading FK relationships
    // do not touch the transcoder's on-disk WebVTT cache — that's a
    // separate filesystem responsibility.
    let path_rows = sqlx::query_scalar::<_, String>(
        "SELECT path FROM media_files WHERE removed_at IS NOT NULL AND removed_at < ?",
    )
    .bind(older_than_ms)
    .fetch_all(pool)
    .await?;
    report.purged_paths = path_rows;

    // Hard-delete the soft-deleted files past the grace window.
    let r = sqlx::query(
        "DELETE FROM media_files WHERE removed_at IS NOT NULL AND removed_at < ?",
    )
    .bind(older_than_ms)
    .execute(pool)
    .await?;
    report.files_purged = r.rows_affected();

    // Now sweep orphaned parents. Order matters: episodes first,
    // then seasons (depend on episodes being gone), then items
    // (depend on either files or seasons being gone). Each step
    // also cascades to its own children — e.g. episode delete
    // takes any leftover markers / images / external subtitles
    // attached to that episode.
    let r = sqlx::query(
        "DELETE FROM episodes
         WHERE NOT EXISTS (SELECT 1 FROM media_files WHERE episode_id = episodes.id)",
    )
    .execute(pool)
    .await?;
    report.episodes_purged = r.rows_affected();

    let r = sqlx::query(
        "DELETE FROM seasons
         WHERE NOT EXISTS (SELECT 1 FROM episodes WHERE season_id = seasons.id)",
    )
    .execute(pool)
    .await?;
    report.seasons_purged = r.rows_affected();

    // Items split by kind: movies are orphans when their files are
    // gone; shows are orphans when their seasons are gone.
    let r = sqlx::query(
        "DELETE FROM items
         WHERE (kind = 'movie' AND NOT EXISTS (SELECT 1 FROM media_files WHERE item_id = items.id))
            OR (kind = 'show'  AND NOT EXISTS (SELECT 1 FROM seasons WHERE show_id = items.id))",
    )
    .execute(pool)
    .await?;
    report.items_purged = r.rows_affected();

    Ok(report)
}

/// Operator-initiated delete of specific `media_files` rows. Skips
/// the soft-delete + grace-window dance — this is the path behind the
/// item modal's "Delete from disk" button. Caller is responsible for
/// having checked that the owning library has `allow_media_deletion`
/// turned on and that the actor is an owner.
///
/// Returns the same `PurgeReport` shape as the scheduled purge so the
/// admin UI / API consumer can show the same summary. `purged_paths`
/// includes preview sprite paths in addition to the source file paths
/// — both need on-disk cleanup by the caller.
pub async fn delete_media_files_force(
    pool: &SqlitePool,
    file_ids: &[i64],
) -> Result<PurgeReport> {
    let mut report = PurgeReport::default();
    if file_ids.is_empty() {
        return Ok(report);
    }
    let placeholders = std::iter::repeat("?")
        .take(file_ids.len())
        .collect::<Vec<_>>()
        .join(",");

    // Collect the on-disk artefacts we need to clean up after the row
    // DELETE: the source file path itself, and the preview sprite path
    // (when present). FK cascade drops media_streams / markers /
    // optimized_versions rows but doesn't touch the filesystem.
    let select_sql = format!(
        "SELECT path, preview_sprite_path FROM media_files WHERE id IN ({placeholders})"
    );
    let mut q = sqlx::query(&select_sql);
    for id in file_ids {
        q = q.bind(*id);
    }
    let rows = q.fetch_all(pool).await?;
    for row in rows {
        let path: String = row.try_get("path")?;
        report.purged_paths.push(path);
        if let Ok(Some(sprite)) = row.try_get::<Option<String>, _>("preview_sprite_path") {
            report.purged_paths.push(sprite);
        }
    }

    let delete_sql = format!("DELETE FROM media_files WHERE id IN ({placeholders})");
    let mut q = sqlx::query(&delete_sql);
    for id in file_ids {
        q = q.bind(*id);
    }
    let r = q.execute(pool).await?;
    report.files_purged = r.rows_affected();

    // Cascade orphan sweep — same order + logic as `purge_removed_media_files`.
    // Pulled out as the shared semantics rather than DRYing because the
    // existing function builds its DELETE off `removed_at` which the
    // force path bypasses entirely.
    let r = sqlx::query(
        "DELETE FROM episodes
         WHERE NOT EXISTS (SELECT 1 FROM media_files WHERE episode_id = episodes.id)",
    )
    .execute(pool)
    .await?;
    report.episodes_purged = r.rows_affected();

    let r = sqlx::query(
        "DELETE FROM seasons
         WHERE NOT EXISTS (SELECT 1 FROM episodes WHERE season_id = seasons.id)",
    )
    .execute(pool)
    .await?;
    report.seasons_purged = r.rows_affected();

    let r = sqlx::query(
        "DELETE FROM items
         WHERE (kind = 'movie' AND NOT EXISTS (SELECT 1 FROM media_files WHERE item_id = items.id))
            OR (kind = 'show'  AND NOT EXISTS (SELECT 1 FROM seasons WHERE show_id = items.id))",
    )
    .execute(pool)
    .await?;
    report.items_purged = r.rows_affected();

    Ok(report)
}

/// At-a-glance numbers for a single library — for the admin UI's
/// library card. Pulled in one round-trip (one query per stat
/// because SQLite doesn't have efficient single-query aggregation
/// across the kind=movie / kind=show split). Costs ~1ms per
/// library; the admin page hits these once per render.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LibraryDetailStats {
    pub library_id: i64,
    pub items: i64,
    pub episodes: i64,
    pub files: i64,
    /// Total size in bytes of all (non-removed) media files. Soft-
    /// deleted files are excluded — they're not actually on disk.
    pub total_bytes: i64,
    /// Soft-deleted file count, surfaced separately so the UI can
    /// badge "N orphan(s) pending" alongside the live total.
    pub orphan_files: i64,
    /// Wall-clock time (ms) of the most recent successful scan job.
    /// None means the library has never been scanned successfully.
    pub last_scanned_at: Option<i64>,
}

pub async fn single_library_stats(
    pool: &SqlitePool,
    library_id: i64,
) -> Result<LibraryDetailStats> {
    let mut s = LibraryDetailStats {
        library_id,
        ..Default::default()
    };

    let row = sqlx::query("SELECT COUNT(*) AS n FROM items WHERE library_id = ?")
        .bind(library_id)
        .fetch_one(pool)
        .await?;
    s.items = row.try_get::<i64, _>("n").unwrap_or(0);

    // Episodes belong to a show via seasons → show item.
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM episodes e
         JOIN seasons s ON s.id = e.season_id
         JOIN items i ON i.id = s.show_id
         WHERE i.library_id = ?",
    )
    .bind(library_id)
    .fetch_one(pool)
    .await?;
    s.episodes = row.try_get::<i64, _>("n").unwrap_or(0);

    // Files: present + total bytes, joined either via item (movie)
    // or via episode → season → show item.
    let row = sqlx::query(
        "SELECT COUNT(*) AS n, COALESCE(SUM(mf.size_bytes), 0) AS bytes
         FROM media_files mf
         LEFT JOIN items i_movie ON mf.item_id = i_movie.id
         LEFT JOIN episodes ep ON mf.episode_id = ep.id
         LEFT JOIN seasons s ON ep.season_id = s.id
         LEFT JOIN items i_show ON s.show_id = i_show.id
         WHERE mf.removed_at IS NULL
           AND (i_movie.library_id = ?1 OR i_show.library_id = ?1)",
    )
    .bind(library_id)
    .fetch_one(pool)
    .await?;
    s.files = row.try_get::<i64, _>("n").unwrap_or(0);
    s.total_bytes = row.try_get::<i64, _>("bytes").unwrap_or(0);

    s.orphan_files = count_removed_media_files(pool, library_id).await?;

    let row = sqlx::query(
        "SELECT MAX(finished_at) AS last_at FROM scan_jobs
         WHERE library_id = ? AND status = 'succeeded'",
    )
    .bind(library_id)
    .fetch_one(pool)
    .await?;
    s.last_scanned_at = row.try_get::<Option<i64>, _>("last_at").ok().flatten();

    Ok(s)
}

/// Count soft-deleted files for a library — used by the admin UI
/// to show a "N orphan(s) pending purge" badge per library card.
pub async fn count_removed_media_files(pool: &SqlitePool, library_id: i64) -> Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n FROM media_files mf
         LEFT JOIN items i_movie ON mf.item_id = i_movie.id
         LEFT JOIN episodes ep ON mf.episode_id = ep.id
         LEFT JOIN seasons s ON ep.season_id = s.id
         LEFT JOIN items i_show ON s.show_id = i_show.id
         WHERE mf.removed_at IS NOT NULL
           AND (i_movie.library_id = ?1 OR i_show.library_id = ?1)",
    )
    .bind(library_id)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get::<i64, _>("n").unwrap_or(0))
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
            scanned_at = excluded.scanned_at,
            -- A file that previously had `removed_at` set is back on
            -- disk (the scanner only emits an upsert when it sees a
            -- candidate file). Clear the soft-delete marker so the
            -- row reappears in listings; preserves the existing id
            -- so play_state / markers / preview sprites stay linked.
            removed_at = NULL
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

/// Reset an item to its filename-derived state by clearing every
/// metadata provider linkage and the metadata fields those providers
/// populated. Used by the admin "Unmatch" action when a TMDB match was
/// wrong — afterwards `Fix Match` can re-bind to the correct title.
///
/// What's cleared:
///   - External IDs: tmdb_id, imdb_id, tvdb_id, anilist_id
///   - Auto-collection link: collection_id (drops out of franchise rails)
///   - Provider-supplied text: summary, tagline, original_title, rating_audience, logo_path
///   - Provider-supplied associations: item_credits, item_extras, item_genres
///   - `refreshed_at` (so the next scheduled refresh will retry)
///   - `locked_fields` (a stale match's locks shouldn't survive an unmatch)
///
/// What's kept:
///   - title, sort_title, year (filename-derived; FileParser regenerates
///     them anyway on the next scan)
///   - duration_ms (probed from the file, not the provider)
///   - poster/backdrop cached via `item_images` — those clear when the
///     next match arrives via `store_image`, and leaving them avoids a
///     jarring placeholder flash between unmatch and re-match.
pub async fn unmatch_item(pool: &SqlitePool, item_id: i64) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let now = now_ms();
    let updated = sqlx::query(
        "UPDATE items SET
            tmdb_id = NULL,
            imdb_id = NULL,
            tvdb_id = NULL,
            anilist_id = NULL,
            collection_id = NULL,
            summary = NULL,
            tagline = NULL,
            original_title = NULL,
            rating_audience = NULL,
            logo_path = NULL,
            refreshed_at = NULL,
            locked_fields = '[]',
            updated_at = ?
         WHERE id = ?",
    )
    .bind(now)
    .bind(item_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if updated == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    sqlx::query("DELETE FROM item_credits WHERE item_id = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM item_extras WHERE item_id = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM item_genres WHERE item_id = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(true)
}

// Per-field lock check. `locked_fields` is a JSON array stored on the
// items row; reading once and passing to `pick` is cheaper than checking
// `locked_fields LIKE '%title%'` in SQL for each field.
async fn fetch_locked_fields(pool: &SqlitePool, item_id: i64) -> Result<Vec<String>> {
    let row = sqlx::query("SELECT locked_fields FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(Vec::new()) };
    let raw: String = row.try_get("locked_fields").unwrap_or_default();
    Ok(serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default())
}

fn is_locked(locked: &[String], field: &str) -> bool {
    locked.iter().any(|s| s == field)
}

/// Returns `Some(value)` if the field is not locked, `None` if it is.
/// Bind sites use `COALESCE(?, existing_column)` so a `None` keeps the
/// current value untouched.
fn pick<T>(locked: &[String], field: &str, value: T) -> Option<T> {
    if is_locked(locked, field) { None } else { Some(value) }
}

pub async fn apply_movie_metadata(pool: &SqlitePool, item_id: i64, meta: &TmdbMovie) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", meta.title.clone());
    let sort_title = title.as_deref().map(make_sort_title);
    let original_title = pick(&locked, "original_title", meta.original_title.clone()).flatten();
    let summary = pick(&locked, "summary", meta.summary.clone()).flatten();
    let tagline = pick(&locked, "tagline", meta.tagline.clone()).flatten();
    let year = pick(&locked, "year", meta.year).flatten();
    let rating_audience = pick(&locked, "rating_audience", meta.rating_audience).flatten();
    let duration_ms = meta.runtime_min.map(|m| (m as i64) * 60_000);

    // Logo art: we resolve the TMDB-relative path to a fully-qualified
    // URL (matching how poster/backdrop are persisted via store_image)
    // so the frontend doesn't need to know the TMDB image base. `w500`
    // is plenty for the modal hero — most title-treatment logos are
    // ≤ 1500px wide at original; w500 keeps payload light.
    let logo_url = meta
        .logo_path
        .as_deref()
        .map(|p| tmdb_image_url(p, "w500"));

    sqlx::query(
        "UPDATE items SET
            title = COALESCE(?, title),
            sort_title = COALESCE(?, sort_title),
            original_title = CASE WHEN ?2 IS NOT NULL THEN ? ELSE original_title END,
            summary = CASE WHEN ?2 IS NOT NULL THEN ? ELSE summary END,
            tagline = CASE WHEN ?2 IS NOT NULL THEN ? ELSE tagline END,
            year = COALESCE(?, year),
            duration_ms = COALESCE(duration_ms, ?),
            rating_audience = COALESCE(?, rating_audience),
            tmdb_id = ?,
            imdb_id = COALESCE(?, imdb_id),
            logo_path = COALESCE(?, logo_path),
            refreshed_at = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&title)
    .bind(&sort_title)
    .bind(&original_title)
    .bind(&original_title)
    .bind(&summary)
    .bind(&summary)
    .bind(&tagline)
    .bind(&tagline)
    .bind(year)
    .bind(duration_ms)
    .bind(rating_audience)
    .bind(meta.tmdb_id)
    .bind(meta.imdb_id.as_deref())
    .bind(logo_url.as_deref())
    .bind(now)
    .bind(now)
    .bind(item_id)
    .execute(pool)
    .await?;

    if !is_locked(&locked, "genres") {
        apply_genres(pool, item_id, &meta.genres).await?;
    }
    if !is_locked(&locked, "poster") {
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
    }
    if !is_locked(&locked, "backdrop") {
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
    }

    Ok(())
}

pub async fn apply_show_metadata(pool: &SqlitePool, item_id: i64, meta: &TmdbShow) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", meta.title.clone());
    let sort_title = title.as_deref().map(make_sort_title);
    let original_title = pick(&locked, "original_title", meta.original_title.clone()).flatten();
    let summary = pick(&locked, "summary", meta.summary.clone()).flatten();
    let year = pick(&locked, "year", meta.year).flatten();
    let logo_url = meta
        .logo_path
        .as_deref()
        .map(|p| tmdb_image_url(p, "w500"));

    sqlx::query(
        "UPDATE items SET
            title = COALESCE(?, title),
            sort_title = COALESCE(?, sort_title),
            original_title = CASE WHEN ?2 IS NOT NULL THEN ? ELSE original_title END,
            summary = CASE WHEN ?2 IS NOT NULL THEN ? ELSE summary END,
            year = COALESCE(?, year),
            tmdb_id = ?,
            imdb_id = COALESCE(?, imdb_id),
            logo_path = COALESCE(?, logo_path),
            refreshed_at = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&title)
    .bind(&sort_title)
    .bind(&original_title)
    .bind(&original_title)
    .bind(&summary)
    .bind(&summary)
    .bind(year)
    .bind(meta.tmdb_id)
    .bind(meta.imdb_id.as_deref())
    .bind(logo_url.as_deref())
    .bind(now)
    .bind(now)
    .bind(item_id)
    .execute(pool)
    .await?;

    if !is_locked(&locked, "genres") {
        apply_genres(pool, item_id, &meta.genres).await?;
    }
    if !is_locked(&locked, "poster") {
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
    }
    if !is_locked(&locked, "backdrop") {
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
    }

    Ok(())
}

// ─── Collections (movie franchises) ────────────────────────────────────────

/// Upsert a collection row by TMDB id, returning the local collection id.
/// `overview` may be None — the `belongs_to_collection` stub doesn't
/// include it; the full /collection/{id} fetch does. We update fields
/// COALESCE-style so a follow-up call enriches the row.
pub async fn upsert_collection_stub(
    pool: &SqlitePool,
    stub: &TmdbCollectionStub,
) -> Result<i64> {
    let now = now_ms();
    let poster = stub.poster_path.as_deref().map(|p| tmdb_image_url(p, "w500"));
    let backdrop = stub.backdrop_path.as_deref().map(|p| tmdb_image_url(p, "w1280"));
    let row = sqlx::query(
        "INSERT INTO collections (tmdb_id, name, poster_path, backdrop_path, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(tmdb_id) DO UPDATE SET
            name = excluded.name,
            poster_path = COALESCE(excluded.poster_path, collections.poster_path),
            backdrop_path = COALESCE(excluded.backdrop_path, collections.backdrop_path),
            updated_at = excluded.updated_at
         RETURNING id",
    )
    .bind(stub.tmdb_id)
    .bind(&stub.name)
    .bind(&poster)
    .bind(&backdrop)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

/// Fill the `overview` field on an existing collection using the full
/// /collection/{id} response. Stub-only inserts leave overview NULL.
pub async fn enrich_collection_overview(
    pool: &SqlitePool,
    collection_id: i64,
    full: &TmdbCollection,
) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE collections SET
            overview = COALESCE(?, overview),
            updated_at = ?
         WHERE id = ?",
    )
    .bind(full.overview.as_deref())
    .bind(now)
    .bind(collection_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn assign_item_collection(
    pool: &SqlitePool,
    item_id: i64,
    collection_id: i64,
) -> Result<()> {
    sqlx::query("UPDATE items SET collection_id = ?, updated_at = ? WHERE id = ?")
        .bind(collection_id)
        .bind(now_ms())
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Lookup an existing collection by TMDB id; returns the local id when
/// already known. Used by the scanner to avoid re-fetching /collection
/// detail on every movie ingest.
pub async fn find_collection_by_tmdb(
    pool: &SqlitePool,
    tmdb_id: i64,
) -> Result<Option<i64>> {
    let row = sqlx::query("SELECT id FROM collections WHERE tmdb_id = ?")
        .bind(tmdb_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.try_get::<i64, _>("id").unwrap_or(0)).filter(|v| *v > 0))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CollectionRow {
    pub id: i64,
    pub tmdb_id: Option<i64>,
    /// `auto` = TMDB-discovered franchise (members via items.collection_id);
    /// `manual` = admin-curated grouping (members via collection_items);
    /// `smart` = rule-evaluated grouping (members computed from rule_json).
    pub kind: String,
    pub name: String,
    pub sort_title: Option<String>,
    pub overview: Option<String>,
    pub description: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub created_by_user_id: Option<i64>,
    /// Whitelisted rule DSL for smart collections; NULL otherwise.
    /// See [`SmartRule`] for the parsed form.
    pub rule_json: Option<String>,
    pub item_count: i64,
}

fn map_collection_row(r: &SqliteRow, count: i64) -> Result<CollectionRow> {
    Ok(CollectionRow {
        id: r.try_get("id")?,
        tmdb_id: r.try_get("tmdb_id")?,
        kind: r.try_get("kind")?,
        name: r.try_get("name")?,
        sort_title: r.try_get("sort_title")?,
        overview: r.try_get("overview")?,
        description: r.try_get("description")?,
        poster_path: r.try_get("poster_path")?,
        backdrop_path: r.try_get("backdrop_path")?,
        created_by_user_id: r.try_get("created_by_user_id")?,
        rule_json: r.try_get("rule_json").ok().flatten(),
        item_count: count,
    })
}

/// All collections (auto + manual + smart) with at least one item
/// visible to the user. Auto + manual are aggregated via SQL; smart
/// collections are listed with `item_count = 0` here (computing the
/// rule for every smart row in the list would be expensive) and the
/// detail endpoint runs the rule on demand.
///
/// `include_auto` controls whether TMDB-discovered franchise rows
/// (kind = 'auto') are returned. The Collections rail on the home
/// page omits them so a fresh server doesn't surface dozens of "John
/// Wick Collection"-style rails before the operator has set up any
/// user-curated collections of their own. The admin panel passes
/// `true` so operators can see + manage everything the scanner found.
pub async fn list_collections(
    pool: &SqlitePool,
    accessible: Option<&[i64]>,
    include_auto: bool,
) -> Result<Vec<CollectionRow>> {
    let lib_filter = library_filter_sql("i.library_id", accessible);
    // Membership CTE: auto rows join via items.collection_id, manual rows
    // via the collection_items junction. Auto leg is skipped entirely
    // when `include_auto = false` so we don't pay for COUNT work on rows
    // we're about to filter out at the outer level.
    let auto_leg = if include_auto {
        format!(
            "SELECT c.id AS cid, i.id AS iid
             FROM collections c
             INNER JOIN items i ON i.collection_id = c.id
             WHERE c.kind = 'auto' AND {lib_filter}
             UNION ALL"
        )
    } else {
        String::new()
    };
    let outer_kind_filter = if include_auto {
        ""
    } else {
        // Drop auto rows from the result set even though the membership
        // CTE didn't add them — defends against stray auto rows being
        // counted as zero-member entries via the LEFT JOIN.
        "AND c.kind != 'auto'"
    };
    let sql = format!(
        "WITH visible_members AS (
             {auto_leg}
             SELECT ci.collection_id AS cid, i.id AS iid
             FROM collection_items ci
             INNER JOIN items i ON i.id = ci.item_id
             INNER JOIN collections c ON c.id = ci.collection_id
             WHERE c.kind = 'manual' AND {lib_filter}
         )
         SELECT c.id, c.tmdb_id, c.kind, c.name, c.sort_title, c.overview,
                c.description, c.poster_path, c.backdrop_path,
                c.created_by_user_id, c.rule_json,
                COUNT(vm.iid) AS item_count
         FROM collections c
         LEFT JOIN visible_members vm ON vm.cid = c.id
         WHERE (c.kind != 'smart' OR vm.cid IS NULL) {outer_kind_filter}
         GROUP BY c.id
         HAVING c.kind = 'smart' OR item_count > 0
         ORDER BY COALESCE(c.sort_title, c.name) COLLATE NOCASE ASC"
    );
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter()
        .map(|r| {
            let count: i64 = r.try_get("item_count")?;
            map_collection_row(r, count)
        })
        .collect()
}

pub async fn get_collection(
    pool: &SqlitePool,
    collection_id: i64,
    accessible: Option<&[i64]>,
) -> Result<Option<CollectionRow>> {
    // First fetch kind so we know which membership path to count.
    let kind_row = sqlx::query("SELECT kind FROM collections WHERE id = ?")
        .bind(collection_id)
        .fetch_optional(pool)
        .await?;
    let Some(kr) = kind_row else { return Ok(None) };
    let kind: String = kr.try_get("kind")?;

    let lib_filter = library_filter_sql("i.library_id", accessible);
    let sql = match kind.as_str() {
        "manual" => format!(
            "SELECT c.id, c.tmdb_id, c.kind, c.name, c.sort_title, c.overview,
                    c.description, c.poster_path, c.backdrop_path,
                    c.created_by_user_id, c.rule_json,
                    COUNT(i.id) AS item_count
             FROM collections c
             LEFT JOIN collection_items ci ON ci.collection_id = c.id
             LEFT JOIN items i ON i.id = ci.item_id AND {lib_filter}
             WHERE c.id = ?
             GROUP BY c.id"
        ),
        "smart" => {
            // Membership for smart collections is rule-driven and not
            // joinable here in a generic way; we fetch the row only,
            // and the caller (`list_items_in_collection`) runs the
            // rule to derive members + the actual count on demand.
            "SELECT c.id, c.tmdb_id, c.kind, c.name, c.sort_title, c.overview,
                    c.description, c.poster_path, c.backdrop_path,
                    c.created_by_user_id, c.rule_json,
                    0 AS item_count
             FROM collections c
             WHERE c.id = ?".to_string()
        }
        _ => format!(
            "SELECT c.id, c.tmdb_id, c.kind, c.name, c.sort_title, c.overview,
                    c.description, c.poster_path, c.backdrop_path,
                    c.created_by_user_id, c.rule_json,
                    COUNT(i.id) AS item_count
             FROM collections c
             LEFT JOIN items i ON i.collection_id = c.id AND {lib_filter}
             WHERE c.id = ?
             GROUP BY c.id"
        ),
    };
    let row = sqlx::query(&sql)
        .bind(collection_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let count: i64 = row.try_get("item_count")?;
    // Manual + smart collections with zero accessible members are
    // still visible to admins; auto collections with zero members are
    // effectively orphan stubs and hidden.
    if count == 0 && kind == "auto" {
        return Ok(None);
    }
    Ok(Some(map_collection_row(&row, count)?))
}

pub async fn list_items_in_collection(
    pool: &SqlitePool,
    collection_id: i64,
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<ListedItem>> {
    // Discover kind to pick the membership path. Three SQL shapes:
    //  - auto: items.collection_id, ordered by release year + sort_title
    //  - manual: collection_items junction, ordered by sort_order
    //  - smart: rule_json compiled to a WHERE clause + dynamic joins
    let kind_row = sqlx::query("SELECT kind, rule_json FROM collections WHERE id = ?")
        .bind(collection_id)
        .fetch_optional(pool)
        .await?;
    let Some(kr) = kind_row else { return Ok(Vec::new()) };
    let kind: String = kr.try_get("kind")?;

    if kind == "smart" {
        let rule_json: Option<String> = kr.try_get("rule_json").ok().flatten();
        let Some(rule_json) = rule_json else {
            return Ok(Vec::new());
        };
        return list_items_via_smart_rule(pool, &rule_json, user_id, accessible).await;
    }

    let lib_filter = library_filter_sql("i.library_id", accessible);
    let sql = if kind == "manual" {
        format!(
            "{ITEM_SELECT}
             INNER JOIN collection_items ci ON ci.item_id = i.id
             WHERE ci.collection_id = ? AND {lib_filter}
             ORDER BY ci.sort_order ASC, ci.added_at ASC"
        )
    } else {
        format!(
            "{ITEM_SELECT}
             WHERE i.collection_id = ? AND {lib_filter}
             ORDER BY i.year IS NULL, i.year ASC, i.sort_title COLLATE NOCASE ASC"
        )
    };
    let rows = sqlx::query(&sql)
        .bind(user_id)
        .bind(collection_id)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|r| {
            let item = Item::from_row(r)?;
            let play_state = PlayStateForItem::from_columns(r)?;
            Ok(ListedItem { item, play_state })
        })
        .collect()
}

async fn list_items_via_smart_rule(
    pool: &SqlitePool,
    rule_json: &str,
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<ListedItem>> {
    use crate::smart_rule::{Bind, SmartRule, compile_to_sql};

    let rule: SmartRule = serde_json::from_str(rule_json)?;
    let compiled = compile_to_sql(&rule)?;
    let lib_filter = library_filter_sql("i.library_id", accessible);
    let joins = compiled.joins.join("\n");
    let sql = format!(
        "{ITEM_SELECT}
         {joins}
         WHERE {} AND {lib_filter}
         GROUP BY i.id
         ORDER BY i.sort_title COLLATE NOCASE ASC
         LIMIT 500",
        compiled.where_clause
    );
    let mut q = sqlx::query(&sql).bind(user_id);
    for bind in &compiled.bindings {
        q = match bind {
            Bind::Text(s) => q.bind(s.clone()),
            Bind::Int(n) => q.bind(*n),
            Bind::Real(f) => q.bind(*f),
        };
    }
    let rows = q.fetch_all(pool).await?;
    rows.iter()
        .map(|r| {
            let item = Item::from_row(r)?;
            let play_state = PlayStateForItem::from_columns(r)?;
            Ok(ListedItem { item, play_state })
        })
        .collect()
}

// ─── Manual collections: CRUD + membership ──────────────────────────────

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct NewManualCollection {
    pub name: String,
    pub sort_title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct ManualCollectionUpdate {
    pub name: Option<String>,
    pub sort_title: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub poster_path: Option<Option<String>>,
    pub backdrop_path: Option<Option<String>>,
}

/// Insert a manual collection owned by `actor_user_id`. Returns the new
/// row id. Caller is expected to have validated `name` is non-empty.
pub async fn create_manual_collection(
    pool: &SqlitePool,
    input: NewManualCollection,
    actor_user_id: i64,
) -> Result<i64> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO collections
            (tmdb_id, kind, name, sort_title, description, created_by_user_id, created_at, updated_at)
         VALUES (NULL, 'manual', ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(input.name)
    .bind(input.sort_title)
    .bind(input.description)
    .bind(actor_user_id)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct NewSmartCollection {
    pub name: String,
    pub sort_title: Option<String>,
    pub description: Option<String>,
    /// Pre-compiled rule JSON. Caller is expected to have validated
    /// this via `smart_rule::compile_to_sql` so that we don't store
    /// an unloadable rule.
    pub rule_json: String,
}

pub async fn create_smart_collection(
    pool: &SqlitePool,
    input: NewSmartCollection,
    actor_user_id: i64,
) -> Result<i64> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO collections
            (tmdb_id, kind, name, sort_title, description,
             created_by_user_id, rule_json, created_at, updated_at)
         VALUES (NULL, 'smart', ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(input.name)
    .bind(input.sort_title)
    .bind(input.description)
    .bind(actor_user_id)
    .bind(input.rule_json)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

/// Update a smart collection's rule_json (with optional name/description
/// changes). Returns false for non-existent or non-smart rows.
pub async fn update_smart_collection_rule(
    pool: &SqlitePool,
    collection_id: i64,
    rule_json: &str,
) -> Result<bool> {
    let kind_row = sqlx::query("SELECT kind FROM collections WHERE id = ?")
        .bind(collection_id)
        .fetch_optional(pool)
        .await?;
    let Some(kr) = kind_row else { return Ok(false) };
    let kind: String = kr.try_get("kind")?;
    if kind != "smart" {
        return Ok(false);
    }
    let now = now_ms();
    sqlx::query(
        "UPDATE collections SET rule_json = ?, updated_at = ? WHERE id = ?",
    )
    .bind(rule_json)
    .bind(now)
    .bind(collection_id)
    .execute(pool)
    .await?;
    Ok(true)
}

pub async fn delete_smart_collection(
    pool: &SqlitePool,
    collection_id: i64,
) -> Result<bool> {
    let res = sqlx::query("DELETE FROM collections WHERE id = ? AND kind = 'smart'")
        .bind(collection_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Patch fields on a manual collection. Returns false if the collection
/// doesn't exist or isn't manual (auto collections aren't editable).
pub async fn update_manual_collection(
    pool: &SqlitePool,
    collection_id: i64,
    patch: ManualCollectionUpdate,
) -> Result<bool> {
    // Verify kind first — refusing to ever touch auto rows from this path.
    let kind_row = sqlx::query("SELECT kind FROM collections WHERE id = ?")
        .bind(collection_id)
        .fetch_optional(pool)
        .await?;
    let Some(kr) = kind_row else { return Ok(false) };
    let kind: String = kr.try_get("kind")?;
    if kind != "manual" {
        return Ok(false);
    }

    let now = now_ms();
    // Build the SET clause dynamically; SQLite happily takes COALESCE
    // pairs but we want to differentiate "field omitted" from
    // "field set to null" (the double-Option pattern), so emit only the
    // columns the caller actually included.
    let mut parts: Vec<&str> = Vec::new();
    if patch.name.is_some() {
        parts.push("name = ?");
    }
    if patch.sort_title.is_some() {
        parts.push("sort_title = ?");
    }
    if patch.description.is_some() {
        parts.push("description = ?");
    }
    if patch.poster_path.is_some() {
        parts.push("poster_path = ?");
    }
    if patch.backdrop_path.is_some() {
        parts.push("backdrop_path = ?");
    }
    if parts.is_empty() {
        return Ok(true);
    }
    parts.push("updated_at = ?");

    let sql = format!(
        "UPDATE collections SET {} WHERE id = ?",
        parts.join(", ")
    );
    let mut q = sqlx::query(&sql);
    if let Some(v) = patch.name {
        q = q.bind(v);
    }
    if let Some(v) = patch.sort_title {
        q = q.bind(v);
    }
    if let Some(v) = patch.description {
        q = q.bind(v);
    }
    if let Some(v) = patch.poster_path {
        q = q.bind(v);
    }
    if let Some(v) = patch.backdrop_path {
        q = q.bind(v);
    }
    q = q.bind(now).bind(collection_id);
    q.execute(pool).await?;
    Ok(true)
}

/// Drop a manual collection (and via ON DELETE CASCADE, its junction
/// rows). Returns false for unknown ids and auto collections.
pub async fn delete_manual_collection(
    pool: &SqlitePool,
    collection_id: i64,
) -> Result<bool> {
    let res = sqlx::query("DELETE FROM collections WHERE id = ? AND kind = 'manual'")
        .bind(collection_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// Append items to a manual collection. New entries land at the end
/// (current max sort_order + 1, +2, …). Existing entries are skipped
/// (INSERT OR IGNORE). Returns the count of rows actually inserted.
pub async fn add_items_to_manual_collection(
    pool: &SqlitePool,
    collection_id: i64,
    item_ids: &[i64],
) -> Result<u64> {
    if item_ids.is_empty() {
        return Ok(0);
    }
    let now = now_ms();
    let max_row = sqlx::query(
        "SELECT COALESCE(MAX(sort_order), -1) AS max_so
         FROM collection_items WHERE collection_id = ?",
    )
    .bind(collection_id)
    .fetch_one(pool)
    .await?;
    let mut next: i64 = max_row.try_get::<i64, _>("max_so")? + 1;
    let mut inserted: u64 = 0;
    let mut tx = pool.begin().await?;
    for &iid in item_ids {
        let res = sqlx::query(
            "INSERT OR IGNORE INTO collection_items
                (collection_id, item_id, sort_order, added_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(collection_id)
        .bind(iid)
        .bind(next)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if res.rows_affected() > 0 {
            inserted += 1;
            next += 1;
        }
    }
    // Bump the parent collection's updated_at so the UI's "last modified"
    // is meaningful and the existing list_collections ordering stays
    // accurate for any tooling that sorts by it.
    sqlx::query("UPDATE collections SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(collection_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(inserted)
}

pub async fn remove_item_from_manual_collection(
    pool: &SqlitePool,
    collection_id: i64,
    item_id: i64,
) -> Result<bool> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    let res = sqlx::query(
        "DELETE FROM collection_items
         WHERE collection_id = ? AND item_id = ?",
    )
    .bind(collection_id)
    .bind(item_id)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() > 0 {
        sqlx::query("UPDATE collections SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(collection_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(res.rows_affected() > 0)
}

/// Replace the membership ordering for a manual collection. The input
/// list defines both *which items are members* and *what order they
/// appear in*. Items not in the list are removed; items present but
/// not already in the collection are added.
pub async fn replace_manual_collection_items(
    pool: &SqlitePool,
    collection_id: i64,
    ordered_item_ids: &[i64],
) -> Result<()> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM collection_items WHERE collection_id = ?")
        .bind(collection_id)
        .execute(&mut *tx)
        .await?;
    for (idx, &iid) in ordered_item_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO collection_items
                (collection_id, item_id, sort_order, added_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(collection_id)
        .bind(iid)
        .bind(idx as i64)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query("UPDATE collections SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(collection_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

// ─── TVMaze (fill-nulls fallback for shows) ────────────────────────────────

/// Apply TVMaze data to a show row. Honors the "fill nulls only" merge
/// policy: every COALESCE keeps the existing column when present.
/// Locked fields skip entirely.
pub async fn apply_show_metadata_tvmaze(
    pool: &SqlitePool,
    item_id: i64,
    meta: &TvMazeShow,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", meta.title.clone());
    let sort_title = title.as_deref().map(make_sort_title);
    let summary = pick(&locked, "summary", meta.summary.clone()).flatten();
    let year = pick(&locked, "year", meta.year).flatten();
    let imdb_id = pick(&locked, "imdb_id", meta.imdb_id.clone()).flatten();
    let tvdb_id = pick(&locked, "tvdb_id", meta.tvdb_id).flatten();

    sqlx::query(
        "UPDATE items SET
            title = COALESCE(title, ?),
            sort_title = COALESCE(sort_title, ?),
            summary = COALESCE(summary, ?),
            year = COALESCE(year, ?),
            imdb_id = COALESCE(imdb_id, ?),
            tvdb_id = COALESCE(tvdb_id, ?),
            refreshed_at = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&title)
    .bind(&sort_title)
    .bind(&summary)
    .bind(year)
    .bind(&imdb_id)
    .bind(tvdb_id)
    .bind(now)
    .bind(now)
    .bind(item_id)
    .execute(pool)
    .await?;

    if !is_locked(&locked, "genres") {
        // Don't replace the existing genre set; just union any missing
        // ones. The genres tag join table makes this cheap.
        apply_genres_additive(pool, item_id, &meta.genres).await?;
    }
    if !is_locked(&locked, "poster") {
        if let Some(p) = &meta.poster_url {
            // Only insert if there isn't already a poster from a higher-
            // priority source (TMDB ingests with source='tmdb').
            store_image_if_missing(pool, Some(item_id), None, "poster", "tvmaze", p).await?;
        }
    }
    if !is_locked(&locked, "backdrop") {
        if let Some(p) = &meta.backdrop_url {
            store_image_if_missing(pool, Some(item_id), None, "backdrop", "tvmaze", p)
                .await?;
        }
    }

    Ok(())
}

/// Apply AniList show metadata as the **primary** source for an anime
/// item. Distinct from `apply_show_metadata_tvmaze`/`_tvdb` which only
/// fill nulls — here we overwrite null-or-stale columns with AniList's
/// canonical values for fields AniList owns (anilist_id, original_title,
/// summary, year, duration_ms), while still honoring per-item locks.
pub async fn apply_show_metadata_anilist(
    pool: &SqlitePool,
    item_id: i64,
    meta: &AniListShow,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", Some(meta.title.clone())).flatten();
    let sort_title = title.as_deref().map(make_sort_title);
    let original_title = pick(&locked, "original_title", meta.original_title.clone()).flatten();
    let summary = pick(&locked, "summary", meta.summary.clone()).flatten();
    let year = pick(&locked, "year", meta.year).flatten();
    let anilist_id = pick(&locked, "anilist_id", Some(meta.anilist_id)).flatten();
    // AniList exposes per-episode runtime in minutes; we store an item-
    // level duration as a rough hint for the UI when no media file has
    // been probed yet.
    let duration_ms = pick(
        &locked,
        "duration_ms",
        meta.episode_duration_minutes.map(|m| i64::from(m) * 60_000),
    )
    .flatten();

    sqlx::query(
        "UPDATE items SET
            title = COALESCE(?, title),
            sort_title = COALESCE(?, sort_title),
            original_title = COALESCE(?, original_title),
            summary = COALESCE(?, summary),
            year = COALESCE(?, year),
            duration_ms = COALESCE(?, duration_ms),
            anilist_id = COALESCE(?, anilist_id),
            refreshed_at = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&title)
    .bind(&sort_title)
    .bind(&original_title)
    .bind(&summary)
    .bind(year)
    .bind(duration_ms)
    .bind(anilist_id)
    .bind(now)
    .bind(now)
    .bind(item_id)
    .execute(pool)
    .await?;

    if !is_locked(&locked, "genres") {
        apply_genres_additive(pool, item_id, &meta.genres).await?;
    }
    if !is_locked(&locked, "poster") {
        if let Some(p) = &meta.poster_url {
            store_image_if_missing(pool, Some(item_id), None, "poster", "anilist", p).await?;
        }
    }
    if !is_locked(&locked, "backdrop") {
        if let Some(p) = &meta.backdrop_url {
            store_image_if_missing(pool, Some(item_id), None, "backdrop", "anilist", p).await?;
        }
    }
    Ok(())
}

/// Apply TVDB show metadata using the same "fill nulls only" merge
/// policy as TVMaze. Distinct query from `apply_show_metadata_tvmaze`
/// because TVDB has its own column projection (notably original_title
/// and the show's own tvdb_id is authoritative when set).
pub async fn apply_show_metadata_tvdb(
    pool: &SqlitePool,
    item_id: i64,
    meta: &TvdbShow,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", meta.title.clone());
    let sort_title = title.as_deref().map(make_sort_title);
    let original_title = pick(&locked, "original_title", meta.original_title.clone()).flatten();
    let summary = pick(&locked, "summary", meta.summary.clone()).flatten();
    let year = pick(&locked, "year", meta.year).flatten();
    let imdb_id = pick(&locked, "imdb_id", meta.imdb_id.clone()).flatten();
    let tvdb_id = pick(&locked, "tvdb_id", Some(meta.tvdb_id)).flatten();

    sqlx::query(
        "UPDATE items SET
            title = COALESCE(title, ?),
            sort_title = COALESCE(sort_title, ?),
            original_title = COALESCE(original_title, ?),
            summary = COALESCE(summary, ?),
            year = COALESCE(year, ?),
            imdb_id = COALESCE(imdb_id, ?),
            tvdb_id = COALESCE(tvdb_id, ?),
            refreshed_at = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&title)
    .bind(&sort_title)
    .bind(&original_title)
    .bind(&summary)
    .bind(year)
    .bind(&imdb_id)
    .bind(tvdb_id)
    .bind(now)
    .bind(now)
    .bind(item_id)
    .execute(pool)
    .await?;

    if !is_locked(&locked, "genres") {
        apply_genres_additive(pool, item_id, &meta.genres).await?;
    }
    if !is_locked(&locked, "poster") {
        if let Some(p) = &meta.poster_url {
            store_image_if_missing(pool, Some(item_id), None, "poster", "tvdb", p).await?;
        }
    }
    if !is_locked(&locked, "backdrop") {
        if let Some(p) = &meta.backdrop_url {
            store_image_if_missing(pool, Some(item_id), None, "backdrop", "tvdb", p).await?;
        }
    }
    Ok(())
}

pub async fn apply_movie_metadata_tvdb(
    pool: &SqlitePool,
    item_id: i64,
    meta: &TvdbMovie,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", meta.title.clone());
    let sort_title = title.as_deref().map(make_sort_title);
    let original_title = pick(&locked, "original_title", meta.original_title.clone()).flatten();
    let summary = pick(&locked, "summary", meta.summary.clone()).flatten();
    let year = pick(&locked, "year", meta.year).flatten();
    let imdb_id = pick(&locked, "imdb_id", meta.imdb_id.clone()).flatten();
    let tvdb_id = pick(&locked, "tvdb_id", Some(meta.tvdb_id)).flatten();
    // TVDB runtime is in minutes; the items schema stores duration in ms.
    let duration_ms = pick(
        &locked,
        "duration_ms",
        meta.runtime_minutes.map(|m| i64::from(m) * 60_000),
    )
    .flatten();

    sqlx::query(
        "UPDATE items SET
            title = COALESCE(title, ?),
            sort_title = COALESCE(sort_title, ?),
            original_title = COALESCE(original_title, ?),
            summary = COALESCE(summary, ?),
            year = COALESCE(year, ?),
            duration_ms = COALESCE(duration_ms, ?),
            imdb_id = COALESCE(imdb_id, ?),
            tvdb_id = COALESCE(tvdb_id, ?),
            refreshed_at = ?,
            updated_at = ?
         WHERE id = ?",
    )
    .bind(&title)
    .bind(&sort_title)
    .bind(&original_title)
    .bind(&summary)
    .bind(year)
    .bind(duration_ms)
    .bind(&imdb_id)
    .bind(tvdb_id)
    .bind(now)
    .bind(now)
    .bind(item_id)
    .execute(pool)
    .await?;

    if !is_locked(&locked, "genres") {
        apply_genres_additive(pool, item_id, &meta.genres).await?;
    }
    if !is_locked(&locked, "poster") {
        if let Some(p) = &meta.poster_url {
            store_image_if_missing(pool, Some(item_id), None, "poster", "tvdb", p).await?;
        }
    }
    if !is_locked(&locked, "backdrop") {
        if let Some(p) = &meta.backdrop_url {
            store_image_if_missing(pool, Some(item_id), None, "backdrop", "tvdb", p).await?;
        }
    }
    Ok(())
}

async fn apply_genres_additive(
    pool: &SqlitePool,
    item_id: i64,
    genres: &[String],
) -> Result<()> {
    for g in genres {
        let row = sqlx::query(
            "INSERT INTO genres (name) VALUES (?)
             ON CONFLICT(name) DO UPDATE SET name = excluded.name
             RETURNING id",
        )
        .bind(g)
        .fetch_one(pool)
        .await?;
        let gid: i64 = row.try_get("id")?;
        sqlx::query("INSERT OR IGNORE INTO item_genres (item_id, genre_id) VALUES (?, ?)")
            .bind(item_id)
            .bind(gid)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn store_image_if_missing(
    pool: &SqlitePool,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    kind: &str,
    source: &str,
    url: &str,
) -> Result<()> {
    // Only insert if no image of this kind already exists for this item.
    let exists = sqlx::query(
        "SELECT 1 FROM images
         WHERE (item_id IS ? AND ? IS NOT NULL OR episode_id IS ? AND ? IS NOT NULL)
           AND kind = ?
         LIMIT 1",
    )
    .bind(item_id)
    .bind(item_id)
    .bind(episode_id)
    .bind(episode_id)
    .bind(kind)
    .fetch_optional(pool)
    .await?;
    if exists.is_some() {
        return Ok(());
    }
    store_image(pool, item_id, episode_id, kind, source, url).await?;
    Ok(())
}

// ─── People / credits / extras (TMDB ingestion side) ───────────────────────

/// Upsert a person by their TMDB id. Returns the local person id so the
/// caller can wire credits to it. Skipped fields stay NULL — we don't have
/// per-person locks (yet) since the UI doesn't surface them.
async fn upsert_person_by_tmdb(
    pool: &SqlitePool,
    tmdb_id: i64,
    name: &str,
    profile_path: Option<&str>,
    known_for_department: Option<&str>,
) -> Result<i64> {
    let profile_url = profile_path.map(|p| tmdb_image_url(p, "w185"));
    let row = sqlx::query(
        "INSERT INTO people (name, tmdb_id, photo_url, known_for_department)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(tmdb_id) DO UPDATE SET
            name = excluded.name,
            photo_url = COALESCE(excluded.photo_url, people.photo_url),
            known_for_department = COALESCE(excluded.known_for_department, people.known_for_department)
         RETURNING id",
    )
    .bind(name)
    .bind(tmdb_id)
    .bind(&profile_url)
    .bind(known_for_department)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

/// Replace all credits for an item. Locked `credits` field skips the
/// whole operation so user-curated cast lists survive re-enrichment.
pub async fn apply_item_credits(
    pool: &SqlitePool,
    item_id: i64,
    credits: &TmdbCredits,
) -> Result<()> {
    let locked = fetch_locked_fields(pool, item_id).await?;
    if is_locked(&locked, "credits") {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM item_credits WHERE item_id = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    for member in &credits.cast {
        insert_credit_cast(pool, item_id, member).await?;
    }
    for (idx, member) in credits.crew.iter().enumerate() {
        insert_credit_crew(pool, item_id, member, idx as i64).await?;
    }
    Ok(())
}

async fn insert_credit_cast(
    pool: &SqlitePool,
    item_id: i64,
    m: &TmdbCastMember,
) -> Result<()> {
    let person_id = upsert_person_by_tmdb(
        pool,
        m.tmdb_person_id,
        &m.name,
        m.profile_path.as_deref(),
        Some("Acting"),
    )
    .await?;
    sqlx::query(
        "INSERT INTO item_credits
            (item_id, person_id, role_kind, role, character_name, sort_order)
         VALUES (?, ?, 'cast', 'Actor', ?, ?)",
    )
    .bind(item_id)
    .bind(person_id)
    .bind(m.character.as_deref())
    .bind(m.order as i64)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_credit_crew(
    pool: &SqlitePool,
    item_id: i64,
    m: &TmdbCrewMember,
    sort_order: i64,
) -> Result<()> {
    let person_id = upsert_person_by_tmdb(
        pool,
        m.tmdb_person_id,
        &m.name,
        m.profile_path.as_deref(),
        Some(&m.department),
    )
    .await?;
    let role_kind = match m.job.as_str() {
        "Director" => "director",
        "Writer" | "Screenplay" => "writer",
        "Producer" | "Executive Producer" => "producer",
        _ => "crew",
    };
    sqlx::query(
        "INSERT INTO item_credits
            (item_id, person_id, role_kind, role, character_name, sort_order)
         VALUES (?, ?, ?, ?, NULL, ?)",
    )
    .bind(item_id)
    .bind(person_id)
    .bind(role_kind)
    .bind(&m.job)
    .bind(sort_order)
    .execute(pool)
    .await?;
    Ok(())
}

/// Replace the entire cast/crew list for an item from a user edit (the
/// "Cast & Crew" tab in Edit Metadata). Always locks the `credits` field
/// so the next metadata refresh won't undo the user's curation, even if
/// the list is empty.
///
/// Each input row may reference an existing `person_id` (e.g. when the
/// user just reordered or renamed an existing role) or come with `name`
/// only (we insert a fresh `people` row for those). We don't try to
/// dedupe by name — the user is in control here.
pub async fn replace_item_credits(
    pool: &SqlitePool,
    item_id: i64,
    edits: &[crate::models::CreditEditInput],
) -> Result<()> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM item_credits WHERE item_id = ?")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;

    for edit in edits {
        let trimmed_name = edit.name.trim();
        if trimmed_name.is_empty() {
            continue;
        }
        let role_kind = match edit.role_kind.as_str() {
            "cast" | "director" | "writer" | "producer" | "crew" => edit.role_kind.clone(),
            _ => "crew".to_string(),
        };
        let person_id = match edit.person_id {
            Some(id) => id,
            None => {
                let row = sqlx::query(
                    "INSERT INTO people (name, photo_url) VALUES (?, ?) RETURNING id",
                )
                .bind(trimmed_name)
                .bind(edit.photo_url.as_deref())
                .fetch_one(&mut *tx)
                .await?;
                row.try_get("id")?
            }
        };
        sqlx::query(
            "INSERT INTO item_credits
                (item_id, person_id, role_kind, role, character_name, sort_order)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(item_id)
        .bind(person_id)
        .bind(&role_kind)
        .bind(edit.role.trim())
        .bind(edit.character_name.as_deref())
        .bind(edit.sort_order)
        .execute(&mut *tx)
        .await?;
    }

    // Lock the `credits` field so future re-enrichment won't overwrite the
    // user's curation.
    let mut locked = {
        let row = sqlx::query("SELECT locked_fields FROM items WHERE id = ?")
            .bind(item_id)
            .fetch_one(&mut *tx)
            .await?;
        let raw: String = row.try_get("locked_fields").unwrap_or_default();
        serde_json::from_str::<Vec<String>>(&raw).unwrap_or_default()
    };
    if !locked.iter().any(|f| f == "credits") {
        locked.push("credits".to_string());
    }
    let serialized = serde_json::to_string(&locked).unwrap_or_else(|_| "[]".into());
    sqlx::query("UPDATE items SET locked_fields = ?, updated_at = ? WHERE id = ?")
        .bind(serialized)
        .bind(now)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

/// Replace all extras for an item. Locked `extras` field skips the
/// operation so user-curated extras survive re-enrichment.
pub async fn apply_item_extras(
    pool: &SqlitePool,
    item_id: i64,
    videos: &[TmdbVideo],
) -> Result<()> {
    let locked = fetch_locked_fields(pool, item_id).await?;
    if is_locked(&locked, "extras") {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM item_extras WHERE item_id = ? AND source = 'youtube'")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    for (idx, v) in videos.iter().enumerate() {
        let kind = match v.kind.as_str() {
            "Trailer" => "trailer",
            "Teaser" => "teaser",
            "Featurette" => "featurette",
            "Behind the Scenes" => "behind_the_scenes",
            "Clip" => "clip",
            _ => "clip",
        };
        let thumb = format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", v.key);
        let published_ms = v.published_at.as_deref().and_then(parse_iso8601_ms);
        sqlx::query(
            "INSERT OR IGNORE INTO item_extras
                (item_id, kind, title, source, source_id, thumb_url, published_at, sort_order)
             VALUES (?, ?, ?, 'youtube', ?, ?, ?, ?)",
        )
        .bind(item_id)
        .bind(kind)
        // Some videos come with empty names. Fall back to the kind string.
        .bind(if v.name.is_empty() {
            v.kind.clone()
        } else {
            v.name.clone()
        })
        .bind(&v.key)
        .bind(&thumb)
        .bind(published_ms)
        .bind(idx as i64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Parse ISO 8601 timestamps (TMDB's `published_at` is e.g.
/// `2024-03-15T12:00:00.000Z`) to epoch ms. Returns None on any parse
/// failure so callers just skip the field.
// ─── Edit metadata & reviews ───────────────────────────────────────────────

/// Apply a user-issued Edit Metadata patch. Each field with `Some(_)` is
/// written and added to `locked_fields` so subsequent re-enrichment leaves
/// it alone. Fields in `edit.unlock` are removed from `locked_fields`.
pub async fn apply_item_edit(pool: &SqlitePool, item_id: i64, edit: &ItemEdit) -> Result<()> {
    let now = now_ms();
    let mut locked = fetch_locked_fields(pool, item_id).await?;

    // Helper: write a column if the patch includes it, and add the field
    // name to `locked` so future enrichment skips it.
    macro_rules! apply {
        ($column:literal, $field:literal, $value:expr) => {{
            if let Some(v) = $value.clone() {
                sqlx::query(&format!(
                    "UPDATE items SET {col} = ?, updated_at = ? WHERE id = ?",
                    col = $column
                ))
                .bind(v)
                .bind(now)
                .bind(item_id)
                .execute(pool)
                .await?;
                if !locked.iter().any(|s| s == $field) {
                    locked.push($field.to_string());
                }
            }
        }};
    }
    apply!("title", "title", edit.title);
    if let Some(t) = edit.title.as_deref() {
        sqlx::query("UPDATE items SET sort_title = ?, updated_at = ? WHERE id = ?")
            .bind(make_sort_title(t))
            .bind(now)
            .bind(item_id)
            .execute(pool)
            .await?;
        if !locked.iter().any(|s| s == "sort_title") {
            locked.push("sort_title".to_string());
        }
    }
    if let Some(t) = edit.sort_title.as_deref() {
        sqlx::query("UPDATE items SET sort_title = ?, updated_at = ? WHERE id = ?")
            .bind(t)
            .bind(now)
            .bind(item_id)
            .execute(pool)
            .await?;
        if !locked.iter().any(|s| s == "sort_title") {
            locked.push("sort_title".to_string());
        }
    }
    apply!("original_title", "original_title", edit.original_title);
    apply!("summary", "summary", edit.summary);
    apply!("tagline", "tagline", edit.tagline);
    apply!("year", "year", edit.year);
    apply!("rating_age", "rating_age", edit.rating_age);
    apply!("rating_audience", "rating_audience", edit.rating_audience);

    // Unlock requested fields.
    locked.retain(|f| !edit.unlock.iter().any(|u| u == f));

    let serialized = serde_json::to_string(&locked).unwrap_or_else(|_| "[]".into());
    sqlx::query("UPDATE items SET locked_fields = ?, updated_at = ? WHERE id = ?")
        .bind(serialized)
        .bind(now)
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Explicit user toggle for watched state. When `watched` is true we
/// also bump `view_count` and set position to duration (so the rail
/// shows a full progress bar); when false we reset to position 0 and
/// keep view_count untouched.
pub async fn set_watched(
    pool: &SqlitePool,
    user_id: i64,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    watched: bool,
) -> Result<()> {
    if item_id.is_some() == episode_id.is_some() {
        anyhow::bail!("exactly one of item_id / episode_id must be set");
    }
    let now = now_ms();
    // Fetch a duration to write into position_ms when marking watched, so
    // the resume rail UI doesn't show an item with no progress bar.
    let duration_ms: Option<i64> = if let Some(id) = item_id {
        sqlx::query("SELECT duration_ms FROM items WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .and_then(|r| r.try_get::<Option<i64>, _>("duration_ms").ok().flatten())
    } else if let Some(id) = episode_id {
        sqlx::query("SELECT duration_ms FROM episodes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?
            .and_then(|r| r.try_get::<Option<i64>, _>("duration_ms").ok().flatten())
    } else {
        None
    };
    let position_ms = if watched { duration_ms.unwrap_or(0) } else { 0 };
    let view_count_delta: i64 = if watched { 1 } else { 0 };

    // Upsert the play_state row. Two indexes (one for item, one for
    // episode) handle the ON CONFLICT lookup; we issue the right one
    // based on which id was provided.
    if let Some(id) = item_id {
        sqlx::query(
            "INSERT INTO play_state
                (user_id, item_id, position_ms, duration_ms, watched, view_count, last_played_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(user_id, item_id) WHERE item_id IS NOT NULL DO UPDATE SET
                position_ms = excluded.position_ms,
                duration_ms = COALESCE(excluded.duration_ms, play_state.duration_ms),
                watched = excluded.watched,
                view_count = play_state.view_count + ?,
                last_played_at = excluded.last_played_at",
        )
        .bind(user_id)
        .bind(id)
        .bind(position_ms)
        .bind(duration_ms)
        .bind(watched as i64)
        .bind(view_count_delta)
        .bind(now)
        .bind(view_count_delta)
        .execute(pool)
        .await?;
    } else if let Some(id) = episode_id {
        sqlx::query(
            "INSERT INTO play_state
                (user_id, episode_id, position_ms, duration_ms, watched, view_count, last_played_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(user_id, episode_id) WHERE episode_id IS NOT NULL DO UPDATE SET
                position_ms = excluded.position_ms,
                duration_ms = COALESCE(excluded.duration_ms, play_state.duration_ms),
                watched = excluded.watched,
                view_count = play_state.view_count + ?,
                last_played_at = excluded.last_played_at",
        )
        .bind(user_id)
        .bind(id)
        .bind(position_ms)
        .bind(duration_ms)
        .bind(watched as i64)
        .bind(view_count_delta)
        .bind(now)
        .bind(view_count_delta)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn list_reviews_for_item(pool: &SqlitePool, item_id: i64) -> Result<Vec<Review>> {
    let rows = sqlx::query(
        "SELECT id, item_id, source, author, author_url, avatar_url,
                rating, body, created_at
         FROM item_reviews
         WHERE item_id = ?
         ORDER BY (rating IS NULL) ASC, rating DESC, created_at DESC",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            Ok(Review {
                id: r.try_get("id")?,
                item_id: r.try_get("item_id")?,
                source: r.try_get("source")?,
                author: r.try_get("author")?,
                author_url: r.try_get("author_url")?,
                avatar_url: r.try_get("avatar_url")?,
                rating: r.try_get("rating")?,
                body: r.try_get("body")?,
                created_at: r.try_get("created_at")?,
            })
        })
        .collect()
}

/// Replace TMDB-sourced reviews for an item. Other-source reviews (when we
/// add them) survive — the WHERE clause scopes the delete to source='tmdb'.
pub async fn apply_tmdb_reviews(
    pool: &SqlitePool,
    item_id: i64,
    reviews: &[chimpflix_metadata::TmdbReview],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM item_reviews WHERE item_id = ? AND source = 'tmdb'")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    for r in reviews {
        sqlx::query(
            "INSERT OR IGNORE INTO item_reviews
                (item_id, source, source_id, author, author_url, avatar_url,
                 rating, body, created_at)
             VALUES (?, 'tmdb', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item_id)
        .bind(&r.source_id)
        .bind(&r.author)
        .bind(r.author_url.as_deref())
        .bind(r.avatar_url.as_deref())
        .bind(r.rating)
        .bind(r.body.as_deref())
        .bind(r.created_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn parse_iso8601_ms(s: &str) -> Option<i64> {
    // Minimal parse: split on 'T', then 'Z'/'+', take date YYYY-MM-DD and
    // time HH:MM:SS. We don't depend on chrono since the metadata crate
    // already keeps its surface tiny.
    let (date, rest) = s.split_once('T')?;
    let time = rest.split(['Z', '+', '.']).next()?;
    let mut date_parts = date.split('-');
    let y: i32 = date_parts.next()?.parse().ok()?;
    let m: u32 = date_parts.next()?.parse().ok()?;
    let d: u32 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hh: u32 = time_parts.next()?.parse().ok()?;
    let mm: u32 = time_parts.next()?.parse().ok()?;
    let ss: u32 = time_parts.next().unwrap_or("0").parse().ok()?;
    // Days since 1970-01-01 (proleptic Gregorian, sane only for 1970+).
    fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
        let y = if m <= 2 { y - 1 } else { y } as i64;
        let m = m as i64;
        let d = d as i64;
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146097 + doe - 719468
    }
    let days = days_from_civil(y, m, d);
    Some(days * 86_400_000 + hh as i64 * 3_600_000 + mm as i64 * 60_000 + ss as i64 * 1_000)
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

/// Public version of `store_image` for handlers that need to replace a
/// poster/backdrop with user-supplied artwork. Sets `is_primary = 1` and
/// adds the corresponding field name to `locked_fields` so the metadata
/// pipeline won't overwrite the user's choice on the next refresh.
pub async fn replace_primary_image(
    pool: &SqlitePool,
    item_id: i64,
    kind: &str,
    source: &str,
    url: &str,
) -> Result<()> {
    store_image(pool, Some(item_id), None, kind, source, url).await?;
    // Lock the corresponding field so re-enrichment leaves it alone.
    let lock_field = match kind {
        "poster" => "poster",
        "backdrop" => "backdrop",
        other => other,
    };
    let mut locked = fetch_locked_fields(pool, item_id).await?;
    if !locked.iter().any(|f| f == lock_field) {
        locked.push(lock_field.to_string());
        let json = serde_json::to_string(&locked)?;
        sqlx::query("UPDATE items SET locked_fields = ?, updated_at = ? WHERE id = ?")
            .bind(json)
            .bind(now_ms())
            .bind(item_id)
            .execute(pool)
            .await?;
    }
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

// ---------------------------------------------------------------------------
// Dashboard aggregations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct LibraryStats {
    pub library_id: i64,
    pub name: String,
    pub kind: String,
    pub item_count: i64,
    pub file_count: i64,
    pub total_bytes: i64,
}

/// Per-library aggregate counts and on-disk size. One row per library;
/// counts join items + media_files (one file per row, summed).
pub async fn library_stats(pool: &SqlitePool) -> Result<Vec<LibraryStats>> {
    // Three small queries instead of one fragile outer-join with COALESCE
    // semantics. SQLite is local; the round-trip cost is negligible vs the
    // readability win.
    let libraries: Vec<(i64, String, String)> =
        sqlx::query("SELECT id, name, kind FROM libraries ORDER BY name")
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(|r| {
                (
                    r.try_get::<i64, _>("id").unwrap_or(0),
                    r.try_get::<String, _>("name").unwrap_or_default(),
                    r.try_get::<String, _>("kind").unwrap_or_default(),
                )
            })
            .collect();

    let mut out = Vec::with_capacity(libraries.len());
    for (id, name, kind) in libraries {
        let item_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE library_id = ?")
                .bind(id)
                .fetch_one(pool)
                .await
                .unwrap_or(0);
        let row = sqlx::query(
            "SELECT COUNT(*) AS c, COALESCE(SUM(size_bytes), 0) AS b
             FROM media_files mf
             JOIN items i ON i.id = mf.item_id
             WHERE i.library_id = ?
             UNION ALL
             SELECT COUNT(*) AS c, COALESCE(SUM(size_bytes), 0) AS b
             FROM media_files mf
             JOIN episodes e ON e.id = mf.episode_id
             JOIN seasons s  ON s.id = e.season_id
             JOIN items   i  ON i.id = s.show_id
             WHERE i.library_id = ?",
        )
        .bind(id)
        .bind(id)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
        let file_count: i64 = row.iter().map(|r| r.try_get::<i64, _>("c").unwrap_or(0)).sum();
        let total_bytes: i64 = row.iter().map(|r| r.try_get::<i64, _>("b").unwrap_or(0)).sum();

        out.push(LibraryStats {
            library_id: id,
            name,
            kind,
            item_count,
            file_count,
            total_bytes,
        });
    }
    Ok(out)
}

/// Recent scan jobs across all libraries. Order: newest first by `created_at`.
pub async fn recent_scan_jobs(pool: &SqlitePool, limit: i64) -> Result<Vec<ScanJob>> {
    let limit = limit.clamp(1, 200);
    let rows = sqlx::query(
        "SELECT * FROM scan_jobs ORDER BY created_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(ScanJob::from_row).collect()
}

// ---------------------------------------------------------------------------
// Server settings (singleton row) + audit log
// ---------------------------------------------------------------------------

pub async fn get_server_settings(pool: &SqlitePool) -> Result<ServerSettings> {
    let row = sqlx::query("SELECT * FROM server_settings WHERE id = 1")
        .fetch_one(pool)
        .await
        .context("fetch server_settings")?;
    ServerSettings::from_row(&row)
}

pub async fn update_server_settings(
    pool: &SqlitePool,
    actor_user_id: Option<i64>,
    patch: ServerSettingsUpdate,
) -> Result<ServerSettings> {
    let mut tx = pool.begin().await?;

    if let Some(v) = patch.server_name {
        sqlx::query("UPDATE server_settings SET server_name = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.public_url {
        sqlx::query("UPDATE server_settings SET public_url = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.cors_origins {
        sqlx::query("UPDATE server_settings SET cors_origins = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.secure_connections {
        sqlx::query("UPDATE server_settings SET secure_connections = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.telemetry_opt_in {
        sqlx::query("UPDATE server_settings SET telemetry_opt_in = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_max_concurrent {
        sqlx::query("UPDATE server_settings SET transcoder_max_concurrent = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_hw_accel {
        sqlx::query("UPDATE server_settings SET transcoder_hw_accel = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_quality_ceiling_kbps {
        sqlx::query("UPDATE server_settings SET transcoder_quality_ceiling_kbps = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_encoder_preset {
        sqlx::query("UPDATE server_settings SET transcoder_encoder_preset = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_hw_strictness {
        sqlx::query("UPDATE server_settings SET transcoder_hw_strictness = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_background_preset {
        sqlx::query("UPDATE server_settings SET transcoder_background_preset = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_max_background_concurrent {
        sqlx::query(
            "UPDATE server_settings SET transcoder_max_background_concurrent = ? WHERE id = 1",
        )
        .bind(v)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(v) = patch.transcoder_hdr_tonemap_enabled {
        sqlx::query(
            "UPDATE server_settings SET transcoder_hdr_tonemap_enabled = ? WHERE id = 1",
        )
        .bind(i64::from(v))
        .execute(&mut *tx)
        .await?;
    }
    if let Some(v) = patch.transcoder_hdr_tonemap_algo {
        sqlx::query("UPDATE server_settings SET transcoder_hdr_tonemap_algo = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.email_smtp_host {
        sqlx::query("UPDATE server_settings SET email_smtp_host = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.email_smtp_port {
        sqlx::query("UPDATE server_settings SET email_smtp_port = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.email_smtp_username {
        sqlx::query("UPDATE server_settings SET email_smtp_username = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.email_smtp_security {
        sqlx::query("UPDATE server_settings SET email_smtp_security = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.email_from_address {
        sqlx::query("UPDATE server_settings SET email_from_address = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.email_from_name {
        sqlx::query("UPDATE server_settings SET email_from_name = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.totp_enforcement {
        sqlx::query("UPDATE server_settings SET totp_enforcement = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.maintenance_window_start {
        sqlx::query("UPDATE server_settings SET maintenance_window_start = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.maintenance_window_end {
        sqlx::query("UPDATE server_settings SET maintenance_window_end = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.scan_automatically {
        sqlx::query("UPDATE server_settings SET scan_automatically = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.detect_markers_on_add {
        sqlx::query("UPDATE server_settings SET detect_markers_on_add = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.continue_watching_max_items {
        sqlx::query("UPDATE server_settings SET continue_watching_max_items = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.continue_watching_max_age_weeks {
        sqlx::query(
            "UPDATE server_settings SET continue_watching_max_age_weeks = ? WHERE id = 1",
        )
        .bind(v)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(v) = patch.continue_watching_include_premieres {
        sqlx::query(
            "UPDATE server_settings SET continue_watching_include_premieres = ? WHERE id = 1",
        )
        .bind(i64::from(v))
        .execute(&mut *tx)
        .await?;
    }
    if let Some(v) = patch.video_completion_behaviour {
        if !matches!(
            v.as_str(),
            "threshold_pct" | "first_credits_marker" | "earliest_of_both"
        ) {
            anyhow::bail!(
                "video_completion_behaviour must be threshold_pct / first_credits_marker / earliest_of_both"
            );
        }
        sqlx::query("UPDATE server_settings SET video_completion_behaviour = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.video_played_threshold_pct {
        sqlx::query("UPDATE server_settings SET video_played_threshold_pct = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.database_cache_size_mb {
        sqlx::query("UPDATE server_settings SET database_cache_size_mb = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.audio_normalize_enabled {
        sqlx::query("UPDATE server_settings SET audio_normalize_enabled = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.scanner_nice_level {
        sqlx::query("UPDATE server_settings SET scanner_nice_level = ? WHERE id = 1")
            .bind(v.clamp(0, 19))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.preroll_path {
        sqlx::query("UPDATE server_settings SET preroll_path = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.preroll_enabled {
        sqlx::query("UPDATE server_settings SET preroll_enabled = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_hevc_encoding_mode {
        if !matches!(v.as_str(), "off" | "when_client_supports" | "always") {
            anyhow::bail!(
                "transcoder_hevc_encoding_mode must be off / when_client_supports / always"
            );
        }
        sqlx::query("UPDATE server_settings SET transcoder_hevc_encoding_mode = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_gpu_device {
        // Whitelist the format. "auto" is the sentinel; numeric
        // indexes are accepted for NVENC; render device paths are
        // accepted for VAAPI. Anything else is a typo or an attempt
        // to inject arbitrary args.
        let valid = v == "auto"
            || v.chars().all(|c| c.is_ascii_digit())
            || (v.starts_with("/dev/dri/renderD")
                && v["/dev/dri/renderD".len()..]
                    .chars()
                    .all(|c| c.is_ascii_digit()));
        if !valid {
            anyhow::bail!(
                "transcoder_gpu_device must be 'auto', a numeric index, or '/dev/dri/renderD<N>'"
            );
        }
        sqlx::query("UPDATE server_settings SET transcoder_gpu_device = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_max_cpu_concurrent {
        sqlx::query("UPDATE server_settings SET transcoder_max_cpu_concurrent = ? WHERE id = 1")
            .bind(v.clamp(1, 16))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_reaper_idle_threshold_ms {
        sqlx::query(
            "UPDATE server_settings SET transcoder_reaper_idle_threshold_ms = ? WHERE id = 1",
        )
        .bind(v)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(v) = patch.max_remote_streams_per_user {
        sqlx::query("UPDATE server_settings SET max_remote_streams_per_user = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.lan_networks {
        sqlx::query("UPDATE server_settings SET lan_networks = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.auth_bypass_cidrs {
        sqlx::query("UPDATE server_settings SET auth_bypass_cidrs = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.bind_interface {
        // Sanity-check the format if non-empty so we don't store
        // garbage that'll fail to parse at next startup.
        let trimmed = v.trim();
        if !trimmed.is_empty() && trimmed.parse::<std::net::SocketAddr>().is_err() {
            anyhow::bail!(
                "bind_interface must be empty or a SocketAddr (e.g. 192.168.1.50:8080)"
            );
        }
        sqlx::query("UPDATE server_settings SET bind_interface = ? WHERE id = 1")
            .bind(trimmed)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.metadata_language {
        // Lightweight BCP-47 sanity: length-bound + alphanumerics/dashes
        // only. We don't try to validate against a full registry — TMDB
        // will silently return original-language fallbacks for tags it
        // doesn't recognise, which is the right failure mode (no error,
        // just no localisation).
        let trimmed = v.trim();
        if trimmed.is_empty() || trimmed.len() > 12 {
            anyhow::bail!(
                "metadata_language must be a 1–12 char BCP-47 tag (e.g. en-US, ja-JP)"
            );
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            anyhow::bail!(
                "metadata_language must contain only ASCII letters, digits, and dashes"
            );
        }
        sqlx::query("UPDATE server_settings SET metadata_language = ? WHERE id = 1")
            .bind(trimmed)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.recently_added_days {
        if !(0..=365).contains(&v) {
            anyhow::bail!(
                "recently_added_days must be between 0 and 365 (0 disables the badge)"
            );
        }
        sqlx::query("UPDATE server_settings SET recently_added_days = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.extras_json {
        sqlx::query("UPDATE server_settings SET extras_json = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }

    sqlx::query("UPDATE server_settings SET updated_at = ?, updated_by = ? WHERE id = 1")
        .bind(now_ms())
        .bind(actor_user_id)
        .execute(&mut *tx)
        .await?;

    let row = sqlx::query("SELECT * FROM server_settings WHERE id = 1")
        .fetch_one(&mut *tx)
        .await?;
    let settings = ServerSettings::from_row(&row)?;

    tx.commit().await?;
    Ok(settings)
}

pub async fn append_audit(pool: &SqlitePool, entry: NewAuditEntry) -> Result<i64> {
    let res = sqlx::query(
        "INSERT INTO audit_log
            (actor_user_id, action, target_kind, target_id, payload_json, ip, user_agent, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(entry.actor_user_id)
    .bind(&entry.action)
    .bind(entry.target_kind.as_deref())
    .bind(entry.target_id.as_deref())
    .bind(entry.payload_json.as_deref())
    .bind(entry.ip.as_deref())
    .bind(entry.user_agent.as_deref())
    .bind(now_ms())
    .execute(pool)
    .await
    .context("insert audit_log")?;
    Ok(res.last_insert_rowid())
}

pub async fn list_audit(
    pool: &SqlitePool,
    before_id: Option<i64>,
    limit: i64,
) -> Result<Vec<AuditLogEntry>> {
    // Cursor pagination by descending id. `before_id` is exclusive.
    let limit = limit.clamp(1, 200);
    let rows = if let Some(before) = before_id {
        sqlx::query(
            "SELECT * FROM audit_log WHERE id < ? ORDER BY id DESC LIMIT ?",
        )
        .bind(before)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query("SELECT * FROM audit_log ORDER BY id DESC LIMIT ?")
            .bind(limit)
            .fetch_all(pool)
            .await?
    };
    rows.iter().map(AuditLogEntry::from_row).collect()
}

// ---------------------------------------------------------------------------
// Optimized versions (Phase 9)
// ---------------------------------------------------------------------------

pub async fn list_optimized_versions(pool: &SqlitePool) -> Result<Vec<OptimizedVersion>> {
    let rows = sqlx::query("SELECT * FROM optimized_versions ORDER BY created_at DESC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(OptimizedVersion::from_row).collect()
}

pub async fn list_optimized_for_item(
    pool: &SqlitePool,
    item_id: i64,
) -> Result<Vec<OptimizedVersion>> {
    let rows = sqlx::query(
        "SELECT o.*
         FROM optimized_versions o
         JOIN media_files mf ON mf.id = o.source_file_id
         WHERE mf.item_id = ? OR mf.episode_id IN (
            SELECT id FROM episodes e
            JOIN seasons s ON s.id = e.season_id
            WHERE s.show_id = ?
         )",
    )
    .bind(item_id)
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(OptimizedVersion::from_row).collect()
}

pub async fn enqueue_optimized_version(
    pool: &SqlitePool,
    input: NewOptimizedVersion,
) -> Result<OptimizedVersion> {
    let now = now_ms();
    // The output_path is filled in by the worker when status flips to
    // 'running'. We seed it as empty so the schema's NOT NULL is satisfied.
    let res = sqlx::query(
        "INSERT INTO optimized_versions
            (source_file_id, preset_id, output_path, status, created_at)
         VALUES (?, ?, '', 'queued', ?)
         ON CONFLICT(source_file_id, preset_id) DO UPDATE
            SET status = 'queued', error = NULL, completed_at = NULL
         RETURNING id",
    )
    .bind(input.source_file_id)
    .bind(input.preset_id)
    .bind(now)
    .fetch_one(pool)
    .await?;
    let id: i64 = res.try_get("id")?;
    let row = sqlx::query("SELECT * FROM optimized_versions WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;
    OptimizedVersion::from_row(&row)
}

pub async fn claim_queued_optimized(
    pool: &SqlitePool,
    limit: i64,
) -> Result<Vec<OptimizedVersion>> {
    let rows = sqlx::query(
        "SELECT * FROM optimized_versions WHERE status = 'queued' ORDER BY created_at ASC LIMIT ?",
    )
    .bind(limit.clamp(1, 16))
    .fetch_all(pool)
    .await?;
    rows.iter().map(OptimizedVersion::from_row).collect()
}

pub async fn mark_optimized_running(
    pool: &SqlitePool,
    id: i64,
    output_path: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE optimized_versions SET status = 'running', output_path = ? WHERE id = ?",
    )
    .bind(output_path)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_optimized_finished(
    pool: &SqlitePool,
    id: i64,
    success: bool,
    output_size_bytes: Option<i64>,
    duration_ms: Option<i64>,
    error: Option<&str>,
) -> Result<()> {
    let now = now_ms();
    let status = if success { "success" } else { "failed" };
    sqlx::query(
        "UPDATE optimized_versions
         SET status = ?,
             output_size_bytes = ?,
             duration_ms = ?,
             error = ?,
             completed_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(output_size_bytes)
    .bind(duration_ms)
    .bind(error)
    .bind(now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_optimized_version(pool: &SqlitePool, id: i64) -> Result<Option<String>> {
    // Return the path so the caller can unlink the file.
    let row = sqlx::query("SELECT output_path FROM optimized_versions WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let path: Option<String> = row.and_then(|r| r.try_get::<String, _>("output_path").ok());
    sqlx::query("DELETE FROM optimized_versions WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Webhooks
// ---------------------------------------------------------------------------

pub async fn list_webhooks(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
) -> Result<Vec<Webhook>> {
    let rows = sqlx::query("SELECT * FROM webhooks ORDER BY id ASC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(|row| Webhook::from_row(row, vault)).collect()
}

pub async fn get_webhook(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
    id: i64,
) -> Result<Option<Webhook>> {
    let row = sqlx::query("SELECT * FROM webhooks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.as_ref()
        .map(|row| Webhook::from_row(row, vault))
        .transpose()
}

pub async fn create_webhook(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
    input: NewWebhook,
) -> Result<Webhook> {
    let now = now_ms();
    let mask = serde_json::to_string(&input.event_mask)?;
    let encrypted = input
        .secret
        .as_deref()
        .map(|s| vault.encrypt_str(s))
        .transpose()?;
    let (secret_enc, secret_nonce) = match encrypted {
        Some(blob) => (Some(blob.value), blob.nonce),
        None => (None, None),
    };
    let id: i64 = sqlx::query(
        "INSERT INTO webhooks
         (name, url, secret_enc, secret_nonce, event_mask, enabled, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&input.name)
    .bind(&input.url)
    .bind(secret_enc)
    .bind(secret_nonce)
    .bind(&mask)
    .bind(i64::from(input.enabled))
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?
    .try_get("id")?;
    get_webhook(pool, vault, id)
        .await?
        .context("inserted webhook disappeared")
}

pub async fn update_webhook(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
    id: i64,
    update: WebhookUpdate,
) -> Result<Option<Webhook>> {
    let mut tx = pool.begin().await?;
    let now = now_ms();
    if let Some(v) = &update.name {
        sqlx::query("UPDATE webhooks SET name = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = &update.url {
        sqlx::query("UPDATE webhooks SET url = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = &update.secret {
        // `Option<Option<String>>` semantics: outer Some = "we're touching
        // this field", inner None = "clear it". Encrypt the new value when
        // present; either way, clear the legacy plaintext column so a
        // half-migrated row doesn't keep an old plaintext copy around.
        let (enc, nonce) = match v {
            Some(s) => {
                let blob = vault.encrypt_str(s)?;
                (Some(blob.value), blob.nonce)
            }
            None => (None, None),
        };
        sqlx::query(
            "UPDATE webhooks
             SET secret_enc = ?, secret_nonce = ?, secret = NULL, updated_at = ?
             WHERE id = ?",
        )
        .bind(enc)
        .bind(nonce)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(v) = &update.event_mask {
        let s = serde_json::to_string(v)?;
        sqlx::query("UPDATE webhooks SET event_mask = ?, updated_at = ? WHERE id = ?")
            .bind(s)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = update.enabled {
        sqlx::query("UPDATE webhooks SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(v))
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    get_webhook(pool, vault, id).await
}

pub async fn delete_webhook(pool: &SqlitePool, id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM webhooks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn create_webhook_delivery(
    pool: &SqlitePool,
    webhook_id: i64,
    event: &str,
    payload_json: &str,
) -> Result<i64> {
    let res = sqlx::query(
        "INSERT INTO webhook_deliveries (webhook_id, event, payload_json, created_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(webhook_id)
    .bind(event)
    .bind(payload_json)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

pub async fn record_webhook_attempt(
    pool: &SqlitePool,
    delivery_id: i64,
    status_code: Option<i64>,
    response_body: Option<&str>,
    error: Option<&str>,
    delivered: bool,
    next_retry_at: Option<i64>,
) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "UPDATE webhook_deliveries
         SET status_code = ?,
             response_body = ?,
             error = ?,
             attempts = attempts + 1,
             next_retry_at = ?,
             delivered_at = CASE WHEN ? = 1 THEN ? ELSE delivered_at END
         WHERE id = ?",
    )
    .bind(status_code)
    .bind(response_body)
    .bind(error)
    .bind(next_retry_at)
    .bind(i64::from(delivered))
    .bind(now)
    .bind(delivery_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_webhook_deliveries(
    pool: &SqlitePool,
    webhook_id: i64,
    limit: i64,
) -> Result<Vec<WebhookDelivery>> {
    let limit = limit.clamp(1, 200);
    let rows = sqlx::query(
        "SELECT * FROM webhook_deliveries WHERE webhook_id = ? ORDER BY created_at DESC LIMIT ?",
    )
    .bind(webhook_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(WebhookDelivery::from_row).collect()
}

// ---------------------------------------------------------------------------
// Transcoder presets
// ---------------------------------------------------------------------------

pub async fn list_transcoder_presets(pool: &SqlitePool) -> Result<Vec<TranscoderPreset>> {
    let rows = sqlx::query(
        "SELECT * FROM transcoder_presets ORDER BY sort_order ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(TranscoderPreset::from_row).collect()
}

pub async fn get_transcoder_preset(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<TranscoderPreset>> {
    let row = sqlx::query("SELECT * FROM transcoder_presets WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(TranscoderPreset::from_row).transpose()
}

pub async fn create_transcoder_preset(
    pool: &SqlitePool,
    input: NewTranscoderPreset,
) -> Result<TranscoderPreset> {
    let now = now_ms();
    let id: i64 = sqlx::query(
        "INSERT INTO transcoder_presets
            (name, max_video_bitrate_kbps, max_height, audio_codec, audio_bitrate_kbps,
             enabled, sort_order, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&input.name)
    .bind(input.max_video_bitrate_kbps)
    .bind(input.max_height)
    .bind(&input.audio_codec)
    .bind(input.audio_bitrate_kbps)
    .bind(i64::from(input.enabled))
    .bind(input.sort_order)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?
    .try_get("id")?;
    get_transcoder_preset(pool, id)
        .await?
        .context("inserted preset disappeared")
}

pub async fn update_transcoder_preset(
    pool: &SqlitePool,
    id: i64,
    update: TranscoderPresetUpdate,
) -> Result<Option<TranscoderPreset>> {
    let mut tx = pool.begin().await?;
    let now = now_ms();
    macro_rules! upd {
        ($col:literal, $val:expr) => {
            if let Some(v) = $val {
                sqlx::query(concat!("UPDATE transcoder_presets SET ", $col, " = ?, updated_at = ? WHERE id = ?"))
                    .bind(v)
                    .bind(now)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
        };
    }
    upd!("name", update.name.as_ref());
    upd!("max_video_bitrate_kbps", update.max_video_bitrate_kbps);
    upd!("max_height", update.max_height);
    upd!("audio_codec", update.audio_codec.as_ref());
    upd!("audio_bitrate_kbps", update.audio_bitrate_kbps);
    upd!("sort_order", update.sort_order);
    if let Some(v) = update.enabled {
        sqlx::query("UPDATE transcoder_presets SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(v))
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    get_transcoder_preset(pool, id).await
}

pub async fn delete_transcoder_preset(pool: &SqlitePool, id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM transcoder_presets WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

// ---------------------------------------------------------------------------
// Scheduled tasks
// ---------------------------------------------------------------------------

pub async fn list_scheduled_tasks(pool: &SqlitePool) -> Result<Vec<ScheduledTask>> {
    let rows = sqlx::query(
        "SELECT * FROM scheduled_tasks ORDER BY enabled DESC, next_run_at ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(ScheduledTask::from_row).collect()
}

pub async fn get_scheduled_task(pool: &SqlitePool, id: i64) -> Result<Option<ScheduledTask>> {
    let row = sqlx::query("SELECT * FROM scheduled_tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(ScheduledTask::from_row).transpose()
}

pub async fn create_scheduled_task(
    pool: &SqlitePool,
    input: NewScheduledTask,
    next_run_at: i64,
) -> Result<ScheduledTask> {
    let now = now_ms();
    let id: i64 = sqlx::query(
        "INSERT INTO scheduled_tasks
            (kind, name, cron_expr, frequency, requires_maintenance_window,
             params_json, enabled, next_run_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&input.kind)
    .bind(&input.name)
    .bind(&input.cron_expr)
    .bind(&input.frequency)
    .bind(i64::from(input.requires_maintenance_window))
    .bind(&input.params_json)
    .bind(i64::from(input.enabled))
    .bind(next_run_at)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?
    .try_get("id")?;
    get_scheduled_task(pool, id)
        .await?
        .context("inserted task disappeared")
}

pub async fn update_scheduled_task(
    pool: &SqlitePool,
    id: i64,
    update: ScheduledTaskUpdate,
    next_run_at: Option<i64>,
) -> Result<Option<ScheduledTask>> {
    let mut tx = pool.begin().await?;
    let now = now_ms();
    if let Some(v) = &update.name {
        sqlx::query("UPDATE scheduled_tasks SET name = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = &update.cron_expr {
        sqlx::query("UPDATE scheduled_tasks SET cron_expr = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = &update.frequency {
        sqlx::query("UPDATE scheduled_tasks SET frequency = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = update.requires_maintenance_window {
        sqlx::query(
            "UPDATE scheduled_tasks SET requires_maintenance_window = ?, updated_at = ? WHERE id = ?",
        )
        .bind(i64::from(v))
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(v) = &update.params_json {
        sqlx::query("UPDATE scheduled_tasks SET params_json = ?, updated_at = ? WHERE id = ?")
            .bind(v)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = update.enabled {
        sqlx::query("UPDATE scheduled_tasks SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(i64::from(v))
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(nrun) = next_run_at {
        sqlx::query("UPDATE scheduled_tasks SET next_run_at = ?, updated_at = ? WHERE id = ?")
            .bind(nrun)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    get_scheduled_task(pool, id).await
}

pub async fn delete_scheduled_task(pool: &SqlitePool, id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM scheduled_tasks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn claim_due_tasks(pool: &SqlitePool, now: i64) -> Result<Vec<ScheduledTask>> {
    // Return tasks whose next_run_at is due and aren't currently running.
    let rows = sqlx::query(
        "SELECT * FROM scheduled_tasks
         WHERE enabled = 1
           AND next_run_at <= ?
           AND (last_status IS NULL OR last_status <> 'running')
         ORDER BY next_run_at ASC
         LIMIT 16",
    )
    .bind(now)
    .fetch_all(pool)
    .await?;
    rows.iter().map(ScheduledTask::from_row).collect()
}

pub async fn mark_task_running(pool: &SqlitePool, task_id: i64, started_at: i64) -> Result<i64> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE scheduled_tasks SET last_status = 'running', last_run_at = ? WHERE id = ?")
        .bind(started_at)
        .bind(task_id)
        .execute(&mut *tx)
        .await?;
    let run_id: i64 = sqlx::query(
        "INSERT INTO task_runs (task_id, started_at, status) VALUES (?, ?, 'running')
         RETURNING id",
    )
    .bind(task_id)
    .bind(started_at)
    .fetch_one(&mut *tx)
    .await?
    .try_get("id")?;
    tx.commit().await?;
    Ok(run_id)
}

pub async fn mark_task_finished(
    pool: &SqlitePool,
    task_id: i64,
    run_id: i64,
    finished_at: i64,
    duration_ms: i64,
    next_run_at: i64,
    status: &str,
    error: Option<&str>,
    log: Option<&str>,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE task_runs SET finished_at = ?, status = ?, error = ?, log = ? WHERE id = ?",
    )
    .bind(finished_at)
    .bind(status)
    .bind(error)
    .bind(log)
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE scheduled_tasks
         SET last_status = ?,
             last_error = ?,
             last_duration_ms = ?,
             next_run_at = ?
         WHERE id = ?",
    )
    .bind(status)
    .bind(error)
    .bind(duration_ms)
    .bind(next_run_at)
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// On startup, any task left in `last_status = 'running'` is the residue of
/// an interrupted previous boot. Flip them to failed so the scheduler will
/// re-schedule them on the next due cycle.
pub async fn mark_interrupted_tasks(pool: &SqlitePool) -> Result<u64> {
    let now = now_ms();
    let res = sqlx::query(
        "UPDATE scheduled_tasks
         SET last_status = 'failed',
             last_error = 'interrupted by server restart'
         WHERE last_status = 'running'",
    )
    .execute(pool)
    .await?;
    let _ = sqlx::query(
        "UPDATE task_runs
         SET status = 'failed',
             error = 'interrupted by server restart',
             finished_at = ?
         WHERE status = 'running'",
    )
    .bind(now)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn list_task_runs(
    pool: &SqlitePool,
    task_id: i64,
    limit: i64,
) -> Result<Vec<TaskRun>> {
    let limit = limit.clamp(1, 200);
    let rows = sqlx::query(
        "SELECT * FROM task_runs WHERE task_id = ? ORDER BY started_at DESC LIMIT ?",
    )
    .bind(task_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(TaskRun::from_row).collect()
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

// ─── Trakt sync (Phase 15) ─────────────────────────────────────────────────
//
// Per-user OAuth tokens (device-flow minted) and per-user ratings. The
// tokens table is upserted on link, deleted on unlink. Ratings have a
// uniqueness constraint per (user, item) and per (user, episode) so
// duplicate inserts no-op.

#[derive(Debug, Clone, serde::Serialize)]
pub struct TraktTokensRow {
    pub user_id: i64,
    pub access_token: String,
    pub refresh_token: String,
    pub scope: Option<String>,
    pub expires_at: i64,
    pub linked_at: i64,
    pub last_synced_at: Option<i64>,
}

pub async fn upsert_trakt_tokens(
    pool: &SqlitePool,
    user_id: i64,
    access_token: &str,
    refresh_token: &str,
    scope: Option<&str>,
    expires_at: i64,
) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO user_trakt_tokens
            (user_id, access_token, refresh_token, scope, expires_at, linked_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET
            access_token = excluded.access_token,
            refresh_token = excluded.refresh_token,
            scope = excluded.scope,
            expires_at = excluded.expires_at",
    )
    .bind(user_id)
    .bind(access_token)
    .bind(refresh_token)
    .bind(scope)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_trakt_tokens(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Option<TraktTokensRow>> {
    let row = sqlx::query(
        "SELECT * FROM user_trakt_tokens WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(TraktTokensRow {
        user_id: row.try_get("user_id")?,
        access_token: row.try_get("access_token")?,
        refresh_token: row.try_get("refresh_token")?,
        scope: row.try_get::<Option<String>, _>("scope").ok().flatten(),
        expires_at: row.try_get("expires_at")?,
        linked_at: row.try_get("linked_at")?,
        last_synced_at: row.try_get::<Option<i64>, _>("last_synced_at").ok().flatten(),
    }))
}

pub async fn delete_trakt_tokens(pool: &SqlitePool, user_id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM user_trakt_tokens WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn update_trakt_last_synced(
    pool: &SqlitePool,
    user_id: i64,
    when_ms: i64,
) -> Result<()> {
    sqlx::query("UPDATE user_trakt_tokens SET last_synced_at = ? WHERE user_id = ?")
        .bind(when_ms)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_trakt_linked_user_ids(pool: &SqlitePool) -> Result<Vec<i64>> {
    let rows = sqlx::query("SELECT user_id FROM user_trakt_tokens")
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().filter_map(|r| r.try_get("user_id").ok()).collect())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UserRatingRow {
    pub id: i64,
    pub user_id: i64,
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub rating: i32,
    pub rated_at: i64,
}

pub async fn set_user_rating(
    pool: &SqlitePool,
    user_id: i64,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    rating: i32,
) -> Result<UserRatingRow> {
    if !(1..=10).contains(&rating) {
        anyhow::bail!("rating must be between 1 and 10");
    }
    if item_id.is_some() == episode_id.is_some() {
        anyhow::bail!("exactly one of item_id or episode_id is required");
    }
    let now = now_ms();
    // SQLite can't ON CONFLICT against a partial-unique-index target
    // cleanly, so do a transactional upsert by hand.
    let mut tx = pool.begin().await?;
    let existing = if let Some(id) = item_id {
        sqlx::query("SELECT id FROM user_ratings WHERE user_id = ? AND item_id = ?")
            .bind(user_id)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
    } else {
        sqlx::query("SELECT id FROM user_ratings WHERE user_id = ? AND episode_id = ?")
            .bind(user_id)
            .bind(episode_id.unwrap())
            .fetch_optional(&mut *tx)
            .await?
    };
    let id: i64 = if let Some(row) = existing {
        let rid: i64 = row.try_get("id")?;
        sqlx::query("UPDATE user_ratings SET rating = ?, rated_at = ? WHERE id = ?")
            .bind(rating)
            .bind(now)
            .bind(rid)
            .execute(&mut *tx)
            .await?;
        rid
    } else {
        sqlx::query(
            "INSERT INTO user_ratings (user_id, item_id, episode_id, rating, rated_at)
             VALUES (?, ?, ?, ?, ?)
             RETURNING id",
        )
        .bind(user_id)
        .bind(item_id)
        .bind(episode_id)
        .bind(rating)
        .bind(now)
        .fetch_one(&mut *tx)
        .await?
        .try_get("id")?
    };
    tx.commit().await?;
    Ok(UserRatingRow {
        id,
        user_id,
        item_id,
        episode_id,
        rating,
        rated_at: now,
    })
}

pub async fn delete_user_rating(
    pool: &SqlitePool,
    user_id: i64,
    item_id: Option<i64>,
    episode_id: Option<i64>,
) -> Result<bool> {
    let res = if let Some(id) = item_id {
        sqlx::query("DELETE FROM user_ratings WHERE user_id = ? AND item_id = ?")
            .bind(user_id)
            .bind(id)
            .execute(pool)
            .await?
    } else if let Some(id) = episode_id {
        sqlx::query("DELETE FROM user_ratings WHERE user_id = ? AND episode_id = ?")
            .bind(user_id)
            .bind(id)
            .execute(pool)
            .await?
    } else {
        anyhow::bail!("one of item_id or episode_id is required");
    };
    Ok(res.rows_affected() > 0)
}

pub async fn get_user_rating_for_item(
    pool: &SqlitePool,
    user_id: i64,
    item_id: i64,
) -> Result<Option<i32>> {
    let row = sqlx::query(
        "SELECT rating FROM user_ratings WHERE user_id = ? AND item_id = ?",
    )
    .bind(user_id)
    .bind(item_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.try_get::<i32, _>("rating").unwrap_or(0)))
}

pub async fn get_user_rating_for_episode(
    pool: &SqlitePool,
    user_id: i64,
    episode_id: i64,
) -> Result<Option<i32>> {
    let row = sqlx::query(
        "SELECT rating FROM user_ratings WHERE user_id = ? AND episode_id = ?",
    )
    .bind(user_id)
    .bind(episode_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.try_get::<i32, _>("rating").unwrap_or(0)))
}

// ─── Tags (Phase 14) ───────────────────────────────────────────────────────
//
// Plain operator-managed labels. Distinct from `genres`, which the
// metadata pipeline writes; tags are never touched by enrichment so a
// user's `rewatch` label survives every refresh.

#[derive(Debug, Clone, serde::Serialize)]
pub struct Tag {
    pub id: i64,
    pub name: String,
}

pub async fn list_tags(pool: &SqlitePool) -> Result<Vec<Tag>> {
    let rows = sqlx::query("SELECT id, name FROM tags ORDER BY name COLLATE NOCASE")
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| {
            Ok(Tag {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
            })
        })
        .collect()
}

pub async fn list_tags_for_item(pool: &SqlitePool, item_id: i64) -> Result<Vec<Tag>> {
    let rows = sqlx::query(
        "SELECT t.id, t.name
         FROM tags t
         JOIN item_tags it ON it.tag_id = t.id
         WHERE it.item_id = ?
         ORDER BY t.name COLLATE NOCASE",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(Tag {
                id: row.try_get("id")?,
                name: row.try_get("name")?,
            })
        })
        .collect()
}

pub async fn add_tag_to_item(
    pool: &SqlitePool,
    item_id: i64,
    name: &str,
) -> Result<Tag> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("tag name must not be empty");
    }
    // Upsert the tag, then bind to the item. The COLLATE NOCASE unique
    // index dedupes case-only variants ("Rewatch" vs "rewatch").
    let id: i64 = sqlx::query(
        "INSERT INTO tags (name) VALUES (?)
         ON CONFLICT(name) DO UPDATE SET name = excluded.name
         RETURNING id",
    )
    .bind(trimmed)
    .fetch_one(pool)
    .await?
    .try_get("id")?;
    sqlx::query("INSERT OR IGNORE INTO item_tags (item_id, tag_id) VALUES (?, ?)")
        .bind(item_id)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(Tag {
        id,
        name: trimmed.to_string(),
    })
}

/// Variant for bulk operations: resolve the tag by name first. Returns
/// `true` only when both the tag existed AND the binding existed.
pub async fn remove_tag_from_item_by_name(
    pool: &SqlitePool,
    item_id: i64,
    name: &str,
) -> Result<bool> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    let tag_id: Option<i64> = sqlx::query_scalar("SELECT id FROM tags WHERE name = ? COLLATE NOCASE")
        .bind(trimmed)
        .fetch_optional(pool)
        .await?;
    let Some(tag_id) = tag_id else { return Ok(false) };
    remove_tag_from_item(pool, item_id, tag_id).await
}

pub async fn remove_tag_from_item(
    pool: &SqlitePool,
    item_id: i64,
    tag_id: i64,
) -> Result<bool> {
    let res = sqlx::query("DELETE FROM item_tags WHERE item_id = ? AND tag_id = ?")
        .bind(item_id)
        .bind(tag_id)
        .execute(pool)
        .await?;
    // Garbage-collect tags no other item references — keeps the tag
    // list curated to what's actually in use.
    sqlx::query(
        "DELETE FROM tags WHERE id = ?
         AND NOT EXISTS (SELECT 1 FROM item_tags WHERE tag_id = tags.id)",
    )
    .bind(tag_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

// ─── Preview sprites ───────────────────────────────────────────────────────
//
// One scrub-preview sprite per media_file. Dimensions live on the row so
// the player can compute tile offsets without an extra round trip.

#[derive(Debug, Clone)]
pub struct PreviewSpriteRecord {
    pub media_file_id: i64,
    pub path: String,
    pub interval_ms: i64,
    pub tile_width: i64,
    pub tile_height: i64,
    pub tile_cols: i64,
    pub tile_count: i64,
}

#[derive(Debug, Clone)]
pub struct MediaFileForPreview {
    pub id: i64,
    pub path: String,
    pub duration_ms: Option<i64>,
}

/// Return media files in the given library (or globally if `None`) that
/// have a non-null duration and no preview sprite yet.
pub async fn list_media_files_needing_previews(
    pool: &SqlitePool,
    library_id: Option<i64>,
    limit: i64,
) -> Result<Vec<MediaFileForPreview>> {
    let rows = if let Some(lid) = library_id {
        sqlx::query(
            "SELECT mf.id AS id, mf.path AS path, mf.duration_ms AS duration_ms
             FROM media_files mf
             LEFT JOIN items i ON i.id = mf.item_id
             LEFT JOIN episodes e ON e.id = mf.episode_id
             LEFT JOIN seasons s ON s.id = e.season_id
             WHERE mf.preview_sprite_path IS NULL
               AND mf.duration_ms IS NOT NULL
               AND (i.library_id = ? OR s.show_id IN
                    (SELECT id FROM items WHERE library_id = ?))
             LIMIT ?",
        )
        .bind(lid)
        .bind(lid)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, path, duration_ms FROM media_files
             WHERE preview_sprite_path IS NULL AND duration_ms IS NOT NULL
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    rows.iter()
        .map(|row| {
            Ok(MediaFileForPreview {
                id: row.try_get("id")?,
                path: row.try_get("path")?,
                duration_ms: row.try_get::<Option<i64>, _>("duration_ms").ok().flatten(),
            })
        })
        .collect()
}

pub async fn record_preview_sprite(
    pool: &SqlitePool,
    record: PreviewSpriteRecord,
) -> Result<()> {
    sqlx::query(
        "UPDATE media_files SET
            preview_sprite_path = ?,
            preview_interval_ms = ?,
            preview_tile_width = ?,
            preview_tile_height = ?,
            preview_tile_cols = ?,
            preview_tile_count = ?
         WHERE id = ?",
    )
    .bind(&record.path)
    .bind(record.interval_ms)
    .bind(record.tile_width)
    .bind(record.tile_height)
    .bind(record.tile_cols)
    .bind(record.tile_count)
    .bind(record.media_file_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_preview_sprite(
    pool: &SqlitePool,
    media_file_id: i64,
) -> Result<Option<PreviewSpriteRecord>> {
    let row = sqlx::query(
        "SELECT id, preview_sprite_path, preview_interval_ms,
                preview_tile_width, preview_tile_height,
                preview_tile_cols, preview_tile_count
         FROM media_files WHERE id = ?",
    )
    .bind(media_file_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let path: Option<String> = row.try_get("preview_sprite_path").ok().flatten();
    let Some(path) = path else { return Ok(None) };
    Ok(Some(PreviewSpriteRecord {
        media_file_id: row.try_get("id")?,
        path,
        interval_ms: row.try_get("preview_interval_ms")?,
        tile_width: row.try_get("preview_tile_width")?,
        tile_height: row.try_get("preview_tile_height")?,
        tile_cols: row.try_get("preview_tile_cols")?,
        tile_count: row.try_get("preview_tile_count")?,
    }))
}

// ─── Chapter thumbnails ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MediaFileForChapterThumbs {
    pub id: i64,
    pub path: String,
}

/// Files that haven't had their chapter-thumb pass run yet. We use a
/// nullable timestamp column rather than a boolean so a future
/// "regenerate after X days" sweep can compare timestamps directly.
pub async fn list_media_files_needing_chapter_thumbs(
    pool: &SqlitePool,
    library_id: Option<i64>,
    limit: i64,
) -> Result<Vec<MediaFileForChapterThumbs>> {
    let rows = if let Some(lid) = library_id {
        sqlx::query(
            "SELECT mf.id AS id, mf.path AS path
             FROM media_files mf
             LEFT JOIN items i ON i.id = mf.item_id
             LEFT JOIN episodes e ON e.id = mf.episode_id
             LEFT JOIN seasons s ON s.id = e.season_id
             WHERE mf.chapter_thumbs_generated_at IS NULL
               AND mf.removed_at IS NULL
               AND (i.library_id = ? OR s.show_id IN
                    (SELECT id FROM items WHERE library_id = ?))
             LIMIT ?",
        )
        .bind(lid)
        .bind(lid)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, path FROM media_files
             WHERE chapter_thumbs_generated_at IS NULL AND removed_at IS NULL
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    rows.iter()
        .map(|row| {
            Ok(MediaFileForChapterThumbs {
                id: row.try_get("id")?,
                path: row.try_get("path")?,
            })
        })
        .collect()
}

/// Stamp `chapter_thumbs_generated_at` + record how many chapters the
/// file had. `chapter_count = 0` is a valid "no chapters" result — we
/// still set the timestamp so the next task run doesn't re-probe.
pub async fn record_chapter_thumbs_generated(
    pool: &SqlitePool,
    media_file_id: i64,
    chapter_count: i64,
) -> Result<()> {
    sqlx::query(
        "UPDATE media_files
         SET chapter_thumbs_generated_at = ?, chapter_count = ?
         WHERE id = ?",
    )
    .bind(now_ms())
    .bind(chapter_count)
    .bind(media_file_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Look up a media_file's stored chapter count + processing timestamp.
/// Returns `None` if the file is unknown; returns `Some((None, _))`
/// for a file that hasn't been processed yet.
pub async fn get_chapter_thumbs_status(
    pool: &SqlitePool,
    media_file_id: i64,
) -> Result<Option<(Option<i64>, Option<i64>)>> {
    let row = sqlx::query(
        "SELECT chapter_thumbs_generated_at, chapter_count, path
         FROM media_files WHERE id = ?",
    )
    .bind(media_file_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some((
        row.try_get::<Option<i64>, _>("chapter_thumbs_generated_at")
            .ok()
            .flatten(),
        row.try_get::<Option<i64>, _>("chapter_count").ok().flatten(),
    )))
}

/// Resolve the disk path for a media_file. Used by the chapter-thumb
/// API to feed `probe_chapters` without rehydrating the full row.
pub async fn get_media_file_path(
    pool: &SqlitePool,
    media_file_id: i64,
) -> Result<Option<String>> {
    let row = sqlx::query("SELECT path FROM media_files WHERE id = ?")
        .bind(media_file_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.try_get("path").ok()))
}

// ─── Loudness analysis ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MediaFileForLoudness {
    pub id: i64,
    pub path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LoudnessMeasurement {
    pub integrated: f64,
    pub true_peak: f64,
    pub lra: f64,
    pub threshold: f64,
}

/// Files that haven't had loudness analysis run yet. Skips files
/// without duration (probe must have failed) since loudnorm wants a
/// known audio stream.
pub async fn list_media_files_needing_loudness(
    pool: &SqlitePool,
    library_id: Option<i64>,
    limit: i64,
) -> Result<Vec<MediaFileForLoudness>> {
    let rows = if let Some(lid) = library_id {
        sqlx::query(
            "SELECT mf.id AS id, mf.path AS path
             FROM media_files mf
             LEFT JOIN items i ON i.id = mf.item_id
             LEFT JOIN episodes e ON e.id = mf.episode_id
             LEFT JOIN seasons s ON s.id = e.season_id
             WHERE mf.loudnorm_analyzed_at IS NULL
               AND mf.removed_at IS NULL
               AND mf.duration_ms IS NOT NULL
               AND (i.library_id = ? OR s.show_id IN
                    (SELECT id FROM items WHERE library_id = ?))
             LIMIT ?",
        )
        .bind(lid)
        .bind(lid)
        .bind(limit)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT id, path FROM media_files
             WHERE loudnorm_analyzed_at IS NULL
               AND removed_at IS NULL
               AND duration_ms IS NOT NULL
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await?
    };
    rows.iter()
        .map(|row| {
            Ok(MediaFileForLoudness {
                id: row.try_get("id")?,
                path: row.try_get("path")?,
            })
        })
        .collect()
}

/// Stamp the four loudnorm values. `None` means "we ran the analysis
/// but the file had no audio / silent input" — still updates the
/// timestamp so we don't re-probe.
pub async fn record_loudness_measurement(
    pool: &SqlitePool,
    media_file_id: i64,
    m: Option<LoudnessMeasurement>,
) -> Result<()> {
    let now = now_ms();
    match m {
        Some(m) => {
            sqlx::query(
                "UPDATE media_files SET
                    loudnorm_integrated = ?,
                    loudnorm_true_peak = ?,
                    loudnorm_lra = ?,
                    loudnorm_threshold = ?,
                    loudnorm_analyzed_at = ?
                 WHERE id = ?",
            )
            .bind(m.integrated)
            .bind(m.true_peak)
            .bind(m.lra)
            .bind(m.threshold)
            .bind(now)
            .bind(media_file_id)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query(
                "UPDATE media_files SET loudnorm_analyzed_at = ? WHERE id = ?",
            )
            .bind(now)
            .bind(media_file_id)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Fetch stored loudness for a file. Returns `None` when analysis
/// hasn't been run OR the analysis found no audio stream (both cases
/// have NULL integrated / etc.).
pub async fn get_loudness_measurement(
    pool: &SqlitePool,
    media_file_id: i64,
) -> Result<Option<LoudnessMeasurement>> {
    let row = sqlx::query(
        "SELECT loudnorm_integrated, loudnorm_true_peak,
                loudnorm_lra, loudnorm_threshold
         FROM media_files WHERE id = ?",
    )
    .bind(media_file_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let integrated: Option<f64> = row
        .try_get::<Option<f64>, _>("loudnorm_integrated")
        .ok()
        .flatten();
    let true_peak: Option<f64> = row
        .try_get::<Option<f64>, _>("loudnorm_true_peak")
        .ok()
        .flatten();
    let lra: Option<f64> = row
        .try_get::<Option<f64>, _>("loudnorm_lra")
        .ok()
        .flatten();
    let threshold: Option<f64> = row
        .try_get::<Option<f64>, _>("loudnorm_threshold")
        .ok()
        .flatten();
    match (integrated, true_peak, lra, threshold) {
        (Some(i), Some(t), Some(l), Some(th)) => Ok(Some(LoudnessMeasurement {
            integrated: i,
            true_peak: t,
            lra: l,
            threshold: th,
        })),
        _ => Ok(None),
    }
}

// ─── External subtitles ────────────────────────────────────────────────────
//
// One row per fetched/uploaded subtitle file. Embedded subtitle streams
// live on media_streams; this is the parallel surface for OpenSubtitles
// and (later) operator uploads. UNIQUE(source, source_file_id) lets the
// fetch task re-run without inserting duplicates.

fn row_to_external_subtitle(row: &SqliteRow) -> Result<ExternalSubtitle> {
    Ok(ExternalSubtitle {
        id: row.try_get("id")?,
        item_id: row.try_get::<Option<i64>, _>("item_id").ok().flatten(),
        episode_id: row.try_get::<Option<i64>, _>("episode_id").ok().flatten(),
        language: row.try_get("language")?,
        source: row.try_get("source")?,
        source_file_id: row.try_get::<Option<String>, _>("source_file_id").ok().flatten(),
        file_path: row.try_get("file_path")?,
        forced: row.try_get::<i64, _>("forced")? != 0,
        sdh: row.try_get::<i64, _>("sdh")? != 0,
        created_at: row.try_get("created_at")?,
    })
}

pub async fn insert_external_subtitle(
    pool: &SqlitePool,
    input: NewExternalSubtitle,
) -> Result<ExternalSubtitle> {
    let now = now_ms();
    let id: i64 = sqlx::query(
        "INSERT INTO external_subtitles
         (item_id, episode_id, language, source, source_file_id, file_path, forced, sdh, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(source, source_file_id) DO UPDATE SET
             file_path = excluded.file_path,
             item_id = excluded.item_id,
             episode_id = excluded.episode_id,
             language = excluded.language,
             forced = excluded.forced,
             sdh = excluded.sdh
         RETURNING id",
    )
    .bind(input.item_id)
    .bind(input.episode_id)
    .bind(&input.language)
    .bind(&input.source)
    .bind(&input.source_file_id)
    .bind(&input.file_path)
    .bind(i64::from(input.forced))
    .bind(i64::from(input.sdh))
    .bind(now)
    .fetch_one(pool)
    .await?
    .try_get("id")?;
    sqlx::query("SELECT * FROM external_subtitles WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map(|row| row_to_external_subtitle(&row))?
}

pub async fn list_external_subtitles_for_item(
    pool: &SqlitePool,
    item_id: i64,
) -> Result<Vec<ExternalSubtitle>> {
    let rows = sqlx::query(
        "SELECT * FROM external_subtitles WHERE item_id = ?
         ORDER BY language, forced DESC, sdh DESC, id",
    )
    .bind(item_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_external_subtitle).collect()
}

pub async fn list_external_subtitles_for_episode(
    pool: &SqlitePool,
    episode_id: i64,
) -> Result<Vec<ExternalSubtitle>> {
    let rows = sqlx::query(
        "SELECT * FROM external_subtitles WHERE episode_id = ?
         ORDER BY language, forced DESC, sdh DESC, id",
    )
    .bind(episode_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_external_subtitle).collect()
}

pub async fn get_external_subtitle(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<ExternalSubtitle>> {
    let row = sqlx::query("SELECT * FROM external_subtitles WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(row_to_external_subtitle).transpose()
}

// ─── Credential vault ──────────────────────────────────────────────────────
//
// Persistence for chimpflix_common::Vault's "named secrets" surface. The
// crypto lives in the vault crate; this module only stores ciphertext +
// nonce, with NULL nonce signalling plaintext mode.

pub async fn vault_get(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
    name: &str,
) -> Result<Option<String>> {
    let row = sqlx::query("SELECT value_enc, nonce FROM secrets WHERE name = ?")
        .bind(name)
        .fetch_optional(pool)
        .await
        .with_context(|| format!("load secret {name}"))?;
    let Some(row) = row else { return Ok(None) };
    let value_enc: Vec<u8> = row.try_get("value_enc")?;
    let nonce: Option<Vec<u8>> = row.try_get("nonce").ok().flatten();
    let blob = chimpflix_common::EncryptedBlob {
        value: value_enc,
        nonce,
    };
    let plaintext = vault
        .decrypt_str(&blob)
        .with_context(|| format!("decrypt secret {name}"))?;
    Ok(Some(plaintext))
}

pub async fn vault_set(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
    name: &str,
    plaintext: &str,
    updated_by: Option<i64>,
) -> Result<()> {
    let blob = vault
        .encrypt_str(plaintext)
        .with_context(|| format!("encrypt secret {name}"))?;
    sqlx::query(
        "INSERT INTO secrets (name, value_enc, nonce, updated_at, updated_by)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET
             value_enc  = excluded.value_enc,
             nonce      = excluded.nonce,
             updated_at = excluded.updated_at,
             updated_by = excluded.updated_by",
    )
    .bind(name)
    .bind(blob.value)
    .bind(blob.nonce)
    .bind(now_ms())
    .bind(updated_by)
    .execute(pool)
    .await
    .with_context(|| format!("upsert secret {name}"))?;
    Ok(())
}

pub async fn vault_delete(pool: &SqlitePool, name: &str) -> Result<bool> {
    let result = sqlx::query("DELETE FROM secrets WHERE name = ?")
        .bind(name)
        .execute(pool)
        .await
        .with_context(|| format!("delete secret {name}"))?;
    Ok(result.rows_affected() > 0)
}

/// List metadata for every stored secret, decrypting only enough to
/// compute the masked `last4`. Returns rows sorted by name.
pub async fn vault_list_metadata(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
) -> Result<Vec<SecretMetadata>> {
    let rows = sqlx::query(
        "SELECT name, value_enc, nonce, updated_at, updated_by
         FROM secrets ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .context("list secrets")?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let name: String = row.try_get("name")?;
        let value_enc: Vec<u8> = row.try_get("value_enc")?;
        let nonce: Option<Vec<u8>> = row.try_get("nonce").ok().flatten();
        let blob = chimpflix_common::EncryptedBlob {
            value: value_enc,
            nonce,
        };
        let last4 = match vault.decrypt_str(&blob) {
            Ok(plain) => masked_last4(&plain),
            // A decrypt failure here usually means the master key changed
            // out from under existing rows. Don't surface raw error to the
            // UI — show "????" so the operator knows the slot is broken
            // but the listing call still succeeds.
            Err(_) => "????".to_string(),
        };
        out.push(SecretMetadata {
            name,
            set: true,
            last4,
            updated_at: row.try_get("updated_at")?,
            updated_by: row.try_get("updated_by").ok().flatten(),
        });
    }
    Ok(out)
}

/// Re-encrypt rows that were stored in plaintext mode (`nonce IS NULL`)
/// after the operator turned encryption on. Without this, the first
/// `vault_get` for any plaintext row would fail with a mismatched-mode
/// error and crash startup. No-op when the vault is itself in plaintext
/// mode. Returns (named_secrets_upgraded, webhook_secrets_upgraded).
pub async fn upgrade_plaintext_secrets(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
) -> Result<(usize, usize)> {
    if !vault.is_encrypted() {
        return Ok((0, 0));
    }

    let mut named = 0;
    let rows = sqlx::query("SELECT name, value_enc FROM secrets WHERE nonce IS NULL")
        .fetch_all(pool)
        .await
        .context("scan secrets for plaintext rows")?;
    for row in rows {
        let name: String = row.try_get("name")?;
        let plaintext: Vec<u8> = row.try_get("value_enc")?;
        let blob = vault.encrypt(&plaintext)?;
        sqlx::query(
            "UPDATE secrets SET value_enc = ?, nonce = ?, updated_at = ? WHERE name = ?",
        )
        .bind(blob.value)
        .bind(blob.nonce)
        .bind(now_ms())
        .bind(&name)
        .execute(pool)
        .await
        .with_context(|| format!("re-encrypt secret {name}"))?;
        named += 1;
    }

    let mut hooks = 0;
    let rows = sqlx::query(
        "SELECT id, secret_enc FROM webhooks
         WHERE secret_enc IS NOT NULL AND secret_nonce IS NULL",
    )
    .fetch_all(pool)
    .await
    .context("scan webhooks for plaintext-encoded secrets")?;
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let plaintext: Vec<u8> = row.try_get("secret_enc")?;
        let blob = vault.encrypt(&plaintext)?;
        sqlx::query(
            "UPDATE webhooks SET secret_enc = ?, secret_nonce = ? WHERE id = ?",
        )
        .bind(blob.value)
        .bind(blob.nonce)
        .bind(id)
        .execute(pool)
        .await
        .with_context(|| format!("re-encrypt webhook secret id={id}"))?;
        hooks += 1;
    }

    Ok((named, hooks))
}

/// One-shot migration from the legacy plaintext `webhooks.secret` column
/// into the encrypted `secret_enc`/`secret_nonce` pair. Idempotent —
/// re-runs are no-ops once every row is converted. Returns the number of
/// rows migrated this call.
pub async fn backfill_webhook_secrets(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
) -> Result<usize> {
    let rows = sqlx::query(
        "SELECT id, secret FROM webhooks
         WHERE secret IS NOT NULL AND secret_enc IS NULL",
    )
    .fetch_all(pool)
    .await
    .context("scan webhooks for plaintext secrets")?;

    let mut count = 0;
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let secret: String = row.try_get("secret")?;
        let blob = vault.encrypt_str(&secret)?;
        sqlx::query(
            "UPDATE webhooks
             SET secret_enc = ?, secret_nonce = ?, secret = NULL
             WHERE id = ?",
        )
        .bind(blob.value)
        .bind(blob.nonce)
        .bind(id)
        .execute(pool)
        .await
        .with_context(|| format!("backfill webhook secret id={id}"))?;
        count += 1;
    }
    Ok(count)
}

fn masked_last4(plaintext: &str) -> String {
    let chars: Vec<char> = plaintext.chars().collect();
    if chars.len() <= 4 {
        "*".repeat(chars.len().max(1))
    } else {
        chars[chars.len() - 4..].iter().collect()
    }
}

// ─── Playback events (admin Stats page) ───────────────────────────────────
//
// One row per "interesting moment" in a stream — start, complete, etc.
// Drives the Tautulli-style stats dashboard: recent activity feed, top
// users / top items leaderboards, transcode mix, now-playing snapshot.
//
// Insertion sites are intentionally narrow: the stream handler emits
// `start` events (with decision/codecs/IP/UA) and the scrobble handler
// emits `complete` events. Progress / pause / resume / stop are
// schema-supported but not currently emitted — adding them later is a
// one-call-site change because all the aggregation queries already
// filter by event_type as needed.

#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaybackEventInput<'a> {
    pub user_id: i64,
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    pub media_file_id: Option<i64>,
    pub event_type: &'a str,
    pub position_ms: Option<i64>,
    pub duration_ms: Option<i64>,
    pub decision: Option<&'a str>,
    pub video_codec: Option<&'a str>,
    pub audio_codec: Option<&'a str>,
    pub container: Option<&'a str>,
    /// Cumulative bytes served by the session emitting this event.
    /// Populated on `stop` events from the transcoder's per-session
    /// counter; `start` / `complete` events leave it None.
    pub bytes_sent: Option<i64>,
    pub ip: Option<&'a str>,
    pub user_agent: Option<&'a str>,
    pub session_token: Option<&'a str>,
}

impl<'a> PlaybackEventInput<'a> {
    /// Constructor that defaults every optional field to None. Use
    /// `.with_*` setters (just direct field assignment via struct
    /// update syntax in practice) to fill what's relevant per
    /// event-type. Saves call sites from listing every None field
    /// when emitting a minimal event.
    pub fn new(user_id: i64, event_type: &'a str) -> Self {
        Self {
            user_id,
            item_id: None,
            episode_id: None,
            media_file_id: None,
            event_type,
            position_ms: None,
            duration_ms: None,
            decision: None,
            video_codec: None,
            audio_codec: None,
            container: None,
            bytes_sent: None,
            ip: None,
            user_agent: None,
            session_token: None,
        }
    }
}

/// Append a playback event. Fire-and-forget — failures are logged at
/// the call site, never bubble up to the user's request. Stats are
/// observability, not load-bearing for playback.
pub async fn record_playback_event(
    pool: &SqlitePool,
    ev: PlaybackEventInput<'_>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO playback_events (
            user_id, item_id, episode_id, media_file_id, event_type,
            occurred_at, position_ms, duration_ms, decision,
            video_codec, audio_codec, container, bytes_sent,
            ip, user_agent, session_token
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(ev.user_id)
    .bind(ev.item_id)
    .bind(ev.episode_id)
    .bind(ev.media_file_id)
    .bind(ev.event_type)
    .bind(now_ms())
    .bind(ev.position_ms)
    .bind(ev.duration_ms)
    .bind(ev.decision)
    .bind(ev.video_codec)
    .bind(ev.audio_codec)
    .bind(ev.container)
    .bind(ev.bytes_sent)
    .bind(ev.ip)
    .bind(ev.user_agent)
    .bind(ev.session_token)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsActivityRow {
    pub id: i64,
    pub occurred_at: i64,
    pub user_id: i64,
    pub username: String,
    pub event_type: String,
    pub decision: Option<String>,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub container: Option<String>,
    pub ip: Option<String>,
    pub item_id: Option<i64>,
    pub episode_id: Option<i64>,
    /// Display title — movie title or "<show> — <episode title>".
    pub title: Option<String>,
}

/// Recent activity feed for the admin Stats page. Joins to users,
/// items, and episodes so the UI gets pre-stitched display strings
/// in one round trip. `limit` clamped to [1, 200]. Pass `user_id` to
/// scope the feed to a single user (drill-in from the Top Users
/// list).
pub async fn list_playback_activity(
    pool: &SqlitePool,
    limit: i64,
    before_id: Option<i64>,
    user_id: Option<i64>,
) -> Result<Vec<StatsActivityRow>> {
    let limit = limit.clamp(1, 200);
    let before_clause = if before_id.is_some() {
        "AND pe.id < ?"
    } else {
        ""
    };
    let user_clause = if user_id.is_some() {
        "AND pe.user_id = ?"
    } else {
        ""
    };
    let sql = format!(
        "SELECT pe.id, pe.occurred_at, pe.user_id, u.username,
                pe.event_type, pe.decision, pe.video_codec, pe.audio_codec,
                pe.container, pe.ip,
                pe.item_id, pe.episode_id,
                COALESCE(
                    i.title,
                    show.title || ' — ' || ep.title
                ) AS title
         FROM playback_events pe
         JOIN users u ON u.id = pe.user_id
         LEFT JOIN items i ON i.id = pe.item_id
         LEFT JOIN episodes ep ON ep.id = pe.episode_id
         LEFT JOIN seasons s ON s.id = ep.season_id
         LEFT JOIN items show ON show.id = s.show_id
         WHERE 1=1 {before_clause} {user_clause}
         ORDER BY pe.id DESC
         LIMIT ?"
    );
    let mut q = sqlx::query(&sql);
    if let Some(b) = before_id {
        q = q.bind(b);
    }
    if let Some(u) = user_id {
        q = q.bind(u);
    }
    q = q.bind(limit);
    let rows = q.fetch_all(pool).await?;
    rows.iter()
        .map(|r| {
            Ok(StatsActivityRow {
                id: r.try_get("id")?,
                occurred_at: r.try_get("occurred_at")?,
                user_id: r.try_get("user_id")?,
                username: r.try_get("username")?,
                event_type: r.try_get("event_type")?,
                decision: r.try_get::<Option<String>, _>("decision").ok().flatten(),
                video_codec: r.try_get::<Option<String>, _>("video_codec").ok().flatten(),
                audio_codec: r.try_get::<Option<String>, _>("audio_codec").ok().flatten(),
                container: r.try_get::<Option<String>, _>("container").ok().flatten(),
                ip: r.try_get::<Option<String>, _>("ip").ok().flatten(),
                item_id: r.try_get::<Option<i64>, _>("item_id").ok().flatten(),
                episode_id: r.try_get::<Option<i64>, _>("episode_id").ok().flatten(),
                title: r.try_get::<Option<String>, _>("title").ok().flatten(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsTopUserRow {
    pub user_id: i64,
    pub username: String,
    pub display_name: Option<String>,
    /// Count of `start` events in the window — distinct streams.
    pub play_count: i64,
    /// Count of `complete` events — actually-finished streams.
    pub completions: i64,
    /// Most recent event timestamp.
    pub last_seen_at: Option<i64>,
}

/// Top users by play count over the last N days. Includes completions
/// so the UI can show "started 12, finished 4" at a glance.
pub async fn top_users_by_plays(
    pool: &SqlitePool,
    since_ms: i64,
    limit: i64,
) -> Result<Vec<StatsTopUserRow>> {
    let limit = limit.clamp(1, 50);
    let rows = sqlx::query(
        "SELECT u.id AS user_id, u.username, u.display_name,
                SUM(CASE WHEN pe.event_type = 'start' THEN 1 ELSE 0 END) AS play_count,
                SUM(CASE WHEN pe.event_type = 'complete' THEN 1 ELSE 0 END) AS completions,
                MAX(pe.occurred_at) AS last_seen_at
         FROM playback_events pe
         JOIN users u ON u.id = pe.user_id
         WHERE pe.occurred_at >= ?
         GROUP BY u.id
         ORDER BY play_count DESC, last_seen_at DESC
         LIMIT ?",
    )
    .bind(since_ms)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(StatsTopUserRow {
                user_id: r.try_get("user_id")?,
                username: r.try_get("username")?,
                display_name: r.try_get::<Option<String>, _>("display_name").ok().flatten(),
                play_count: r.try_get("play_count")?,
                completions: r.try_get("completions")?,
                last_seen_at: r.try_get::<Option<i64>, _>("last_seen_at").ok().flatten(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsTopItemRow {
    pub item_id: Option<i64>,
    pub title: String,
    pub kind: String,
    pub play_count: i64,
    pub last_played_at: Option<i64>,
    pub year: Option<i32>,
}

/// Top items by play count. For TV shows we roll episodes up under
/// their parent show; movies are counted directly. `start` events
/// only — completions would skew toward short content.
pub async fn top_items_by_plays(
    pool: &SqlitePool,
    since_ms: i64,
    limit: i64,
) -> Result<Vec<StatsTopItemRow>> {
    let limit = limit.clamp(1, 50);
    let rows = sqlx::query(
        "WITH events AS (
             -- Movie events: direct item_id.
             SELECT pe.item_id AS rolled_id, pe.occurred_at
             FROM playback_events pe
             WHERE pe.event_type = 'start'
               AND pe.occurred_at >= ?
               AND pe.item_id IS NOT NULL
             UNION ALL
             -- Episode events: roll up to the parent show via seasons.show_id.
             SELECT s.show_id AS rolled_id, pe.occurred_at
             FROM playback_events pe
             JOIN episodes ep ON ep.id = pe.episode_id
             JOIN seasons s ON s.id = ep.season_id
             WHERE pe.event_type = 'start'
               AND pe.occurred_at >= ?
               AND pe.episode_id IS NOT NULL
         )
         SELECT i.id AS item_id, i.title, i.kind, i.year,
                COUNT(*) AS play_count,
                MAX(events.occurred_at) AS last_played_at
         FROM events
         JOIN items i ON i.id = events.rolled_id
         GROUP BY i.id
         ORDER BY play_count DESC, last_played_at DESC
         LIMIT ?",
    )
    .bind(since_ms)
    .bind(since_ms)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(StatsTopItemRow {
                item_id: r.try_get::<Option<i64>, _>("item_id").ok().flatten(),
                title: r.try_get("title")?,
                kind: r.try_get("kind")?,
                play_count: r.try_get("play_count")?,
                last_played_at: r.try_get::<Option<i64>, _>("last_played_at").ok().flatten(),
                year: r.try_get::<Option<i32>, _>("year").ok().flatten(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, serde::Serialize, Default)]
pub struct StatsOverview {
    pub total_plays: i64,
    pub completions: i64,
    pub direct_plays: i64,
    pub transcoded_plays: i64,
    pub unique_users: i64,
}

/// Aggregate counters for the Stats page hero tiles. One query, window
/// is the last N days.
pub async fn stats_overview(pool: &SqlitePool, since_ms: i64) -> Result<StatsOverview> {
    let row = sqlx::query(
        "SELECT
            SUM(CASE WHEN event_type = 'start' THEN 1 ELSE 0 END) AS total_plays,
            SUM(CASE WHEN event_type = 'complete' THEN 1 ELSE 0 END) AS completions,
            SUM(CASE WHEN event_type = 'start' AND decision = 'direct' THEN 1 ELSE 0 END)
                AS direct_plays,
            SUM(CASE WHEN event_type = 'start' AND decision = 'transcode' THEN 1 ELSE 0 END)
                AS transcoded_plays,
            COUNT(DISTINCT user_id) AS unique_users
         FROM playback_events
         WHERE occurred_at >= ?",
    )
    .bind(since_ms)
    .fetch_one(pool)
    .await?;
    Ok(StatsOverview {
        total_plays: row.try_get::<Option<i64>, _>("total_plays").ok().flatten().unwrap_or(0),
        completions: row.try_get::<Option<i64>, _>("completions").ok().flatten().unwrap_or(0),
        direct_plays: row.try_get::<Option<i64>, _>("direct_plays").ok().flatten().unwrap_or(0),
        transcoded_plays: row
            .try_get::<Option<i64>, _>("transcoded_plays")
            .ok()
            .flatten()
            .unwrap_or(0),
        unique_users: row.try_get::<i64, _>("unique_users")?,
    })
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsDailyBucket {
    /// Local-date bucket key as `YYYY-MM-DD` (UTC). Empty days inside
    /// the window are returned with zero counts so the chart renders a
    /// continuous x-axis instead of skipping gaps.
    pub day: String,
    pub starts: i64,
    pub completions: i64,
}

/// Per-day play counts for the activity chart. Window is the last N
/// days inclusive of today. Empty buckets are filled in so the chart
/// stays gap-free even on a quiet server.
pub async fn plays_per_day(
    pool: &SqlitePool,
    days: i64,
) -> Result<Vec<StatsDailyBucket>> {
    let days = days.clamp(1, 365);
    let now = now_ms();
    let since_ms_value = now - days * 86_400_000;
    // SQLite's `date(epoch, 'unixepoch')` produces `YYYY-MM-DD`. Group
    // by that and pivot start vs complete in a single scan.
    let rows = sqlx::query(
        "SELECT date(occurred_at / 1000, 'unixepoch') AS day,
                SUM(CASE WHEN event_type = 'start' THEN 1 ELSE 0 END) AS starts,
                SUM(CASE WHEN event_type = 'complete' THEN 1 ELSE 0 END) AS completions
         FROM playback_events
         WHERE occurred_at >= ?
         GROUP BY day
         ORDER BY day ASC",
    )
    .bind(since_ms_value)
    .fetch_all(pool)
    .await?;
    let mut found: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::with_capacity(rows.len());
    for r in &rows {
        let d: String = r.try_get("day")?;
        let s: i64 = r.try_get("starts")?;
        let c: i64 = r.try_get("completions")?;
        found.insert(d, (s, c));
    }
    // Walk forward `days` days from the start of the window so empty
    // days show as zero rather than dropping out of the series.
    let mut out = Vec::with_capacity(days as usize);
    for n in 0..days {
        let bucket_ms = since_ms_value + n * 86_400_000;
        let day = format_yyyymmdd_utc(bucket_ms);
        let (starts, completions) = found.get(&day).copied().unwrap_or((0, 0));
        out.push(StatsDailyBucket {
            day,
            starts,
            completions,
        });
    }
    Ok(out)
}

/// Format an epoch-ms value as a `YYYY-MM-DD` UTC date string. Used by
/// `plays_per_day` to fill gaps; chrono is already a workspace dep via
/// other crates so reaching for it here is free.
fn format_yyyymmdd_utc(epoch_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(epoch_ms)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "????-??-??".to_string())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsHourBucket {
    pub hour: i64,
    pub starts: i64,
}

/// Histogram of play starts by hour-of-day (0..=23). Useful for "when
/// does my household actually watch" — Plex/Tautulli's most popular
/// chart. Bucket aligned to server local time so it matches the
/// operator's intuition rather than UTC.
pub async fn plays_per_hour(pool: &SqlitePool, days: i64) -> Result<Vec<StatsHourBucket>> {
    let days = days.clamp(1, 365);
    let since = now_ms() - days * 86_400_000;
    let rows = sqlx::query(
        "SELECT CAST(strftime('%H', occurred_at / 1000, 'unixepoch', 'localtime') AS INTEGER) AS hour,
                COUNT(*) AS starts
         FROM playback_events
         WHERE event_type = 'start' AND occurred_at >= ?
         GROUP BY hour",
    )
    .bind(since)
    .fetch_all(pool)
    .await?;
    let mut found: [i64; 24] = [0; 24];
    for r in &rows {
        let h: i64 = r.try_get("hour")?;
        let s: i64 = r.try_get("starts")?;
        if (0..24).contains(&h) {
            found[h as usize] = s;
        }
    }
    Ok((0..24)
        .map(|h| StatsHourBucket {
            hour: h,
            starts: found[h as usize],
        })
        .collect())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsLibraryBucket {
    pub library_id: i64,
    pub name: String,
    pub kind: String,
    pub starts: i64,
}

/// Top libraries by play count in the window. Movies join through
/// `items.library_id` directly; episodes roll up via
/// `episodes → seasons → items.library_id` (the parent show carries
/// the library reference). UNION-ALL pattern mirrors `top_items_by_plays`.
pub async fn top_libraries_by_plays(
    pool: &SqlitePool,
    days: i64,
    limit: i64,
) -> Result<Vec<StatsLibraryBucket>> {
    let days = days.clamp(1, 365);
    let limit = limit.clamp(1, 50);
    let since = now_ms() - days * 86_400_000;
    let rows = sqlx::query(
        "WITH events AS (
             SELECT i.library_id AS lib_id
             FROM playback_events pe
             JOIN items i ON i.id = pe.item_id
             WHERE pe.event_type = 'start'
               AND pe.occurred_at >= ?
               AND pe.item_id IS NOT NULL
             UNION ALL
             SELECT show.library_id AS lib_id
             FROM playback_events pe
             JOIN episodes ep ON ep.id = pe.episode_id
             JOIN seasons s ON s.id = ep.season_id
             JOIN items show ON show.id = s.show_id
             WHERE pe.event_type = 'start'
               AND pe.occurred_at >= ?
               AND pe.episode_id IS NOT NULL
         )
         SELECT l.id AS library_id, l.name, l.kind, COUNT(*) AS starts
         FROM events
         JOIN libraries l ON l.id = events.lib_id
         GROUP BY l.id
         ORDER BY starts DESC, l.name COLLATE NOCASE ASC
         LIMIT ?",
    )
    .bind(since)
    .bind(since)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(StatsLibraryBucket {
                library_id: r.try_get("library_id")?,
                name: r.try_get("name")?,
                kind: r.try_get("kind")?,
                starts: r.try_get("starts")?,
            })
        })
        .collect()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct StatsPlatformBucket {
    /// Coarse-grained platform name derived from the user-agent
    /// string. The list is intentionally small — operators want "are
    /// they on a phone or a TV", not the full UA — and falls into
    /// "Other" when the UA doesn't match any pattern.
    pub platform: String,
    pub starts: i64,
}

/// Top platforms over the window. The bucketing is purely
/// pattern-matching against the user_agent string; we keep it in SQL
/// (CASE WHEN LIKE) so the dashboard can render a sorted list with
/// one round trip and no per-row UA-parsing logic in Rust.
pub async fn top_platforms(
    pool: &SqlitePool,
    days: i64,
    limit: i64,
) -> Result<Vec<StatsPlatformBucket>> {
    let days = days.clamp(1, 365);
    let limit = limit.clamp(1, 50);
    let since = now_ms() - days * 86_400_000;
    let rows = sqlx::query(
        "SELECT CASE
                  WHEN user_agent IS NULL OR user_agent = '' THEN 'Unknown'
                  WHEN user_agent LIKE '%Android%' THEN 'Android'
                  WHEN user_agent LIKE '%iPhone%' OR user_agent LIKE '%iPad%' THEN 'iOS'
                  WHEN user_agent LIKE '%Mac OS%' AND user_agent NOT LIKE '%Chrome%'
                       AND user_agent NOT LIKE '%Firefox%' THEN 'macOS'
                  WHEN user_agent LIKE '%Tizen%' THEN 'Samsung TV'
                  WHEN user_agent LIKE '%Web0S%' OR user_agent LIKE '%webOS%' THEN 'LG TV'
                  WHEN user_agent LIKE '%AppleTV%' OR user_agent LIKE '%tvOS%' THEN 'Apple TV'
                  WHEN user_agent LIKE '%Roku%' THEN 'Roku'
                  WHEN user_agent LIKE '%Edg/%' THEN 'Edge'
                  WHEN user_agent LIKE '%Firefox%' THEN 'Firefox'
                  WHEN user_agent LIKE '%Chrome%' THEN 'Chrome'
                  WHEN user_agent LIKE '%Safari%' THEN 'Safari'
                  ELSE 'Other'
                END AS platform,
                COUNT(*) AS starts
         FROM playback_events
         WHERE event_type = 'start' AND occurred_at >= ?
         GROUP BY platform
         ORDER BY starts DESC
         LIMIT ?",
    )
    .bind(since)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(StatsPlatformBucket {
                platform: r.try_get("platform")?,
                starts: r.try_get("starts")?,
            })
        })
        .collect()
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
