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
    // `busy_timeout` lets SQLite poll for up to 30s before returning
    // SQLITE_BUSY when another connection holds the write lock — the
    // default of 0 surfaces as "database is locked" 500s on any
    // mid-transaction collision (e.g. the merge endpoint racing a
    // scanner upsert on a parallel connection).
    //
    // 30s is generous on purpose. The original 5s was enough for
    // light contention but lost under load — a library scan racing
    // 8 active workers (markers + loudness + bootstrap) was hitting
    // BUSY on inserts within seconds and the scanner would bail
    // mid-run, half-populating the library. Bumping to 30s absorbs
    // burst contention from the worker pool. Layer 2's
    // `library_scan_exclusive` flag pauses workers during a fresh
    // library scan so the 30s shouldn't ever actually elapse in
    // practice — it's a safety net for the unexpected case (e.g.
    // a workflow that holds an admin write open longer than usual).
    let mut opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_secs(30))
        .foreign_keys(true);
    if let Some(mb) = cache_size_mb.filter(|n| *n > 0) {
        // Negative N = "N KiB of cache"; positive N = "N pages". We
        // want the size-based form because page size varies (default
        // 4 KiB on modern SQLite but not guaranteed).
        let kib = mb.saturating_mul(1024);
        opts = opts.pragma("cache_size", format!("-{kib}"));
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
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
