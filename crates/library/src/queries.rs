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
    TmdbEpisode, TmdbMovie, TmdbShow, TvMazeShow, TvdbMovie, TvdbShow, tmdb_image_url,
};
use chimpflix_transcoder::ProbeStream;
use serde::Serialize;
use sha2::Digest as _;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};

use crate::models::{
    AccessGroup, AccessGroupDetail, AccessGroupUpdate, AuditLogEntry, Credit, Episode,
    EpisodeDetail, EpisodeListed, ExternalSubtitle, Extra, Invite, Item, ItemDetail, ItemEdit,
    ItemFilter, ItemKind, ItemPage, ItemSort, JobRow, JobStatus, JobSummary, Library, LibraryAgent,
    LibraryUpdate, ListedItem, Marker, MediaFileLocator, MediaFileSummary, MediaStreamSummary,
    NewAccessGroup, NewAuditEntry, NewExternalSubtitle, NewLibrary, NewOptimizedVersion,
    NewScheduledTask, NewTranscoderPreset, NewWebhook, Notification, OnDeckEntry, OnDeckResponse,
    OptimizedVersion, Person, PlayStateBatch, PlayStateForItem, Review, ReviewsSummary, ScanJob,
    ScheduledTask, ScheduledTaskUpdate, Season, SeasonDetail, SeasonSummary, SecretMetadata,
    ServerSettings, ServerSettingsUpdate, SessionRow, ShowWatchStats, TaskRun, TranscoderPreset,
    TranscoderPresetUpdate, User, UserRole, UserWithSecret, Webhook, WebhookDelivery,
    WebhookUpdate, WriteMode, make_sort_title,
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

    // Anime libraries default to TVDB primary; everything else defaults
    // to TMDB. The operator can flip this via PATCH /libraries/{id}.
    let primary_agent = if matches!(input.kind, crate::models::LibraryKind::Anime) {
        "tvdb"
    } else {
        "tmdb"
    };
    let lib_id: i64 = sqlx::query(
        "INSERT INTO libraries
            (name, kind, scan_interval_s, primary_metadata_agent, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(&input.name)
    .bind(input.kind.as_str())
    .bind(scan_interval)
    .bind(primary_agent)
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

    // Seed the default metadata agent chain.
    //   Movies: TMDB primary, then TVDB + OMDb for fill-nulls.
    //   Shows:  TVDB primary, then TMDB + TVMaze + OMDb.
    //   Anime:  TVDB primary (English titles), then AniList for
    //           per-episode coverage + absolute-numbering id, then OMDb.
    // Putting AniList behind TVDB avoids native-Japanese titles
    // overwriting English ones in primary-mode writes, while keeping
    // AniList available for the episodes TVDB doesn't have. Operators
    // can reorder via /admin/libraries/{id}/agents.
    match input.kind {
        crate::models::LibraryKind::Movies => {
            sqlx::query(
                "INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
                 VALUES (?, 'tmdb', 0, 1, '{}'), (?, 'tvdb', 1, 1, '{}'), (?, 'omdb', 2, 1, '{}')",
            )
            .bind(lib_id).bind(lib_id).bind(lib_id)
            .execute(&mut *tx)
            .await?;
        }
        crate::models::LibraryKind::Shows => {
            sqlx::query(
                "INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
                 VALUES (?, 'tvdb', 0, 1, '{}'), (?, 'tmdb', 1, 1, '{}'),
                        (?, 'tvmaze', 2, 1, '{}'), (?, 'omdb', 3, 1, '{}')",
            )
            .bind(lib_id).bind(lib_id).bind(lib_id).bind(lib_id)
            .execute(&mut *tx)
            .await?;
        }
        crate::models::LibraryKind::Anime => {
            sqlx::query(
                "INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
                 VALUES (?, 'tvdb', 0, 1, '{}'), (?, 'anilist', 1, 1, '{}'),
                        (?, 'omdb', 2, 1, '{}')",
            )
            .bind(lib_id).bind(lib_id).bind(lib_id)
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;
    get_library(pool, lib_id)
        .await?
        .context("library disappeared after insert")
}

pub async fn list_libraries(pool: &SqlitePool, accessible: Option<&[i64]>) -> Result<Vec<Library>> {
    let filter = library_filter_sql("id", accessible);
    let sql = format!("SELECT * FROM libraries WHERE {filter} ORDER BY created_at ASC",);
    let rows = sqlx::query(&sql).fetch_all(pool).await?;

    // Pull every library_paths row in a single query and group in
    // memory. The old per-row `library_paths(pool, id).await?` was a
    // classic N+1 — 20 libraries cost 21 round-trips to SQLite, and
    // the cost showed up as a visible delay in the admin /libraries
    // listing on cold-cache page loads.
    let paths_filter = library_filter_sql("library_id", accessible);
    let paths_sql = format!(
        "SELECT library_id, path FROM library_paths WHERE {paths_filter} ORDER BY path ASC",
    );
    let path_rows = sqlx::query(&paths_sql).fetch_all(pool).await?;
    let mut paths_by_lib: HashMap<i64, Vec<String>> = HashMap::new();
    for r in &path_rows {
        let lib_id: i64 = r.try_get("library_id")?;
        let path: String = r.try_get("path")?;
        paths_by_lib.entry(lib_id).or_default().push(path);
    }

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let id: i64 = row.try_get("id")?;
        let paths = paths_by_lib.remove(&id).unwrap_or_default();
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
    if let Some(v) = &update.primary_metadata_agent {
        if v != "tmdb" && v != "tvdb" {
            anyhow::bail!("primary_metadata_agent must be 'tmdb' or 'tvdb'");
        }
        sqlx::query(
            "UPDATE libraries SET primary_metadata_agent = ?, updated_at = ? WHERE id = ?",
        )
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

pub async fn list_library_agents(pool: &SqlitePool, library_id: i64) -> Result<Vec<LibraryAgent>> {
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
    // Periodic counter update racing 8 concurrent process_file writers
    // is a textbook BUSY/517 trigger. The retry wrapper absorbs the
    // common case; the call site in scanner.rs also treats failure as
    // non-fatal so a worst-case timeout never aborts the scan.
    crate::db::with_busy_retry(|| async {
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
    })
    .await
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
    // `succeeded` (not `completed`) matches the convention every other
    // place in the codebase reads — `mark_scan_completed` was an
    // outlier writing `completed` while stats / dashboard / purge
    // queries all filter on `succeeded`. That mismatch is why the
    // admin drawer's "Last scanned" rendered as "never" forever even
    // after successful scans. See the phase84 migration which renames
    // existing rows so historical scans surface correctly too.
    sqlx::query(
        "UPDATE scan_jobs
         SET status = 'succeeded', finished_at = ?,
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
        ps.last_played_at  AS ps_last_played_at,
        (CASE i.kind
            WHEN 'movie' THEN (
                SELECT MAX(height) FROM media_files
                WHERE item_id = i.id AND removed_at IS NULL AND height IS NOT NULL
            )
            WHEN 'show' THEN (
                SELECT MAX(mf.height) FROM media_files mf
                JOIN episodes e ON e.id = mf.episode_id
                JOIN seasons s  ON s.id = e.season_id
                WHERE s.show_id = i.id AND mf.removed_at IS NULL AND mf.height IS NOT NULL
            )
         END) AS best_height,
        (CASE i.kind
            WHEN 'movie' THEN (
                SELECT mf.hdr_format FROM media_files mf
                WHERE mf.item_id = i.id AND mf.removed_at IS NULL AND mf.hdr_format IS NOT NULL
                ORDER BY
                    (mf.hdr_format = 'dolby_vision') DESC,
                    (mf.hdr_format LIKE 'hdr10_plus%') DESC,
                    (mf.hdr_format LIKE 'hdr10%') DESC,
                    (mf.hdr_format = 'hlg') DESC
                LIMIT 1
            )
            WHEN 'show' THEN (
                SELECT mf.hdr_format FROM media_files mf
                JOIN episodes e ON e.id = mf.episode_id
                JOIN seasons s  ON s.id = e.season_id
                WHERE s.show_id = i.id AND mf.removed_at IS NULL AND mf.hdr_format IS NOT NULL
                ORDER BY
                    (mf.hdr_format = 'dolby_vision') DESC,
                    (mf.hdr_format LIKE 'hdr10_plus%') DESC,
                    (mf.hdr_format LIKE 'hdr10%') DESC,
                    (mf.hdr_format = 'hlg') DESC
                LIMIT 1
            )
         END) AS best_hdr_format
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

/// EXISTS clause used by the base [`list_items`] WHERE to hide items
/// whose every backing file is soft-deleted. Without this, titles the
/// scanner has marked `removed_at` linger in the browse grid until the
/// next purge cycle (default 7-day grace), which surfaces as "I
/// deleted those files but they're still in the UI."
///
/// Performance note: the previous shape used
/// `COALESCE(s.show_id, mf.item_id) = i.id` as a unified join. SQLite's
/// planner can't push that into either `idx_media_files_item_active`
/// or `idx_media_files_episode_active` (those partials are keyed on a
/// raw column, not a COALESCE expression), so every items.list COUNT
/// degraded to ~1.2s on a real-size library. The kind-gated form below
/// uses two narrow EXISTS clauses — movie items take only the first,
/// shows take only the second — each able to use its own partial
/// index seek. Verified: COUNT path drops from ~1.2s to <50ms on a
/// 1k-item library.
fn has_active_files_clause() -> &'static str {
    "(\
        (i.kind = 'movie' AND EXISTS ( \
            SELECT 1 FROM media_files mf \
            WHERE mf.item_id = i.id AND mf.removed_at IS NULL \
        )) \
        OR (i.kind = 'show' AND EXISTS ( \
            SELECT 1 FROM media_files mf \
            JOIN episodes e ON e.id = mf.episode_id \
            JOIN seasons s ON s.id = e.season_id \
            WHERE s.show_id = i.id AND mf.removed_at IS NULL \
        )) \
    )"
}

/// Watch-status filters for SHOW rows need to aggregate from
/// episode-level `play_state` rows — there's never a `play_state` with
/// `item_id = <show id>` (the CHECK constraint on play_state enforces
/// exactly one of `item_id` / `episode_id` is set, and progress writes
/// always target the episode for TV/anime). The library-browse
/// "Unwatched / In progress / Watched" chips therefore can't rely on
/// the top-level `LEFT JOIN play_state ps ON ps.item_id = i.id` for
/// shows — that join is uniformly NULL for every show row, which used
/// to send the in_progress filter to "no results" and the unwatched
/// filter to "every show including ones the user has fully watched."
///
/// Each helper returns a self-contained predicate suitable for the
/// `show` branch of an `OR` against the (correct) movie branch that
/// reads from `ps.*` directly. Every helper has exactly one `?` bind
/// for `user_id`, except `in_progress` which has two — keep the bind
/// sequence in `list_items` in sync.
///
/// Semantics chosen to match the Continue Watching rail
/// ([`on_deck`]) and the title-page `show_watch_stats` rollup, so all
/// three surfaces agree on what "watched" / "in progress" mean for a
/// show:
///   * Watched (show) = at least one active episode exists AND every
///     active episode has `watched = 1` (mirrors what makes the
///     show-level "Mark watched" toggle flip on the title page).
///   * In progress (show) = at least one episode has play activity
///     (mid-episode position or a watched episode) AND at least one
///     active episode is still unwatched.
///   * Unwatched (show) = no episode has any play_state activity at
///     all (matches what the user expects when filtering for "haven't
///     touched this yet").
fn show_unwatched_clause() -> &'static str {
    "NOT EXISTS ( \
        SELECT 1 FROM play_state psx \
        JOIN episodes ex ON ex.id = psx.episode_id \
        JOIN seasons sx ON sx.id = ex.season_id \
        WHERE sx.show_id = i.id AND psx.user_id = ? \
          AND (psx.watched = 1 OR psx.position_ms > 0) \
    )"
}

fn show_in_progress_clause() -> &'static str {
    "EXISTS ( \
        SELECT 1 FROM play_state psx \
        JOIN episodes ex ON ex.id = psx.episode_id \
        JOIN seasons sx ON sx.id = ex.season_id \
        WHERE sx.show_id = i.id AND psx.user_id = ? \
          AND (psx.watched = 1 OR psx.position_ms > 0) \
     ) \
     AND EXISTS ( \
        SELECT 1 FROM episodes ey \
        JOIN seasons sy ON sy.id = ey.season_id \
        JOIN media_files mfy ON mfy.episode_id = ey.id AND mfy.removed_at IS NULL \
        LEFT JOIN play_state psy ON psy.episode_id = ey.id AND psy.user_id = ? \
        WHERE sy.show_id = i.id \
          AND (psy.watched IS NULL OR psy.watched = 0) \
     )"
}

fn show_watched_clause() -> &'static str {
    "EXISTS ( \
        SELECT 1 FROM episodes ex \
        JOIN seasons sx ON sx.id = ex.season_id \
        JOIN media_files mfx ON mfx.episode_id = ex.id AND mfx.removed_at IS NULL \
        WHERE sx.show_id = i.id \
     ) \
     AND NOT EXISTS ( \
        SELECT 1 FROM episodes ey \
        JOIN seasons sy ON sy.id = ey.season_id \
        JOIN media_files mfy ON mfy.episode_id = ey.id AND mfy.removed_at IS NULL \
        LEFT JOIN play_state psy ON psy.episode_id = ey.id AND psy.user_id = ? \
        WHERE sy.show_id = i.id \
          AND (psy.watched IS NULL OR psy.watched = 0) \
     )"
}

/// Wrap a per-file WHERE fragment in an EXISTS subquery that resolves
/// the right item for both movies (`mf.item_id`) and TV/anime episodes
/// (`mf.episode_id` → seasons → show item). `removed_at IS NULL` keeps
/// pending-cleanup files out of the match so the browse grid reflects
/// what the user can actually play right now.
fn file_exists_clause(inner: &str) -> String {
    format!(
        "EXISTS (\
            SELECT 1 FROM media_files mf \
            LEFT JOIN episodes e ON e.id = mf.episode_id \
            LEFT JOIN seasons s ON s.id = e.season_id \
            WHERE COALESCE(s.show_id, mf.item_id) = i.id \
              AND mf.removed_at IS NULL \
              AND ({inner}) \
        )"
    )
}

/// Codec filtering needs `media_streams` joined on top of the standard
/// file-exists shape because codec lives per-stream, not per-file.
/// `codec_predicate` is a self-contained, parenthesized predicate that
/// references `ms.codec` directly — built from a validated whitelist,
/// no user-supplied SQL flows through.
fn codec_exists_clause(codec_predicate: &str) -> String {
    format!(
        "EXISTS (\
            SELECT 1 FROM media_files mf \
            JOIN media_streams ms ON ms.media_file_id = mf.id \
            LEFT JOIN episodes e ON e.id = mf.episode_id \
            LEFT JOIN seasons s ON s.id = e.season_id \
            WHERE COALESCE(s.show_id, mf.item_id) = i.id \
              AND mf.removed_at IS NULL \
              AND ms.kind = 'video' \
              AND {codec_predicate} \
        )"
    )
}

/// Translate the resolution bucket enum to a height-range WHERE
/// fragment. Returns `None` when every input is unrecognised (treat as
/// "no filter" rather than rejecting the request — the UI may evolve
/// faster than the wire spec).
fn resolution_height_clause(values: &[String]) -> Option<String> {
    let buckets: Vec<&'static str> = values
        .iter()
        .filter_map(|v| match v.as_str() {
            "sd" => Some("(mf.height IS NOT NULL AND mf.height < 720)"),
            "720" => Some("(mf.height >= 720 AND mf.height < 1080)"),
            "1080" => Some("(mf.height >= 1080 AND mf.height < 2160)"),
            "4k" => Some("(mf.height >= 2160)"),
            _ => None,
        })
        .collect();
    if buckets.is_empty() {
        None
    } else {
        Some(buckets.join(" OR "))
    }
}

/// HDR fragment. `sdr` means `hdr_format IS NULL` (the scanner stores
/// SDR as the absence of a tag); `hdr10` / `hlg` match by literal.
/// Dolby Vision detection is on the roadmap but not yet wired in
/// transcoder/probe.rs, so it's deliberately not exposed.
fn hdr_format_clause(values: &[String]) -> Option<String> {
    let parts: Vec<&'static str> = values
        .iter()
        .filter_map(|v| match v.as_str() {
            "sdr" => Some("(mf.hdr_format IS NULL)"),
            "hdr10" => Some("(mf.hdr_format = 'hdr10')"),
            "hlg" => Some("(mf.hdr_format = 'hlg')"),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" OR "))
    }
}

/// Build the parenthesised codec predicate plugged into
/// [`codec_exists_clause`]. References `ms.codec` directly so it stays
/// self-contained — no operator-precedence surprises when concatenated
/// into the outer AND chain. `other` matches anything not in the known
/// whitelist (covers obscure codecs without exposing every ffprobe
/// name through the UI).
fn codec_in_list(values: &[String]) -> Option<String> {
    const KNOWN: &[&str] = &["hevc", "h264", "av1", "vp9", "mpeg4", "mpeg2video"];
    let mut wanted: Vec<&'static str> = Vec::new();
    let mut wants_other = false;
    for v in values {
        match v.as_str() {
            "hevc" | "h265" => wanted.push("'hevc'"),
            "h264" | "avc" => wanted.push("'h264'"),
            "av1" => wanted.push("'av1'"),
            "vp9" => wanted.push("'vp9'"),
            "mpeg4" => wanted.push("'mpeg4'"),
            "mpeg2" | "mpeg2video" => wanted.push("'mpeg2video'"),
            "other" => wants_other = true,
            _ => {}
        }
    }
    wanted.sort();
    wanted.dedup();
    if wanted.is_empty() && !wants_other {
        return None;
    }
    let codec_expr = "LOWER(COALESCE(ms.codec, ''))";
    if wants_other && wanted.is_empty() {
        let known_csv = KNOWN
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(",");
        return Some(format!("({codec_expr} NOT IN ({known_csv}))"));
    }
    let csv = wanted.join(",");
    if wants_other {
        let known_csv = KNOWN
            .iter()
            .map(|c| format!("'{c}'"))
            .collect::<Vec<_>>()
            .join(",");
        Some(format!(
            "({codec_expr} IN ({csv}) OR {codec_expr} NOT IN ({known_csv}))"
        ))
    } else {
        Some(format!("({codec_expr} IN ({csv}))"))
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
        // Full-text via items_fts. When q is set we INNER JOIN items_fts
        // (see list_sql below) so the MATCH lives directly on the joined
        // virtual table — that's what makes `bm25(items_fts, ...)`
        // available to ORDER BY for relevance ranking. The MATCH query
        // itself is built by [`fts_match_query`] with each token quoted
        // to defang FTS5 operators.
        where_clauses.push("items_fts MATCH ?".to_string());
    }
    if let Some(matched) = filter.auto_matched {
        where_clauses.push(format!(
            "i.auto_matched = {}",
            if matched { "1" } else { "0" }
        ));
    }
    // Per-user watch-status filters. play_state is LEFT JOINed in ITEM_SELECT,
    // but the COUNT query has to JOIN itself — see count_sql below.
    //
    // Movie branch reads `ps.*` from the top-level LEFT JOIN. Show
    // branch can't (play_state is never keyed on a show item_id) — it
    // aggregates from episode-level rows via the helpers above. Each
    // show-side EXISTS introduces its own `?` for user_id; the bind
    // sequence in count_q / list_q below has matching `.bind(user_id)`
    // calls in the same conditional order.
    if filter.unwatched_only.unwrap_or(false) {
        where_clauses.push(format!(
            "((i.kind = 'movie' AND (ps.watched IS NULL OR ps.watched = 0) \
                                AND COALESCE(ps.position_ms, 0) = 0) \
              OR (i.kind = 'show' AND {}))",
            show_unwatched_clause()
        ));
    }
    if filter.in_progress_only.unwrap_or(false) {
        where_clauses.push(format!(
            "((i.kind = 'movie' AND ps.watched = 0 AND ps.position_ms > 0) \
              OR (i.kind = 'show' AND {}))",
            show_in_progress_clause()
        ));
    }
    if filter.watched_only.unwrap_or(false) {
        where_clauses.push(format!(
            "((i.kind = 'movie' AND ps.watched = 1) \
              OR (i.kind = 'show' AND {}))",
            show_watched_clause()
        ));
    }
    if filter.year_min.is_some() {
        where_clauses.push("i.year IS NOT NULL AND i.year >= ?".into());
    }
    if filter.year_max.is_some() {
        where_clauses.push("i.year IS NOT NULL AND i.year <= ?".into());
    }
    // File-attribute filters. All values are validated against a static
    // whitelist below and inlined as SQL literals — no binds — so we don't
    // perturb the carefully-ordered bind sequence for the existing
    // filters. Each becomes an EXISTS over media_files keyed by
    // `COALESCE(s.show_id, mf.item_id) = i.id` so the same shape works
    // for both movies (mf.item_id direct) and shows (via episodes/seasons).
    if let Some(clause) = filter
        .resolutions
        .as_deref()
        .and_then(resolution_height_clause)
    {
        where_clauses.push(file_exists_clause(&clause));
    }
    if let Some(clause) = filter.hdr.as_deref().and_then(hdr_format_clause) {
        where_clauses.push(file_exists_clause(&clause));
    }
    if let Some(clause) = filter.codecs.as_deref().and_then(codec_in_list) {
        where_clauses.push(codec_exists_clause(&clause));
    }
    // Hide ghost items whose every file the scanner already soft-deleted.
    // Always-on — there's no operator surface that needs to see items with
    // zero playable files, and the row sticks around in the DB during the
    // 7-day purge grace anyway (so legitimate transient unmounts still
    // recover when files come back, since the upsert clears removed_at).
    where_clauses.push(has_active_files_clause().to_string());
    where_clauses.push(library_filter_sql("i.library_id", accessible));
    let where_sql = format!("WHERE {}", where_clauses.join(" AND "));

    // Ranking precedence: an explicit `?sort=...` always wins (the user
    // asked for it). With no explicit sort *and* a search query, default
    // to FTS bm25 relevance — searching "Breaking Bad" used to lose to
    // any movie added more recently because the previous default sort was
    // recently_added, which is backwards for search. Column weights tuned
    // for the common case: title=10 outweighs original_title=5 (foreign
    // re-runs of the same show), cast_names=3 outranks summary=1 (people
    // searching an actor name expect the actor's titles first, not films
    // that mention them in passing). bm25 returns negative scores so
    // smaller (more negative) = better match → ASC sorts best-first.
    let order_by = match (&filter.sort, filter.q.is_some()) {
        (Some(s), _) => s.order_by(filter.random_seed),
        (None, true) => "bm25(items_fts, 10.0, 5.0, 1.0, 3.0) ASC".to_string(),
        (None, false) => ItemSort::default().order_by(None),
    };

    // FTS5 path uses an INNER JOIN so the MATCH clause references the
    // virtual table directly + the `bm25()` function works in ORDER BY.
    // The non-search path keeps the simpler shape — no JOIN cost when
    // FTS isn't in play.
    let fts_join = if filter.q.is_some() {
        " JOIN items_fts ON items_fts.rowid = i.id"
    } else {
        ""
    };

    // The list SELECT already LEFT JOINs play_state; the COUNT path needs
    // the same join whenever watch-status filters OR the LastPlayed sort
    // reference `ps.*`. Cheap to always include the join — the user_id
    // bind is added below.
    //
    // MONTH 1 in `docs/PUBLIC_RELEASE_HARDENING.md`: cap the COUNT(*)
    // at 10_000 rows. FTS5 with bm25 ranking over a huge library will
    // happily walk every match for the count; the UI only ever needs
    // "N or 10000+" for pagination, so the saturating cap is a free
    // bound on the worst case.
    let count_sql = format!(
        "SELECT COUNT(*) AS n FROM (SELECT 1 FROM items i{fts_join} \
         LEFT JOIN play_state ps ON ps.item_id = i.id AND ps.user_id = ? \
         {where_sql} LIMIT 10000)"
    );
    let list_sql = if filter.q.is_some() {
        // SELECT mirrors ITEM_SELECT but with the items_fts JOIN so
        // `bm25(items_fts, ...)` is in scope for the ORDER BY clause.
        format!(
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
                (CASE i.kind \
                    WHEN 'movie' THEN (SELECT MAX(height) FROM media_files \
                        WHERE item_id = i.id AND removed_at IS NULL AND height IS NOT NULL) \
                    WHEN 'show' THEN (SELECT MAX(mf.height) FROM media_files mf \
                        JOIN episodes e ON e.id = mf.episode_id \
                        JOIN seasons s  ON s.id = e.season_id \
                        WHERE s.show_id = i.id AND mf.removed_at IS NULL AND mf.height IS NOT NULL) \
                 END) AS best_height, \
                (CASE i.kind \
                    WHEN 'movie' THEN (SELECT mf.hdr_format FROM media_files mf \
                        WHERE mf.item_id = i.id AND mf.removed_at IS NULL AND mf.hdr_format IS NOT NULL \
                        ORDER BY (mf.hdr_format = 'dolby_vision') DESC, \
                                 (mf.hdr_format LIKE 'hdr10_plus%') DESC, \
                                 (mf.hdr_format LIKE 'hdr10%') DESC, \
                                 (mf.hdr_format = 'hlg') DESC LIMIT 1) \
                    WHEN 'show' THEN (SELECT mf.hdr_format FROM media_files mf \
                        JOIN episodes e ON e.id = mf.episode_id \
                        JOIN seasons s  ON s.id = e.season_id \
                        WHERE s.show_id = i.id AND mf.removed_at IS NULL AND mf.hdr_format IS NOT NULL \
                        ORDER BY (mf.hdr_format = 'dolby_vision') DESC, \
                                 (mf.hdr_format LIKE 'hdr10_plus%') DESC, \
                                 (mf.hdr_format LIKE 'hdr10%') DESC, \
                                 (mf.hdr_format = 'hlg') DESC LIMIT 1) \
                 END) AS best_hdr_format \
             FROM items i \
             JOIN items_fts ON items_fts.rowid = i.id \
             LEFT JOIN play_state ps \
                 ON ps.item_id = i.id AND ps.user_id = ? \
             {where_sql} ORDER BY {order_by} LIMIT ? OFFSET ?"
        )
    } else {
        format!("{ITEM_SELECT} {where_sql} ORDER BY {order_by} LIMIT ? OFFSET ?")
    };

    let fts_query = filter.q.as_ref().and_then(|s| fts_match_query(s));

    let mut count_q = sqlx::query(&count_sql).bind(user_id);
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
    // Watch-status user_id binds — one per `?` in the show-side EXISTS
    // helpers, pushed in the same conditional order the WHERE clauses
    // were added above so positional binding stays aligned.
    if filter.unwatched_only.unwrap_or(false) {
        count_q = count_q.bind(user_id);
    }
    if filter.in_progress_only.unwrap_or(false) {
        count_q = count_q.bind(user_id).bind(user_id);
    }
    if filter.watched_only.unwrap_or(false) {
        count_q = count_q.bind(user_id);
    }
    if let Some(ymin) = filter.year_min {
        count_q = count_q.bind(ymin);
    }
    if let Some(ymax) = filter.year_max {
        count_q = count_q.bind(ymax);
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
    if filter.unwatched_only.unwrap_or(false) {
        list_q = list_q.bind(user_id);
    }
    if filter.in_progress_only.unwrap_or(false) {
        list_q = list_q.bind(user_id).bind(user_id);
    }
    if filter.watched_only.unwrap_or(false) {
        list_q = list_q.bind(user_id);
    }
    if let Some(ymin) = filter.year_min {
        list_q = list_q.bind(ymin);
    }
    if let Some(ymax) = filter.year_max {
        list_q = list_q.bind(ymax);
    }
    list_q = list_q.bind(page_size as i64).bind(offset);
    let rows = list_q.fetch_all(pool).await?;

    let items = rows
        .iter()
        .map(|row| -> Result<ListedItem> {
            let (best_quality_height, best_hdr_format) =
                ListedItem::quality_from_columns(row);
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
                best_quality_height,
                best_hdr_format,
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
/// Surface items the user has played, ordered most-recent first.
///
/// `play_state` is keyed by exactly one of `item_id` (movies) or
/// `episode_id` (TV/anime episodes) — never both, enforced by a CHECK
/// constraint. A naïve `JOIN play_state ON ps.item_id = i.id` therefore
/// only surfaces movies, because for shows the play state lives one
/// table down (on episodes). The pre-fix history page was empty for
/// every show the user had watched.
///
/// This query unifies both shapes via a CTE: each row in the inner
/// `played` set is `(item_id, play_state_columns)` where `item_id`
/// resolves to the *show*'s id for episode-keyed play states (via
/// `episodes → seasons → show_id`). A window function picks the
/// most-recent play state per item — for shows, that's the most-recent
/// episode the user touched, which matches what users expect to see in
/// a "continue watching" / "recently watched" feed.
pub async fn list_watch_history(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
    offset: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<ListedItem>> {
    let filter = library_filter_sql("i.library_id", accessible);
    // Note: `?1` / `?2` / `?3` bind by position — SQLite's positional
    // parameters let us reference `user_id` repeatedly without
    // re-binding. The user_id appears in both branches of the UNION
    // (movie + show), so the alternative would be three identical
    // `.bind(user_id)` calls; positional is just less error-prone.
    let sql = format!(
        "WITH effective AS (
            SELECT
                src.item_id AS item_id,
                src.position_ms AS position_ms,
                src.duration_ms AS duration_ms,
                src.watched AS watched,
                src.view_count AS view_count,
                src.last_played_at AS last_played_at,
                row_number() OVER (
                    PARTITION BY src.item_id
                    ORDER BY src.last_played_at DESC
                ) AS rn
            FROM (
                -- Movie-style: item-level play state. `item_id` already
                -- points at the movie's items row.
                SELECT ps.item_id AS item_id,
                       ps.position_ms, ps.duration_ms, ps.watched,
                       ps.view_count, ps.last_played_at
                  FROM play_state ps
                 WHERE ps.user_id = ?1
                   AND ps.item_id IS NOT NULL
                   AND ps.last_played_at IS NOT NULL
                UNION ALL
                -- Show-style: episode-level play state, rolled up to the
                -- show. The position/duration here belong to whichever
                -- episode this row represents; the window function above
                -- picks the most-recent one per show.
                SELECT s.show_id AS item_id,
                       ps.position_ms, ps.duration_ms, ps.watched,
                       ps.view_count, ps.last_played_at
                  FROM play_state ps
                  JOIN episodes e ON e.id = ps.episode_id
                  JOIN seasons s ON s.id = e.season_id
                 WHERE ps.user_id = ?1
                   AND ps.episode_id IS NOT NULL
                   AND ps.last_played_at IS NOT NULL
            ) AS src
        )
        SELECT i.*,
            (SELECT source_url FROM images
                WHERE item_id = i.id AND kind = 'poster'
                ORDER BY is_primary DESC, id ASC LIMIT 1) AS poster_path,
            (SELECT source_url FROM images
                WHERE item_id = i.id AND kind = 'backdrop'
                ORDER BY is_primary DESC, id ASC LIMIT 1) AS backdrop_path,
            eff.position_ms     AS ps_position_ms,
            eff.duration_ms     AS ps_duration_ms,
            eff.watched         AS ps_watched,
            eff.view_count      AS ps_view_count,
            eff.last_played_at  AS ps_last_played_at,
            (CASE i.kind
                WHEN 'movie' THEN (SELECT MAX(height) FROM media_files
                    WHERE item_id = i.id AND removed_at IS NULL AND height IS NOT NULL)
                WHEN 'show' THEN (SELECT MAX(mf.height) FROM media_files mf
                    JOIN episodes e ON e.id = mf.episode_id
                    JOIN seasons s  ON s.id = e.season_id
                    WHERE s.show_id = i.id AND mf.removed_at IS NULL AND mf.height IS NOT NULL)
             END) AS best_height,
            (CASE i.kind
                WHEN 'movie' THEN (SELECT mf.hdr_format FROM media_files mf
                    WHERE mf.item_id = i.id AND mf.removed_at IS NULL AND mf.hdr_format IS NOT NULL
                    ORDER BY (mf.hdr_format = 'dolby_vision') DESC,
                             (mf.hdr_format LIKE 'hdr10_plus%') DESC,
                             (mf.hdr_format LIKE 'hdr10%') DESC,
                             (mf.hdr_format = 'hlg') DESC LIMIT 1)
                WHEN 'show' THEN (SELECT mf.hdr_format FROM media_files mf
                    JOIN episodes e ON e.id = mf.episode_id
                    JOIN seasons s  ON s.id = e.season_id
                    WHERE s.show_id = i.id AND mf.removed_at IS NULL AND mf.hdr_format IS NOT NULL
                    ORDER BY (mf.hdr_format = 'dolby_vision') DESC,
                             (mf.hdr_format LIKE 'hdr10_plus%') DESC,
                             (mf.hdr_format LIKE 'hdr10%') DESC,
                             (mf.hdr_format = 'hlg') DESC LIMIT 1)
             END) AS best_hdr_format
          FROM effective eff
          JOIN items i ON i.id = eff.item_id
         WHERE eff.rn = 1 AND {filter}
         ORDER BY eff.last_played_at DESC
         LIMIT ?2 OFFSET ?3",
    );
    let rows = sqlx::query(&sql)
        .bind(user_id)
        .bind(limit)
        .bind(offset.max(0))
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| -> Result<ListedItem> {
            let (best_quality_height, best_hdr_format) =
                ListedItem::quality_from_columns(row);
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
                best_quality_height,
                best_hdr_format,
            })
        })
        .collect()
}

/// Total number of distinct items in the user's watch history,
/// honouring the same library-access filter as `list_watch_history`.
/// Drives the pagination footer on `/history` so the user can see
/// "1–60 of 240 titles" rather than wondering whether there's more.
pub async fn count_watch_history(
    pool: &SqlitePool,
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<i64> {
    let filter = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "WITH effective AS (
            SELECT
                src.item_id AS item_id,
                row_number() OVER (
                    PARTITION BY src.item_id
                    ORDER BY src.last_played_at DESC
                ) AS rn
            FROM (
                SELECT ps.item_id AS item_id, ps.last_played_at
                  FROM play_state ps
                 WHERE ps.user_id = ?1
                   AND ps.item_id IS NOT NULL
                   AND ps.last_played_at IS NOT NULL
                UNION ALL
                SELECT s.show_id AS item_id, ps.last_played_at
                  FROM play_state ps
                  JOIN episodes e ON e.id = ps.episode_id
                  JOIN seasons s ON s.id = e.season_id
                 WHERE ps.user_id = ?1
                   AND ps.episode_id IS NOT NULL
                   AND ps.last_played_at IS NOT NULL
            ) AS src
        )
        SELECT COUNT(*) AS n
          FROM effective eff
          JOIN items i ON i.id = eff.item_id
         WHERE eff.rn = 1 AND {filter}",
    );
    let row = sqlx::query(&sql).bind(user_id).fetch_one(pool).await?;
    Ok(row.try_get::<i64, _>("n")?)
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
        Some([]) => format!("{column} = 0"),
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
pub async fn media_file_library_id(pool: &SqlitePool, file_id: i64) -> Result<Option<i64>> {
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

/// Owning library for an item — straight lookup, used by the access
/// helper to enforce library_access on item-scoped endpoints.
pub async fn item_library_id(pool: &SqlitePool, item_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query("SELECT library_id FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.try_get("library_id").ok()))
}

/// Owning library for an episode — walks episode → season → show → library.
pub async fn episode_library_id(pool: &SqlitePool, episode_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT i.library_id AS library_id
           FROM episodes e
           JOIN seasons s ON s.id = e.season_id
           JOIN items i ON i.id = s.show_id
           WHERE e.id = ?",
    )
    .bind(episode_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.try_get("library_id").ok()))
}

/// Owning library for an external subtitle row — derived from whichever
/// of item_id / episode_id is set on the row.
pub async fn external_subtitle_library_id(pool: &SqlitePool, sub_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query(
        "SELECT COALESCE(
             (SELECT library_id FROM items WHERE id = es.item_id),
             (SELECT i.library_id FROM seasons s
                JOIN items i ON i.id = s.show_id
                JOIN episodes e ON e.season_id = s.id
                WHERE e.id = es.episode_id)
         ) AS library_id
         FROM external_subtitles es WHERE es.id = ?",
    )
    .bind(sub_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|r| r.try_get("library_id").ok()))
}

/// User IDs that currently have access to the given library.
pub async fn list_library_user_ids(pool: &SqlitePool, library_id: i64) -> Result<Vec<i64>> {
    let rows =
        sqlx::query("SELECT user_id FROM library_access WHERE library_id = ? ORDER BY user_id")
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
    let owners: Vec<i64> = sqlx::query("SELECT id FROM users WHERE role = 'owner'")
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

pub async fn list_hidden_libraries(pool: &SqlitePool, user_id: i64) -> Result<Vec<i64>> {
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
            let (best_quality_height, best_hdr_format) =
                ListedItem::quality_from_columns(row);
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
                best_quality_height,
                best_hdr_format,
            })
        })
        .collect()
}

