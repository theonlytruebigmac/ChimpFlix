//! SQLite library database: connection pool + migrations.

use std::path::Path;
use std::str::FromStr;

use anyhow::Context;
use sqlx::Row;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tracing::info;

/// Open the library database, creating it if missing, and run all
/// migrations. The DB file lives at `${data_dir}/chimpflix.db`.
///
/// Equivalent to `open_with(data_dir, None)` — kept as a thin wrapper
/// so callers that don't care about the cache_size knob (tests, CLI
/// tools) don't need to thread it through.
pub async fn open(data_dir: &Path) -> anyhow::Result<SqlitePool> {
    open_with(data_dir, None).await
}

/// Open the library database with an explicit SQLite page-cache size
/// (in MiB). Pass `None` to leave SQLite's default in place. Negative
/// or zero values are treated as "default" so the operator can clear
/// the override by setting `database_cache_size_mb = 0`.
///
/// Applied via `PRAGMA cache_size = -<KiB>`, which is a per-connection
/// setting; baking it into `SqliteConnectOptions` ensures every pooled
/// connection picks it up at creation, not just the first one.
///
/// ## Two-pool startup
///
/// Migrations run on a dedicated single-connection pool with `PRAGMA
/// foreign_keys = OFF`. After they complete (and a `foreign_key_check`
/// validates the result), that pool is closed and the real app pool is
/// opened with FK enforcement on.
///
/// Why: the rebuild-dance migrations (phase 36 + 41) drop and recreate
/// `collections`. With FK enabled, that fires the `ON DELETE SET NULL`
/// cascade on `items.collection_id` and wipes every franchise link on
/// populated databases (it also surfaces as `SQLITE_LOCKED` on WAL in
/// some cases). `PRAGMA foreign_keys` toggles are no-ops inside a
/// transaction, and sqlx-sqlite 0.8 unconditionally wraps every
/// migration in one (it ignores the `-- no-transaction` marker —
/// that's a Postgres-only feature). The connection-level pragma, set
/// at pool startup *before* any transaction begins, is the only knob
/// that actually disables enforcement during the migration body.
pub async fn open_with(data_dir: &Path, cache_size_mb: Option<i64>) -> anyhow::Result<SqlitePool> {
    tokio::fs::create_dir_all(data_dir)
        .await
        .with_context(|| format!("create data dir {}", data_dir.display()))?;

    // SECURITY: lock the data dir down to owner-only. The DB carries
    // hashed passwords, session-cookie HMAC, SMTP creds, TOTP secrets,
    // and the credential vault. Inheriting umask (typically 022 → 0755
    // / 0644) means any local UID could read it; a hostile sibling
    // container or compromised neighbouring service would walk in.
    // Best-effort: log on failure but don't abort the boot (FAT/SMB
    // mounts and Windows containers don't honour Unix perms).
    #[cfg(unix)]
    if let Err(e) = lock_down_data_dir(data_dir) {
        tracing::warn!(
            data_dir = %data_dir.display(),
            error = %format!("{e:#}"),
            "could not chmod data dir to 0700; secrets may be readable by other users"
        );
    }

    let db_path = data_dir.join("chimpflix.db");
    let url = format!("sqlite://{}", db_path.display());

    // Pre-migration auto-backup (MONTH 1 in
    // `docs/PUBLIC_RELEASE_HARDENING.md`). When the DB exists and has
    // applied migrations whose max version is below what the embedded
    // migrator is about to run, copy the file to
    // `<data_dir>/backups/pre-migration/<unix_stamp>.db` first. A bad
    // migration on upgrade is otherwise an unrecoverable wedge —
    // SQLite ALTER TABLE is forward-only, no automatic rollback.
    //
    // First boot (no DB) or already-current (no pending migrations)
    // both skip the snapshot — it's cheap-to-detect and there's
    // nothing to lose.
    if let Err(e) = pre_migration_snapshot(data_dir, &db_path).await {
        tracing::warn!(
            error = %format!("{e:#}"),
            "pre-migration auto-backup failed; continuing boot anyway",
        );
    }

    // ── Migration pool: single connection, FK off ───────────────────
    let migrate_opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(false);

    let migrate_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(migrate_opts)
        .await
        .context("open library database for migrations")?;

    sqlx::migrate!("./migrations")
        .run(&migrate_pool)
        .await
        .context("apply library migrations")?;

    // Validate that nothing the migrations did left dangling foreign
    // keys behind. `PRAGMA foreign_key_check` returns one row per
    // violation (child_table, rowid, parent_table, fk_id) and zero
    // rows on a healthy DB.
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&migrate_pool)
        .await
        .context("foreign_key_check after migrations")?;
    if !violations.is_empty() {
        let mut sample: Vec<String> = violations
            .iter()
            .take(5)
            .map(|row| {
                let table: String = row.try_get(0).unwrap_or_default();
                let rowid: i64 = row.try_get(1).unwrap_or(-1);
                let parent: String = row.try_get(2).unwrap_or_default();
                format!("{table}#{rowid} → {parent}")
            })
            .collect();
        if violations.len() > 5 {
            sample.push(format!("(+{} more)", violations.len() - 5));
        }
        anyhow::bail!(
            "foreign_key_check found {} violation(s) after migrations: {}",
            violations.len(),
            sample.join(", "),
        );
    }

    migrate_pool.close().await;

    // ── App pool: full concurrency, FK on ──────────────────────────
    // `busy_timeout` lets SQLite poll for up to this long before returning
    // SQLITE_BUSY when another connection holds the write lock — the
    // default of 0 surfaces as "database is locked" 500s on any
    // mid-transaction collision (e.g. the merge endpoint racing a
    // scanner upsert on a parallel connection).
    //
    // History: 5s lost under load (a scan racing 8 active workers bailed
    // mid-run); we then over-corrected to 30s, which — stacked on sqlx's
    // *default 30s acquire wait* (see the pool builder below, which had no
    // explicit `acquire_timeout`) — meant a write blocked behind the single
    // WAL writer could hold a pool connection for up to 60s and return
    // `rows_affected=0` (the write LOST). Under a download-burst convoy this
    // exhausted the 24-conn pool and wedged the server (2026-06-13 incident).
    //
    // 12s is the middle ground: long enough that the scanner's (un-retried)
    // upserts don't bail when a fast single-row foreground write briefly
    // holds the lock, but far below the old 60s stacked stall. The real
    // anti-wedge levers are now (1) the explicit `acquire_timeout` on the
    // pool below, which caps the waiter backlog, and (2) the `with_write_gate`
    // serialization of the hot foreground writers (see below), which collapses
    // concurrent foreground writers to one so the pool can't fill with
    // connections all blocked on the same lock. Layer-2 `library_scan_exclusive`
    // still pauses the worker pool during scans, so 12s is rarely approached.
    let mut opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(12))
        .foreign_keys(true);
    if let Some(mb) = cache_size_mb.filter(|n| *n > 0) {
        // Negative N = "N KiB of cache"; positive N = "N pages". We
        // want the size-based form because page size varies (default
        // 4 KiB on modern SQLite but not guaranteed).
        let kib = mb.saturating_mul(1024);
        opts = opts.pragma("cache_size", format!("-{kib}"));
    }

    // Pool sizing: 24 connections. Bumped from 8 by WEEK 1 #7 in
    // `docs/PUBLIC_RELEASE_HARDENING.md`. With the previous 8-conn
    // cap + 30s `busy_timeout`, ~50 concurrent mixed-read users
    // could exhaust the pool and queue behind it for up to 30s
    // before timing out. 24 is comfortable for a busy multi-user
    // deployment; going higher than this without profiling tends
    // to hurt SQLite write contention (the WAL serialises writes,
    // so more readers waiting on a writer doesn't help throughput).
    // `acquire_timeout` caps how long a caller waits for a free pool
    // connection. sqlx's default is 30s, which — stacked on the 12s
    // `busy_timeout` above — let a write that couldn't get a slot hang the
    // request for tens of seconds and let the waiter backlog snowball until
    // the pool wedged (2026-06-13). A short 5s ceiling makes an exhausted
    // pool fail FAST and shed load instead of accumulating blocked tasks; the
    // `with_busy_retry` backoff (~5s over 8 attempts) absorbs genuine bursts.
    // (The probe/vacuum pools already set a 10s acquire timeout — this brings
    // the hot pool in line.) NOTE: do NOT raise max_connections to "fix"
    // contention — SQLite serialises writes through one WAL writer, so more
    // readers waiting on a writer doesn't help throughput.
    let pool = SqlitePoolOptions::new()
        .max_connections(24)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect_with(opts)
        .await
        .context("open library database")?;

    // Tighten DB file perms after the pool has opened (and thus
    // created `chimpflix.db` plus the WAL / SHM sidecar files).
    #[cfg(unix)]
    if let Err(e) = lock_down_db_files(data_dir) {
        tracing::warn!(
            data_dir = %data_dir.display(),
            error = %format!("{e:#}"),
            "could not chmod chimpflix.db to 0600"
        );
    }

    info!(
        ?db_path,
        cache_size_mb = ?cache_size_mb,
        "library database ready",
    );
    Ok(pool)
}

