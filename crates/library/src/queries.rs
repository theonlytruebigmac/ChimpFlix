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
    TmdbCastMember, TmdbCollection, TmdbCollectionStub, TmdbCredits, TmdbCrewMember, TmdbEpisode,
    TmdbMovie, TmdbShow, TmdbVideo, TvMazeShow, tmdb_image_url,
};
use chimpflix_transcoder::ProbeStream;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::models::{
    AuditLogEntry, Credit, Episode, EpisodeDetail, EpisodeListed, Extra, Invite, Item, ItemDetail,
    ItemEdit, ItemFilter, ItemKind, ItemPage, Library, LibraryAgent, LibraryUpdate, ListedItem,
    Marker, MediaFileLocator, MediaFileSummary, MediaStreamSummary, NewAuditEntry, NewLibrary,
    NewOptimizedVersion, NewScheduledTask, NewTranscoderPreset, NewWebhook, OnDeckEntry,
    OnDeckResponse, OptimizedVersion, Person, PlayStateBatch, PlayStateForItem, Review,
    ReviewsSummary, ScanJob, ScheduledTask, ScheduledTaskUpdate, Season, SeasonDetail,
    SeasonSummary, ServerSettings, ServerSettingsUpdate, SessionRow, TaskRun, TranscoderPreset,
    TranscoderPresetUpdate, User, UserRole, UserWithSecret, Webhook, WebhookDelivery, WebhookUpdate,
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

    // Seed the default metadata agent chain. Movies: TMDB only. Shows:
    // TMDB primary, TVMaze fallback. Owners can reorder/disable later via
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
/// of the library IDs in their `library_access` rows; an empty Vec means
/// they're locked out of everything.
pub async fn user_library_filter(
    pool: &SqlitePool,
    user_id: i64,
    role: UserRole,
) -> Result<Option<Vec<i64>>> {
    if matches!(role, UserRole::Owner) {
        return Ok(None);
    }
    let rows = sqlx::query(
        "SELECT library_id FROM library_access WHERE user_id = ? ORDER BY library_id",
    )
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

#[derive(Debug, Default)]
pub struct UserSelfUpdate {
    pub display_name: Option<Option<String>>,
    pub avatar_url: Option<Option<String>>,
    pub default_audio_lang: Option<Option<String>>,
    pub default_subtitle_lang: Option<Option<String>>,
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
    if patch.default_audio_lang.is_some() {
        sets.push("default_audio_lang = ?");
    }
    if patch.default_subtitle_lang.is_some() {
        sets.push("default_subtitle_lang = ?");
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
    if let Some(v) = patch.default_audio_lang {
        q = q.bind(v);
    }
    if let Some(v) = patch.default_subtitle_lang {
        q = q.bind(v);
    }
    q = q.bind(chimpflix_common::now_ms()).bind(user_id);
    let res = q.fetch_optional(pool).await?;
    res.as_ref().map(User::from_row).transpose()
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
#[derive(Debug, Clone, serde::Serialize)]
pub struct AccessMatrixEntry {
    pub user_id: i64,
    pub username: String,
    pub library_id: i64,
    pub library_name: String,
    pub allowed: bool,
}

pub async fn access_matrix(pool: &SqlitePool) -> Result<Vec<AccessMatrixEntry>> {
    let rows = sqlx::query(
        "SELECT u.id AS user_id, u.username,
                l.id AS library_id, l.name AS library_name,
                CASE WHEN la.user_id IS NULL THEN 0 ELSE 1 END AS allowed
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
        out.push(AccessMatrixEntry {
            user_id: r.try_get("user_id")?,
            username: r.try_get("username")?,
            library_id: r.try_get("library_id")?,
            library_name: r.try_get("library_name")?,
            allowed: r.try_get::<i64, _>("allowed")? != 0,
        });
    }
    Ok(out)
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

pub async fn on_deck(
    pool: &SqlitePool,
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<OnDeckResponse> {
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
            if let Some(item) = get_item(pool, iid, user_id, accessible).await? {
                out.push(OnDeckEntry::Movie { item, play_state });
            }
        } else if let Some(eid) = episode_id {
            if let Some(detail) = get_episode_detail(pool, eid, user_id, accessible).await? {
                let show_id = detail.episode.show_id;
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

    sqlx::query(
        "UPDATE items SET
            title = COALESCE(?, title),
            sort_title = COALESCE(?, sort_title),
            original_title = CASE WHEN ?2 IS NOT NULL THEN ? ELSE original_title END,
            summary = CASE WHEN ?2 IS NOT NULL THEN ? ELSE summary END,
            year = COALESCE(?, year),
            tmdb_id = ?,
            imdb_id = COALESCE(?, imdb_id),
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
    pub tmdb_id: i64,
    pub name: String,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
    pub backdrop_path: Option<String>,
    pub item_count: i64,
}

/// All collections that have at least one item in the user's accessible
/// libraries. Ordered alphabetically by name.
pub async fn list_collections(
    pool: &SqlitePool,
    accessible: Option<&[i64]>,
) -> Result<Vec<CollectionRow>> {
    let lib_filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "SELECT c.id, c.tmdb_id, c.name, c.overview, c.poster_path, c.backdrop_path,
                COUNT(i.id) AS item_count
         FROM collections c
         INNER JOIN items i ON i.collection_id = c.id
         WHERE {lib_filter}
         GROUP BY c.id
         HAVING item_count > 0
         ORDER BY c.name COLLATE NOCASE ASC"
    );
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.into_iter()
        .map(|r| {
            Ok(CollectionRow {
                id: r.try_get("id")?,
                tmdb_id: r.try_get("tmdb_id")?,
                name: r.try_get("name")?,
                overview: r.try_get("overview")?,
                poster_path: r.try_get("poster_path")?,
                backdrop_path: r.try_get("backdrop_path")?,
                item_count: r.try_get("item_count")?,
            })
        })
        .collect()
}

pub async fn get_collection(
    pool: &SqlitePool,
    collection_id: i64,
    accessible: Option<&[i64]>,
) -> Result<Option<CollectionRow>> {
    // Only expose the collection if the user can see at least one item in
    // it — otherwise it's effectively a stub for content they don't have.
    let lib_filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "SELECT c.id, c.tmdb_id, c.name, c.overview, c.poster_path, c.backdrop_path,
                COUNT(i.id) AS item_count
         FROM collections c
         LEFT JOIN items i ON i.collection_id = c.id AND {lib_filter}
         WHERE c.id = ?
         GROUP BY c.id"
    );
    let row = sqlx::query(&sql)
        .bind(collection_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let count: i64 = row.try_get("item_count")?;
    if count == 0 {
        return Ok(None);
    }
    Ok(Some(CollectionRow {
        id: row.try_get("id")?,
        tmdb_id: row.try_get("tmdb_id")?,
        name: row.try_get("name")?,
        overview: row.try_get("overview")?,
        poster_path: row.try_get("poster_path")?,
        backdrop_path: row.try_get("backdrop_path")?,
        item_count: count,
    }))
}

pub async fn list_items_in_collection(
    pool: &SqlitePool,
    collection_id: i64,
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<ListedItem>> {
    let lib_filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "{ITEM_SELECT}
         WHERE i.collection_id = ? AND {lib_filter}
         ORDER BY i.year IS NULL, i.year ASC, i.sort_title COLLATE NOCASE ASC"
    );
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

pub async fn list_webhooks(pool: &SqlitePool) -> Result<Vec<Webhook>> {
    let rows = sqlx::query("SELECT * FROM webhooks ORDER BY id ASC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(Webhook::from_row).collect()
}

pub async fn get_webhook(pool: &SqlitePool, id: i64) -> Result<Option<Webhook>> {
    let row = sqlx::query("SELECT * FROM webhooks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(Webhook::from_row).transpose()
}

pub async fn create_webhook(pool: &SqlitePool, input: NewWebhook) -> Result<Webhook> {
    let now = now_ms();
    let mask = serde_json::to_string(&input.event_mask)?;
    let id: i64 = sqlx::query(
        "INSERT INTO webhooks (name, url, secret, event_mask, enabled, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&input.name)
    .bind(&input.url)
    .bind(input.secret.as_deref())
    .bind(&mask)
    .bind(i64::from(input.enabled))
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?
    .try_get("id")?;
    get_webhook(pool, id)
        .await?
        .context("inserted webhook disappeared")
}

pub async fn update_webhook(
    pool: &SqlitePool,
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
        sqlx::query("UPDATE webhooks SET secret = ?, updated_at = ? WHERE id = ?")
            .bind(v.as_deref())
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
    get_webhook(pool, id).await
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
            (kind, name, cron_expr, params_json, enabled, next_run_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&input.kind)
    .bind(&input.name)
    .bind(&input.cron_expr)
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