pub async fn add_to_my_list(pool: &SqlitePool, user_id: i64, item_id: i64) -> Result<()> {
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

pub async fn remove_from_my_list(pool: &SqlitePool, user_id: i64, item_id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM user_my_list WHERE user_id = ? AND item_id = ?")
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
    let placeholders = std::iter::repeat_n("?", tmdb_ids.len())
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
            let (best_quality_height, best_hdr_format) =
                ListedItem::quality_from_columns(row);
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
                best_quality_height,
                best_hdr_format,
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
    // Hybrid "Top 10": TMDB global weekly trending ranks first, then we
    // top up to `limit` from local signals so the rail isn't bare when
    // the library doesn't overlap TMDB much. Tie-breakers, in order:
    //   1. tc.rank ASC (TMDB matches keep their global order)
    //   2. play_count DESC (server-wide; episodes roll up to their show)
    //   3. ps.last_played_at DESC (per-user; nudges things this viewer
    //      has actually engaged with)
    //   4. i.added_at DESC (final fallback so the slots fill predictably
    //      with whatever was last imported)
    // Rank is returned as the row position (1..N) rather than tc.rank,
    // since fallback items have no tc.rank and the frontend renumbers
    // anyway.
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
            (CASE i.kind \
                WHEN 'movie' THEN (SELECT MAX(height) FROM media_files \
                    WHERE item_id = i.id AND removed_at IS NULL AND height IS NOT NULL) \
                WHEN 'show' THEN (SELECT MAX(mf.height) FROM media_files mf \
                    JOIN episodes e ON e.id = mf.episode_id \
                    JOIN seasons s  ON s.id = e.season_id \
                    WHERE s.show_id = i.id AND mf.removed_at IS NULL AND mf.height IS NOT NULL) \
             END) AS best_height, \
            (CASE i.kind \
                WHEN 'movie' THEN (SELECT mf.hdr_format FROM media_files mf \
                    WHERE mf.item_id = i.id AND mf.removed_at IS NULL AND mf.hdr_format IS NOT NULL \
                    ORDER BY (mf.hdr_format = 'dolby_vision') DESC, \
                             (mf.hdr_format LIKE 'hdr10_plus%') DESC, \
                             (mf.hdr_format LIKE 'hdr10%') DESC, \
                             (mf.hdr_format = 'hlg') DESC LIMIT 1) \
                WHEN 'show' THEN (SELECT mf.hdr_format FROM media_files mf \
                    JOIN episodes e ON e.id = mf.episode_id \
                    JOIN seasons s  ON s.id = e.season_id \
                    WHERE s.show_id = i.id AND mf.removed_at IS NULL AND mf.hdr_format IS NOT NULL \
                    ORDER BY (mf.hdr_format = 'dolby_vision') DESC, \
                             (mf.hdr_format LIKE 'hdr10_plus%') DESC, \
                             (mf.hdr_format LIKE 'hdr10%') DESC, \
                             (mf.hdr_format = 'hlg') DESC LIMIT 1) \
             END) AS best_hdr_format \
         FROM items i \
         LEFT JOIN trending_cache tc \
           ON tc.tmdb_id = i.tmdb_id \
          AND tc.source = 'tmdb' \
          AND tc.media_kind = ? \
         LEFT JOIN play_state ps \
           ON ps.item_id = i.id AND ps.user_id = ? \
         LEFT JOIN ( \
            SELECT i2.id AS item_id, COUNT(*) AS play_count, MAX(occurred_at) AS last_at FROM ( \
                SELECT pe.item_id AS rolled_id, pe.occurred_at \
                FROM playback_events pe \
                WHERE pe.event_type = 'start' AND pe.item_id IS NOT NULL \
                UNION ALL \
                SELECT s.show_id AS rolled_id, pe.occurred_at \
                FROM playback_events pe \
                JOIN episodes ep ON ep.id = pe.episode_id \
                JOIN seasons s ON s.id = ep.season_id \
                WHERE pe.event_type = 'start' AND pe.episode_id IS NOT NULL \
            ) ev JOIN items i2 ON i2.id = ev.rolled_id \
            GROUP BY i2.id \
         ) pop ON pop.item_id = i.id \
         WHERE i.kind = ? AND {lib_filter} \
         ORDER BY (tc.rank IS NULL), tc.rank ASC, \
                  COALESCE(pop.play_count, 0) DESC, \
                  COALESCE(ps.last_played_at, 0) DESC, \
                  i.added_at DESC \
         LIMIT ?",
    );
    let rows = sqlx::query(&sql)
        .bind(kind.as_str()) // tc.media_kind
        .bind(user_id) // ps.user_id
        .bind(kind.as_str()) // i.kind
        .bind(limit)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .enumerate()
        .map(|(idx, row)| -> Result<(i64, ListedItem)> {
            let (best_quality_height, best_hdr_format) =
                ListedItem::quality_from_columns(row);
            Ok((
                (idx as i64) + 1,
                ListedItem {
                    item: Item::from_row(row)?,
                    play_state: PlayStateForItem::from_columns(row)?,
                    best_quality_height,
                    best_hdr_format,
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
    let reviews = reviews_summary_for_item(pool, id).await.unwrap_or_default();
    let watch_stats = match item.kind {
        ItemKind::Show => Some(show_watch_stats(pool, id, user_id).await?),
        ItemKind::Movie => None,
    };

    Ok(Some(ItemDetail {
        item,
        genres,
        play_state,
        files,
        seasons,
        credits,
        extras,
        reviews,
        watch_stats,
    }))
}

/// Returns total + watched episode counts for a show, for the given
/// user. Used by the show-level Mark-watched toggle to decide which
/// action label to render.
async fn show_watch_stats(pool: &SqlitePool, show_id: i64, user_id: i64) -> Result<ShowWatchStats> {
    let row = sqlx::query(
        "SELECT
            COUNT(*) AS total,
            COUNT(CASE WHEN ps.watched = 1 THEN 1 END) AS watched
         FROM episodes e
         JOIN seasons s ON s.id = e.season_id
         LEFT JOIN play_state ps
             ON ps.episode_id = e.id AND ps.user_id = ?
         WHERE s.show_id = ?",
    )
    .bind(user_id)
    .bind(show_id)
    .fetch_one(pool)
    .await?;
    Ok(ShowWatchStats {
        total_episodes: row.try_get("total")?,
        watched_episodes: row.try_get("watched")?,
    })
}

/// Fetch a single person's full profile by local id. Returns None when
/// the row doesn't exist (route should 404). Person rows survive even
/// after every item they're credited on is removed, so this isn't
/// gated on the cast being non-empty — the empty filmography is the
/// caller's signal to show "no titles in your library".
pub async fn get_person(pool: &SqlitePool, id: i64) -> Result<Option<Person>> {
    let row = sqlx::query(
        "SELECT id, name, tmdb_id, imdb_id, photo_url, \
                biography, birthday, deathday, place_of_birth, \
                known_for_department \
         FROM people \
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    Ok(Some(Person {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        tmdb_id: row.try_get("tmdb_id")?,
        imdb_id: row.try_get("imdb_id")?,
        photo_url: row.try_get("photo_url")?,
        biography: row.try_get("biography")?,
        birthday: row.try_get("birthday")?,
        deathday: row.try_get("deathday")?,
        place_of_birth: row.try_get("place_of_birth")?,
        known_for_department: row.try_get("known_for_department")?,
    }))
}

/// All items the user can access that credit this person. Movies +
/// shows; for shows, the credit lives on `item_credits` (series-level)
/// rather than `episode_credits` (per-episode guest appearances) so
/// this is the natural "what is this person known for in my library"
/// surface. Caller renders empty result as "no titles in your
/// library."
pub async fn list_items_for_person(
    pool: &SqlitePool,
    person_id: i64,
    user_id: i64,
    accessible: Option<&[i64]>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ListedItem>> {
    let lib_filter = library_filter_sql("i.library_id", accessible);
    let active_files = has_active_files_clause();
    // Most-recent first by year; sort_title alphabetic tiebreaker so
    // multi-year overlaps (re-releases, restorations) don't jitter.
    // LIMIT / OFFSET added MONTH 1 in `docs/PUBLIC_RELEASE_HARDENING.md`
    // — a prolific actor (500+ credits) used to serialize the full
    // set on every request.
    let limit = limit.clamp(1, 200);
    let offset = offset.max(0);
    let sql = format!(
        "{ITEM_SELECT} \
         WHERE EXISTS (SELECT 1 FROM item_credits ic \
                       WHERE ic.item_id = i.id AND ic.person_id = ?) \
           AND {active_files} \
           AND {lib_filter} \
         ORDER BY i.year IS NULL, i.year DESC, \
                  i.sort_title COLLATE NOCASE ASC, i.id ASC \
         LIMIT ? OFFSET ?"
    );
    let rows = sqlx::query(&sql)
        .bind(user_id)
        .bind(person_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    rows.iter()
        .map(|row| -> Result<ListedItem> {
            let (best_quality_height, best_hdr_format) =
                ListedItem::quality_from_columns(row);
            Ok(ListedItem {
                item: Item::from_row(row)?,
                play_state: PlayStateForItem::from_columns(row)?,
                best_quality_height,
                best_hdr_format,
            })
        })
        .collect::<Result<Vec<_>>>()
}

/// Count of items credited to `person_id` and visible to `user_id`.
/// Used by the filmography page to render "showing N of M" + the
/// pagination footer.
pub async fn count_items_for_person(
    pool: &SqlitePool,
    person_id: i64,
    accessible: Option<&[i64]>,
) -> Result<i64> {
    let lib_filter = library_filter_sql("i.library_id", accessible);
    let active_files = has_active_files_clause();
    let sql = format!(
        "SELECT COUNT(*) AS n FROM items i \
         WHERE EXISTS (SELECT 1 FROM item_credits ic \
                       WHERE ic.item_id = i.id AND ic.person_id = ?) \
           AND {active_files} \
           AND {lib_filter}"
    );
    let row = sqlx::query(&sql).bind(person_id).fetch_one(pool).await?;
    Ok(row.try_get("n").unwrap_or(0))
}

/// Fetch full [`ListedItem`]s for a known list of item ids, preserving
/// the input order. Inaccessible / unknown ids are silently dropped.
/// Used by the Trakt recommendations rail to hydrate the matched
/// local items with play_state in one query.
pub async fn list_items_by_ids(
    pool: &SqlitePool,
    ids: &[i64],
    user_id: i64,
    accessible: Option<&[i64]>,
) -> Result<Vec<ListedItem>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let lib_filter = library_filter_sql("i.library_id", accessible);
    // IDs come from trusted server-side lookups, never user input, so
    // direct interpolation is safe (mirrors `library_filter_sql`).
    let id_list = ids
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let active_files = has_active_files_clause();
    let sql = format!(
        "{ITEM_SELECT} \
         WHERE i.id IN ({id_list}) \
           AND {active_files} \
           AND {lib_filter}"
    );
    let rows = sqlx::query(&sql).bind(user_id).fetch_all(pool).await?;
    // Re-order to match input. Trakt recommendation order is
    // signal-bearing (most-recommended first) so preserving it
    // matters for the rail.
    let mut by_id: std::collections::HashMap<i64, ListedItem> = std::collections::HashMap::new();
    for row in &rows {
        let item = Item::from_row(row)?;
        let play_state = PlayStateForItem::from_columns(row)?;
        let (best_quality_height, best_hdr_format) =
            ListedItem::quality_from_columns(row);
        by_id.insert(
            item.id,
            ListedItem {
                item,
                play_state,
                best_quality_height,
                best_hdr_format,
            },
        );
    }
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
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
    let Some(row) = sqlx::query(&sql).bind(id).fetch_optional(pool).await? else {
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

pub async fn list_markers_for_file(pool: &SqlitePool, media_file_id: i64) -> Result<Vec<Marker>> {
    let rows = sqlx::query(
        "SELECT kind, start_ms, end_ms, label, source FROM markers \
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
                source: r.try_get("source")?,
            })
        })
        .collect()
}

/// Replace previously auto-detected markers for this file with
/// `new_markers`. Each row carries its own `source` so the operator
/// UI can distinguish between detection methods:
///
/// * `embedded` — container chapter title matched an intro/credits
///   pattern (highest confidence, no audio decode ran).
/// * `tacet` — audio fingerprint match against the season's
///   reference set.
/// * `blackframe` — fallback heuristic for credits when no other
///   signal is available.
///
/// Operator-edited rows (`source = 'manual'`) are preserved. The
/// DELETE clause covers every known auto source plus the legacy
/// `'auto'` value so historical rows from before phase 71 are
/// also replaced on the next pass.
pub async fn replace_detected_markers(
    pool: &SqlitePool,
    media_file_id: i64,
    new_markers: &[(String, i64, i64, String)],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "DELETE FROM markers WHERE media_file_id = ? \
         AND source IN ('auto', 'embedded', 'tacet', 'blackframe')",
    )
    .bind(media_file_id)
    .execute(&mut *tx)
    .await?;
    for (kind, start_ms, end_ms, source) in new_markers {
        sqlx::query(
            "INSERT INTO markers (media_file_id, kind, start_ms, end_ms, source) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(media_file_id)
        .bind(kind)
        .bind(start_ms)
        .bind(end_ms)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Full markers row including id + source. Used by the operator-side
/// marker editor; the player surface uses the slimmer [`Marker`]
/// (without id/source) because client code never edits markers.
#[derive(Debug, Clone, Serialize)]
pub struct MarkerRow {
    pub id: i64,
    pub kind: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub label: Option<String>,
    pub source: String,
}

/// List every marker on a media file with row ids + source. Sorted by
/// start_ms so the editor renders them in playback order.
pub async fn list_markers_full(pool: &SqlitePool, media_file_id: i64) -> Result<Vec<MarkerRow>> {
    let rows = sqlx::query(
        "SELECT id, kind, start_ms, end_ms, label, source FROM markers \
         WHERE media_file_id = ? ORDER BY start_ms",
    )
    .bind(media_file_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(MarkerRow {
                id: r.try_get::<i64, _>("id")?,
                kind: r.try_get::<String, _>("kind")?,
                start_ms: r.try_get::<i64, _>("start_ms")?,
                end_ms: r.try_get::<i64, _>("end_ms")?,
                label: r.try_get::<Option<String>, _>("label").ok().flatten(),
                source: r.try_get::<String, _>("source")?,
            })
        })
        .collect()
}

/// Resolve the parent show id for a media file. Returns `None` for
/// files that belong to a movie (no show), or when the file row /
/// linked episode / season can't be found. Used by the
/// detect_markers + capture pipelines to scope fingerprints to a
/// show.
pub async fn show_id_for_media_file(pool: &SqlitePool, media_file_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT s.show_id
         FROM media_files mf
         JOIN episodes e ON e.id = mf.episode_id
         JOIN seasons s ON s.id = e.season_id
         WHERE mf.id = ?",
    )
    .bind(media_file_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.flatten())
}

/// Resolve `(show_id, season_id)` for a media file. Symmetric to
/// [`show_id_for_media_file`] but also returns the season — used by
/// the capture path so per-season fingerprints can be written when
/// the operator wants seasonal granularity (future polish; the
/// current capture writes show-wide).
pub async fn show_and_season_for_media_file(
    pool: &SqlitePool,
    media_file_id: i64,
) -> Result<Option<(i64, i64)>> {
    let row = sqlx::query(
        "SELECT s.show_id, s.id AS season_id
         FROM media_files mf
         JOIN episodes e ON e.id = mf.episode_id
         JOIN seasons s ON s.id = e.season_id
         WHERE mf.id = ?",
    )
    .bind(media_file_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some((row.try_get("show_id")?, row.try_get("season_id")?)))
}

/// Replace every manual marker on a media file with `new_markers`.
/// Auto-detected rows (any non-manual source) are preserved so a
/// re-run of the detection task overlaps cleanly with the
/// operator's edits. Symmetric to [`replace_detected_markers`].
pub async fn replace_manual_markers(
    pool: &SqlitePool,
    media_file_id: i64,
    new_markers: &[(String, i64, i64, Option<String>)],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM markers WHERE media_file_id = ? AND source = 'manual'")
        .bind(media_file_id)
        .execute(&mut *tx)
        .await?;
    for (kind, start_ms, end_ms, label) in new_markers {
        sqlx::query(
            "INSERT INTO markers (media_file_id, kind, start_ms, end_ms, label, source) \
             VALUES (?, ?, ?, ?, ?, 'manual')",
        )
        .bind(media_file_id)
        .bind(kind)
        .bind(start_ms)
        .bind(end_ms)
        .bind(label.as_deref())
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
/// detection pass). Operator-triggered re-detection still uses
/// `list_media_files_in_library` and overwrites via
/// `replace_detected_markers`.
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
               SELECT 1 FROM markers WHERE media_file_id = mf.id AND source != 'manual'
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

pub async fn set_user_role(pool: &SqlitePool, id: i64, role: UserRole) -> Result<Option<User>> {
    // Same last-owner guard for demotion. Promoting to owner has no
    // such constraint — extra owners are fine.
    if !matches!(role, UserRole::Owner) {
        let current = current_user_role(pool, id).await?;
        if matches!(current, Some(UserRole::Owner)) && count_owners(pool).await? <= 1 {
            anyhow::bail!("cannot demote the last owner — promote another user to owner first");
        }
    }
    let res = sqlx::query("UPDATE users SET role = ?, updated_at = ? WHERE id = ? RETURNING *")
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
    pub subtitle_font_size_px: Option<Option<i64>>,
    pub subtitle_text_color: Option<Option<String>>,
    pub subtitle_background_color: Option<Option<String>>,
    pub subtitle_font_family: Option<Option<String>>,
    pub subtitle_edge: Option<Option<String>>,
    pub subtitle_bottom_inset_pct: Option<Option<i64>>,
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
    if patch.subtitle_font_size_px.is_some() {
        sets.push("subtitle_font_size_px = ?");
    }
    if patch.subtitle_text_color.is_some() {
        sets.push("subtitle_text_color = ?");
    }
    if patch.subtitle_background_color.is_some() {
        sets.push("subtitle_background_color = ?");
    }
    if patch.subtitle_font_family.is_some() {
        sets.push("subtitle_font_family = ?");
    }
    if patch.subtitle_edge.is_some() {
        sets.push("subtitle_edge = ?");
    }
    if patch.subtitle_bottom_inset_pct.is_some() {
        sets.push("subtitle_bottom_inset_pct = ?");
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
    if let Some(v) = patch.subtitle_font_size_px {
        q = q.bind(v);
    }
    if let Some(v) = patch.subtitle_text_color {
        q = q.bind(v);
    }
    if let Some(v) = patch.subtitle_background_color {
        q = q.bind(v);
    }
    if let Some(v) = patch.subtitle_font_family {
        q = q.bind(v);
    }
    if let Some(v) = patch.subtitle_edge {
        q = q.bind(v);
    }
    if let Some(v) = patch.subtitle_bottom_inset_pct {
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
/// Consume an email-change token. Returns the user's previous email
/// (if any) so the caller can send a heads-up to that address — "your
/// email was just changed to <new>, if this wasn't you, contact your
/// admin." Without that notification an attacker who briefly had a
/// session (via XSS or a stolen cookie) can silently re-bind the
/// account email to one they control and then trigger a password
/// reset, completing account takeover with the real owner none the
/// wiser.
///
/// Also invalidates every OTHER pending email-change token for the
/// same user — a single confirmed change supersedes any
/// concurrently-issued requests.
pub async fn consume_email_change(
    pool: &SqlitePool,
    token_id: i64,
    user_id: i64,
    new_email: &str,
) -> Result<Option<String>> {
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
    // Snapshot the previous email BEFORE the UPDATE so the caller can
    // notify that address.
    let prior: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
    sqlx::query("UPDATE users SET email = ?, updated_at = ? WHERE id = ?")
        .bind(new_email)
        .bind(now)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    // Burn every other pending email-change token for this user.
    sqlx::query(
        "UPDATE email_change_tokens
            SET consumed_at = ?
          WHERE user_id = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(prior)
}

/// Replace the user's password hash. Caller is responsible for hashing
/// the plaintext (via the argon2 helper) and for any session-rotation
/// that should follow. We bump `updated_at` so the touch_audit pattern
/// still works.
pub async fn update_user_password(pool: &SqlitePool, user_id: i64, new_hash: &str) -> Result<bool> {
    let now = now_ms();
    let res = sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
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
pub async fn find_user_by_email(pool: &SqlitePool, email: &str) -> Result<Option<User>> {
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
    let Some(row) = sqlx::query("SELECT * FROM users WHERE role = 'owner' ORDER BY id ASC LIMIT 1")
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
    // Store SHA-256 of the cookie nonce, NOT the raw nonce. The cookie
    // still carries the raw 32 bytes; only the hash lives at rest. A
    // stolen DB no longer hands the attacker a working session cookie.
    let nonce_hash = sha2::Sha256::digest(&nonce[..]);
    let row = sqlx::query(
        "INSERT INTO sessions
            (user_id, nonce, user_agent, ip, last_seen_at, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id",
    )
    .bind(user_id)
    .bind(nonce_hash.as_slice())
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
        anyhow::bail!("corrupt session nonce-hash length: {}", nonce_blob.len());
    }
    let mut nonce_hash = [0u8; 32];
    nonce_hash.copy_from_slice(&nonce_blob);
    Ok(Some(SessionRow {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        nonce_hash,
        expires_at: row.try_get("expires_at")?,
        last_seen_at: row.try_get("last_seen_at")?,
        created_at: row.try_get("created_at")?,
    }))
}

/// Hash a cookie-supplied nonce the same way `create_session` does, so
/// the extractor can verify `sha256(cookie_nonce) == session.nonce_hash`.
pub fn hash_session_nonce(nonce: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(sha2::Sha256::digest(&nonce[..]).as_slice());
    out
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
    // Invalidate every OTHER pending password-reset token (a user
    // resetting because they suspect compromise shouldn't leave a
    // stolen sibling token live) and every pending email-change token
    // (an attacker who captured an email-change token before the
    // reset shouldn't be able to complete it after).
    sqlx::query(
        "UPDATE password_reset_tokens
            SET consumed_at = ?
          WHERE user_id = ? AND consumed_at IS NULL",
    )
    .bind(now)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE email_change_tokens
            SET consumed_at = ?
          WHERE user_id = ? AND consumed_at IS NULL",
    )
    .bind(now)
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

/// Same shape as `cleanup_expired_password_reset_tokens` for the
/// email-change flow. Both tables grow on every probe / abandoned
/// request; the daily retention task sweeps them together.
pub async fn cleanup_expired_email_change_tokens(pool: &SqlitePool) -> Result<u64> {
    let now = now_ms();
    let res = sqlx::query("DELETE FROM email_change_tokens WHERE expires_at < ?")
        .bind(now)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Trim audit_log to a retention window. Defaults applied at the
/// caller (currently 90 days). Returns the row count removed.
pub async fn cleanup_old_audit_log(pool: &SqlitePool, older_than_ms: i64) -> Result<u64> {
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

pub async fn get_user_totp(pool: &SqlitePool, user_id: i64) -> Result<Option<UserTotpRecord>> {
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
        secret_nonce: row
            .try_get::<Option<Vec<u8>>, _>("secret_nonce")
            .ok()
            .flatten(),
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
    offset: i64,
) -> Result<Vec<Notification>> {
    let rows = sqlx::query(
        "SELECT * FROM notifications
          WHERE user_id = ?
          ORDER BY created_at DESC
          LIMIT ? OFFSET ?",
    )
    .bind(user_id)
    .bind(limit.clamp(1, 200))
    .bind(offset.max(0))
    .fetch_all(pool)
    .await?;
    rows.iter().map(Notification::from_row).collect()
}

pub async fn count_notifications(pool: &SqlitePool, user_id: i64) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM notifications WHERE user_id = ?")
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(row.try_get("n")?)
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
pub async fn record_user_login(pool: &SqlitePool, user_id: i64, ip: Option<&str>) -> Result<()> {
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

pub async fn mark_all_notifications_read(pool: &SqlitePool, user_id: i64) -> Result<u64> {
    let now = now_ms();
    let res =
        sqlx::query("UPDATE notifications SET read_at = ? WHERE user_id = ? AND read_at IS NULL")
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
    /// Role of the user the session belongs to. Surfaced so admin
    /// callers can filter out Owner-tier rows when the actor is only
    /// a (non-Owner) Admin — see `list_all_sessions_for_actor`.
    pub user_role: crate::models::UserRole,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub last_seen_at: i64,
    pub expires_at: i64,
    pub created_at: i64,
}

pub async fn list_all_sessions(pool: &SqlitePool) -> Result<Vec<SessionSummary>> {
    list_all_sessions_filtered(pool, false).await
}

/// List every live session. `exclude_owners=true` drops sessions
/// belonging to Owner-role users — used when a non-Owner Admin
/// requests `/admin/sessions`, so they can't see Owner IPs / UAs
/// (a privilege boundary the audit flagged).
pub async fn list_all_sessions_filtered(
    pool: &SqlitePool,
    exclude_owners: bool,
) -> Result<Vec<SessionSummary>> {
    let now = now_ms();
    let where_role = if exclude_owners {
        "AND u.role != 'owner'"
    } else {
        ""
    };
    let sql = format!(
        "SELECT s.id, s.user_id, u.username, u.role AS user_role,
                s.user_agent, s.ip,
                s.last_seen_at, s.expires_at, s.created_at
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.expires_at >= ? {where_role}
         ORDER BY s.last_seen_at DESC",
    );
    let rows = sqlx::query(&sql).bind(now).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let role_str: String = r.try_get("user_role")?;
        let user_role =
            crate::models::UserRole::from_db(&role_str).unwrap_or(crate::models::UserRole::User);
        out.push(SessionSummary {
            id: r.try_get("id")?,
            user_id: r.try_get("user_id")?,
            username: r.try_get("username")?,
            user_role,
            user_agent: r.try_get::<Option<String>, _>("user_agent").ok().flatten(),
            ip: r.try_get::<Option<String>, _>("ip").ok().flatten(),
            last_seen_at: r.try_get("last_seen_at")?,
            expires_at: r.try_get("expires_at")?,
            created_at: r.try_get("created_at")?,
        });
    }
    Ok(out)
}

pub async fn list_user_sessions(pool: &SqlitePool, user_id: i64) -> Result<Vec<SessionSummary>> {
    let now = now_ms();
    let rows = sqlx::query(
        "SELECT s.id, s.user_id, u.username, u.role AS user_role,
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
        let role_str: String = r.try_get("user_role")?;
        let user_role =
            crate::models::UserRole::from_db(&role_str).unwrap_or(crate::models::UserRole::User);
        out.push(SessionSummary {
            id: r.try_get("id")?,
            user_id: r.try_get("user_id")?,
            username: r.try_get("username")?,
            user_role,
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

pub async fn create_access_group(pool: &SqlitePool, input: NewAccessGroup) -> Result<AccessGroup> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        anyhow::bail!("name must not be empty");
    }
    let description = input
        .description
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
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

    let member_rows =
        sqlx::query("SELECT user_id FROM user_access_groups WHERE group_id = ? ORDER BY user_id")
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
        return row
            .as_ref()
            .map(AccessGroup::from_row_with_counts)
            .transpose();
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
        sqlx::query("INSERT INTO access_group_libraries (group_id, library_id) VALUES (?, ?)")
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
        sqlx::query("INSERT INTO user_access_groups (user_id, group_id) VALUES (?, ?)")
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
    let rows =
        sqlx::query("SELECT group_id FROM user_access_groups WHERE user_id = ? ORDER BY group_id")
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
pub async fn set_user_groups(pool: &SqlitePool, user_id: i64, group_ids: &[i64]) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_access_groups WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    for gid in group_ids {
        sqlx::query("INSERT INTO user_access_groups (user_id, group_id) VALUES (?, ?)")
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
pub async fn list_user_group_library_ids(pool: &SqlitePool, user_id: i64) -> Result<Vec<i64>> {
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
    let rows =
        sqlx::query("SELECT group_id FROM invite_groups WHERE invite_id = ? ORDER BY group_id")
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
pub async fn consume_invite(pool: &SqlitePool, code_hash: &str, user_id: i64) -> Result<()> {
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

/// Write a resume-point sourced from an external provider (Trakt's
/// `/sync/playback`) without touching the row's watched flag. The
/// live-player batch path defaults `watched` to false on missing
/// updates, which is correct for "user is actively watching" but
/// destructive when applied to a row the user already marked watched
/// — pulling a Trakt playback entry would silently un-watch the item.
///
/// Use this from any external-sync code path where you only want to
/// nudge position; the local watched/in-progress decision should
/// remain authoritative.
pub async fn upsert_external_position(
    pool: &SqlitePool,
    user_id: i64,
    item_id: Option<i64>,
    episode_id: Option<i64>,
    position_ms: i64,
) -> Result<()> {
    let now = now_ms();
    match (item_id, episode_id) {
        (Some(id), None) => {
            sqlx::query(
                "INSERT INTO play_state
                    (user_id, item_id, position_ms, duration_ms, watched, view_count, last_played_at)
                 VALUES (?, ?, ?, NULL, 0, 0, ?)
                 ON CONFLICT (user_id, item_id) WHERE item_id IS NOT NULL DO UPDATE SET
                    position_ms     = excluded.position_ms,
                    last_played_at  = excluded.last_played_at",
            )
            .bind(user_id)
            .bind(id)
            .bind(position_ms)
            .bind(now)
            .execute(pool)
            .await?;
        }
        (None, Some(id)) => {
            sqlx::query(
                "INSERT INTO play_state
                    (user_id, episode_id, position_ms, duration_ms, watched, view_count, last_played_at)
                 VALUES (?, ?, ?, NULL, 0, 0, ?)
                 ON CONFLICT (user_id, episode_id) WHERE episode_id IS NOT NULL DO UPDATE SET
                    position_ms     = excluded.position_ms,
                    last_played_at  = excluded.last_played_at",
            )
            .bind(user_id)
            .bind(id)
            .bind(position_ms)
            .bind(now)
            .execute(pool)
            .await?;
        }
        _ => anyhow::bail!("upsert_external_position requires exactly one of item_id or episode_id"),
    }
    Ok(())
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

    // Next-episode augmentation. Plex/Netflix surface a show on
    // Continue Watching for as long as you're mid-season — finish ep
    // 13, see ep 14 ready to play, even if you never started 14.
    // The in-progress loop above only catches *partially-played*
    // episodes (`position_ms > 0`), so a fully-watched-then-stop
    // pattern would drop the show off the rail entirely. This pass
    // backfills that case: for each show the user has any play_state
    // on (within the same recency cutoff as the in-progress filter),
    // find the chronologically next episode with no play_state row
    // and surface it. `seen_show_ids` already contains shows we
    // emitted from the in-progress pass, so an active mid-watch
    // takes precedence over its own "next up".
    if out.len() < cap {
        let next_ups = list_user_next_episode_in_show(pool, user_id, cutoff_ms).await?;
        for (show_id, episode_id, last_played_at) in next_ups {
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
            // Carry the *show's* most-recent last_played_at into the
            // stub so the rail orders this tile by when the user
            // last engaged with the show, not "now" — otherwise every
            // next-up freshly-computed entry would race to the front
            // ahead of genuine in-progress items on subsequent renders.
            let play_state = PlayStateForItem {
                position_ms: 0,
                duration_ms: detail.episode.duration_ms,
                watched: false,
                view_count: 0,
                last_played_at,
            };
            out.push(OnDeckEntry::Episode {
                episode: detail.episode,
                show,
                play_state,
            });
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

/// For each show the user has touched (watched or in-progress on any
/// episode), find the chronologically next episode that has *no*
/// play_state row — the natural "next to watch" tile. Returns
/// `(show_id, episode_id, last_played_at_of_show)` ordered by recency
/// of the show's most-recent activity, so the Continue Watching rail
/// can keep its DESC ordering.
///
/// Complements [`list_user_show_premieres`]: the premiere path only
/// fires when the user is fully *between* seasons. This one covers the
/// far more common mid-season case (finished ep N, ep N+1 is unstarted).
///
/// `cutoff_ms` mirrors the in-progress filter — shows whose latest
/// play is older than the cutoff are excluded so long-since-finished
/// shows don't reappear when the operator's max-age-weeks window
/// changes.
async fn list_user_next_episode_in_show(
    pool: &SqlitePool,
    user_id: i64,
    cutoff_ms: i64,
) -> Result<Vec<(i64, i64, i64)>> {
    let rows = sqlx::query(
        "WITH user_progress AS (
             SELECT s.show_id AS show_id,
                    ps.last_played_at AS last_played_at,
                    s.season_number AS season,
                    e.episode_number AS episode
             FROM play_state ps
             JOIN episodes e ON ps.episode_id = e.id
             JOIN seasons s ON e.season_id = s.id
             WHERE ps.user_id = ?1
               AND (ps.watched = 1 OR ps.position_ms > 0)
         ),
         latest_per_show AS (
             SELECT show_id, MAX(last_played_at) AS last_played_at
             FROM user_progress
             GROUP BY show_id
         ),
         tip AS (
             SELECT up.show_id, up.last_played_at, up.season, up.episode
             FROM user_progress up
             JOIN latest_per_show lps
               ON lps.show_id = up.show_id
              AND lps.last_played_at = up.last_played_at
         )
         SELECT t.show_id AS show_id,
                t.last_played_at AS last_played_at,
                (SELECT e2.id
                   FROM episodes e2
                   JOIN seasons s2 ON e2.season_id = s2.id
                   WHERE s2.show_id = t.show_id
                     AND (s2.season_number > t.season
                          OR (s2.season_number = t.season AND e2.episode_number > t.episode))
                     AND NOT EXISTS (
                         SELECT 1 FROM play_state ps2
                         WHERE ps2.user_id = ?1 AND ps2.episode_id = e2.id
                     )
                   ORDER BY s2.season_number ASC, e2.episode_number ASC
                   LIMIT 1) AS episode_id
         FROM tip t
         WHERE t.last_played_at >= ?2
         ORDER BY t.last_played_at DESC",
    )
    .bind(user_id)
    .bind(cutoff_ms)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    let mut seen = std::collections::HashSet::<i64>::new();
    for r in rows {
        let show_id: i64 = r.try_get("show_id")?;
        // A show with two tied `last_played_at` rows would emit twice
        // from the `tip` CTE — dedup here so the caller doesn't have
        // to think about it.
        if !seen.insert(show_id) {
            continue;
        }
        let episode_id: Option<i64> = r.try_get("episode_id").ok().flatten();
        let last_played_at: i64 = r.try_get("last_played_at")?;
        if let Some(eid) = episode_id {
            out.push((show_id, eid, last_played_at));
        }
    }
    Ok(out)
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
async fn list_user_show_premieres(pool: &SqlitePool, user_id: i64) -> Result<Vec<(i64, i64)>> {
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
    let placeholders = std::iter::repeat_n("?", ids.len())
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
pub async fn verify_library(pool: &SqlitePool, library_id: i64) -> Result<VerifyReport> {
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
/// `media_streams`, `markers` rows attached to these files vanish
/// automatically (`ON DELETE CASCADE`).
///
/// After the file delete, sweep parent rows that have been left
/// childless:
///   * Episodes with zero media_files
///   * Seasons with zero episodes
///   * Items (movies) with zero media_files
///   * Items (shows) with zero seasons
///
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

    // Wrap the four cascading DELETEs in a single transaction so a
    // failure mid-cascade can't leave parents pointing at vanished
    // children (or vice versa). Without this, a writer panic between
    // the media_files DELETE and the orphan-sweep would leave
    // dangling rows that the next purge tick would handle anyway —
    // but a *reader* hitting the window would see an inconsistent
    // tree (an item with zero seasons, or a season with zero
    // episodes). One BEGIN/COMMIT keeps the entire cascade atomic
    // from the reader's perspective.
    let mut tx = pool.begin().await?;

    // Hard-delete the soft-deleted files past the grace window.
    let r = sqlx::query("DELETE FROM media_files WHERE removed_at IS NOT NULL AND removed_at < ?")
        .bind(older_than_ms)
        .execute(&mut *tx)
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
    .execute(&mut *tx)
    .await?;
    report.episodes_purged = r.rows_affected();

    let r = sqlx::query(
        "DELETE FROM seasons
         WHERE NOT EXISTS (SELECT 1 FROM episodes WHERE season_id = seasons.id)",
    )
    .execute(&mut *tx)
    .await?;
    report.seasons_purged = r.rows_affected();

    // Items split by kind: movies are orphans when their files are
    // gone; shows are orphans when their seasons are gone.
    let r = sqlx::query(
        "DELETE FROM items
         WHERE (kind = 'movie' AND NOT EXISTS (SELECT 1 FROM media_files WHERE item_id = items.id))
            OR (kind = 'show'  AND NOT EXISTS (SELECT 1 FROM seasons WHERE show_id = items.id))",
    )
    .execute(&mut *tx)
    .await?;
    report.items_purged = r.rows_affected();

    tx.commit().await?;

    Ok(report)
}

/// Operator-initiated delete of specific `media_files` rows. Skips
/// the soft-delete + grace-window dance — this is the path behind the
/// item modal's "Delete from disk" button. Caller is responsible for
/// having checked that the owning library has `allow_media_deletion`
/// turned on and that the actor is an owner.
///
/// Returns the same `PurgeReport` shape as the scheduled purge so the
/// admin UI / API consumer can show the same summary.
pub async fn delete_media_files_force(pool: &SqlitePool, file_ids: &[i64]) -> Result<PurgeReport> {
    let mut report = PurgeReport::default();
    if file_ids.is_empty() {
        return Ok(report);
    }
    let placeholders = std::iter::repeat_n("?", file_ids.len())
        .collect::<Vec<_>>()
        .join(",");

    // Collect the on-disk artefacts we need to clean up after the row
    // DELETE. FK cascade drops media_streams / markers /
    // optimized_versions rows but doesn't touch the filesystem.
    let select_sql =
        format!("SELECT path FROM media_files WHERE id IN ({placeholders})");
    let mut q = sqlx::query(&select_sql);
    for id in file_ids {
        q = q.bind(*id);
    }
    let rows = q.fetch_all(pool).await?;
    for row in rows {
        let path: String = row.try_get("path")?;
        report.purged_paths.push(path);
    }

    // Wrap the file DELETE + cascade in a single transaction. Same
    // atomicity reasoning as `purge_removed_media_files`: a reader
    // hitting the window between the media_files DELETE and the
    // orphan-sweep would otherwise see a half-pruned tree.
    let mut tx = pool.begin().await?;

    let delete_sql = format!("DELETE FROM media_files WHERE id IN ({placeholders})");
    let mut q = sqlx::query(&delete_sql);
    for id in file_ids {
        q = q.bind(*id);
    }
    let r = q.execute(&mut *tx).await?;
    report.files_purged = r.rows_affected();

    // Cascade orphan sweep — same order + logic as `purge_removed_media_files`.
    // Pulled out as the shared semantics rather than DRYing because the
    // existing function builds its DELETE off `removed_at` which the
    // force path bypasses entirely.
    let r = sqlx::query(
        "DELETE FROM episodes
         WHERE NOT EXISTS (SELECT 1 FROM media_files WHERE episode_id = episodes.id)",
    )
    .execute(&mut *tx)
    .await?;
    report.episodes_purged = r.rows_affected();

    let r = sqlx::query(
        "DELETE FROM seasons
         WHERE NOT EXISTS (SELECT 1 FROM episodes WHERE season_id = seasons.id)",
    )
    .execute(&mut *tx)
    .await?;
    report.seasons_purged = r.rows_affected();

    let r = sqlx::query(
        "DELETE FROM items
         WHERE (kind = 'movie' AND NOT EXISTS (SELECT 1 FROM media_files WHERE item_id = items.id))
            OR (kind = 'show'  AND NOT EXISTS (SELECT 1 FROM seasons WHERE show_id = items.id))",
    )
    .execute(&mut *tx)
    .await?;
    report.items_purged = r.rows_affected();

    tx.commit().await?;

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
    upsert_item_with_match(pool, library_id, kind, title, sort_title, year, true).await
}

/// Same as [`upsert_item`] but lets the caller flag a row as
/// `auto_matched = false`. Used by the scanner when the parser
/// couldn't extract a confident title — the file still becomes
/// visible/playable, just with the "unmatched" affordance in the UI.
pub async fn upsert_item_with_match(
    pool: &SqlitePool,
    library_id: i64,
    kind: ItemKind,
    title: &str,
    sort_title: &str,
    year: Option<i32>,
    auto_matched: bool,
) -> Result<i64> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO items (library_id, kind, title, sort_title, year, auto_matched, added_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(i64::from(auto_matched))
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get("id")?)
}

/// Flip `items.auto_matched` to true. Called by the Fix Match flow
/// once an operator confirms the metadata so the row drops out of
/// the "Unmatched" surface.
pub async fn mark_item_auto_matched(pool: &SqlitePool, item_id: i64) -> Result<()> {
    let now = now_ms();
    sqlx::query("UPDATE items SET auto_matched = 1, updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
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

/// Move an episode row to a different (season_number, episode_number)
/// under the same show. Used by the absolute-episode resolver after a
/// metadata agent reports per-season episode counts and we determine
/// the file's on-disk number maps to a different season-relative
/// position. Idempotent — if the episode is already at the target
/// position, no-op.
///
/// When a row already exists at the target position (from a prior
/// scan or a manual edit), the move is refused so we don't double-up
/// `media_files` pointers. The caller logs and continues.
pub async fn move_episode_to_season(
    pool: &SqlitePool,
    episode_id: i64,
    target_season_number: i32,
    target_episode_number: i32,
) -> Result<bool> {
    let show_id_row = sqlx::query(
        "SELECT s.show_id FROM episodes e JOIN seasons s ON e.season_id = s.id WHERE e.id = ?",
    )
    .bind(episode_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = show_id_row else {
        return Ok(false);
    };
    let show_id: i64 = row.try_get("show_id")?;

    let target_season_id = upsert_season(pool, show_id, target_season_number).await?;

    // Refuse the move if a different episode already occupies the
    // target slot. Updating into it would either UNIQUE-violate or
    // overwrite another scan's row.
    let existing = sqlx::query(
        "SELECT id FROM episodes WHERE season_id = ? AND episode_number = ? AND id != ?",
    )
    .bind(target_season_id)
    .bind(target_episode_number)
    .bind(episode_id)
    .fetch_optional(pool)
    .await?;
    if existing.is_some() {
        return Ok(false);
    }

    let now = now_ms();
    let res = sqlx::query(
        "UPDATE episodes
         SET season_id = ?, episode_number = ?, updated_at = ?
         WHERE id = ?",
    )
    .bind(target_season_id)
    .bind(target_episode_number)
    .bind(now)
    .bind(episode_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Set the per-show episode-numbering mode. Used by the absolute-ep
/// resolver after first detection so subsequent files in the same
/// show take the fast path through resolved (season, episode) instead
/// of re-running detection. Default is `'season_relative'`; only
/// flipped to `'absolute'` for anime shows where the on-disk numbering
/// is the absolute episode index across the whole series.
pub async fn set_episode_numbering_mode(
    pool: &SqlitePool,
    show_id: i64,
    mode: &str,
) -> Result<()> {
    sqlx::query(
        "UPDATE items SET episode_numbering_mode = ?, updated_at = ? WHERE id = ?",
    )
    .bind(mode)
    .bind(now_ms())
    .bind(show_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Return the current `episode_numbering_mode` for a show.
pub async fn get_episode_numbering_mode(pool: &SqlitePool, show_id: i64) -> Result<String> {
    let row = sqlx::query("SELECT episode_numbering_mode FROM items WHERE id = ?")
        .bind(show_id)
        .fetch_optional(pool)
        .await?;
    Ok(row
        .and_then(|r| r.try_get::<String, _>("episode_numbering_mode").ok())
        .unwrap_or_else(|| "season_relative".to_string()))
}

/// One-shot heal pass: re-sanitize every episode title that matches
/// the "looks filename-derived" pattern set. Runs at server startup so
/// rows that were upserted before the parser sanitizer + the
/// `upsert_episode` CASE were widened still get cleaned up without
/// forcing the operator to touch every file's mtime + rescan.
///
/// Idempotent: matches the same GLOB / LIKE patterns
/// `upsert_episode` uses to decide overwrite-eligibility, runs them
/// through [`crate::parser::sanitize_title_pub`], and only updates
/// rows where the sanitized form differs. A second invocation is a
/// no-op.
///
/// Doesn't touch episodes whose row has `locked_fields` containing
/// `'title'` — operators who hand-edited a title don't want their
/// edit revoked by a routine startup pass.
pub async fn heal_filename_derived_episode_titles(pool: &SqlitePool) -> Result<u64> {
    let candidates = sqlx::query(
        "SELECT e.id, e.title
         FROM episodes e
         LEFT JOIN items i ON i.id = (SELECT show_id FROM seasons WHERE id = e.season_id)
         WHERE
              length(trim(e.title)) = 0
           OR e.title LIKE 'Episode %'
           OR LOWER(e.title) LIKE '%1080p%'
           OR LOWER(e.title) LIKE '%720p%'
           OR LOWER(e.title) LIKE '%2160p%'
           OR LOWER(e.title) LIKE '%480p%'
           OR LOWER(e.title) LIKE '%web-dl%'
           OR LOWER(e.title) LIKE '%webrip%'
           OR LOWER(e.title) LIKE '%bluray%'
           OR LOWER(e.title) LIKE '%blu-ray%'
           OR LOWER(e.title) LIKE '%hevc%'
           OR LOWER(e.title) LIKE '%x265%'
           OR LOWER(e.title) LIKE '%x264%'
           OR LOWER(e.title) LIKE '%10bit%'
           OR LOWER(e.title) LIKE '%remux%'
           OR e.title GLOB '[0-9][0-9] *'
           OR e.title GLOB '[0-9][0-9][0-9] *'
           OR e.title GLOB '[0-9][0-9][0-9][0-9] *'
           OR e.title GLOB '[0-9][0-9]-*'
           OR e.title GLOB '[0-9][0-9][0-9]-*'
           OR e.title GLOB '[0-9][0-9][0-9][0-9]-*'
           OR e.title GLOB '* -[A-Z]*'
           OR e.title GLOB '* -[0-9]*'",
    )
    .fetch_all(pool)
    .await?;

    let mut healed = 0u64;
    let now = now_ms();
    for row in candidates {
        let id: i64 = row.try_get("id")?;
        let title: String = row.try_get("title").unwrap_or_default();
        let sanitized = crate::parser::sanitize_title_pub(&title);
        // Skip when sanitization didn't change anything (the GLOB
        // matched a substring but sanitize_title's narrower rules
        // decided to keep it) or produced an empty result (no
        // recoverable signal — leave the row alone so a later scan
        // can replace it with `Episode N`).
        if sanitized.is_empty() || sanitized == title {
            continue;
        }
        let res = sqlx::query(
            "UPDATE episodes SET title = ?, updated_at = ? WHERE id = ?",
        )
        .bind(&sanitized)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
        healed += res.rows_affected();
    }
    Ok(healed)
}

/// Delete `images` rows whose `source_url` is blank — these end up in
/// the DB when an agent surfaces an empty string instead of `None`
/// (notably TVDB's episode `image` field on episodes without stills).
/// `<img src="">` reloads the current page and renders as a black tile
/// in the UI; deleting the row makes the renderer fall through to the
/// "no thumbnail" placeholder instead.
///
/// Idempotent — safe to run on every startup. Returns the number of
/// rows removed so the boot path can log a sensible heal counter.
pub async fn heal_blank_image_rows(pool: &SqlitePool) -> Result<u64> {
    let res = sqlx::query(
        "DELETE FROM images
         WHERE source_url IS NULL
            OR length(trim(source_url)) = 0",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn upsert_episode(
    pool: &SqlitePool,
    season_id: i64,
    episode_number: i32,
    title: &str,
    absolute_number: Option<i32>,
) -> Result<i64> {
    let now = now_ms();
    // The CASE preserves a metadata-derived title across rescans
    // (we don't want a TMDB / AniList title to revert to the parser's
    // filename stem just because the file was re-seen) BUT must allow
    // overwriting when the stored title clearly came from the parser
    // — otherwise rows that pre-date the parser's sanitize_title fix
    // stay broken forever ("026 - Each Ones Promise -OZR" never
    // heals to "Each Ones Promise" because the CASE used to only
    // catch "Episode N" stems).
    //
    // The pattern set below matches the same heuristic
    // `looks_filename_derived` (queries.rs) uses for AniList fill-nulls:
    //   - empty / "Episode N" — parser fallback
    //   - quality tokens — unbracketed release-name leakage
    //   - leading 2-4 digit prefix + dash — anime absolute-episode prefix
    //   - " -Token$" — trailing kebab-style release group
    //   - "Token-XYZ$" where the trailing token is uppercase / digit
    //     (handled by the suffix glob `'*[a-z] -[A-Z]*'` and the
    //     no-space variant)
    //
    // Owner-locked titles aren't checked here (upsert_episode predates
    // the locks system); fix-match / Edit Metadata uses
    // `replace_item_credits` and stores titles via a different path
    // that goes through `fetch_locked_fields`.
    let row = sqlx::query(
        "INSERT INTO episodes (season_id, episode_number, title, absolute_number, added_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(season_id, episode_number) DO UPDATE SET
             title = CASE
                WHEN length(trim(episodes.title)) = 0
                  OR episodes.title LIKE 'Episode %'
                  OR LOWER(episodes.title) LIKE '%1080p%'
                  OR LOWER(episodes.title) LIKE '%720p%'
                  OR LOWER(episodes.title) LIKE '%2160p%'
                  OR LOWER(episodes.title) LIKE '%480p%'
                  OR LOWER(episodes.title) LIKE '%web-dl%'
                  OR LOWER(episodes.title) LIKE '%webrip%'
                  OR LOWER(episodes.title) LIKE '%bluray%'
                  OR LOWER(episodes.title) LIKE '%blu-ray%'
                  OR LOWER(episodes.title) LIKE '%hevc%'
                  OR LOWER(episodes.title) LIKE '%x265%'
                  OR LOWER(episodes.title) LIKE '%x264%'
                  OR LOWER(episodes.title) LIKE '%10bit%'
                  OR LOWER(episodes.title) LIKE '%remux%'
                  OR episodes.title GLOB '[0-9][0-9] *'
                  OR episodes.title GLOB '[0-9][0-9][0-9] *'
                  OR episodes.title GLOB '[0-9][0-9][0-9][0-9] *'
                  OR episodes.title GLOB '[0-9][0-9]-*'
                  OR episodes.title GLOB '[0-9][0-9][0-9]-*'
                  OR episodes.title GLOB '[0-9][0-9][0-9][0-9]-*'
                  OR episodes.title GLOB '* -[A-Z]*'
                  OR episodes.title GLOB '* -[0-9]*'
                THEN excluded.title
                ELSE episodes.title
             END,
             absolute_number = COALESCE(episodes.absolute_number, excluded.absolute_number),
             updated_at = excluded.updated_at
         RETURNING id",
    )
    .bind(season_id)
    .bind(episode_number)
    .bind(title)
    .bind(absolute_number)
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
            -- so play_state / markers stay linked.
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
    let Some(row) = row else {
        return Ok(Vec::new());
    };
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
    if is_locked(locked, field) {
        None
    } else {
        Some(value)
    }
}

/// Write TMDB movie metadata to `items`.
///
/// `is_primary` selects the merge mode:
/// - `true`: TMDB is the canonical source for this library — its title /
///   summary / tagline / year / genres / posters overwrite existing
///   values (subject to per-field locks). The legacy semantics; used
///   when TMDB is first in the library's agent chain, or when the
///   operator hits the Refresh button.
/// - `false`: TMDB is running behind another primary agent (e.g. AniList
///   for an anime library where the operator ranked TMDB lower). All
///   shared fields fill nulls only; TMDB-owned identifiers (tmdb_id,
///   imdb_id, logo_path) still backfill if missing. Genres are unioned
///   rather than replaced; posters/backdrops insert only when none of
///   that kind already exist.
pub async fn apply_movie_metadata(
    pool: &SqlitePool,
    item_id: i64,
    meta: &TmdbMovie,
    is_primary: bool,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", meta.title.clone());
    // Deliberately NOT updating sort_title here. The scanner's upsert
    // key is (library_id, kind, sort_title) — derived from the parsed
    // folder name. Overwriting it from the TMDB title means that the
    // next rescan of any file in the same folder produces a stale key
    // and inserts a *new* item row, causing a duplicate. Sort order
    // is therefore driven by the on-disk folder name, which matches
    // how the operator already organizes the library.
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
    let logo_url = meta.logo_path.as_deref().map(|p| tmdb_image_url(p, "w500"));

    if is_primary {
        // `title_present` is the gate for the "title-derived fields"
        // (original_title / summary / tagline) — if TMDB returned no
        // title we don't overwrite them. Bound once, referenced via
        // three CASEs.
        let title_present = title.is_some();
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(?, title),
                original_title = CASE WHEN ? THEN ? ELSE original_title END,
                summary = CASE WHEN ? THEN ? ELSE summary END,
                tagline = CASE WHEN ? THEN ? ELSE tagline END,
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
        .bind(title_present)
        .bind(&original_title)
        .bind(title_present)
        .bind(&summary)
        .bind(title_present)
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
    } else {
        // Non-primary mode: fill nulls only. `COALESCE(col, ?)` keeps
        // the existing value when set. `tmdb_id` is TMDB-owned so we
        // still backfill it when missing — needed for downstream
        // operations (collection lookup, credits) to function.
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(title, ?),
                original_title = COALESCE(original_title, ?),
                summary = COALESCE(summary, ?),
                tagline = COALESCE(tagline, ?),
                year = COALESCE(year, ?),
                duration_ms = COALESCE(duration_ms, ?),
                rating_audience = COALESCE(rating_audience, ?),
                tmdb_id = COALESCE(tmdb_id, ?),
                imdb_id = COALESCE(imdb_id, ?),
                logo_path = COALESCE(logo_path, ?),
                refreshed_at = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(&original_title)
        .bind(&summary)
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
    }

    if !is_locked(&locked, "genres") {
        if is_primary {
            apply_genres(pool, item_id, &meta.genres).await?;
        } else {
            apply_genres_additive(pool, item_id, &meta.genres).await?;
        }
    }
    if !is_locked(&locked, "poster") {
        if let Some(p) = &meta.poster_path {
            let url = tmdb_image_url(p, "w500");
            if is_primary {
                store_image(pool, Some(item_id), None, "poster", "tmdb", &url).await?;
            } else {
                store_image_if_missing(pool, Some(item_id), None, "poster", "tmdb", &url).await?;
            }
        }
    }
    if !is_locked(&locked, "backdrop") {
        if let Some(p) = &meta.backdrop_path {
            let url = tmdb_image_url(p, "w1280");
            if is_primary {
                store_image(pool, Some(item_id), None, "backdrop", "tmdb", &url).await?;
            } else {
                store_image_if_missing(pool, Some(item_id), None, "backdrop", "tmdb", &url).await?;
            }
        }
    }

    Ok(())
}

/// Polymorphic movie writer. Accepts a [`MovieData`] (the
/// agent-agnostic common shape) and writes only the columns the agent
/// populated. `mode` selects between Primary (overwrite null-or-stale)
/// and FillNulls (only write to NULL columns). Locked fields are
/// honored in both modes.
///
/// This is the path scan-time enrichment uses post-Slice-3. Provider-
/// specific writers (`apply_movie_metadata` for TMDB,
/// `apply_movie_metadata_tvdb` for TVDB) remain available for legacy
/// refresh paths until those are migrated too — they share the same
/// underlying columns so running them in sequence is safe.
pub async fn apply_movie_data(
    pool: &SqlitePool,
    item_id: i64,
    data: &chimpflix_metadata::MovieData,
    mode: WriteMode,
    source: &str,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", data.title.clone()).flatten();
    let original_title = pick(&locked, "original_title", data.original_title.clone()).flatten();
    let summary = pick(&locked, "summary", data.summary.clone()).flatten();
    let tagline = pick(&locked, "tagline", data.tagline.clone()).flatten();
    let year = pick(&locked, "year", data.year).flatten();
    let rating_audience = pick(&locked, "rating_audience", data.rating_audience).flatten();
    let duration_ms = data.runtime_ms;

    if mode.overwrites() {
        let title_present = title.is_some();
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(?, title),
                original_title = CASE WHEN ? THEN ? ELSE original_title END,
                summary = CASE WHEN ? THEN ? ELSE summary END,
                tagline = CASE WHEN ? THEN ? ELSE tagline END,
                year = COALESCE(?, year),
                duration_ms = COALESCE(duration_ms, ?),
                rating_audience = COALESCE(?, rating_audience),
                tmdb_id = COALESCE(?, tmdb_id),
                imdb_id = COALESCE(?, imdb_id),
                tvdb_id = COALESCE(?, tvdb_id),
                logo_path = COALESCE(?, logo_path),
                refreshed_at = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(title_present)
        .bind(&original_title)
        .bind(title_present)
        .bind(&summary)
        .bind(title_present)
        .bind(&tagline)
        .bind(year)
        .bind(duration_ms)
        .bind(rating_audience)
        .bind(data.tmdb_id)
        .bind(data.imdb_id.as_deref())
        .bind(data.tvdb_id)
        .bind(data.logo_url.as_deref())
        .bind(now)
        .bind(now)
        .bind(item_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(title, ?),
                original_title = COALESCE(original_title, ?),
                summary = COALESCE(summary, ?),
                tagline = COALESCE(tagline, ?),
                year = COALESCE(year, ?),
                duration_ms = COALESCE(duration_ms, ?),
                rating_audience = COALESCE(rating_audience, ?),
                tmdb_id = COALESCE(tmdb_id, ?),
                imdb_id = COALESCE(imdb_id, ?),
                tvdb_id = COALESCE(tvdb_id, ?),
                logo_path = COALESCE(logo_path, ?),
                refreshed_at = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(&original_title)
        .bind(&summary)
        .bind(&tagline)
        .bind(year)
        .bind(duration_ms)
        .bind(rating_audience)
        .bind(data.tmdb_id)
        .bind(data.imdb_id.as_deref())
        .bind(data.tvdb_id)
        .bind(data.logo_url.as_deref())
        .bind(now)
        .bind(now)
        .bind(item_id)
        .execute(pool)
        .await?;
    }

    if !is_locked(&locked, "genres") && !data.genres.is_empty() {
        if mode.overwrites() {
            apply_genres(pool, item_id, &data.genres).await?;
        } else {
            apply_genres_additive(pool, item_id, &data.genres).await?;
        }
    }

    if !is_locked(&locked, "poster") {
        for variant in data.posters.iter().take(1) {
            if mode.overwrites() {
                store_image(pool, Some(item_id), None, "poster", source, &variant.url).await?;
            } else {
                store_image_if_missing(
                    pool,
                    Some(item_id),
                    None,
                    "poster",
                    source,
                    &variant.url,
                )
                .await?;
            }
        }
    }
    if !is_locked(&locked, "backdrop") {
        for variant in data.backdrops.iter().take(1) {
            if mode.overwrites() {
                store_image(pool, Some(item_id), None, "backdrop", source, &variant.url).await?;
            } else {
                store_image_if_missing(
                    pool,
                    Some(item_id),
                    None,
                    "backdrop",
                    source,
                    &variant.url,
                )
                .await?;
            }
        }
    }

    Ok(())
}

/// Polymorphic show writer. Mirror of [`apply_movie_data`] for TV.
/// AniList ids and TVMaze ids are written exclusively here — they were
/// previously kept in agent-specific writers. Columns are added
/// idempotently by each agent's pass through the chain.
pub async fn apply_show_data(
    pool: &SqlitePool,
    item_id: i64,
    data: &chimpflix_metadata::ShowData,
    mode: WriteMode,
    source: &str,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", data.title.clone()).flatten();
    let original_title = pick(&locked, "original_title", data.original_title.clone()).flatten();
    let summary = pick(&locked, "summary", data.summary.clone()).flatten();
    let year = pick(&locked, "year", data.year).flatten();

    if mode.overwrites() {
        let title_present = title.is_some();
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(?, title),
                original_title = CASE WHEN ? THEN ? ELSE original_title END,
                summary = CASE WHEN ? THEN ? ELSE summary END,
                year = COALESCE(?, year),
                tmdb_id = COALESCE(?, tmdb_id),
                imdb_id = COALESCE(?, imdb_id),
                tvdb_id = COALESCE(?, tvdb_id),
                anilist_id = COALESCE(?, anilist_id),
                tvmaze_id = COALESCE(?, tvmaze_id),
                refreshed_at = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(title_present)
        .bind(&original_title)
        .bind(title_present)
        .bind(&summary)
        .bind(year)
        .bind(data.tmdb_id)
        .bind(data.imdb_id.as_deref())
        .bind(data.tvdb_id)
        .bind(data.anilist_id)
        .bind(data.tvmaze_id)
        .bind(now)
        .bind(now)
        .bind(item_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(title, ?),
                original_title = COALESCE(original_title, ?),
                summary = COALESCE(summary, ?),
                year = COALESCE(year, ?),
                tmdb_id = COALESCE(tmdb_id, ?),
                imdb_id = COALESCE(imdb_id, ?),
                tvdb_id = COALESCE(tvdb_id, ?),
                anilist_id = COALESCE(anilist_id, ?),
                tvmaze_id = COALESCE(tvmaze_id, ?),
                refreshed_at = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(&original_title)
        .bind(&summary)
        .bind(year)
        .bind(data.tmdb_id)
        .bind(data.imdb_id.as_deref())
        .bind(data.tvdb_id)
        .bind(data.anilist_id)
        .bind(data.tvmaze_id)
        .bind(now)
        .bind(now)
        .bind(item_id)
        .execute(pool)
        .await?;
    }

    if !is_locked(&locked, "genres") && !data.genres.is_empty() {
        if mode.overwrites() {
            apply_genres(pool, item_id, &data.genres).await?;
        } else {
            apply_genres_additive(pool, item_id, &data.genres).await?;
        }
    }

    if !is_locked(&locked, "poster") {
        for variant in data.posters.iter().take(1) {
            if mode.overwrites() {
                store_image(pool, Some(item_id), None, "poster", source, &variant.url).await?;
            } else {
                store_image_if_missing(
                    pool,
                    Some(item_id),
                    None,
                    "poster",
                    source,
                    &variant.url,
                )
                .await?;
            }
        }
    }
    if !is_locked(&locked, "backdrop") {
        for variant in data.backdrops.iter().take(1) {
            if mode.overwrites() {
                store_image(pool, Some(item_id), None, "backdrop", source, &variant.url).await?;
            } else {
                store_image_if_missing(
                    pool,
                    Some(item_id),
                    None,
                    "backdrop",
                    source,
                    &variant.url,
                )
                .await?;
            }
        }
    }

    Ok(())
}

/// Write TMDB show metadata to `items`.
///
/// `is_primary` selects the merge mode — see `apply_movie_metadata`
/// for the rationale. Briefly: primary overwrites shared fields, non-
/// primary fills nulls. Episode-level enrichment is unaffected — see
/// `apply_episode_metadata` (always runs when TMDB is enabled, since
/// no other agent supplies per-episode metadata today).
pub async fn apply_show_metadata(
    pool: &SqlitePool,
    item_id: i64,
    meta: &TmdbShow,
    is_primary: bool,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", meta.title.clone());
    // Deliberately NOT updating sort_title here — see [apply_movie_metadata]
    // for the reason. Same dedup-key argument applies to shows: the
    // scanner upserts by (library_id, kind, sort_title), and overwriting
    // it from a TMDB-renamed title is what caused duplicate show rows
    // (e.g. one folder "The Agency (2024)" producing both "The Agency"
    // and "The Agency: Central Intelligence" items).
    let original_title = pick(&locked, "original_title", meta.original_title.clone()).flatten();
    let summary = pick(&locked, "summary", meta.summary.clone()).flatten();
    let year = pick(&locked, "year", meta.year).flatten();
    let logo_url = meta.logo_path.as_deref().map(|p| tmdb_image_url(p, "w500"));

    if is_primary {
        let title_present = title.is_some();
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(?, title),
                original_title = CASE WHEN ? THEN ? ELSE original_title END,
                summary = CASE WHEN ? THEN ? ELSE summary END,
                year = COALESCE(?, year),
                tmdb_id = ?,
                imdb_id = COALESCE(?, imdb_id),
                logo_path = COALESCE(?, logo_path),
                refreshed_at = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(title_present)
        .bind(&original_title)
        .bind(title_present)
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
    } else {
        // Non-primary mode: fill nulls only. `tmdb_id` still backfills
        // because downstream episode enrichment depends on it.
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(title, ?),
                original_title = COALESCE(original_title, ?),
                summary = COALESCE(summary, ?),
                year = COALESCE(year, ?),
                tmdb_id = COALESCE(tmdb_id, ?),
                imdb_id = COALESCE(imdb_id, ?),
                logo_path = COALESCE(logo_path, ?),
                refreshed_at = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
        .bind(&original_title)
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
    }

    if !is_locked(&locked, "genres") {
        if is_primary {
            apply_genres(pool, item_id, &meta.genres).await?;
        } else {
            apply_genres_additive(pool, item_id, &meta.genres).await?;
        }
    }
    if !is_locked(&locked, "poster") {
        if let Some(p) = &meta.poster_path {
            let url = tmdb_image_url(p, "w500");
            if is_primary {
                store_image(pool, Some(item_id), None, "poster", "tmdb", &url).await?;
            } else {
                store_image_if_missing(pool, Some(item_id), None, "poster", "tmdb", &url).await?;
            }
        }
    }
    if !is_locked(&locked, "backdrop") {
        if let Some(p) = &meta.backdrop_path {
            let url = tmdb_image_url(p, "w1280");
            if is_primary {
                store_image(pool, Some(item_id), None, "backdrop", "tmdb", &url).await?;
            } else {
                store_image_if_missing(pool, Some(item_id), None, "backdrop", "tmdb", &url).await?;
            }
        }
    }

    Ok(())
}

// ─── Collections (movie franchises) ────────────────────────────────────────

/// Upsert a collection row by TMDB id, returning the local collection id.
/// `overview` may be None — the `belongs_to_collection` stub doesn't
/// include it; the full /collection/{id} fetch does. We update fields
/// COALESCE-style so a follow-up call enriches the row.
pub async fn upsert_collection_stub(pool: &SqlitePool, stub: &TmdbCollectionStub) -> Result<i64> {
    let now = now_ms();
    let poster = stub
        .poster_path
        .as_deref()
        .map(|p| tmdb_image_url(p, "w500"));
    let backdrop = stub
        .backdrop_path
        .as_deref()
        .map(|p| tmdb_image_url(p, "w1280"));
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
pub async fn find_collection_by_tmdb(pool: &SqlitePool, tmdb_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query("SELECT id FROM collections WHERE tmdb_id = ?")
        .bind(tmdb_id)
        .fetch_optional(pool)
        .await?;
    Ok(row
        .map(|r| r.try_get::<i64, _>("id").unwrap_or(0))
        .filter(|v| *v > 0))
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
             WHERE c.id = ?"
                .to_string()
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
    let Some(kr) = kind_row else {
        return Ok(Vec::new());
    };
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
            let (best_quality_height, best_hdr_format) =
                ListedItem::quality_from_columns(r);
            Ok(ListedItem {
                item,
                play_state,
                best_quality_height,
                best_hdr_format,
            })
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
            let (best_quality_height, best_hdr_format) =
                ListedItem::quality_from_columns(r);
            Ok(ListedItem {
                item,
                play_state,
                best_quality_height,
                best_hdr_format,
            })
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
    sqlx::query("UPDATE collections SET rule_json = ?, updated_at = ? WHERE id = ?")
        .bind(rule_json)
        .bind(now)
        .bind(collection_id)
        .execute(pool)
        .await?;
    Ok(true)
}

pub async fn delete_smart_collection(pool: &SqlitePool, collection_id: i64) -> Result<bool> {
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

    let sql = format!("UPDATE collections SET {} WHERE id = ?", parts.join(", "));
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
pub async fn delete_manual_collection(pool: &SqlitePool, collection_id: i64) -> Result<bool> {
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
            store_image_if_missing(pool, Some(item_id), None, "backdrop", "tvmaze", p).await?;
        }
    }

    Ok(())
}

/// Write AniList show metadata to `items`.
///
/// `is_primary` selects the merge mode — see `apply_movie_metadata`
/// for the rationale. AniList is the typical primary for anime
/// libraries (the seed-default ordering puts it first), but operators
/// can rank it lower and have it run as a null-filler behind TMDB.
pub async fn apply_show_metadata_anilist(
    pool: &SqlitePool,
    item_id: i64,
    meta: &AniListShow,
    is_primary: bool,
) -> Result<()> {
    let now = now_ms();
    let locked = fetch_locked_fields(pool, item_id).await?;
    let title = pick(&locked, "title", Some(meta.title.clone())).flatten();
    // Deliberately NOT updating sort_title — see [apply_movie_metadata]
    // for the reason. AniList enrichment was the same source of duplicate
    // item rows: original folder name "K-On!" → enriched to "Keion!" →
    // dedup-key mismatch on next scan.
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

    if is_primary {
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(?, title),
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
    } else {
        // Non-primary mode: fill nulls only on shared fields. The
        // anilist_id column is AniList-owned, so it backfills even
        // in non-primary mode (downstream episode enrichment will
        // depend on it once that lands).
        sqlx::query(
            "UPDATE items SET
                title = COALESCE(title, ?),
                original_title = COALESCE(original_title, ?),
                summary = COALESCE(summary, ?),
                year = COALESCE(year, ?),
                duration_ms = COALESCE(duration_ms, ?),
                anilist_id = COALESCE(anilist_id, ?),
                refreshed_at = ?,
                updated_at = ?
             WHERE id = ?",
        )
        .bind(&title)
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
    }

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

async fn apply_genres_additive(pool: &SqlitePool, item_id: i64, genres: &[String]) -> Result<()> {
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
/// Polymorphic item-credits writer. Mirror of
/// [`apply_episode_credits_for_source`] for the show / movie level —
/// reads `Vec<PersonCredit>` from [`MovieData`] or [`ShowData`] and
/// writes to `item_credits` scoped by `source` so multi-source cast
/// from different agents can coexist.
pub async fn apply_item_credits_for_source(
    pool: &SqlitePool,
    item_id: i64,
    people: &[chimpflix_metadata::PersonCredit],
    source: &str,
) -> Result<()> {
    let locked = fetch_locked_fields(pool, item_id).await?;
    if is_locked(&locked, "credits") {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM item_credits WHERE item_id = ? AND source = ?")
        .bind(item_id)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    if people.is_empty() {
        tx.commit().await?;
        return Ok(());
    }
    for credit in people {
        let person_id: i64 = {
            let row = sqlx::query("INSERT INTO people (name, photo_url) VALUES (?, ?) RETURNING id")
                .bind(&credit.name)
                .bind(credit.profile_url.as_deref())
                .fetch_one(&mut *tx)
                .await?;
            row.try_get("id")?
        };
        let role_kind = match credit.role.as_str() {
            "actor" => "cast",
            "director" | "writer" | "producer" | "crew" => credit.role.as_str(),
            _ => "crew",
        };
        sqlx::query(
            "INSERT INTO item_credits
                (item_id, person_id, role_kind, role, character_name, sort_order, source)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item_id)
        .bind(person_id)
        .bind(role_kind)
        .bind(&credit.role)
        .bind(credit.character.as_deref())
        .bind(credit.order as i64)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Polymorphic episode-credits writer. Reads the
/// `people: Vec<PersonCredit>` field from [`EpisodeData`] and writes
/// it to `episode_credits` with the supplied `source` (agent name).
/// Scopes the DELETE-before-insert pattern by source so two agents'
/// episode-cast lists can coexist (the apply layer reads them ordered
/// by primary-source first, secondary-source second).
///
/// People are upserted by `(name, source_external_id)` — if a
/// `PersonCredit::external_id` is present, we dedupe across runs of
/// the same source via that id; otherwise we fall back to inserting a
/// fresh `people` row each time (the cleanup pass can dedupe by name
/// later).
///
/// No-op when `people` is empty.
pub async fn apply_episode_credits_for_source(
    pool: &SqlitePool,
    episode_id: i64,
    people: &[chimpflix_metadata::PersonCredit],
    source: &str,
) -> Result<()> {
    if people.is_empty() {
        // Even when empty we still scope-delete: the agent declared
        // it returned no cast for this episode, which is a meaningful
        // signal — e.g. clearing out stale rows from a prior scan.
        sqlx::query("DELETE FROM episode_credits WHERE episode_id = ? AND source = ?")
            .bind(episode_id)
            .bind(source)
            .execute(pool)
            .await?;
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM episode_credits WHERE episode_id = ? AND source = ?")
        .bind(episode_id)
        .bind(source)
        .execute(&mut *tx)
        .await?;

    for credit in people {
        // Upsert the person. People are deduped within a source by
        // external_id; across sources they're separate rows for now
        // (later cleanup can merge by canonical name). This keeps
        // each agent's people graph independent so a TVDB rescan
        // doesn't accidentally mutate TMDB-attributed rows.
        let person_id: i64 = if let Some(ext) = credit.external_id.as_deref() {
            // people table doesn't currently have a (source, external_id)
            // unique constraint — fall through to a plain insert and
            // accept that two scans may insert duplicate rows for the
            // same external_id. The cleanup pass tracked in Slice 9
            // will introduce that constraint.
            let _ = ext;
            let row = sqlx::query("INSERT INTO people (name, photo_url) VALUES (?, ?) RETURNING id")
                .bind(&credit.name)
                .bind(credit.profile_url.as_deref())
                .fetch_one(&mut *tx)
                .await?;
            row.try_get("id")?
        } else {
            let row = sqlx::query("INSERT INTO people (name, photo_url) VALUES (?, ?) RETURNING id")
                .bind(&credit.name)
                .bind(credit.profile_url.as_deref())
                .fetch_one(&mut *tx)
                .await?;
            row.try_get("id")?
        };

        let role_kind = match credit.role.as_str() {
            "actor" => "cast",
            "guest" => "guest",
            other => other,
        };
        sqlx::query(
            "INSERT INTO episode_credits
                (episode_id, person_id, role_kind, role, character_name, sort_order, source)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(episode_id)
        .bind(person_id)
        .bind(role_kind)
        .bind(&credit.role)
        .bind(credit.character.as_deref())
        .bind(credit.order as i64)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn apply_item_credits(
    pool: &SqlitePool,
    item_id: i64,
    credits: &TmdbCredits,
) -> Result<()> {
    let locked = fetch_locked_fields(pool, item_id).await?;
    if is_locked(&locked, "credits") {
        return Ok(());
    }
    // Scope the DELETE to the TMDB source so a future TVDB credits
    // pass doesn't wipe TMDB's rows from under it. The `source`
    // column landed in phase 74 with a default of 'tmdb' for existing
    // rows, so this DELETE catches both legacy untagged rows AND new
    // TMDB writes from this function below.
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM item_credits WHERE item_id = ? AND source = 'tmdb'")
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

async fn insert_credit_cast(pool: &SqlitePool, item_id: i64, m: &TmdbCastMember) -> Result<()> {
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
                let row =
                    sqlx::query("INSERT INTO people (name, photo_url) VALUES (?, ?) RETURNING id")
                        .bind(trimmed_name)
                        .bind(edit.photo_url.as_deref())
                        .fetch_one(&mut *tx)
                        .await?;
                row.try_get("id")?
            }
        };
        sqlx::query(
            "INSERT INTO item_credits
                (item_id, person_id, role_kind, role, character_name, sort_order, source)
             VALUES (?, ?, ?, ?, ?, ?, 'manual')",
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
    videos: &[chimpflix_metadata::VideoLink],
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
        // VideoLink.kind already comes in the lowercased common shape
        // ("trailer", "teaser", etc.). Map directly into the column
        // value, with "other" -> "clip" as the catch-all.
        let kind = match v.kind.as_str() {
            "trailer" | "teaser" | "featurette" | "clip" => v.kind.as_str(),
            "behind-the-scenes" => "behind_the_scenes",
            _ => "clip",
        };
        let thumb = format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", v.provider_key);
        let published_ms = v.published_at_ms;
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
        .bind(&v.provider_key)
        .bind(&thumb)
        .bind(published_ms)
        .bind(idx as i64)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

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

// ---------------------------------------------------------------------------
// Background job queue
// ---------------------------------------------------------------------------

/// Input for `enqueue_job`. Defaults: priority 0, max_attempts 3,
/// run_after = now (immediately eligible).
#[derive(Debug, Clone)]
pub struct JobInput {
    pub kind: String,
    pub payload: serde_json::Value,
    pub priority: i64,
    pub max_attempts: i64,
    pub run_after: Option<i64>,
}

impl JobInput {
    pub fn new(kind: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            kind: kind.into(),
            payload,
            priority: 0,
            max_attempts: 3,
            run_after: None,
        }
    }

    pub fn with_priority(mut self, p: i64) -> Self {
        self.priority = p;
        self
    }

    pub fn with_max_attempts(mut self, n: i64) -> Self {
        self.max_attempts = n;
        self
    }
}

/// Insert a new job. Returns the row id so the caller can correlate
/// (e.g. surface a "view job" link in the UI response).
pub async fn enqueue_job(pool: &SqlitePool, input: JobInput) -> Result<i64> {
    let now = now_ms();
    let run_after = input.run_after.unwrap_or(now);
    let payload = serde_json::to_string(&input.payload)?;
    let res = sqlx::query(
        "INSERT INTO jobs
            (kind, payload, status, priority, attempts, max_attempts, run_after, created_at)
         VALUES (?, ?, 'queued', ?, 0, ?, ?, ?)",
    )
    .bind(&input.kind)
    .bind(payload)
    .bind(input.priority)
    .bind(input.max_attempts)
    .bind(run_after)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

/// Enqueue with per-target dedup. Skips if a non-terminal job
/// (queued / running / failed-pending) already exists with the same
/// `kind` AND payload-extracted `dedup_key`. Returns `Some(new_id)`
/// when inserted, `None` when a duplicate was found.
///
/// `dedup_key` should be a string drawn from the payload (e.g.
/// `format!("file:{file_id}")` or `format!("item:{item_id}")`) — we
/// match against `json_extract(payload, '$.<field>')` via a literal
/// `payload LIKE` to avoid the json1 dependency. The payload must
/// include a top-level field matching `dedup_field`.
/// Enqueue a job IF no equivalent row already exists. Returns the new
/// row id on insert, `None` on dedup-skip.
///
/// **Uses `BEGIN IMMEDIATE`** to acquire the writer lock up front rather
/// than upgrading from a read snapshot. The previous deferred-BEGIN
/// implementation hit `SQLITE_BUSY_SNAPSHOT` (code 517) constantly under
/// load — its SELECT-for-dedup grabbed a read snapshot, and any concurrent
/// writer (scanner per-file inserts, worker job-status updates) invalidated
/// it by the time the INSERT tried to commit. The whole tx then needs an
/// application-level retry, which busy_timeout does NOT provide.
///
/// **Wrapped in [`with_busy_retry`]** as a defensive net for the remaining
/// race window: even with `BEGIN IMMEDIATE`, deep contention can still
/// produce code 5 (BUSY waiting for the writer slot) if 30s of
/// busy_timeout polling exhausts. Eight retries with exponential backoff
/// absorb that.
pub async fn enqueue_job_unique(
    pool: &SqlitePool,
    input: JobInput,
    dedup_field: &str,
    dedup_value: i64,
) -> Result<Option<i64>> {
    crate::db::with_busy_retry(|| {
        let input = input.clone();
        async move {
            let mut conn = pool.acquire().await?;
            sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
            let result = enqueue_job_unique_tx(&mut conn, input, dedup_field, dedup_value).await;
            match &result {
                Ok(_) => {
                    sqlx::query("COMMIT").execute(&mut *conn).await?;
                }
                Err(_) => {
                    // Best-effort rollback — sqlx will reset the
                    // connection on return-to-pool regardless.
                    let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                }
            }
            result
        }
    })
    .await
}

/// Connection-scoped variant — call within an existing transaction
/// (the caller is responsible for BEGIN/COMMIT) when enqueuing
/// multiple jobs as a group (e.g. the discovery pipeline's
/// four-kind fan-out per FileAdded). One tx = one fsync instead of
/// one per kind. Takes a raw `SqliteConnection` rather than a
/// `Transaction` so the caller can choose `BEGIN IMMEDIATE` over
/// sqlx's default deferred BEGIN — important on the discovery path
/// where a concurrent scanner write can otherwise trigger
/// `SQLITE_BUSY_SNAPSHOT` (517) when this fn upgrades its read
/// snapshot to a write.
pub async fn enqueue_job_unique_tx(
    conn: &mut sqlx::SqliteConnection,
    input: JobInput,
    dedup_field: &str,
    dedup_value: i64,
) -> Result<Option<i64>> {
    // Switched from a LIKE-on-payload pattern to `json_extract` —
    // SQLite ships with json1 enabled by default, and pulling the
    // field out structurally eliminates the prefix-collision class
    // of bug entirely (no need to reason about `,`/`}` delimiters
    // in the pattern). Cheap: json1 functions are O(payload size)
    // and our payloads are tiny.
    let key = format!("$.{dedup_field}");
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM jobs
         WHERE kind = ?
           AND status IN ('queued', 'running', 'failed')
           AND json_extract(payload, ?) = ?
         LIMIT 1",
    )
    .bind(&input.kind)
    .bind(&key)
    .bind(dedup_value)
    .fetch_optional(&mut *conn)
    .await?;
    if existing.is_some() {
        return Ok(None);
    }
    let id = enqueue_job_tx(conn, input).await?;
    Ok(Some(id))
}

/// Connection-scoped variant of `enqueue_job` — pairs with
/// `enqueue_job_unique_tx` for the batched-pipeline path. Caller
/// owns the surrounding transaction.
pub async fn enqueue_job_tx(conn: &mut sqlx::SqliteConnection, input: JobInput) -> Result<i64> {
    let now = now_ms();
    let run_after = input.run_after.unwrap_or(now);
    let payload = serde_json::to_string(&input.payload)?;
    let res = sqlx::query(
        "INSERT INTO jobs
            (kind, payload, status, priority, attempts, max_attempts, run_after, created_at)
         VALUES (?, ?, 'queued', ?, 0, ?, ?, ?)",
    )
    .bind(&input.kind)
    .bind(payload)
    .bind(input.priority)
    .bind(input.max_attempts)
    .bind(run_after)
    .bind(now)
    .execute(&mut *conn)
    .await?;
    Ok(res.last_insert_rowid())
}

/// Bulk-enqueue one job per file_id under `kind`, with per-file
/// dedup keyed on `file_id`. Wraps the entire batch in a single
/// `BEGIN IMMEDIATE` transaction so the writer lock is held once for
/// the whole batch (one fsync, no 517 race between SELECT and INSERT)
/// instead of acquired N times.
///
/// This is the canonical replacement for the "loop calling
/// `enqueue_job_unique`" pattern in the handler `enqueue_for_files`
/// helpers, which was the hottest 517-trigger when the operator
/// clicked "Process all pending" on a large library.
///
/// Payload shape is fixed at `{ "file_id": <id> }` — matches what every
/// `_for_files` caller in the codebase needs. Bigger / heterogeneous
/// payloads can build their own batched helper alongside this one;
/// the savings come from one tx, not from the payload shape.
///
/// Returns the count of jobs actually inserted (dedup-skipped rows
/// don't count). Wrapped in [`crate::db::with_busy_retry`] so the
/// whole batch retries on transient contention.
pub async fn enqueue_jobs_for_files_batched(
    pool: &SqlitePool,
    kind: &str,
    file_ids: &[i64],
) -> Result<usize> {
    if file_ids.is_empty() {
        return Ok(0);
    }
    crate::db::with_busy_retry(|| async {
        let mut conn = pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        let result: Result<usize> = async {
            let mut queued = 0usize;
            for &file_id in file_ids {
                let payload = serde_json::json!({ "file_id": file_id });
                let input = JobInput::new(kind, payload);
                if enqueue_job_unique_tx(&mut conn, input, "file_id", file_id)
                    .await?
                    .is_some()
                {
                    queued += 1;
                }
            }
            Ok(queued)
        }
        .await;
        match &result {
            Ok(_) => {
                sqlx::query("COMMIT").execute(&mut *conn).await?;
            }
            Err(_) => {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            }
        }
        result
    })
    .await
}

/// Atomically claim the next eligible job. Uses an UPDATE…RETURNING
/// on a SELECT subquery so two concurrent workers can't race onto
/// the same row. SQLite serializes writes, so the UPDATE wins
/// exactly once.
///
/// Eligibility filter:
///   - status = 'queued' OR (status = 'failed' AND run_after <= now)
///   - run_after <= now
///
/// Order: priority DESC, id ASC (FIFO at the same priority).
pub async fn claim_next_job(pool: &SqlitePool) -> Result<Option<JobRow>> {
    let now = now_ms();
    let row = sqlx::query(
        "UPDATE jobs
         SET status     = 'running',
             attempts   = attempts + 1,
             locked_at  = ?,
             started_at = COALESCE(started_at, ?)
         WHERE id = (
             SELECT id FROM jobs
             WHERE (status = 'queued' OR status = 'failed')
               AND run_after <= ?
             ORDER BY priority DESC, id ASC
             LIMIT 1
         )
         RETURNING *",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .fetch_optional(pool)
    .await?;
    row.map(|r| JobRow::from_row(&r)).transpose()
}

/// Force a job into terminal `dead` state without consulting
/// `attempts`/`max_attempts`. Used by the worker when no handler
/// is registered for a job's kind — there's no point in burning
/// retries against an impossible-to-process row.
pub async fn mark_job_dead(pool: &SqlitePool, job_id: i64, error: &str) -> Result<()> {
    crate::db::with_busy_retry(|| async {
        let now = now_ms();
        sqlx::query(
            "UPDATE jobs
             SET status      = 'dead',
                 locked_at   = NULL,
                 finished_at = ?,
                 last_error  = ?
             WHERE id = ?",
        )
        .bind(now)
        .bind(error)
        .bind(job_id)
        .execute(pool)
        .await?;
        Ok(())
    })
    .await
}

/// Heartbeat for a long-running job — refreshes `locked_at` to
/// `now()` so the orphan-reclaim sweep doesn't grab a row out
/// from under a still-alive worker. Cheap (one UPDATE).
pub async fn touch_job_lease(pool: &SqlitePool, job_id: i64) -> Result<()> {
    crate::db::with_busy_retry(|| async {
        sqlx::query("UPDATE jobs SET locked_at = ? WHERE id = ? AND status = 'running'")
            .bind(now_ms())
            .bind(job_id)
            .execute(pool)
            .await?;
        Ok(())
    })
    .await
}

/// Saturation-aware variant of `claim_next_job` — skips any kind
/// listed in `exclude_kinds`. The worker passes the set of kinds
/// whose in-process semaphore is currently full so we never claim
/// a row we'd then have to roll back. Without this, a saturated
/// kind sitting at the head of the queue (lots of pending
/// `detect_markers_file` after a bulk backfill) would starve all
/// other kinds because every poll would re-claim and roll back the
/// same row.
pub async fn claim_next_job_excluding_kinds(
    pool: &SqlitePool,
    exclude_kinds: &[String],
) -> Result<Option<JobRow>> {
    if exclude_kinds.is_empty() {
        return claim_next_job(pool).await;
    }
    let now = now_ms();
    // Build the IN-list of placeholders dynamically. `exclude_kinds`
    // is small (≤ number of registered job kinds) so the SQL stays
    // short. Bind each kind separately so sqlx escapes correctly.
    let placeholders = std::iter::repeat_n("?", exclude_kinds.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "UPDATE jobs
         SET status     = 'running',
             attempts   = attempts + 1,
             locked_at  = ?,
             started_at = COALESCE(started_at, ?)
         WHERE id = (
             SELECT id FROM jobs
             WHERE (status = 'queued' OR status = 'failed')
               AND run_after <= ?
               AND kind NOT IN ({placeholders})
             ORDER BY priority DESC, id ASC
             LIMIT 1
         )
         RETURNING *",
    );
    let mut q = sqlx::query(&sql).bind(now).bind(now).bind(now);
    for k in exclude_kinds {
        q = q.bind(k);
    }
    let row = q.fetch_optional(pool).await?;
    row.map(|r| JobRow::from_row(&r)).transpose()
}

/// Terminal success state.
pub async fn mark_job_succeeded(pool: &SqlitePool, job_id: i64) -> Result<()> {
    // Called by every worker on every successful job completion —
    // wraps in retry because under heavy scan + worker concurrency
    // this is one of the hottest writer paths (job-status churn).
    crate::db::with_busy_retry(|| async {
        let now = now_ms();
        sqlx::query(
            "UPDATE jobs
             SET status       = 'succeeded',
                 locked_at    = NULL,
                 finished_at  = ?,
                 last_error   = NULL
             WHERE id = ?",
        )
        .bind(now)
        .bind(job_id)
        .execute(pool)
        .await?;
        Ok(())
    })
    .await
}

/// Write a per-stage timing JSON blob for one job row. Handlers
/// call this from within `JobContext::scope` after a tacet
/// `analyze_audio` call so the persisted breakdown is visible to
/// the operator UI ("done in 4m 12s · decode 3m 02s · fingerprint
/// 1m 04s") without needing a separate API.
///
/// Best-effort: a write failure here is logged but never fails the
/// surrounding handler — the timings are operator-facing nicety,
/// not data integrity.
pub async fn record_job_stage_timings(
    pool: &SqlitePool,
    job_id: i64,
    timings_json: &str,
) -> Result<()> {
    sqlx::query("UPDATE jobs SET stage_timings_json = ? WHERE id = ?")
        .bind(timings_json)
        .bind(job_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Mark the current attempt as failed. If `attempts < max_attempts`,
/// bump `run_after = now + backoff_ms` and set status back to
/// `failed` (eligible for re-claim later); otherwise transition to
/// `dead` (terminal — admin must re-queue manually).
///
/// Returns `true` when the row transitioned to terminal `dead`, so
/// callers can fan out a one-time notification on that edge instead
/// of querying the row again afterward.
pub async fn mark_job_failed(
    pool: &SqlitePool,
    job_id: i64,
    error: &str,
    backoff_ms: i64,
) -> Result<bool> {
    mark_job_failed_with_class(pool, job_id, error, backoff_ms, None).await
}

/// Same as [`mark_job_failed`] but records an `error_class` (one of
/// the variants in [`crate::jobs::error_class::ErrorClass`] when the
/// caller is the server crate, or an arbitrary string otherwise).
///
/// `force_terminal` short-circuits the attempts/max_attempts check
/// and immediately moves the row to `dead`. Used for terminal
/// failure classes (auth failure, permanent file errors) so the
/// retry budget isn't wasted on errors that won't recover.
///
/// Returns `true` iff the row was moved to terminal `dead`. Callers
/// that need to notify on terminal failure (operator alerts, audit
/// fan-out) gate on the bool so a retryable hiccup doesn't spam
/// notifications.
pub async fn mark_job_failed_with_class(
    pool: &SqlitePool,
    job_id: i64,
    error: &str,
    backoff_ms: i64,
    error_class: Option<&str>,
) -> Result<bool> {
    crate::db::with_busy_retry(|| async {
        let now = now_ms();
        let row = sqlx::query("SELECT attempts, max_attempts FROM jobs WHERE id = ?")
            .bind(job_id)
            .fetch_one(pool)
            .await?;
        let attempts: i64 = row.try_get("attempts")?;
        let max_attempts: i64 = row.try_get("max_attempts")?;
        let force_terminal = matches!(error_class, Some("external_auth") | Some("permanent"));
        let went_terminal = force_terminal || attempts >= max_attempts;
        if went_terminal {
            sqlx::query(
                "UPDATE jobs
                 SET status      = 'dead',
                     locked_at   = NULL,
                     finished_at = ?,
                     last_error  = ?,
                     error_class = ?
                 WHERE id = ?",
            )
            .bind(now)
            .bind(error)
            .bind(error_class)
            .bind(job_id)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE jobs
                 SET status      = 'failed',
                     locked_at   = NULL,
                     run_after   = ?,
                     last_error  = ?,
                     error_class = ?
                 WHERE id = ?",
            )
            .bind(now + backoff_ms)
            .bind(error)
            .bind(error_class)
            .bind(job_id)
            .execute(pool)
            .await?;
        }
        Ok(went_terminal)
    })
    .await
}

/// On startup: any row left as `running` whose `locked_at` is older
/// than the lease ttl is treated as orphaned (worker died) and
/// flipped back to `queued`. `attempts` is preserved — the
/// next worker that picks it up will see attempts already bumped
/// from the previous claim, so a job that keeps crashing the worker
/// still hits `max_attempts` instead of looping forever.
///
/// Returns the count of reclaimed rows for logging.
pub async fn reclaim_orphan_jobs(pool: &SqlitePool, lease_ttl_ms: i64) -> Result<u64> {
    let cutoff = now_ms() - lease_ttl_ms;
    let res = sqlx::query(
        "UPDATE jobs
         SET status    = 'queued',
             locked_at = NULL
         WHERE status = 'running'
           AND (locked_at IS NULL OR locked_at < ?)",
    )
    .bind(cutoff)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// Aggregate counts by status — backs the admin queue dashboard.
pub async fn job_summary(pool: &SqlitePool) -> Result<JobSummary> {
    let row = sqlx::query(
        "SELECT
            SUM(CASE WHEN status = 'queued'    THEN 1 ELSE 0 END) AS queued,
            SUM(CASE WHEN status = 'running'   THEN 1 ELSE 0 END) AS running,
            SUM(CASE WHEN status = 'succeeded' THEN 1 ELSE 0 END) AS succeeded,
            SUM(CASE WHEN status = 'failed'    THEN 1 ELSE 0 END) AS failed,
            SUM(CASE WHEN status = 'dead'      THEN 1 ELSE 0 END) AS dead
         FROM jobs",
    )
    .fetch_one(pool)
    .await?;
    Ok(JobSummary {
        queued: row.try_get::<Option<i64>, _>("queued")?.unwrap_or(0),
        running: row.try_get::<Option<i64>, _>("running")?.unwrap_or(0),
        succeeded: row.try_get::<Option<i64>, _>("succeeded")?.unwrap_or(0),
        failed: row.try_get::<Option<i64>, _>("failed")?.unwrap_or(0),
        dead: row.try_get::<Option<i64>, _>("dead")?.unwrap_or(0),
    })
}

/// List recent jobs for the admin queue page. `kind_filter` narrows
/// to one kind; `status_filter` narrows to one status. `limit`
/// defaults to 100 and is clamped to 500; `offset` defaults to 0.
pub async fn list_jobs(
    pool: &SqlitePool,
    kind_filter: Option<&str>,
    status_filter: Option<JobStatus>,
    limit: i64,
    offset: i64,
) -> Result<Vec<JobRow>> {
    let limit = limit.clamp(1, 500);
    let offset = offset.max(0);
    let mut sql = String::from("SELECT * FROM jobs WHERE 1 = 1");
    if kind_filter.is_some() {
        sql.push_str(" AND kind = ?");
    }
    if status_filter.is_some() {
        sql.push_str(" AND status = ?");
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
    let mut q = sqlx::query(&sql);
    if let Some(k) = kind_filter {
        q = q.bind(k);
    }
    if let Some(s) = status_filter {
        q = q.bind(s.as_db_str());
    }
    q = q.bind(limit).bind(offset);
    let rows = q.fetch_all(pool).await?;
    rows.iter().map(JobRow::from_row).collect()
}

/// Total job count matching the same filters as [`list_jobs`].
/// Companion to that function for the admin queue pagination
/// footer — knowing the total lets us render "X–Y of Z" + a
/// last-page button without an extra round trip on every page tick.
pub async fn count_jobs(
    pool: &SqlitePool,
    kind_filter: Option<&str>,
    status_filter: Option<JobStatus>,
) -> Result<i64> {
    let mut sql = String::from("SELECT COUNT(*) AS n FROM jobs WHERE 1 = 1");
    if kind_filter.is_some() {
        sql.push_str(" AND kind = ?");
    }
    if status_filter.is_some() {
        sql.push_str(" AND status = ?");
    }
    let mut q = sqlx::query(&sql);
    if let Some(k) = kind_filter {
        q = q.bind(k);
    }
    if let Some(s) = status_filter {
        q = q.bind(s.as_db_str());
    }
    let row = q.fetch_one(pool).await?;
    Ok(row.try_get("n").unwrap_or(0))
}

/// Periodic cleanup that bounds the `jobs` table size. Without
/// this, succeeded rows accumulate forever — a single library
/// backfill produces tens of thousands of succeeded rows, and a
/// year of daily subtitle sweeps adds more on top.
///
/// Default retention:
///   - `succeeded` rows older than `succeeded_ttl_ms` → DELETE
///   - `dead` rows older than `dead_ttl_ms` → DELETE
///
/// Returns `(succeeded_removed, dead_removed)` for the operator's
/// task log.
pub async fn cleanup_old_jobs(
    pool: &SqlitePool,
    succeeded_ttl_ms: i64,
    dead_ttl_ms: i64,
) -> Result<(u64, u64)> {
    let now = now_ms();
    let succ = sqlx::query(
        "DELETE FROM jobs
         WHERE status = 'succeeded'
           AND finished_at IS NOT NULL
           AND finished_at < ?",
    )
    .bind(now - succeeded_ttl_ms)
    .execute(pool)
    .await?
    .rows_affected();
    let dead = sqlx::query(
        "DELETE FROM jobs
         WHERE status = 'dead'
           AND finished_at IS NOT NULL
           AND finished_at < ?",
    )
    .bind(now - dead_ttl_ms)
    .execute(pool)
    .await?
    .rows_affected();
    Ok((succ, dead))
}

/// Outcome of a [merge_items] call. All counts are zero on failure
/// (the transaction is rolled back), so a non-zero `moved_files`
/// indicates the merge committed.
#[derive(Debug, serde::Serialize)]
pub struct MergeReport {
    /// Number of media files re-pointed from the source item / its
    /// episodes onto the target.
    pub moved_files: u64,
    /// New season rows created on the target because the source had a
    /// season number the target lacked.
    pub created_seasons: u64,
    /// New episode rows created on the target because the source had
    /// an (season, episode) the target lacked.
    pub created_episodes: u64,
}

/// Move every media file from `source_id` onto `target_id`, then delete
/// the source item row. Used by the admin "Merge into…" affordance to
/// fix duplicate items (typically created when TMDB enrichment renamed
/// a show after initial scan, breaking the sort_title-based dedup).
///
/// Both items must be in the same library and have the same kind.
/// For shows, seasons/episodes are matched by `(season_number, episode_number)`
/// and created on the target when missing. If both items have a media
/// file attached to the same `(season, episode)` the merge is refused —
/// the operator must remove one before retrying.
///
/// Transactional: any error rolls back the whole operation.
pub async fn merge_items(pool: &SqlitePool, source_id: i64, target_id: i64) -> Result<MergeReport> {
    if source_id == target_id {
        anyhow::bail!("source and target are the same item");
    }

    // Acquire a connection and start the tx with BEGIN IMMEDIATE so
    // we hold the WAL write lock from the first statement. The
    // default `pool.begin()` issues plain BEGIN (deferred), which
    // upgrades from a read snapshot on the first write and can race
    // other writers → SQLITE_BUSY mid-merge. Merge touches many
    // rows (every episode of the source); the upfront lock is worth
    // the small contention cost. We commit/rollback manually rather
    // than via sqlx::Transaction because the latter always issues a
    // deferred BEGIN.
    let mut conn = pool.acquire().await?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .context("BEGIN IMMEDIATE for merge")?;
    let result = merge_items_inner(&mut conn, source_id, target_id).await;
    match &result {
        Ok(_) => {
            sqlx::query("COMMIT").execute(&mut *conn).await?;
        }
        Err(_) => {
            // Best-effort rollback. If this fails the connection
            // gets dropped, which auto-rolls-back the tx anyway.
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
        }
    }
    result
}

async fn merge_items_inner(
    conn: &mut sqlx::SqliteConnection,
    source_id: i64,
    target_id: i64,
) -> Result<MergeReport> {
    #[derive(sqlx::FromRow)]
    struct ItemHeader {
        library_id: i64,
        kind: String,
    }
    let source: ItemHeader = sqlx::query_as("SELECT library_id, kind FROM items WHERE id = ?")
        .bind(source_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source item {source_id} not found"))?;
    let target: ItemHeader = sqlx::query_as("SELECT library_id, kind FROM items WHERE id = ?")
        .bind(target_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("target item {target_id} not found"))?;
    if source.library_id != target.library_id {
        anyhow::bail!("source and target are in different libraries");
    }
    if source.kind != target.kind {
        anyhow::bail!(
            "kind mismatch: source is {} but target is {}",
            source.kind,
            target.kind
        );
    }

    let mut report = MergeReport {
        moved_files: 0,
        created_seasons: 0,
        created_episodes: 0,
    };

    match source.kind.as_str() {
        "movie" => {
            // Movies are flat: re-point media_files.item_id from source to
            // target. CHECK constraint on media_files allows item_id OR
            // episode_id but not both, so we never need to touch episode_id.
            let moved = sqlx::query("UPDATE media_files SET item_id = ? WHERE item_id = ?")
                .bind(target_id)
                .bind(source_id)
                .execute(&mut *conn)
                .await?
                .rows_affected();
            report.moved_files = moved;
        }
        "show" => {
            #[derive(sqlx::FromRow)]
            struct SourceEp {
                episode_id: i64,
                season_number: i32,
                episode_number: i32,
                episode_title: String,
            }
            // Pull every source episode in one query — cheaper than
            // walking seasons one at a time, and the result fits
            // comfortably in memory even for long-running shows.
            let source_eps: Vec<SourceEp> = sqlx::query_as(
                "SELECT e.id AS episode_id, s.season_number, e.episode_number, e.title AS episode_title
                 FROM episodes e
                 JOIN seasons s ON e.season_id = s.id
                 WHERE s.show_id = ?",
            )
            .bind(source_id)
            .fetch_all(&mut *conn)
            .await?;

            let now = now_ms();
            for ep in source_eps {
                // Find or create the matching season on the target.
                let existing_season: Option<(i64,)> = sqlx::query_as(
                    "SELECT id FROM seasons WHERE show_id = ? AND season_number = ?",
                )
                .bind(target_id)
                .bind(ep.season_number)
                .fetch_optional(&mut *conn)
                .await?;
                let target_season_id = if let Some((id,)) = existing_season {
                    id
                } else {
                    let row = sqlx::query(
                        "INSERT INTO seasons (show_id, season_number) VALUES (?, ?) RETURNING id",
                    )
                    .bind(target_id)
                    .bind(ep.season_number)
                    .fetch_one(&mut *conn)
                    .await?;
                    report.created_seasons += 1;
                    row.try_get::<i64, _>("id")?
                };

                // Find or create the matching episode on the target.
                let existing_episode: Option<(i64,)> = sqlx::query_as(
                    "SELECT id FROM episodes WHERE season_id = ? AND episode_number = ?",
                )
                .bind(target_season_id)
                .bind(ep.episode_number)
                .fetch_optional(&mut *conn)
                .await?;
                let target_episode_id = if let Some((id,)) = existing_episode {
                    // Conflict guard: if the target already has a
                    // media_file for this episode, we have two physical
                    // copies of the same logical episode. Refuse the
                    // merge rather than guess which to keep.
                    let target_has_file: Option<(i64,)> =
                        sqlx::query_as("SELECT id FROM media_files WHERE episode_id = ? LIMIT 1")
                            .bind(id)
                            .fetch_optional(&mut *conn)
                            .await?;
                    let source_has_file: Option<(i64,)> =
                        sqlx::query_as("SELECT id FROM media_files WHERE episode_id = ? LIMIT 1")
                            .bind(ep.episode_id)
                            .fetch_optional(&mut *conn)
                            .await?;
                    if target_has_file.is_some() && source_has_file.is_some() {
                        anyhow::bail!(
                            "merge refused: both items have a media file for S{:02}E{:02}. Delete one before merging.",
                            ep.season_number,
                            ep.episode_number,
                        );
                    }
                    id
                } else {
                    let row = sqlx::query(
                        "INSERT INTO episodes (season_id, episode_number, title, added_at, updated_at)
                         VALUES (?, ?, ?, ?, ?) RETURNING id",
                    )
                    .bind(target_season_id)
                    .bind(ep.episode_number)
                    .bind(&ep.episode_title)
                    .bind(now)
                    .bind(now)
                    .fetch_one(&mut *conn)
                    .await?;
                    report.created_episodes += 1;
                    row.try_get::<i64, _>("id")?
                };

                // Re-point any media_files on the source episode to the
                // target episode. We've already verified above that we
                // don't double-attach.
                let moved =
                    sqlx::query("UPDATE media_files SET episode_id = ? WHERE episode_id = ?")
                        .bind(target_episode_id)
                        .bind(ep.episode_id)
                        .execute(&mut *conn)
                        .await?
                        .rows_affected();
                report.moved_files += moved;
            }
        }
        other => anyhow::bail!("unsupported kind {other:?}"),
    }

    // Cascade-delete the source. Seasons → episodes go via ON DELETE
    // CASCADE; the media files we re-pointed above survive because
    // they no longer reference any source row.
    sqlx::query("DELETE FROM items WHERE id = ?")
        .bind(source_id)
        .execute(&mut *conn)
        .await?;

    Ok(report)
}

/// Immediately delete every `dead` row regardless of `finished_at`.
/// Backs the admin "Clear dead" button. Useful when stale rows
/// accumulate from a renamed/removed job kind — those rows never get
/// retried (no handler) and `cleanup_old_jobs` waits for the TTL
/// before sweeping. Returns the count of rows deleted.
pub async fn clear_dead_jobs(pool: &SqlitePool) -> Result<u64> {
    let res = sqlx::query("DELETE FROM jobs WHERE status = 'dead'")
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Bulk delete queued jobs — admin "wipe queue" affordance for the
/// case where the operator clicked Process all by mistake on a
/// huge library and wants to bail out. Only `queued` rows are
/// removed; `running` jobs are mid-work and `failed` rows are in
/// retry-backoff so the operator is presumed to want those kept.
/// Returns the count of rows deleted.
///
/// Optional `kind_filter`: when set, only wipes that kind. Useful
/// when the operator regrets only one type of pending work (e.g.
/// "cancel all queued loudness jobs but keep the markers ones").
pub async fn wipe_queued_jobs(pool: &SqlitePool, kind_filter: Option<&str>) -> Result<u64> {
    let res = if let Some(k) = kind_filter {
        sqlx::query("DELETE FROM jobs WHERE status = 'queued' AND kind = ?")
            .bind(k)
            .execute(pool)
            .await?
    } else {
        sqlx::query("DELETE FROM jobs WHERE status = 'queued'")
            .execute(pool)
            .await?
    };
    Ok(res.rows_affected())
}

/// Admin "retry" — push a dead/failed row back into the queue with
/// a fresh attempt counter. No-op on terminal succeeded rows.
pub async fn requeue_job(pool: &SqlitePool, job_id: i64) -> Result<bool> {
    let res = sqlx::query(
        "UPDATE jobs
         SET status      = 'queued',
             attempts    = 0,
             run_after   = ?,
             locked_at   = NULL,
             started_at  = NULL,
             finished_at = NULL,
             last_error  = NULL
         WHERE id = ?
           AND status IN ('failed', 'dead')",
    )
    .bind(now_ms())
    .bind(job_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Bulk variant of `set_watched` for whole shows. Marks every
/// episode of `show_id` as watched/unwatched in one transaction and
/// returns the affected episode_ids so the caller can fan out the
/// Trakt history push. Position is set to each episode's own
/// duration_ms (so the resume rail reflects the full progress bar).
pub async fn set_all_episodes_watched_for_show(
    pool: &SqlitePool,
    user_id: i64,
    show_id: i64,
    watched: bool,
) -> Result<Vec<i64>> {
    let now = now_ms();
    let view_count_delta: i64 = if watched { 1 } else { 0 };

    let episode_rows = sqlx::query(
        "SELECT e.id, e.duration_ms
         FROM episodes e
         JOIN seasons s ON s.id = e.season_id
         WHERE s.show_id = ?",
    )
    .bind(show_id)
    .fetch_all(pool)
    .await?;

    let mut episode_ids = Vec::with_capacity(episode_rows.len());
    let mut tx = pool.begin().await?;
    for r in &episode_rows {
        let id: i64 = r.try_get("id")?;
        let duration_ms: Option<i64> = r.try_get("duration_ms").ok().flatten();
        let position_ms = if watched { duration_ms.unwrap_or(0) } else { 0 };
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
        .execute(&mut *tx)
        .await?;
        episode_ids.push(id);
    }
    tx.commit().await?;
    Ok(episode_ids)
}

/// Top reviews for one item, ordered by rating desc then recency.
/// Paginated so popular items (TMDB hands back hundreds of rows for
/// blockbusters) don't dump megabytes of JSON on every modal open.
/// `limit` is clamped to `1..=100`; `offset` is clamped to `>= 0`.
pub async fn list_reviews_for_item(
    pool: &SqlitePool,
    item_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<Review>> {
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);
    let rows = sqlx::query(
        "SELECT id, item_id, source, author, author_url, avatar_url,
                rating, body, created_at
         FROM item_reviews
         WHERE item_id = ?
         ORDER BY (rating IS NULL) ASC, rating DESC, created_at DESC
         LIMIT ? OFFSET ?",
    )
    .bind(item_id)
    .bind(limit)
    .bind(offset)
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

/// Total review count for one item — drives the pagination footer on
/// the reviews response so the client can render "Showing 12 of 240".
pub async fn count_reviews_for_item(pool: &SqlitePool, item_id: i64) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM item_reviews WHERE item_id = ?")
        .bind(item_id)
        .fetch_one(pool)
        .await?;
    Ok(row.try_get("n")?)
}

/// Replace one source's reviews for an item. Source-scoped DELETE so
/// reviews from other agents (and per-user reviews tagged `source =
/// 'user'`) survive. Provider-agnostic — takes the common
/// [`chimpflix_metadata::ReviewEntry`] shape any agent can populate.
pub async fn apply_item_reviews_for_source(
    pool: &SqlitePool,
    item_id: i64,
    reviews: &[chimpflix_metadata::ReviewEntry],
    source: &str,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM item_reviews WHERE item_id = ? AND source = ?")
        .bind(item_id)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    for r in reviews {
        sqlx::query(
            "INSERT OR IGNORE INTO item_reviews
                (item_id, source, source_id, author, author_url, avatar_url,
                 rating, body, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(item_id)
        .bind(source)
        .bind(&r.source_id)
        .bind(&r.author)
        .bind(r.author_url.as_deref())
        .bind(r.avatar_url.as_deref())
        .bind(r.rating)
        .bind(r.body.as_deref())
        .bind(r.created_at_ms)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

// `parse_iso8601_ms` removed — its only caller was the old
// `apply_item_extras(&[TmdbVideo])`. The new VideoLink-based signature
// receives `published_at_ms` already parsed by the metadata crate
// (`chrono_lite` in agents.rs).

/// Write per-episode AniList data (title from `streamingEpisodes`, plus
/// thumbnail) honoring chain-position semantics.
///
/// In `WriteMode::Primary`, AniList is the primary episode source: the
/// title overwrites any existing value that wasn't owner-locked, and
/// the thumbnail replaces any existing one. In `WriteMode::FillNulls`,
/// AniList only writes when the existing title looks filename-derived
/// (a heuristic — `episodes.title` is `NOT NULL` so we can't use a raw
/// IS NULL check) and the thumbnail is set via `store_image_if_missing`.
///
/// **Why this got reworked:** the prior implementation used
/// `COALESCE(title, ?)` against a `NOT NULL` column, which is a
/// universal no-op — the existing title (filename stem inserted at
/// upsert time) always won, so AniList's title never landed. That bug
/// was masked by TMDB unconditionally overwriting episode titles
/// regardless of chain position; with TMDB removed from a chain the
/// bug surfaced as "every anime episode shows the raw filename stem."
///
/// Returns `Ok(true)` when at least one column was written; `Ok(false)`
/// otherwise. Owner-locked title columns are honored in both modes via
/// the existing `fetch_locked_fields` machinery.
pub async fn apply_episode_metadata_anilist(
    pool: &SqlitePool,
    episode_id: i64,
    title: &str,
    thumbnail_url: Option<&str>,
    mode: WriteMode,
) -> Result<bool> {
    let now = now_ms();
    // Decide whether to overwrite the title. Primary mode always wins
    // over filename-derived stems; fill-nulls only wins over stems that
    // look like they came from the parser fallback.
    let should_overwrite = match mode {
        WriteMode::Primary => true,
        WriteMode::FillNulls => {
            let row = sqlx::query("SELECT title FROM episodes WHERE id = ?")
                .bind(episode_id)
                .fetch_optional(pool)
                .await?;
            match row {
                Some(r) => {
                    let existing: String = r.try_get("title").unwrap_or_default();
                    looks_filename_derived(&existing)
                }
                None => false,
            }
        }
    };

    let res = if should_overwrite {
        sqlx::query(
            "UPDATE episodes SET title = ?, updated_at = ? WHERE id = ?",
        )
        .bind(title)
        .bind(now)
        .bind(episode_id)
        .execute(pool)
        .await?
    } else {
        sqlx::query("UPDATE episodes SET updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(episode_id)
            .execute(pool)
            .await?
    };
    let mut any_change = should_overwrite && res.rows_affected() > 0;

    if let Some(url) = thumbnail_url {
        let wrote_thumb = match mode {
            WriteMode::Primary => {
                store_image(pool, None, Some(episode_id), "thumb", "anilist", url).await?;
                true
            }
            WriteMode::FillNulls => {
                store_image_if_missing(pool, None, Some(episode_id), "thumb", "anilist", url)
                    .await?;
                // store_image_if_missing doesn't report whether it wrote; assume
                // it might have. Cheap to assume true for the activity log.
                true
            }
        };
        any_change = any_change || wrote_thumb;
    }
    Ok(any_change)
}

/// Heuristic — does this title string look like it came from a raw
/// filename stem rather than a metadata agent? Matches:
///   - Empty / whitespace-only
///   - Starts with `Episode \d+` (parser fallback)
///   - Contains common quality / codec tokens (1080p / WEB-DL / HEVC / x265 / x264 / BluRay / 2160p / 720p)
///   - Starts with `\d{2,4}\s*-` (anime absolute-episode prefix like "013 - ")
///   - Ends with `-Word` where Word is a single capitalized release-group
///     token (`-Kitsune`, `-ToonsHub`, `-AnoZu`) — heuristic: dash with
///     no surrounding space immediately before a non-whitespace run at
///     end of string.
///
/// Errs on the side of overwriting. A real metadata-derived title that
/// happens to contain "1080p" (very rare) will get overwritten by a
/// later AniList write — acceptable; metadata writers come from a
/// higher-quality source than the filename anyway.
fn looks_filename_derived(title: &str) -> bool {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.starts_with("Episode ") || trimmed == "Episode" {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    const TOKENS: &[&str] = &[
        "1080p", "720p", "2160p", "480p", "web-dl", "webdl", "bluray",
        "blu-ray", "hevc", "x265", "x264", "h.264", "h264", "h.265", "h265",
        "10bit", "8bit", "aac", "flac", "ddp5.1", "ddp", "remux",
    ];
    if TOKENS.iter().any(|t| lower.contains(t)) {
        return true;
    }
    // Anime absolute-episode prefix: "^\d{2,4}\s*-"
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if (2..=4).contains(&i) {
        let rest = &trimmed[i..];
        if rest.starts_with(|c: char| c.is_whitespace() || c == '-') {
            return true;
        }
    }
    // Trailing release-group: ends with `-Token` where Token has no
    // internal whitespace and starts with an uppercase letter or digit.
    // The char before the dash is irrelevant (real release patterns are
    // both " -Kitsune" and "Day-Kitsune"). What disambiguates is no
    // whitespace *after* the dash and the capitalized-token shape.
    // A real title like "Mockingjay - Part 1" is preserved because the
    // dash is followed by whitespace.
    if let Some(last_dash) = trimmed.rfind('-') {
        let after = &trimmed[last_dash + 1..];
        if !after.is_empty() && !after.contains(char::is_whitespace) {
            let first = after.chars().next().unwrap();
            if first.is_ascii_uppercase() || first.is_ascii_digit() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod looks_filename_derived_tests {
    use super::looks_filename_derived;

    #[test]
    fn detects_anime_absolute_prefix() {
        assert!(looks_filename_derived("013 - Barrier Day"));
        assert!(looks_filename_derived("014 - The Party from Hell"));
        assert!(looks_filename_derived("01 - Pilot"));
    }

    #[test]
    fn detects_trailing_release_group() {
        assert!(looks_filename_derived("Barrier Day -Kitsune"));
        assert!(looks_filename_derived("The First Bloom -ToonsHub"));
        assert!(looks_filename_derived("The Day of Departure -AnoZu"));
    }

    #[test]
    fn detects_quality_tokens() {
        assert!(looks_filename_derived("Episode Name 1080p WEB-DL"));
        assert!(looks_filename_derived("Movie x265 HEVC"));
        assert!(looks_filename_derived("Show 2160p BluRay"));
    }

    #[test]
    fn detects_parser_fallback() {
        assert!(looks_filename_derived(""));
        assert!(looks_filename_derived("Episode 7"));
        assert!(looks_filename_derived("Episode"));
    }

    #[test]
    fn accepts_real_titles() {
        assert!(!looks_filename_derived("Commanders' Meeting"));
        assert!(!looks_filename_derived("Ren's Shadow"));
        assert!(!looks_filename_derived("A Storm Rolls In"));
        assert!(!looks_filename_derived("self-titled debut"));
    }

    #[test]
    fn handles_dash_in_real_title() {
        // "The Hunger Games: Mockingjay - Part 1" should NOT be filtered:
        // there's whitespace before the dash → not a release tag.
        assert!(!looks_filename_derived("Mockingjay - Part 1"));
    }
}

/// Polymorphic episode writer. Mirror of [`apply_movie_data`] /
/// [`apply_show_data`] for the per-episode level. The first agent in
/// the chain to return non-`None` from `fetch_episode` lands its data
/// in `Primary` mode (overwriting filename-derived columns); later
/// agents fill nulls. Title overwrite uses the same
/// `looks_filename_derived` heuristic from
/// [`apply_episode_metadata_anilist`] so a stem-derived title yields
/// to any real metadata title.
///
/// `source` is the agent name ("tmdb" / "tvdb" / "tvmaze" / "anilist")
/// used to attribute the still image in the `images` table.
pub async fn apply_episode_data(
    pool: &SqlitePool,
    episode_id: i64,
    data: &chimpflix_metadata::EpisodeData,
    mode: WriteMode,
    source: &str,
) -> Result<()> {
    let now = now_ms();

    // Decide title overwrite based on mode + heuristic. `episodes.title`
    // is NOT NULL so we can't use a raw IS NULL check.
    let should_overwrite_title = match (mode, data.title.as_deref()) {
        (WriteMode::Primary, Some(_)) => true,
        (WriteMode::FillNulls, Some(_)) => {
            let row = sqlx::query("SELECT title FROM episodes WHERE id = ?")
                .bind(episode_id)
                .fetch_optional(pool)
                .await?;
            match row {
                Some(r) => {
                    let existing: String = r.try_get("title").unwrap_or_default();
                    looks_filename_derived(&existing)
                }
                None => false,
            }
        }
        _ => false,
    };

    if should_overwrite_title
        && let Some(title) = &data.title
    {
        sqlx::query("UPDATE episodes SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title)
            .bind(now)
            .bind(episode_id)
            .execute(pool)
            .await?;
    }

    // Other columns honor mode via COALESCE.
    let runtime_ms = data.runtime_ms;
    let air_date_ms = data.air_date_ms;
    let summary = data.summary.as_deref();
    let tmdb_id = data.tmdb_id;
    let tvdb_id = data.tvdb_id;

    if mode.overwrites() {
        sqlx::query(
            "UPDATE episodes SET
                summary = COALESCE(?, summary),
                duration_ms = COALESCE(duration_ms, ?),
                air_date = COALESCE(air_date, ?),
                tmdb_id = COALESCE(?, tmdb_id),
                tvdb_id = COALESCE(?, tvdb_id),
                updated_at = ?
             WHERE id = ?",
        )
        .bind(summary)
        .bind(runtime_ms)
        .bind(air_date_ms)
        .bind(tmdb_id)
        .bind(tvdb_id)
        .bind(now)
        .bind(episode_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE episodes SET
                summary = COALESCE(summary, ?),
                duration_ms = COALESCE(duration_ms, ?),
                air_date = COALESCE(air_date, ?),
                tmdb_id = COALESCE(tmdb_id, ?),
                tvdb_id = COALESCE(tvdb_id, ?),
                updated_at = ?
             WHERE id = ?",
        )
        .bind(summary)
        .bind(runtime_ms)
        .bind(air_date_ms)
        .bind(tmdb_id)
        .bind(tvdb_id)
        .bind(now)
        .bind(episode_id)
        .execute(pool)
        .await?;
    }

    // Refuse to write a blank/whitespace still URL — that would land
    // as `<img src="">` in the UI and render as a black thumbnail (the
    // browser interprets "" as a self-reference to the current page).
    // Belt-and-suspenders against agents that surface empty strings
    // instead of None for episodes without stills.
    let still_url = data
        .still_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(url) = still_url {
        if mode.overwrites() {
            store_image(pool, None, Some(episode_id), "thumb", source, url).await?;
        } else {
            store_image_if_missing(pool, None, Some(episode_id), "thumb", source, url).await?;
        }
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
    .bind(meta.tmdb_id)
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
    let lock_field = kind;
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
        let item_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM items WHERE library_id = ?")
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
        let file_count: i64 = row
            .iter()
            .map(|r| r.try_get::<i64, _>("c").unwrap_or(0))
            .sum();
        let total_bytes: i64 = row
            .iter()
            .map(|r| r.try_get::<i64, _>("b").unwrap_or(0))
            .sum();

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
    let rows = sqlx::query("SELECT * FROM scan_jobs ORDER BY created_at DESC LIMIT ?")
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
    if let Some(v) = patch.setup_completed {
        sqlx::query("UPDATE server_settings SET setup_completed = ? WHERE id = 1")
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
    if let Some(v) = patch.job_workers {
        sqlx::query("UPDATE server_settings SET job_workers = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.job_kind_concurrency {
        sqlx::query("UPDATE server_settings SET job_kind_concurrency = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.transcoder_hdr_tonemap_enabled {
        sqlx::query("UPDATE server_settings SET transcoder_hdr_tonemap_enabled = ? WHERE id = 1")
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
    if let Some(v) = patch.file_watcher_use_polling {
        sqlx::query("UPDATE server_settings SET file_watcher_use_polling = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.file_watcher_poll_interval_secs {
        // Clamp on the way in so the watcher startup doesn't have to
        // defend against a 0-second poll loop (which would peg CPU).
        let clamped = v.clamp(5, 3600);
        sqlx::query("UPDATE server_settings SET file_watcher_poll_interval_secs = ? WHERE id = 1")
            .bind(clamped)
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
        sqlx::query("UPDATE server_settings SET continue_watching_max_age_weeks = ? WHERE id = 1")
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
    if let Some(v) = patch.subtitle_default_offset_ms {
        sqlx::query("UPDATE server_settings SET subtitle_default_offset_ms = ? WHERE id = 1")
            .bind(v.clamp(-30_000, 30_000))
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
    if let Some(v) = patch.preroll_volume {
        sqlx::query("UPDATE server_settings SET preroll_volume = ? WHERE id = 1")
            .bind(v.clamp(0, 100))
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
            anyhow::bail!("bind_interface must be empty or a SocketAddr (e.g. 192.168.1.50:8080)");
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
            anyhow::bail!("metadata_language must be a 1–12 char BCP-47 tag (e.g. en-US, ja-JP)");
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            anyhow::bail!("metadata_language must contain only ASCII letters, digits, and dashes");
        }
        sqlx::query("UPDATE server_settings SET metadata_language = ? WHERE id = 1")
            .bind(trimmed)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.recently_added_days {
        if !(0..=365).contains(&v) {
            anyhow::bail!("recently_added_days must be between 0 and 365 (0 disables the badge)");
        }
        sqlx::query("UPDATE server_settings SET recently_added_days = ? WHERE id = 1")
            .bind(v)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.loudness_analysis_enabled {
        sqlx::query("UPDATE server_settings SET loudness_analysis_enabled = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.subtitle_fetch_enabled {
        sqlx::query("UPDATE server_settings SET subtitle_fetch_enabled = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.embedded_subs_extract_enabled {
        sqlx::query("UPDATE server_settings SET embedded_subs_extract_enabled = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.external_ratings_enabled {
        sqlx::query("UPDATE server_settings SET external_ratings_enabled = ? WHERE id = 1")
            .bind(i64::from(v))
            .execute(&mut *tx)
            .await?;
    }
    if let Some(v) = patch.backup_retention_count {
        // Clamp to a sane range — operators occasionally fat-finger
        // 0 or a wild number. 0 disables pruning (documented in the
        // setting); cap at 365 (≈ a year of dailies) so nobody hides
        // a leak behind a giant retention window.
        let clamped = v.clamp(0, 365);
        sqlx::query("UPDATE server_settings SET backup_retention_count = ? WHERE id = 1")
            .bind(clamped)
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
        sqlx::query("SELECT * FROM audit_log WHERE id < ? ORDER BY id DESC LIMIT ?")
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

/// Offset/limit variant for the admin UI's paged audit table.
/// Cursor-based [`list_audit`] is kept for any future API consumer
/// that wants O(1) deep-scroll; the admin page wants jump-to-page and
/// total-count, which means we pay an OFFSET (acceptable here:
/// audit_log is bounded by retention, typically <100k rows).
pub async fn list_audit_paged(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditLogEntry>> {
    let limit = limit.clamp(1, 200);
    let offset = offset.max(0);
    let rows = sqlx::query("SELECT * FROM audit_log ORDER BY id DESC LIMIT ? OFFSET ?")
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    rows.iter().map(AuditLogEntry::from_row).collect()
}

/// Per-actor variant for the user drawer's Audit tab. Filters by
/// `actor_user_id` so an admin can see what one user has done across
/// the server. Offset/limit shape matches [`list_audit_paged`]; the
/// drawer doesn't expose jump-to-page but the consistent shape lets
/// callers swap freely.
pub async fn list_audit_for_user(
    pool: &SqlitePool,
    actor_user_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditLogEntry>> {
    let limit = limit.clamp(1, 200);
    let offset = offset.max(0);
    let rows = sqlx::query(
        "SELECT * FROM audit_log \
         WHERE actor_user_id = ? \
         ORDER BY id DESC LIMIT ? OFFSET ?",
    )
    .bind(actor_user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    rows.iter().map(AuditLogEntry::from_row).collect()
}

/// Total audit_log rows. Companion to [`list_audit_paged`] for the
/// pagination footer.
pub async fn count_audit(pool: &SqlitePool) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM audit_log")
        .fetch_one(pool)
        .await?;
    Ok(row.try_get("n").unwrap_or(0))
}

/// Count of audit_log rows attributed to a specific actor.
pub async fn count_audit_for_user(pool: &SqlitePool, actor_user_id: i64) -> Result<i64> {
    let row = sqlx::query("SELECT COUNT(*) AS n FROM audit_log WHERE actor_user_id = ?")
        .bind(actor_user_id)
        .fetch_one(pool)
        .await?;
    Ok(row.try_get("n").unwrap_or(0))
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
    // Atomically flip the chosen rows from 'queued' to 'running' inside
    // the same statement that selects them, then return the updated
    // rows. Same hazard as `claim_due_tasks`: a separate
    // SELECT-then-UPDATE leaves a window where a concurrent scheduler
    // tick can claim the same row, spawning two ffmpegs against the
    // same output path. SQLite serializes overlapping UPDATEs, so
    // the second tick observes 'running' for any rows the first
    // grabbed and skips them.
    //
    // `mark_optimized_running` is still called afterward to record
    // the resolved output_path — its UPDATE on `status` becomes a
    // no-op (already 'running') but the path write is meaningful.
    let rows = sqlx::query(
        "UPDATE optimized_versions
         SET status = 'running'
         WHERE id IN (
             SELECT id FROM optimized_versions
             WHERE status = 'queued'
             ORDER BY created_at ASC
             LIMIT ?
         )
         RETURNING *",
    )
    .bind(limit.clamp(1, 16))
    .fetch_all(pool)
    .await?;
    rows.iter().map(OptimizedVersion::from_row).collect()
}

pub async fn mark_optimized_running(pool: &SqlitePool, id: i64, output_path: &str) -> Result<()> {
    sqlx::query("UPDATE optimized_versions SET status = 'running', output_path = ? WHERE id = ?")
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
    // Return the path so the caller can unlink the file. Single
    // statement so the SELECT and DELETE can't be interleaved by a
    // concurrent caller that ALSO wants to delete this row (e.g. an
    // admin and a scheduler cleanup both reacting to the same
    // "finished" event). `DELETE ... RETURNING` lands the row's
    // output_path in our hand before SQLite drops the row.
    let row = sqlx::query("DELETE FROM optimized_versions WHERE id = ? RETURNING output_path")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let path: Option<String> = row.and_then(|r| r.try_get::<String, _>("output_path").ok());
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
    rows.iter()
        .map(|row| Webhook::from_row(row, vault))
        .collect()
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
    offset: i64,
) -> Result<Vec<WebhookDelivery>> {
    let limit = limit.clamp(1, 200);
    let offset = offset.max(0);
    let rows = sqlx::query(
        "SELECT * FROM webhook_deliveries
         WHERE webhook_id = ?
         ORDER BY created_at DESC
         LIMIT ? OFFSET ?",
    )
    .bind(webhook_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    rows.iter().map(WebhookDelivery::from_row).collect()
}

pub async fn count_webhook_deliveries(pool: &SqlitePool, webhook_id: i64) -> Result<i64> {
    let total: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id = ?")
            .bind(webhook_id)
            .fetch_one(pool)
            .await?;
    Ok(total.0)
}

// ---------------------------------------------------------------------------
// Transcoder presets
// ---------------------------------------------------------------------------

pub async fn list_transcoder_presets(pool: &SqlitePool) -> Result<Vec<TranscoderPreset>> {
    let rows = sqlx::query("SELECT * FROM transcoder_presets ORDER BY sort_order ASC, id ASC")
        .fetch_all(pool)
        .await?;
    rows.iter().map(TranscoderPreset::from_row).collect()
}

pub async fn get_transcoder_preset(pool: &SqlitePool, id: i64) -> Result<Option<TranscoderPreset>> {
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
                sqlx::query(concat!(
                    "UPDATE transcoder_presets SET ",
                    $col,
                    " = ?, updated_at = ? WHERE id = ?"
                ))
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
    let rows =
        sqlx::query("SELECT * FROM scheduled_tasks ORDER BY enabled DESC, next_run_at ASC, id ASC")
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
    // INSERT + the follow-up SELECT have to use the same connection,
    // otherwise the SELECT can land on a pool connection that still
    // holds a pre-INSERT WAL snapshot and the row appears to have
    // "disappeared". Hit this in production during seeding — the
    // INSERT committed (next AUTOINCREMENT id confirmed it) but a
    // sibling pool connection couldn't see it yet.
    let mut conn = pool.acquire().await?;
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
    .fetch_one(&mut *conn)
    .await?
    .try_get("id")?;
    let row = sqlx::query("SELECT * FROM scheduled_tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    row.as_ref()
        .map(ScheduledTask::from_row)
        .transpose()?
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
    // Atomically claim due tasks: flip `last_status` to 'running' inside
    // the same statement that selects them, then return the updated
    // rows. Without atomicity, two scheduler ticks firing within the
    // SELECT→`mark_task_running` UPDATE window could both observe the
    // same task as not-running and dispatch it twice — duplicate scan
    // and marker jobs racing on the same files were the failure mode.
    //
    // The UPDATE...WHERE id IN (SELECT ...) pattern serializes against
    // any other UPDATE on the same rows because SQLite holds a write
    // lock for the duration of the statement; concurrent ticks queue
    // up and the second one sees `last_status='running'` for the rows
    // the first already grabbed.
    let rows = sqlx::query(
        "UPDATE scheduled_tasks
         SET last_status = 'running', last_run_at = ?
         WHERE id IN (
             SELECT id FROM scheduled_tasks
             WHERE enabled = 1
               AND next_run_at <= ?
               AND (last_status IS NULL OR last_status <> 'running')
             ORDER BY next_run_at ASC
             LIMIT 16
         )
         RETURNING *",
    )
    .bind(now)
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

#[allow(clippy::too_many_arguments)]
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

pub async fn list_task_runs(pool: &SqlitePool, task_id: i64, limit: i64) -> Result<Vec<TaskRun>> {
    let limit = limit.clamp(1, 200);
    let rows =
        sqlx::query("SELECT * FROM task_runs WHERE task_id = ? ORDER BY started_at DESC LIMIT ?")
            .bind(task_id)
            .bind(limit)
            .fetch_all(pool)
            .await?;
    rows.iter().map(TaskRun::from_row).collect()
}

/// Count failed task_runs since the most recent success. Used by the
/// scheduler to compute exponential backoff: a healthy task returns 0;
/// after the first failure it returns 1, after the second consecutive
/// failure 2, and so on. The result is bounded by SQLite's count
/// itself (always finite) but the caller should cap the exponent it
/// derives from it to avoid `2^N` overflow.
pub async fn count_consecutive_task_failures(pool: &SqlitePool, task_id: i64) -> Result<i64> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM task_runs
         WHERE task_id = ?
           AND status = 'failed'
           AND started_at > COALESCE(
               (SELECT MAX(started_at) FROM task_runs
                WHERE task_id = ? AND status = 'success'),
               0
           )",
    )
    .bind(task_id)
    .bind(task_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
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
    vault: &chimpflix_common::Vault,
    user_id: i64,
    access_token: &str,
    refresh_token: &str,
    scope: Option<&str>,
    expires_at: i64,
) -> Result<()> {
    let now = now_ms();
    // Encrypt before insert. The plaintext columns get empty strings so
    // a refresh-token rotation never leaves the old plaintext value
    // behind. Empty strings rather than NULLs because the phase-15
    // schema declared these columns as NOT NULL; phase 79 relaxes
    // that, but binding `""` works under both schemas (and
    // `decrypt_or_plaintext` ignores the plaintext column whenever the
    // encrypted blob is present, so reads are unaffected).
    let access_blob = vault
        .encrypt_str(access_token)
        .context("encrypt trakt access_token")?;
    let refresh_blob = vault
        .encrypt_str(refresh_token)
        .context("encrypt trakt refresh_token")?;
    sqlx::query(
        "INSERT INTO user_trakt_tokens
            (user_id, access_token, refresh_token,
             access_token_enc, access_token_nonce,
             refresh_token_enc, refresh_token_nonce,
             scope, expires_at, linked_at)
         VALUES (?, '', '', ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id) DO UPDATE SET
            access_token = '',
            refresh_token = '',
            access_token_enc = excluded.access_token_enc,
            access_token_nonce = excluded.access_token_nonce,
            refresh_token_enc = excluded.refresh_token_enc,
            refresh_token_nonce = excluded.refresh_token_nonce,
            scope = excluded.scope,
            expires_at = excluded.expires_at",
    )
    .bind(user_id)
    .bind(&access_blob.value)
    .bind(&access_blob.nonce)
    .bind(&refresh_blob.value)
    .bind(&refresh_blob.nonce)
    .bind(scope)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_trakt_tokens(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
    user_id: i64,
) -> Result<Option<TraktTokensRow>> {
    let row = sqlx::query("SELECT * FROM user_trakt_tokens WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else { return Ok(None) };
    // Prefer the encrypted column. Fall back to the legacy plaintext
    // column for rows that haven't been migrated by the boot backfill
    // yet — same pattern Webhook.from_row uses.
    let access_token = decrypt_or_plaintext(
        vault,
        row.try_get::<Option<Vec<u8>>, _>("access_token_enc")
            .ok()
            .flatten(),
        row.try_get::<Option<Vec<u8>>, _>("access_token_nonce")
            .ok()
            .flatten(),
        row.try_get::<Option<String>, _>("access_token")
            .ok()
            .flatten(),
    )
    .context("decrypt trakt access_token")?
    .ok_or_else(|| {
        anyhow::anyhow!("trakt access_token row has neither plaintext nor ciphertext")
    })?;
    let refresh_token = decrypt_or_plaintext(
        vault,
        row.try_get::<Option<Vec<u8>>, _>("refresh_token_enc")
            .ok()
            .flatten(),
        row.try_get::<Option<Vec<u8>>, _>("refresh_token_nonce")
            .ok()
            .flatten(),
        row.try_get::<Option<String>, _>("refresh_token")
            .ok()
            .flatten(),
    )
    .context("decrypt trakt refresh_token")?
    .ok_or_else(|| {
        anyhow::anyhow!("trakt refresh_token row has neither plaintext nor ciphertext")
    })?;
    Ok(Some(TraktTokensRow {
        user_id: row.try_get("user_id")?,
        access_token,
        refresh_token,
        scope: row.try_get::<Option<String>, _>("scope").ok().flatten(),
        expires_at: row.try_get("expires_at")?,
        linked_at: row.try_get("linked_at")?,
        last_synced_at: row
            .try_get::<Option<i64>, _>("last_synced_at")
            .ok()
            .flatten(),
    }))
}

/// One-shot backfill: encrypt every plaintext Trakt token row using
/// the active vault, then NULL the legacy plaintext columns. Idempotent
/// — re-runs are no-ops once every row is converted. Returns the
/// number of rows migrated this call. Run from `main()` at boot.
pub async fn backfill_trakt_tokens(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
) -> Result<usize> {
    let rows = sqlx::query(
        "SELECT user_id, access_token, refresh_token FROM user_trakt_tokens
         WHERE (access_token IS NOT NULL OR refresh_token IS NOT NULL)
           AND (access_token_enc IS NULL OR refresh_token_enc IS NULL)",
    )
    .fetch_all(pool)
    .await
    .context("scan user_trakt_tokens for plaintext rows")?;

    let mut count = 0;
    for row in rows {
        let user_id: i64 = row.try_get("user_id")?;
        let access_token: Option<String> = row
            .try_get::<Option<String>, _>("access_token")
            .ok()
            .flatten();
        let refresh_token: Option<String> = row
            .try_get::<Option<String>, _>("refresh_token")
            .ok()
            .flatten();

        let access_blob = match access_token.as_deref() {
            Some(p) => Some(
                vault
                    .encrypt_str(p)
                    .context("encrypt access_token backfill")?,
            ),
            None => None,
        };
        let refresh_blob = match refresh_token.as_deref() {
            Some(p) => Some(
                vault
                    .encrypt_str(p)
                    .context("encrypt refresh_token backfill")?,
            ),
            None => None,
        };

        sqlx::query(
            "UPDATE user_trakt_tokens
             SET access_token = NULL,
                 refresh_token = NULL,
                 access_token_enc = COALESCE(?, access_token_enc),
                 access_token_nonce = COALESCE(?, access_token_nonce),
                 refresh_token_enc = COALESCE(?, refresh_token_enc),
                 refresh_token_nonce = COALESCE(?, refresh_token_nonce)
             WHERE user_id = ?",
        )
        .bind(access_blob.as_ref().map(|b| b.value.clone()))
        .bind(access_blob.as_ref().and_then(|b| b.nonce.clone()))
        .bind(refresh_blob.as_ref().map(|b| b.value.clone()))
        .bind(refresh_blob.as_ref().and_then(|b| b.nonce.clone()))
        .bind(user_id)
        .execute(pool)
        .await
        .with_context(|| format!("backfill trakt tokens user_id={user_id}"))?;
        count += 1;
    }
    Ok(count)
}

/// Shared helper: prefer ciphertext if present, otherwise hand back the
/// legacy plaintext. Returns `None` only when neither is set.
fn decrypt_or_plaintext(
    vault: &chimpflix_common::Vault,
    enc_value: Option<Vec<u8>>,
    enc_nonce: Option<Vec<u8>>,
    plaintext: Option<String>,
) -> Result<Option<String>> {
    if let Some(value) = enc_value {
        let blob = chimpflix_common::EncryptedBlob {
            value,
            nonce: enc_nonce,
        };
        return Ok(Some(vault.decrypt_str(&blob)?));
    }
    Ok(plaintext)
}

pub async fn delete_trakt_tokens(pool: &SqlitePool, user_id: i64) -> Result<bool> {
    let res = sqlx::query("DELETE FROM user_trakt_tokens WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn update_trakt_last_synced(pool: &SqlitePool, user_id: i64, when_ms: i64) -> Result<()> {
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
    Ok(rows
        .iter()
        .filter_map(|r| r.try_get("user_id").ok())
        .collect())
}

/// Read the cursor used by the bidirectional sync. Same column the
/// pull-side already drives off — the push step uses it as a "what
/// counts as new locally" lower bound so a Sync now after marking
/// items watched picks them up even when the fire-and-forget hook
/// failed (token expiry, transient HTTP error, etc).
pub async fn get_trakt_last_synced(pool: &SqlitePool, user_id: i64) -> Result<Option<i64>> {
    let row = sqlx::query("SELECT last_synced_at FROM user_trakt_tokens WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| r.try_get::<Option<i64>, _>("last_synced_at").ok().flatten()))
}

/// Read the user's last-observed `/sync/last_activities` `all`
/// timestamp. The cursor is the raw ISO-8601 string Trakt returns;
/// we never parse it. None means we've never checked.
pub async fn get_trakt_last_activities_seen(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Option<String>> {
    let row = sqlx::query("SELECT last_activities_seen_at FROM user_trakt_tokens WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.and_then(|r| {
        r.try_get::<Option<String>, _>("last_activities_seen_at")
            .ok()
            .flatten()
    }))
}

/// Persist the `all` timestamp from the user's most recent successful
/// `/sync/last_activities` fetch.
pub async fn update_trakt_last_activities_seen(
    pool: &SqlitePool,
    user_id: i64,
    seen: &str,
) -> Result<()> {
    sqlx::query("UPDATE user_trakt_tokens SET last_activities_seen_at = ? WHERE user_id = ?")
        .bind(seen)
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Locally-watched movies for a user. When `since_ms` is Some, only
/// rows whose last_played_at falls in `(since_ms, now]` are returned —
/// the typical "what's new since last Sync now" query. Returns
/// (item_id, tmdb_id, watched_at_ms); rows whose item has no tmdb_id
/// are skipped (Trakt is TMDB-keyed).
#[derive(Debug, Clone)]
pub struct WatchedMovieForPush {
    pub item_id: i64,
    pub tmdb_id: Option<i64>,
    pub imdb_id: Option<String>,
    pub tvdb_id: Option<i64>,
    pub watched_at: i64,
}

pub async fn list_watched_movies_for_push(
    pool: &SqlitePool,
    user_id: i64,
    since_ms: Option<i64>,
) -> Result<Vec<WatchedMovieForPush>> {
    // Need *any* Trakt-compatible id to be useful. Anime libraries
    // matched only via AniList have none of (tmdb / imdb / tvdb) on
    // the items row — those will never resolve on Trakt regardless
    // of how often we push, so we filter them out here rather than
    // wasting a round-trip per cycle.
    let rows = if let Some(since) = since_ms {
        sqlx::query(
            "SELECT ps.item_id AS item_id, i.tmdb_id, i.imdb_id, i.tvdb_id, \
                    ps.last_played_at AS watched_at \
             FROM play_state ps \
             JOIN items i ON i.id = ps.item_id \
             WHERE ps.user_id = ? AND ps.watched = 1 \
               AND ps.item_id IS NOT NULL \
               AND (i.tmdb_id IS NOT NULL OR i.imdb_id IS NOT NULL OR i.tvdb_id IS NOT NULL) \
               AND ps.last_played_at > ? \
             ORDER BY ps.last_played_at ASC",
        )
        .bind(user_id)
        .bind(since)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT ps.item_id AS item_id, i.tmdb_id, i.imdb_id, i.tvdb_id, \
                    ps.last_played_at AS watched_at \
             FROM play_state ps \
             JOIN items i ON i.id = ps.item_id \
             WHERE ps.user_id = ? AND ps.watched = 1 \
               AND ps.item_id IS NOT NULL \
               AND (i.tmdb_id IS NOT NULL OR i.imdb_id IS NOT NULL OR i.tvdb_id IS NOT NULL) \
             ORDER BY ps.last_played_at ASC",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?
    };
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(WatchedMovieForPush {
            item_id: r.try_get("item_id")?,
            tmdb_id: r.try_get::<Option<i64>, _>("tmdb_id").ok().flatten(),
            imdb_id: r.try_get::<Option<String>, _>("imdb_id").ok().flatten(),
            tvdb_id: r.try_get::<Option<i64>, _>("tvdb_id").ok().flatten(),
            watched_at: r.try_get("watched_at")?,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub struct WatchedEpisodeForPush {
    pub episode_id: i64,
    pub show_tmdb_id: Option<i64>,
    pub show_imdb_id: Option<String>,
    pub show_tvdb_id: Option<i64>,
    pub season: i32,
    pub episode: i32,
    pub watched_at: i64,
}

/// Locally-watched episodes for a user, since `since_ms`. Same
/// id-fallback story as movies — anime shows matched only via AniList
/// have none of (tmdb / imdb / tvdb) on the show row and are filtered
/// out here.
pub async fn list_watched_episodes_for_push(
    pool: &SqlitePool,
    user_id: i64,
    since_ms: Option<i64>,
) -> Result<Vec<WatchedEpisodeForPush>> {
    let rows = if let Some(since) = since_ms {
        sqlx::query(
            "SELECT ps.episode_id AS episode_id, \
                    i.tmdb_id AS show_tmdb, i.imdb_id AS show_imdb, i.tvdb_id AS show_tvdb, \
                    s.season_number AS season, e.episode_number AS episode, \
                    ps.last_played_at AS watched_at \
             FROM play_state ps \
             JOIN episodes e ON e.id = ps.episode_id \
             JOIN seasons s  ON s.id = e.season_id \
             JOIN items i    ON i.id = s.show_id \
             WHERE ps.user_id = ? AND ps.watched = 1 \
               AND ps.episode_id IS NOT NULL \
               AND (i.tmdb_id IS NOT NULL OR i.imdb_id IS NOT NULL OR i.tvdb_id IS NOT NULL) \
               AND ps.last_played_at > ? \
             ORDER BY ps.last_played_at ASC",
        )
        .bind(user_id)
        .bind(since)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query(
            "SELECT ps.episode_id AS episode_id, \
                    i.tmdb_id AS show_tmdb, i.imdb_id AS show_imdb, i.tvdb_id AS show_tvdb, \
                    s.season_number AS season, e.episode_number AS episode, \
                    ps.last_played_at AS watched_at \
             FROM play_state ps \
             JOIN episodes e ON e.id = ps.episode_id \
             JOIN seasons s  ON s.id = e.season_id \
             JOIN items i    ON i.id = s.show_id \
             WHERE ps.user_id = ? AND ps.watched = 1 \
               AND ps.episode_id IS NOT NULL \
               AND (i.tmdb_id IS NOT NULL OR i.imdb_id IS NOT NULL OR i.tvdb_id IS NOT NULL) \
             ORDER BY ps.last_played_at ASC",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?
    };
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        out.push(WatchedEpisodeForPush {
            episode_id: r.try_get("episode_id")?,
            show_tmdb_id: r.try_get::<Option<i64>, _>("show_tmdb").ok().flatten(),
            show_imdb_id: r.try_get::<Option<String>, _>("show_imdb").ok().flatten(),
            show_tvdb_id: r.try_get::<Option<i64>, _>("show_tvdb").ok().flatten(),
            season: r.try_get("season")?,
            episode: r.try_get("episode")?,
            watched_at: r.try_get("watched_at")?,
        });
    }
    Ok(out)
}

/// Snapshot of the user's Trakt watchlist as we last observed it.
/// Lets the watchlist reconcile propagate Trakt-side removes back to
/// My List without us needing a separate "did the user remove this?"
/// signal — we just diff this snapshot against the next pull.
/// Returns `(movie_tmdb_ids, show_tmdb_ids)` as two HashSets.
pub async fn list_trakt_watchlist_state(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<(
    std::collections::HashSet<i64>,
    std::collections::HashSet<i64>,
)> {
    use std::collections::HashSet;
    let rows = sqlx::query(
        "SELECT kind, tmdb_id FROM user_trakt_watchlist_state WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let mut movies = HashSet::new();
    let mut shows = HashSet::new();
    for r in &rows {
        let kind: String = r.try_get("kind")?;
        let tmdb_id: i64 = r.try_get("tmdb_id")?;
        match kind.as_str() {
            "movie" => {
                movies.insert(tmdb_id);
            }
            "show" => {
                shows.insert(tmdb_id);
            }
            _ => {}
        }
    }
    Ok((movies, shows))
}

/// Atomically replace the per-user watchlist-state snapshot with the
/// post-reconcile set. Same delete-then-bulk-insert pattern as
/// `replace_trakt_collection_state`; partial state would make the
/// next diff miss removes.
pub async fn replace_trakt_watchlist_state(
    pool: &SqlitePool,
    user_id: i64,
    movies: &[i64],
    shows: &[i64],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_trakt_watchlist_state WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let now = now_ms();
    for tmdb_id in movies {
        sqlx::query(
            "INSERT INTO user_trakt_watchlist_state (user_id, kind, tmdb_id, seen_at)
             VALUES (?, 'movie', ?, ?)",
        )
        .bind(user_id)
        .bind(tmdb_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    for tmdb_id in shows {
        sqlx::query(
            "INSERT INTO user_trakt_watchlist_state (user_id, kind, tmdb_id, seen_at)
             VALUES (?, 'show', ?, ?)",
        )
        .bind(user_id)
        .bind(tmdb_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Snapshot of what we previously pushed to Trakt's collection for
/// this user, so the nightly reconcile can compute add/remove deltas
/// without nuking items the user collected via another media server
/// or by hand on the Trakt site. Returns `(movies, episodes)` where
/// movies is the set of show_tmdb_ids and episodes is the set of
/// (show_tmdb_id, season, episode) tuples.
pub async fn list_trakt_collection_state(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<(
    std::collections::HashSet<i64>,
    std::collections::HashSet<(i64, i32, i32)>,
)> {
    use std::collections::HashSet;
    let rows = sqlx::query(
        "SELECT kind, tmdb_id, season, episode_num
         FROM user_trakt_collection_state
         WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let mut movies = HashSet::new();
    let mut episodes = HashSet::new();
    for r in &rows {
        let kind: String = r.try_get("kind")?;
        let tmdb_id: i64 = r.try_get("tmdb_id")?;
        match kind.as_str() {
            "movie" => {
                movies.insert(tmdb_id);
            }
            "episode" => {
                let season: i32 = r.try_get("season")?;
                let episode_num: i32 = r.try_get("episode_num")?;
                episodes.insert((tmdb_id, season, episode_num));
            }
            _ => {}
        }
    }
    Ok((movies, episodes))
}

/// Atomically replace the per-user collection state snapshot with the
/// post-push set. Called after a successful `/sync/collection` +
/// `/sync/collection/remove` round-trip so the next nightly diff
/// computes against the new known-pushed set. Delete + bulk-insert in
/// a single transaction — partial state would make the next diff
/// either re-push the world or miss removes.
pub async fn replace_trakt_collection_state(
    pool: &SqlitePool,
    user_id: i64,
    movies: &[i64],
    episodes: &[(i64, i32, i32)],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM user_trakt_collection_state WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    let now = now_ms();
    for tmdb_id in movies {
        sqlx::query(
            "INSERT INTO user_trakt_collection_state
                (user_id, kind, tmdb_id, season, episode_num, pushed_at)
             VALUES (?, 'movie', ?, 0, 0, ?)",
        )
        .bind(user_id)
        .bind(tmdb_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    for (show_tmdb, season, episode_num) in episodes {
        sqlx::query(
            "INSERT INTO user_trakt_collection_state
                (user_id, kind, tmdb_id, season, episode_num, pushed_at)
             VALUES (?, 'episode', ?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(show_tmdb)
        .bind(season)
        .bind(episode_num)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Every movie that has at least one active (non-deleted) media file
/// and a tmdb_id, scoped to the user's library access. Used to bulk-
/// push the user's "collection" to Trakt — Trakt's `/sync/collection`
/// dedupes by ids, so re-pushing on every nightly run is harmless.
pub async fn list_collected_movies_for_user(
    pool: &SqlitePool,
    accessible: Option<&[i64]>,
) -> Result<Vec<i64>> {
    let lib_clause = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "SELECT i.tmdb_id AS tmdb_id \
         FROM items i \
         WHERE i.kind = 'movie' \
           AND i.tmdb_id IS NOT NULL \
           AND EXISTS (
               SELECT 1 FROM media_files mf
               WHERE mf.item_id = i.id AND mf.removed_at IS NULL
           ) \
           AND ({lib_clause})"
    );
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    rows.iter()
        .map(|r| Ok(r.try_get::<i64, _>("tmdb_id")?))
        .collect()
}

/// Every episode that has at least one active media file, with the
/// parent show's tmdb_id. Like [`list_collected_movies_for_user`] but
/// for shows — Trakt collection wants per-episode entries so
/// "complete season" badges show correctly.
pub async fn list_collected_episodes_for_user(
    pool: &SqlitePool,
    accessible: Option<&[i64]>,
) -> Result<Vec<(i64, i32, i32)>> {
    let lib_clause = library_filter_sql("i.library_id", accessible);
    let sql = format!(
        "SELECT i.tmdb_id AS show_tmdb, s.season_number AS season, e.episode_number AS episode \
         FROM episodes e \
         JOIN seasons s ON s.id = e.season_id \
         JOIN items i ON i.id = s.show_id \
         WHERE i.tmdb_id IS NOT NULL \
           AND EXISTS (
               SELECT 1 FROM media_files mf
               WHERE mf.episode_id = e.id AND mf.removed_at IS NULL
           ) \
           AND ({lib_clause}) \
         ORDER BY i.tmdb_id, s.season_number, e.episode_number"
    );
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
        let show_tmdb: i64 = r.try_get("show_tmdb")?;
        let season: i32 = r.try_get("season")?;
        let episode: i32 = r.try_get("episode")?;
        out.push((show_tmdb, season, episode));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Federated auth providers (Plex OAuth)
// ---------------------------------------------------------------------------

/// One row of `user_auth_providers`. Snapshot of an external identity
/// (Plex today; Google later) bound to a ChimpFlix user.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UserAuthProvider {
    pub id: i64,
    pub user_id: i64,
    pub provider: String,
    pub external_id: String,
    pub external_email: Option<String>,
    pub external_username: Option<String>,
    pub linked_at: i64,
    pub last_login_at: Option<i64>,
}

fn auth_provider_from_row(row: &SqliteRow) -> Result<UserAuthProvider> {
    Ok(UserAuthProvider {
        id: row.try_get("id")?,
        user_id: row.try_get("user_id")?,
        provider: row.try_get("provider")?,
        external_id: row.try_get("external_id")?,
        external_email: row
            .try_get::<Option<String>, _>("external_email")
            .ok()
            .flatten(),
        external_username: row
            .try_get::<Option<String>, _>("external_username")
            .ok()
            .flatten(),
        linked_at: row.try_get("linked_at")?,
        last_login_at: row
            .try_get::<Option<i64>, _>("last_login_at")
            .ok()
            .flatten(),
    })
}

/// Resolve a (provider, external_id) pair to the linked local user.
/// Returns None when no link exists — the caller decides whether that
/// means "auto-provision" (invite-bearing signup) or "reject"
/// (invite-less login).
pub async fn find_user_by_provider(
    pool: &SqlitePool,
    provider: &str,
    external_id: &str,
) -> Result<Option<(User, UserAuthProvider)>> {
    let row = sqlx::query(
        "SELECT users.*, p.id AS p_id, p.user_id AS p_user_id, p.provider AS p_provider,
                p.external_id AS p_external_id, p.external_email AS p_external_email,
                p.external_username AS p_external_username, p.linked_at AS p_linked_at,
                p.last_login_at AS p_last_login_at
         FROM user_auth_providers p
         JOIN users ON users.id = p.user_id
         WHERE p.provider = ? AND p.external_id = ?
         LIMIT 1",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let user = User::from_row(&row)?;
    let link = UserAuthProvider {
        id: row.try_get("p_id")?,
        user_id: row.try_get("p_user_id")?,
        provider: row.try_get("p_provider")?,
        external_id: row.try_get("p_external_id")?,
        external_email: row
            .try_get::<Option<String>, _>("p_external_email")
            .ok()
            .flatten(),
        external_username: row
            .try_get::<Option<String>, _>("p_external_username")
            .ok()
            .flatten(),
        linked_at: row.try_get("p_linked_at")?,
        last_login_at: row
            .try_get::<Option<i64>, _>("p_last_login_at")
            .ok()
            .flatten(),
    };
    Ok(Some((user, link)))
}

/// List every external-identity link a user has. Used by the Settings
/// → Account page to render "Linked accounts" + by `delete_auth_provider`
/// to enforce "you can't unlink your last way to sign in".
pub async fn list_user_auth_providers(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Vec<UserAuthProvider>> {
    let rows = sqlx::query(
        "SELECT id, user_id, provider, external_id,
                external_email, external_username, linked_at, last_login_at
         FROM user_auth_providers WHERE user_id = ? ORDER BY linked_at ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(auth_provider_from_row).collect()
}

/// Insert a provider link. Returns an error wrapping the UNIQUE
/// constraint violation when either (provider, external_id) is already
/// bound to a different user OR (user_id, provider) already has a
/// link — callers translate that into a 409 with a sensible message.
pub async fn insert_auth_provider(
    pool: &SqlitePool,
    user_id: i64,
    provider: &str,
    external_id: &str,
    external_email: Option<&str>,
    external_username: Option<&str>,
) -> Result<UserAuthProvider> {
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO user_auth_providers
            (user_id, provider, external_id, external_email, external_username,
             linked_at, last_login_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING *",
    )
    .bind(user_id)
    .bind(provider)
    .bind(external_id)
    .bind(external_email)
    .bind(external_username)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    auth_provider_from_row(&row)
}

/// Bump `last_login_at` after a successful provider-driven session
/// issue. Best-effort — surfacing a failure here would needlessly fail
/// the login.
pub async fn touch_auth_provider_login(pool: &SqlitePool, link_id: i64) -> Result<()> {
    sqlx::query("UPDATE user_auth_providers SET last_login_at = ? WHERE id = ?")
        .bind(now_ms())
        .bind(link_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Remove one provider link. Returns the number of rows deleted (0 if
/// the link didn't exist). The "can the user still sign in?" guard
/// lives in the API handler — this is a pure mutation.
pub async fn delete_auth_provider(
    pool: &SqlitePool,
    user_id: i64,
    provider: &str,
) -> Result<u64> {
    let res = sqlx::query("DELETE FROM user_auth_providers WHERE user_id = ? AND provider = ?")
        .bind(user_id)
        .bind(provider)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

/// Whether the given user has a non-NULL password_hash. Combined with
/// `list_user_auth_providers` it answers "after this unlink, can the
/// user still sign in by any means?"
pub async fn user_has_password(pool: &SqlitePool, user_id: i64) -> Result<bool> {
    let row =
        sqlx::query("SELECT password_hash IS NOT NULL AS has_pw FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    Ok(row
        .and_then(|r| r.try_get::<i64, _>("has_pw").ok())
        .map(|v| v != 0)
        .unwrap_or(false))
}

/// Create a user without a local password. Used by the invite-bearing
/// Plex signup flow: the new account starts with `password_hash = NULL`
/// and a linked provider row. The user can later set a password via
/// the forgot-password email flow if they ever want a password fallback.
///
/// Refuses to mint an `Owner` without a password — the existing Owner
/// safety guarantee (plex.tv being down can't lock out the only admin)
/// is enforced by callers passing `UserRole::User` here. Owner creation
/// stays in [`create_user`] where a password is mandatory.
pub async fn create_user_no_password(
    pool: &SqlitePool,
    username: &str,
    role: UserRole,
    display_name: Option<&str>,
    email: Option<&str>,
) -> Result<User> {
    if matches!(role, UserRole::Owner) {
        anyhow::bail!("create_user_no_password refuses to mint an Owner without a password");
    }
    let now = now_ms();
    let row = sqlx::query(
        "INSERT INTO users (username, password_hash, role, display_name, email, created_at, updated_at)
         VALUES (?, NULL, ?, ?, ?, ?, ?)
         RETURNING *",
    )
    .bind(username)
    .bind(role.as_str())
    .bind(display_name)
    .bind(email)
    .bind(now)
    .bind(now)
    .fetch_one(pool)
    .await?;
    User::from_row(&row)
}

/// Generate a username that won't collide with an existing row. The
/// caller passes the *preferred* base (e.g. the Plex username);
/// suffixes get appended only when needed. Returns the chosen string
/// so the caller can pass it to `create_user_no_password`.
///
/// Plex usernames can contain characters our `validate_username` rule
/// rejects (apostrophes, spaces). We sanitize first by replacing
/// disallowed chars with `_`, then dedupe — so `Zach O'Connor` becomes
/// `Zach_O_Connor` (and `Zach_O_Connor-2` if taken).
pub async fn allocate_username_from_external(
    pool: &SqlitePool,
    preferred: &str,
) -> Result<String> {
    let base: String = preferred
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let base = if base.is_empty() || base.starts_with('_') {
        format!("user{base}")
    } else {
        base
    };
    for suffix in 0..1000 {
        let candidate = if suffix == 0 {
            base.clone()
        } else {
            format!("{base}-{}", suffix + 1)
        };
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM users WHERE username = ? COLLATE NOCASE LIMIT 1",
        )
        .bind(&candidate)
        .fetch_optional(pool)
        .await?;
        if existing.is_none() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("could not allocate a unique username from {preferred:?}")
}

// ---------------------------------------------------------------------------
// Plex client identifier (lazy-persisted in server_settings)
// ---------------------------------------------------------------------------

/// Clear the stored Plex client identifier so the next call to
/// `ensure_plex_client_identifier` mints a fresh UUID. Operator
/// surface for the admin "rotate Plex identity" action; the caller
/// is also responsible for clearing the cached `PlexOAuthHandle` so
/// the new identifier is picked up immediately rather than after a
/// process restart.
pub async fn clear_plex_client_identifier(pool: &SqlitePool) -> Result<()> {
    sqlx::query("UPDATE server_settings SET plex_client_identifier = NULL")
        .execute(pool)
        .await?;
    Ok(())
}

/// Return the stored Plex client identifier or generate-and-persist a
/// fresh UUID on first read. Stable across restarts so re-launching
/// the server doesn't break in-flight authorizations.
pub async fn ensure_plex_client_identifier(pool: &SqlitePool) -> Result<String> {
    if let Some(existing) =
        sqlx::query_scalar::<_, Option<String>>("SELECT plex_client_identifier FROM server_settings")
            .fetch_optional(pool)
            .await?
            .flatten()
    {
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    // Generate a UUID-shaped identifier without pulling in the `uuid`
    // crate dependency just for one value. 16 random bytes formatted
    // as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` is what Plex expects;
    // we set the version + variant bits so the value is technically a
    // v4 UUID, but Plex doesn't actually care.
    let mut bytes = [0u8; 16];
    {
        use rand_core::{OsRng, RngCore};
        OsRng.fill_bytes(&mut bytes);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let id = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    sqlx::query("UPDATE server_settings SET plex_client_identifier = ?")
        .bind(&id)
        .execute(pool)
        .await?;
    Ok(id)
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
    let row = sqlx::query("SELECT rating FROM user_ratings WHERE user_id = ? AND item_id = ?")
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
    let row = sqlx::query("SELECT rating FROM user_ratings WHERE user_id = ? AND episode_id = ?")
        .bind(user_id)
        .bind(episode_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.try_get::<i32, _>("rating").unwrap_or(0)))
}

/// All of a user's stored ratings in two flat lists. Callers (the
/// browse rails' Like buttons) need to know whether each visible item
/// is rated without firing one request per card — the per-id endpoint
/// fan-out was tripping the global rate limiter on the home page.
pub async fn list_user_ratings(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<(Vec<(i64, i32)>, Vec<(i64, i32)>)> {
    let item_rows = sqlx::query(
        "SELECT item_id, rating FROM user_ratings \
         WHERE user_id = ? AND item_id IS NOT NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let episode_rows = sqlx::query(
        "SELECT episode_id, rating FROM user_ratings \
         WHERE user_id = ? AND episode_id IS NOT NULL",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    let items = item_rows
        .into_iter()
        .map(|r| {
            let id: i64 = r.try_get("item_id").unwrap_or(0);
            let rating: i32 = r.try_get("rating").unwrap_or(0);
            (id, rating)
        })
        .filter(|(id, _)| *id > 0)
        .collect();
    let episodes = episode_rows
        .into_iter()
        .map(|r| {
            let id: i64 = r.try_get("episode_id").unwrap_or(0);
            let rating: i32 = r.try_get("rating").unwrap_or(0);
            (id, rating)
        })
        .filter(|(id, _)| *id > 0)
        .collect();
    Ok((items, episodes))
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

pub async fn add_tag_to_item(pool: &SqlitePool, item_id: i64, name: &str) -> Result<Tag> {
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
    let tag_id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM tags WHERE name = ? COLLATE NOCASE")
            .bind(trimmed)
            .fetch_optional(pool)
            .await?;
    let Some(tag_id) = tag_id else {
        return Ok(false);
    };
    remove_tag_from_item(pool, item_id, tag_id).await
}

pub async fn remove_tag_from_item(pool: &SqlitePool, item_id: i64, tag_id: i64) -> Result<bool> {
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
            sqlx::query("UPDATE media_files SET loudnorm_analyzed_at = ? WHERE id = ?")
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
    let lra: Option<f64> = row.try_get::<Option<f64>, _>("loudnorm_lra").ok().flatten();
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
        source_file_id: row
            .try_get::<Option<String>, _>("source_file_id")
            .ok()
            .flatten(),
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

pub async fn get_external_subtitle(pool: &SqlitePool, id: i64) -> Result<Option<ExternalSubtitle>> {
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

/// Result of a vault self-test: pick one encrypted row and attempt to
/// decrypt it with the supplied vault. Three outcomes:
///
/// - `NoEncryptedRows` — neither `secrets` nor `webhooks` nor
///   `user_totp` has any encrypted (nonce IS NOT NULL) row. A vault
///   key change here is safe: there's nothing to lose.
/// - `Ok { sampled }` — at least one encrypted row exists, and the
///   sample decrypted successfully. The current key matches the data.
/// - `Mismatch { sampled, error }` — encrypted rows exist but the
///   sample failed to decrypt. Either the key was rotated without
///   running `vault-rotate`, or the DB was restored from a backup
///   that was encrypted under a different key.
#[derive(Debug, Clone)]
pub enum VaultSelfTest {
    NoEncryptedRows,
    Ok {
        sampled: String,
    },
    Mismatch {
        sampled: String,
        error: String,
    },
}

/// Decrypt-check helper used by:
///
/// - The server boot path (refuses to start when a restore + key
///   mismatch are both detected).
/// - The `/admin/backups` list endpoint (surfaces a UI banner when
///   encrypted data exists, so operators know the backup is only
///   useful with the matching vault key).
/// - The `verify_backups` scheduled task (flags individual backup
///   files where decryption with the current key fails).
///
/// We scan `secrets` first because it's the most likely to have data;
/// fall back to `webhooks`, then `user_totp`, before declaring no
/// encrypted rows exist.
pub async fn vault_self_test(
    pool: &SqlitePool,
    vault: &chimpflix_common::Vault,
) -> Result<VaultSelfTest> {
    if let Some((label, blob)) = sample_encrypted_blob(pool).await? {
        match vault.decrypt(&blob) {
            Ok(_) => Ok(VaultSelfTest::Ok { sampled: label }),
            Err(e) => Ok(VaultSelfTest::Mismatch {
                sampled: label,
                error: format!("{e:#}"),
            }),
        }
    } else {
        Ok(VaultSelfTest::NoEncryptedRows)
    }
}

/// Return the first encrypted (nonce IS NOT NULL) row from any
/// vault-protected table, paired with a human-readable label for the
/// source. Returns None when nothing is encrypted at rest.
async fn sample_encrypted_blob(
    pool: &SqlitePool,
) -> Result<Option<(String, chimpflix_common::EncryptedBlob)>> {
    if let Some(row) = sqlx::query(
        "SELECT name, value_enc, nonce FROM secrets
         WHERE nonce IS NOT NULL LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("sample secrets for vault self-test")?
    {
        let name: String = row.try_get("name")?;
        let value_enc: Vec<u8> = row.try_get("value_enc")?;
        let nonce: Option<Vec<u8>> = row.try_get("nonce").ok().flatten();
        return Ok(Some((
            format!("secrets.{name}"),
            chimpflix_common::EncryptedBlob {
                value: value_enc,
                nonce,
            },
        )));
    }
    if let Some(row) = sqlx::query(
        "SELECT id, secret_enc, secret_nonce FROM webhooks
         WHERE secret_enc IS NOT NULL AND secret_nonce IS NOT NULL LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("sample webhooks for vault self-test")?
    {
        let id: i64 = row.try_get("id")?;
        let value_enc: Vec<u8> = row.try_get("secret_enc")?;
        let nonce: Option<Vec<u8>> = row.try_get("secret_nonce").ok().flatten();
        return Ok(Some((
            format!("webhooks.id={id}"),
            chimpflix_common::EncryptedBlob {
                value: value_enc,
                nonce,
            },
        )));
    }
    if let Some(row) = sqlx::query(
        "SELECT user_id, secret_enc, secret_nonce FROM user_totp
         WHERE secret_nonce IS NOT NULL LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .context("sample user_totp for vault self-test")?
    {
        let user_id: i64 = row.try_get("user_id")?;
        let value_enc: Vec<u8> = row.try_get("secret_enc")?;
        let nonce: Option<Vec<u8>> = row.try_get("secret_nonce").ok().flatten();
        return Ok(Some((
            format!("user_totp.user_id={user_id}"),
            chimpflix_common::EncryptedBlob {
                value: value_enc,
                nonce,
            },
        )));
    }
    Ok(None)
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
        sqlx::query("UPDATE secrets SET value_enc = ?, nonce = ?, updated_at = ? WHERE name = ?")
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
        sqlx::query("UPDATE webhooks SET secret_enc = ?, secret_nonce = ? WHERE id = ?")
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

/// Look up the IP + user-agent of an earlier event for the same
/// transcoder session token. Used by the stop-event emitter — the
/// transcoder doesn't carry the client's IP through to its
/// `SessionSnapshot`, but the matching `start` event recorded the
/// values at request time, so reusing them keeps the activity feed
/// consistent (previously every stop row rendered as `unknown`).
///
/// Returns the most recent non-null pair seen for the token; the
/// caller treats `(None, None)` as "no info" and the partial cases
/// preserve whatever the start row had.
pub async fn lookup_session_origin(
    pool: &SqlitePool,
    session_token: &str,
) -> Result<(Option<String>, Option<String>)> {
    let row = sqlx::query(
        "SELECT ip, user_agent FROM playback_events \
         WHERE session_token = ? AND (ip IS NOT NULL OR user_agent IS NOT NULL) \
         ORDER BY occurred_at DESC LIMIT 1",
    )
    .bind(session_token)
    .fetch_optional(pool)
    .await?;
    match row {
        Some(r) => Ok((
            r.try_get::<Option<String>, _>("ip").unwrap_or(None),
            r.try_get::<Option<String>, _>("user_agent").unwrap_or(None),
        )),
        None => Ok((None, None)),
    }
}

/// Append a playback event. Fire-and-forget — failures are logged at
/// the call site, never bubble up to the user's request. Stats are
/// observability, not load-bearing for playback.
pub async fn record_playback_event(pool: &SqlitePool, ev: PlaybackEventInput<'_>) -> Result<()> {
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
                display_name: r
                    .try_get::<Option<String>, _>("display_name")
                    .ok()
                    .flatten(),
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
        total_plays: row
            .try_get::<Option<i64>, _>("total_plays")
            .ok()
            .flatten()
            .unwrap_or(0),
        completions: row
            .try_get::<Option<i64>, _>("completions")
            .ok()
            .flatten()
            .unwrap_or(0),
        direct_plays: row
            .try_get::<Option<i64>, _>("direct_plays")
            .ok()
            .flatten()
            .unwrap_or(0),
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
pub async fn plays_per_day(pool: &SqlitePool, days: i64) -> Result<Vec<StatsDailyBucket>> {
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

// ─── Task metrics rollup ──────────────────────────────────────────────

/// One finished job's summary, used to build the daily rollup. The
/// rollup task fetches a slice of these for the prior UTC day and
/// aggregates in-process.
#[derive(Debug, Clone)]
pub struct FinishedJobSummary {
    pub kind: String,
    /// "succeeded" or "dead". Anything else is filtered out at SQL.
    pub status: String,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

/// Return finished jobs (succeeded + dead) with finished_at in
/// `[start_ms, end_ms)`. Window is inclusive of start, exclusive of
/// end — matches the "previous-day rollup" pattern used by the
/// scheduler task.
pub async fn list_finished_jobs_in_window(
    pool: &SqlitePool,
    start_ms: i64,
    end_ms: i64,
) -> Result<Vec<FinishedJobSummary>> {
    let rows = sqlx::query(
        "SELECT kind, status, started_at, finished_at
         FROM jobs
         WHERE finished_at IS NOT NULL
           AND finished_at >= ?
           AND finished_at < ?
           AND status IN ('succeeded', 'dead')",
    )
    .bind(start_ms)
    .bind(end_ms)
    .fetch_all(pool)
    .await?;
    let out: Vec<FinishedJobSummary> = rows
        .iter()
        .map(|r| FinishedJobSummary {
            kind: r.try_get::<String, _>("kind").unwrap_or_default(),
            status: r.try_get::<String, _>("status").unwrap_or_default(),
            started_at: r.try_get::<Option<i64>, _>("started_at").ok().flatten(),
            finished_at: r.try_get::<Option<i64>, _>("finished_at").ok().flatten(),
        })
        .collect();
    Ok(out)
}

/// Upsert a per-kind daily aggregate. `(day, kind)` is the natural
/// key. Re-running the rollup for the same day overwrites — which
/// is the intent (the previous-day computation is deterministic
/// modulo what's still in the `jobs` table).
#[allow(clippy::too_many_arguments)]
pub async fn upsert_task_metrics_daily(
    pool: &SqlitePool,
    day: i64,
    kind: &str,
    success_count: i64,
    failure_count: i64,
    p50_duration_ms: Option<i64>,
    p95_duration_ms: Option<i64>,
    targets_processed: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO task_kind_metrics_daily
            (day, kind, success_count, failure_count, p50_duration_ms, p95_duration_ms, targets_processed)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(day, kind) DO UPDATE SET
            success_count = excluded.success_count,
            failure_count = excluded.failure_count,
            p50_duration_ms = excluded.p50_duration_ms,
            p95_duration_ms = excluded.p95_duration_ms,
            targets_processed = excluded.targets_processed",
    )
    .bind(day)
    .bind(kind)
    .bind(success_count)
    .bind(failure_count)
    .bind(p50_duration_ms)
    .bind(p95_duration_ms)
    .bind(targets_processed)
    .execute(pool)
    .await?;
    Ok(())
}

// ─── Tacet season fingerprint references ───────────────────────────────

/// Resolve which show + season a media_file belongs to. Returns
/// `None` when the file is a movie (no associated episode row),
/// when it's been removed, or when the metadata isn't populated
/// yet (a brand-new episode whose item match hasn't landed).
pub async fn resolve_show_and_season_for_file(
    pool: &SqlitePool,
    file_id: i64,
) -> Result<Option<(i64, i32)>> {
    let row = sqlx::query(
        "SELECT s.show_id AS show_id, s.season_number AS season_number
         FROM media_files mf
         JOIN episodes e ON e.id = mf.episode_id
         JOIN seasons s ON s.id = e.season_id
         WHERE mf.id = ? AND mf.removed_at IS NULL",
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| {
        (
            r.try_get::<i64, _>("show_id").unwrap_or(0),
            r.try_get::<i32, _>("season_number").unwrap_or(0),
        )
    }))
}

/// Count media_files in a given season with non-null durations. The
/// duration filter mirrors what `bootstrap_season_refs` itself
/// requires — files without a probe-known duration can't be
/// fingerprinted reliably.
pub async fn count_episodes_in_season(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n
         FROM media_files mf
         JOIN episodes e ON e.id = mf.episode_id
         JOIN seasons s ON s.id = e.season_id
         WHERE s.show_id = ?
           AND s.season_number = ?
           AND mf.removed_at IS NULL
           AND mf.duration_ms IS NOT NULL",
    )
    .bind(show_id)
    .bind(season_number)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get::<i64, _>("n").unwrap_or(0))
}

/// Count episodes in a season that still need marker detection
/// (`markers_detected_at IS NULL`). Used to gate
/// `bootstrap_season_refs`: when every episode already has markers
/// (e.g. all from Phase A embedded chapter labels), running tacet
/// bootstrap would decode every episode for zero useful output.
pub async fn count_episodes_needing_markers_in_season(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<i64> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n
         FROM media_files mf
         JOIN episodes e ON e.id = mf.episode_id
         JOIN seasons s ON s.id = e.season_id
         WHERE s.show_id = ?
           AND s.season_number = ?
           AND mf.removed_at IS NULL
           AND mf.duration_ms IS NOT NULL
           AND mf.markers_detected_at IS NULL",
    )
    .bind(show_id)
    .bind(season_number)
    .fetch_one(pool)
    .await?;
    Ok(row.try_get::<i64, _>("n").unwrap_or(0))
}

/// One row returned by [`list_episodes_in_season_for_detection`]. Carries
/// the file_id so tacet's per-episode markers can be mapped back to the
/// originating row when written to the markers table.
pub struct EpisodeForDetection {
    pub file_id: i64,
    pub path: String,
    pub episode_number: i32,
}

/// List every media_file in a season alongside its episode number — used by
/// `bootstrap_season_refs` to feed tacet's [`tacet::detection::detect_season`]
/// entry point. Ordered by `episode_number ASC` so tacet's internal logging
/// reads cleanly and any future cross-episode heuristics see episodes in
/// broadcast order.
pub async fn list_episodes_in_season_for_detection(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<Vec<EpisodeForDetection>> {
    let rows = sqlx::query(
        "SELECT mf.id AS file_id, mf.path AS path, e.episode_number AS episode_number
         FROM media_files mf
         JOIN episodes e ON e.id = mf.episode_id
         JOIN seasons s ON s.id = e.season_id
         WHERE s.show_id = ?
           AND s.season_number = ?
           AND mf.removed_at IS NULL
           AND mf.duration_ms IS NOT NULL
         ORDER BY e.episode_number ASC",
    )
    .bind(show_id)
    .bind(season_number)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(EpisodeForDetection {
                file_id: r.try_get("file_id")?,
                path: r.try_get("path")?,
                episode_number: r.try_get("episode_number")?,
            })
        })
        .collect()
}

/// Loaded blobs for a season's intro + credits reference fingerprints,
/// or `None` if the season hasn't been bootstrapped yet.
pub struct SeasonRefsBlobs {
    pub intro: Vec<u8>,
    pub credits: Vec<u8>,
    pub built_at: i64,
}

pub async fn load_season_refs_blobs(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<Option<SeasonRefsBlobs>> {
    let row = sqlx::query(
        "SELECT intro_refs_blob, credits_refs_blob, refs_built_at
         FROM show_season_intro_refs
         WHERE show_id = ? AND season_number = ?",
    )
    .bind(show_id)
    .bind(season_number)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| SeasonRefsBlobs {
        intro: r
            .try_get::<Vec<u8>, _>("intro_refs_blob")
            .unwrap_or_default(),
        credits: r
            .try_get::<Vec<u8>, _>("credits_refs_blob")
            .unwrap_or_default(),
        built_at: r.try_get::<i64, _>("refs_built_at").unwrap_or(0),
    }))
}

pub async fn upsert_season_refs_blobs(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
    intro_blob: &[u8],
    credits_blob: &[u8],
) -> Result<()> {
    sqlx::query(
        "INSERT INTO show_season_intro_refs
            (show_id, season_number, intro_refs_blob, credits_refs_blob, refs_built_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(show_id, season_number) DO UPDATE SET
            intro_refs_blob = excluded.intro_refs_blob,
            credits_refs_blob = excluded.credits_refs_blob,
            refs_built_at = excluded.refs_built_at",
    )
    .bind(show_id)
    .bind(season_number)
    .bind(intro_blob)
    .bind(credits_blob)
    .bind(now_ms())
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn job_queue_full_lifecycle() {
        let pool = test_pool().await;
        // Enqueue two jobs at the same priority and verify FIFO claim
        // by id (priority DESC, id ASC).
        let a = enqueue_job(
            &pool,
            JobInput::new("test_kind", serde_json::json!({"n": 1})),
        )
        .await
        .unwrap();
        let b = enqueue_job(
            &pool,
            JobInput::new("test_kind", serde_json::json!({"n": 2})),
        )
        .await
        .unwrap();

        // First claim returns `a` (lower id at same priority).
        let claimed = claim_next_job(&pool).await.unwrap().unwrap();
        assert_eq!(claimed.id, a);
        assert_eq!(claimed.status, JobStatus::Running);
        assert!(claimed.locked_at.is_some());
        assert_eq!(claimed.attempts, 1);

        // Second claim returns `b`.
        let claimed = claim_next_job(&pool).await.unwrap().unwrap();
        assert_eq!(claimed.id, b);

        // No more queued rows.
        assert!(claim_next_job(&pool).await.unwrap().is_none());

        // Mark `a` succeeded, `b` failed with a retry.
        mark_job_succeeded(&pool, a).await.unwrap();
        mark_job_failed(&pool, b, "boom", 1000).await.unwrap();

        // `b` should be `failed` with run_after in the future.
        let row = sqlx::query("SELECT status, run_after FROM jobs WHERE id = ?")
            .bind(b)
            .fetch_one(&pool)
            .await
            .unwrap();
        let status: String = row.try_get("status").unwrap();
        assert_eq!(status, "failed");
        let run_after: i64 = row.try_get("run_after").unwrap();
        assert!(run_after > now_ms());

        // Manually rewind run_after so the next claim returns b.
        sqlx::query("UPDATE jobs SET run_after = 0 WHERE id = ?")
            .bind(b)
            .execute(&pool)
            .await
            .unwrap();
        let claimed = claim_next_job(&pool).await.unwrap().unwrap();
        assert_eq!(claimed.id, b);
        assert_eq!(claimed.attempts, 2);

        // Fail it past max_attempts (default 3) to drive it to dead.
        mark_job_failed(&pool, b, "boom2", 1000).await.unwrap();
        sqlx::query("UPDATE jobs SET run_after = 0 WHERE id = ?")
            .bind(b)
            .execute(&pool)
            .await
            .unwrap();
        let claimed = claim_next_job(&pool).await.unwrap().unwrap();
        assert_eq!(claimed.attempts, 3);
        mark_job_failed(&pool, b, "boom3", 1000).await.unwrap();
        let status: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id = ?")
            .bind(b)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "dead");
    }

    #[tokio::test]
    async fn job_queue_reclaim_orphans() {
        let pool = test_pool().await;
        let id = enqueue_job(&pool, JobInput::new("test_kind", serde_json::json!({})))
            .await
            .unwrap();
        let _ = claim_next_job(&pool).await.unwrap().unwrap();
        // Backdate the lock so reclaim treats it as orphaned.
        sqlx::query("UPDATE jobs SET locked_at = 0 WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        let n = reclaim_orphan_jobs(&pool, 60_000).await.unwrap();
        assert_eq!(n, 1);
        let claimed = claim_next_job(&pool).await.unwrap().unwrap();
        assert_eq!(claimed.id, id);
        // attempts counter should NOT reset across reclaim — the
        // first run already consumed one attempt.
        assert_eq!(claimed.attempts, 2);
    }

    #[tokio::test]
    async fn dedup_does_not_prefix_collide() {
        // file_id=42 already queued; enqueueing file_id=421 must NOT
        // be treated as a duplicate. The original LIKE pattern
        // `%"file_id":42%` matched both because 42 is a prefix of
        // 421. The fix anchors the pattern with the JSON-value
        // terminator (`,` or `}`).
        let pool = test_pool().await;
        let _id_42 = enqueue_job_unique(
            &pool,
            JobInput::new("test_kind", serde_json::json!({ "file_id": 42 })),
            "file_id",
            42,
        )
        .await
        .unwrap()
        .expect("first enqueue should insert");
        let id_421 = enqueue_job_unique(
            &pool,
            JobInput::new("test_kind", serde_json::json!({ "file_id": 421 })),
            "file_id",
            421,
        )
        .await
        .unwrap()
        .expect("file_id 421 must not be deduped against file_id 42");
        // And re-enqueueing 42 should still dedup correctly.
        let id_42_again = enqueue_job_unique(
            &pool,
            JobInput::new("test_kind", serde_json::json!({ "file_id": 42 })),
            "file_id",
            42,
        )
        .await
        .unwrap();
        assert!(id_42_again.is_none(), "re-enqueue of 42 should be a no-op");
        // Multi-field payloads (subtitle handler shape) also dedup
        // correctly on the targeted field.
        let _ = enqueue_job_unique(
            &pool,
            JobInput::new(
                "subs_kind",
                serde_json::json!({ "item_id": 99, "languages": ["en"] }),
            ),
            "item_id",
            99,
        )
        .await
        .unwrap()
        .expect("first subs enqueue should insert");
        let dup = enqueue_job_unique(
            &pool,
            JobInput::new(
                "subs_kind",
                serde_json::json!({ "item_id": 99, "languages": ["es"] }),
            ),
            "item_id",
            99,
        )
        .await
        .unwrap();
        assert!(
            dup.is_none(),
            "second subs enqueue for same item should dedup"
        );
        assert!(id_421 > 0);
    }

    async fn test_pool() -> SqlitePool {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                payload TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'queued',
                priority INTEGER NOT NULL DEFAULT 0,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 3,
                run_after INTEGER NOT NULL DEFAULT 0,
                locked_at INTEGER,
                last_error TEXT,
                error_class TEXT,
                created_at INTEGER NOT NULL,
                started_at INTEGER,
                finished_at INTEGER
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[test]
    fn air_date_epoch() {
        // 1970-01-01 = 0
        assert_eq!(parse_air_date_to_ms("1970-01-01"), Some(0));
        // 2024-01-19 = 1705622400000 (UTC midnight)
        assert_eq!(parse_air_date_to_ms("2024-01-19"), Some(1_705_622_400_000));
    }

    /// In-memory DB with just the tables `vault_self_test` looks at.
    /// Hand-rolled (rather than running migrations) so the test is
    /// fast and self-contained.
    async fn vault_test_pool() -> SqlitePool {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE secrets (
                name TEXT PRIMARY KEY,
                value_enc BLOB NOT NULL,
                nonce BLOB,
                updated_at INTEGER NOT NULL DEFAULT 0,
                updated_by INTEGER
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE webhooks (
                id INTEGER PRIMARY KEY,
                secret_enc BLOB,
                secret_nonce BLOB
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE user_totp (
                user_id INTEGER PRIMARY KEY,
                secret_enc BLOB NOT NULL,
                secret_nonce BLOB
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn vault_self_test_reports_no_encrypted_rows_when_empty() {
        let pool = vault_test_pool().await;
        let vault = chimpflix_common::Vault::with_key(&[0u8; 32]).unwrap();
        let result = vault_self_test(&pool, &vault).await.unwrap();
        assert!(
            matches!(result, VaultSelfTest::NoEncryptedRows),
            "expected NoEncryptedRows, got {result:?}",
        );
    }

    #[tokio::test]
    async fn vault_self_test_ok_when_keys_match() {
        let pool = vault_test_pool().await;
        let vault = chimpflix_common::Vault::with_key(&[1u8; 32]).unwrap();
        vault_set(&pool, &vault, "tmdb", "secret-value", None)
            .await
            .unwrap();
        let result = vault_self_test(&pool, &vault).await.unwrap();
        assert!(
            matches!(result, VaultSelfTest::Ok { .. }),
            "expected Ok, got {result:?}",
        );
    }

    #[tokio::test]
    async fn vault_self_test_mismatch_when_key_rotated() {
        let pool = vault_test_pool().await;
        let original = chimpflix_common::Vault::with_key(&[1u8; 32]).unwrap();
        vault_set(&pool, &original, "tmdb", "secret-value", None)
            .await
            .unwrap();
        let rotated = chimpflix_common::Vault::with_key(&[2u8; 32]).unwrap();
        let result = vault_self_test(&pool, &rotated).await.unwrap();
        assert!(
            matches!(result, VaultSelfTest::Mismatch { .. }),
            "expected Mismatch, got {result:?}",
        );
    }
}