// ---------------------------------------------------------------------------
// BUSY / SNAPSHOT retry helper
// ---------------------------------------------------------------------------

/// Run `f` and transparently retry on the two flavors of SQLite write
/// contention that aren't already absorbed by `busy_timeout`:
///
/// - **Code 5 (`SQLITE_BUSY`)** — the writer lock is held by another
///   connection. `busy_timeout` polls for this up to 30s; the retry
///   here is defensive for the edge case where the timeout itself
///   expires (deep contention, slow writes).
/// - **Code 517 (`SQLITE_BUSY_SNAPSHOT`)** — extended-result-code-only
///   error specific to WAL mode. Fires at *commit* time when our read
///   snapshot started before another writer advanced the WAL. This is
///   the variant that the Movies backfill kept hitting because
///   `enqueue_job_unique` does SELECT (snapshot) → INSERT (upgrade);
///   `busy_timeout` does NOT poll-retry 517 because it's not a wait-
///   for-lock condition — it's an optimistic-concurrency conflict.
///
/// Retries with exponential backoff capped at ~1.6s (25, 50, 100, 200,
/// 400, 800, 1600 ms, then 1600 ms each thereafter). `MAX_ATTEMPTS = 8`
/// means a worst-case wait of ~5s of cumulative backoff before
/// surrendering — well under any reasonable HTTP timeout but enough
/// burst absorption for the scanner+worker race.
///
/// When a retry actually fires, logs at `info` so an operator watching
/// the activity feed can see contention without surprises. Retries
/// silently succeed are NOT logged — we don't want a log line per
/// successful write under load.
pub async fn with_busy_retry<F, Fut, T>(mut f: F) -> anyhow::Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    const MAX_ATTEMPTS: usize = 8;
    let mut backoff_ms: u64 = 25;
    for attempt in 1..=MAX_ATTEMPTS {
        match f().await {
            Ok(v) => return Ok(v),
            Err(e) => {
                if !is_sqlite_busy_anyhow(&e) || attempt == MAX_ATTEMPTS {
                    return Err(e);
                }
                let code = anyhow_sqlite_code(&e).unwrap_or_else(|| "?".into());
                tracing::info!(
                    attempt,
                    code = %code,
                    wait_ms = backoff_ms,
                    "SQLite write retry after BUSY/SNAPSHOT contention"
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(1600);
            }
        }
    }
    // Unreachable — the loop either returns Ok or returns the inner
    // Err on the final attempt. Kept for the compiler.
    unreachable!("with_busy_retry exited loop without returning")
}

