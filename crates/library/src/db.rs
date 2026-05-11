//! SQLite library database: connection pool + migrations.

use std::path::Path;
use std::str::FromStr;

use anyhow::Context;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use tracing::info;

/// Open the library database, creating it if missing, and run all
/// migrations. The DB file lives at `${data_dir}/chimpflix.db`.
pub async fn open(data_dir: &Path) -> anyhow::Result<SqlitePool> {
    tokio::fs::create_dir_all(data_dir)
        .await
        .with_context(|| format!("create data dir {}", data_dir.display()))?;

    let db_path = data_dir.join("chimpflix.db");
    let url = format!("sqlite://{}", db_path.display());

    let opts = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(opts)
        .await
        .context("open library database")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("apply library migrations")?;

    info!(?db_path, "library database ready");
    Ok(pool)
}
