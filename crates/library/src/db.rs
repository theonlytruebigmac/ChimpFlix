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
pub async fn open_with(
    data_dir: &Path,
    cache_size_mb: Option<i64>,
) -> anyhow::Result<SqlitePool> {
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
    let mut opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
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