// ---------------------------------------------------------------------------
// Single-writer GATE
// ---------------------------------------------------------------------------

/// Process-global write gate: a single permit.
///
/// SQLite has exactly one WAL writer, so concurrent write transactions
/// serialise on the file lock no matter what. The danger isn't the
/// serialisation — it's that blocked writers hold a scarce POOL connection
/// while they wait out `busy_timeout`, so a burst of concurrent writes can
/// pin every connection in the 24-slot pool and wedge the whole server
/// (2026-06-13 incident: a scan convoy + per-request session touches +
/// play-state ticks all blocked at once → pool exhausted → manual restart).
///
/// Acquiring this in-memory permit BEFORE touching a pool connection moves the
/// waiting OFF the pool: only one gated writer runs at a time, so the rest
/// queue cheaply in memory instead of each holding a connection blocked on the
/// SQLite lock. Reads never take the gate (WAL keeps reads non-blocking).
///
/// Scope: this serialises the high-volume FOREGROUND leaf writers (play-state
/// batch, session touch, job claim). It is deliberately NOT threaded through
/// the scanner's enrich writes — those are already kept off the worker pool by
/// `library_scan_exclusive`, and routing every repo write through the gate
/// would be a large, risky change for little marginal benefit here.
static WRITE_GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

/// Run `f` while holding the single global write permit. See [`WRITE_GATE`].
///
/// CONTRACT: wrap only LEAF write operations. Never call `with_write_gate`
/// (directly or transitively) from inside another `with_write_gate` closure —
/// the semaphore is not re-entrant, so a nested acquire self-deadlocks. The
/// three current call sites (`apply_play_state_batch`, `touch_session`, the
/// job-claim UPDATE) are all leaf writes that call no other gated write.
pub async fn with_write_gate<F, Fut, T>(f: F) -> anyhow::Result<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<T>>,
{
    // `acquire()` only errors if the semaphore is closed; we never close it.
    let _permit = WRITE_GATE
        .acquire()
        .await
        .expect("WRITE_GATE is never closed");
    f().await
}

/// True when the error chain contains a SQLite BUSY (code 5) or
/// BUSY_SNAPSHOT (code 517). Walks the anyhow chain so callers can
/// freely wrap sqlx errors in context().
fn is_sqlite_busy_anyhow(e: &anyhow::Error) -> bool {
    anyhow_sqlite_code(e)
        .as_deref()
        .is_some_and(|c| c == "5" || c == "517")
}

fn anyhow_sqlite_code(e: &anyhow::Error) -> Option<String> {
    for cause in e.chain() {
        if let Some(sqlx::Error::Database(db)) = cause.downcast_ref::<sqlx::Error>() {
            if let Some(code) = db.code() {
                return Some(code.into_owned());
            }
        }
    }
    None
}

/// Compare the embedded migrator's max version against the DB's
/// `_sqlx_migrations` max version. When the DB is behind, snapshot
/// `chimpflix.db` to `<data_dir>/backups/pre-migration/<unix_stamp>.db`
/// before letting the live migration run.
///
/// Skips cleanly when:
///   * `chimpflix.db` doesn't exist (first boot, no schema to lose).
///   * The file exists but is empty (interrupted prior boot).
///   * The DB has no `_sqlx_migrations` table yet (also first boot).
///   * Embedded max == DB max (no pending migrations).
///
/// Best-effort — any failure logs and returns Ok so a flaky snapshot
/// doesn't block the boot.
async fn pre_migration_snapshot(
    data_dir: &Path,
    db_path: &Path,
) -> anyhow::Result<()> {
    let meta = match tokio::fs::metadata(db_path).await {
        Ok(m) if m.len() > 0 => m,
        _ => return Ok(()),
    };

    let embedded_max = sqlx::migrate!("./migrations")
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0);
    if embedded_max == 0 {
        return Ok(());
    }

    // Open a read-only probe pool — never touches the WAL, never
    // creates the file. Returns None when `_sqlx_migrations` doesn't
    // exist yet (fresh-ish DB about to receive its first migration
    // run from this binary).
    let url = format!("sqlite://{}?mode=ro", db_path.display());
    let probe_opts = SqliteConnectOptions::from_str(&url)?;
    let probe_pool = match SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(probe_opts)
        .await
    {
        Ok(p) => p,
        Err(_) => return Ok(()),
    };
    let applied_max: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(version) FROM _sqlx_migrations",
    )
    .fetch_optional(&probe_pool)
    .await
    .ok()
    .flatten();
    probe_pool.close().await;

    let applied = applied_max.unwrap_or(-1);
    if applied >= embedded_max {
        // Already current — nothing to back up against.
        return Ok(());
    }

    let backup_dir = data_dir.join("backups").join("pre-migration");
    tokio::fs::create_dir_all(&backup_dir).await?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let dest = backup_dir.join(format!("chimpflix-pre-{stamp}-v{applied}-to-v{embedded_max}.db"));

    // Use `VACUUM INTO` instead of a raw file copy: in WAL mode the main
    // database file may lag behind the WAL by many frames.  `VACUUM INTO`
    // reads the current consistent snapshot (including uncheckpointed WAL
    // frames) and writes a single, self-contained SQLite file — no sidecar
    // needed and no torn-state risk.
    let src_url = format!("sqlite://{}?mode=ro", db_path.display());
    let vacuum_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect_with(SqliteConnectOptions::from_str(&src_url)?)
        .await
        .context("open source DB for pre-migration VACUUM INTO")?;
    let dest_str = dest.to_string_lossy().into_owned();
    sqlx::query("VACUUM INTO ?")
        .bind(&dest_str)
        .execute(&vacuum_pool)
        .await
        .context("VACUUM INTO pre-migration snapshot")?;
    vacuum_pool.close().await;

    info!(
        from = %db_path.display(),
        to = %dest.display(),
        bytes = meta.len(),
        applied_version = applied,
        target_version = embedded_max,
        "pre-migration snapshot written",
    );
    Ok(())
}

#[cfg(unix)]
fn lock_down_data_dir(data_dir: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(data_dir, perms)?;
    Ok(())
}

#[cfg(unix)]
fn lock_down_db_files(data_dir: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    for name in ["chimpflix.db", "chimpflix.db-wal", "chimpflix.db-shm"] {
        let p = data_dir.join(name);
        if p.exists() {
            std::fs::set_permissions(&p, perms.clone())?;
        }
    }
    Ok(())
}
