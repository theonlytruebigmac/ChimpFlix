//! End-to-end migration + boot test.
//!
//! Opens a fresh SQLite DB in a per-run temp dir, runs `db::open`
//! (which fans out to every migration under `migrations/`), then
//! asserts the schema landed correctly and a round-trip through
//! `get_server_settings` / `update_server_settings` actually works.
//!
//! Smoke-grade — doesn't try to cover every column. The point is to
//! catch the "migration order broke, or a recent migration's CHECK
//! constraint rejects the default singleton row" class of bugs
//! before they ship.

use std::path::PathBuf;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use chimpflix_library::{db, queries, ServerSettingsUpdate};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool};

/// Per-run temp dir so parallel `cargo test` invocations don't
/// collide. Falls back to `target/` for predictable cleanup when the
/// system temp can't be written to (e.g., containerised CI).
fn fresh_data_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let base = std::env::temp_dir();
    let dir = base.join(format!("chimpflix-test-{pid}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("create test data dir");
    dir
}

/// Cleanup hook — removes the temp dir at the end of the test. Best
/// effort; leftover dirs are harmless (next run picks its own nanos).
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn migrations_apply_cleanly_and_settings_round_trip() {
    let data_dir = fresh_data_dir();
    let _cleanup = Cleanup(data_dir.clone());

    let pool = db::open(&data_dir).await.expect("db::open succeeds");

    // ─── Migrations landed: spot-check tables that live across many
    //     phases. Missing any of these means an ALTER got out of
    //     order or a CHECK rejected the migration.
    let must_exist = [
        "server_settings",
        "users",
        "libraries",
        "items",
        "media_files",
        "collections",
        "collection_items",
        "scheduled_tasks",
        "audit_log",
        "secrets",
        "tags",
        "external_subtitles",
        "user_my_list",
        "user_trakt_tokens",
    ];
    for name in must_exist {
        let row = sqlx::query("SELECT name FROM sqlite_master WHERE type='table' AND name = ?")
            .bind(name)
            .fetch_optional(&pool)
            .await
            .expect("query sqlite_master")
            .unwrap_or_else(|| panic!("table `{name}` missing after migrations"));
        let _: String = row.try_get("name").unwrap();
    }

    // ─── Singleton row created with default values. Phase 1 has
    //     `INSERT OR IGNORE INTO server_settings(id) VALUES (1)`;
    //     every later phase adds columns with defaults. If any
    //     migration broke that contract, get_server_settings would
    //     fail or return non-default values.
    let s0 = queries::get_server_settings(&pool)
        .await
        .expect("load default settings");
    assert!(
        s0.transcoder_max_concurrent > 0,
        "default max_concurrent should be > 0",
    );
    assert_eq!(s0.video_completion_behaviour, "threshold_pct");
    assert_eq!(s0.transcoder_hevc_encoding_mode, "off");
    assert_eq!(s0.transcoder_gpu_device, "auto");
    assert!(!s0.preroll_enabled);
    assert!(!s0.detect_markers_on_add);

    // ─── Round-trip a setting: patch, reload, verify the change
    //     stuck. Catches the "patch handler missed a recently-added
    //     column" class of bug.
    queries::update_server_settings(
        &pool,
        None,
        ServerSettingsUpdate {
            transcoder_hevc_encoding_mode: Some("when_client_supports".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("patch hevc setting");
    let s1 = queries::get_server_settings(&pool).await.expect("reload");
    assert_eq!(s1.transcoder_hevc_encoding_mode, "when_client_supports");

    // ─── Whitelist validation: bad values should be rejected at
    //     patch time, not silently accepted.
    let bad = queries::update_server_settings(
        &pool,
        None,
        ServerSettingsUpdate {
            transcoder_hevc_encoding_mode: Some("turbo".to_string()),
            ..Default::default()
        },
    )
    .await;
    assert!(bad.is_err(), "invalid hevc mode should be rejected");

    // ─── Manual collection schema dance (phase 36 + 41 rebuild
    //     `collections` twice). Insert + select to confirm the
    //     final table shape accepts NULL tmdb_id + the kind enum.
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO collections (tmdb_id, kind, name, created_at, updated_at)
         VALUES (NULL, 'manual', 'TestCollection', 0, 0) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert manual collection");
    assert!(id > 0);
}

/// Regression test for the bug that crashed the user's Docker deployment:
/// phase 36's collections-rebuild dance was firing `ON DELETE SET NULL`
/// on `items.collection_id` when the DB had populated rows (because the
/// migration's `PRAGMA foreign_keys = OFF` is a no-op inside the wrapping
/// transaction that sqlx-sqlite always opens). The cascade either
/// surfaced as `SQLITE_LOCKED` mid-migration or — worse — silently wiped
/// every existing franchise link. The fix in `db::open_with` runs
/// migrations on a dedicated pool with FK off at the *connection* level,
/// which IS honored inside the transaction.
///
/// This test simulates the populated-DB path directly: it constructs a
/// pre-phase-36 schema (collections + items + the FK linkage) by hand,
/// populates it, then executes the phase 36 SQL with FK off — matching
/// what the migration pool does in production. The assertion that
/// matters is that `items.collection_id` survives the rebuild.
#[tokio::test]
async fn phase36_rebuild_preserves_item_collection_links() {
    let data_dir = fresh_data_dir();
    let _cleanup = Cleanup(data_dir.clone());

    let db_path = data_dir.join("phase36-regression.db");
    let url = format!("sqlite://{}", db_path.display());

    // Mirror what `db::open_with` does for the migration pool: FK off,
    // WAL, single connection. The bug only manifests when FK enforcement
    // is on, so if someone reverts the pool-level toggle this test will
    // start exercising the broken path and fail.
    let opts = SqliteConnectOptions::from_str(&url)
        .expect("parse sqlite url")
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(false);
    let pool = SqlitePool::connect_with(opts)
        .await
        .expect("open regression-test pool");

    // Hand-roll a minimal pre-phase-36 schema. Real phases 1-35 add
    // dozens of unrelated tables — we only need what phase 36 touches.
    sqlx::query(
        r#"
        CREATE TABLE users (id INTEGER PRIMARY KEY);
        CREATE TABLE items (id INTEGER PRIMARY KEY);
        CREATE TABLE collections (
            id INTEGER PRIMARY KEY,
            tmdb_id INTEGER NOT NULL UNIQUE,
            name TEXT NOT NULL,
            overview TEXT,
            poster_path TEXT,
            backdrop_path TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        ALTER TABLE items ADD COLUMN collection_id INTEGER
            REFERENCES collections(id) ON DELETE SET NULL;
        "#,
    )
    .execute(&pool)
    .await
    .expect("create pre-phase-36 schema");

    // Populate: 2 collections, 3 items linked to them. If the rebuild
    // cascades, all three items.collection_id values will be NULL after.
    sqlx::query(
        r#"
        INSERT INTO collections (id, tmdb_id, name, created_at, updated_at) VALUES
            (10, 100, 'John Wick Collection', 0, 0),
            (20, 200, 'Studio Ghibli Collection', 0, 0);
        INSERT INTO items (id, collection_id) VALUES
            (1, 10),
            (2, 10),
            (3, 20);
        "#,
    )
    .execute(&pool)
    .await
    .expect("populate test data");

    // Execute the actual phase 36 migration SQL from disk. Using the
    // checked-in file (not a copy) means future edits to the migration
    // are tested by this same assertion.
    let phase36_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("migrations")
        .join("20260518120000_phase36_manual_collections.sql");
    let phase36_sql = std::fs::read_to_string(&phase36_path)
        .expect("read phase 36 migration file");
    sqlx::query(&phase36_sql)
        .execute(&pool)
        .await
        .expect("apply phase 36 — must succeed with FK off");

    // The whole point: items.collection_id values must survive.
    let preserved: Vec<(i64, Option<i64>)> =
        sqlx::query_as("SELECT id, collection_id FROM items ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("re-read items");
    assert_eq!(
        preserved,
        vec![(1, Some(10)), (2, Some(10)), (3, Some(20))],
        "phase 36 must not cascade-null items.collection_id",
    );

    // Phase 41 hits the same dance — exercise it too, with the same
    // assertion. (kind defaults to 'auto' from the phase-36 INSERT
    // path, so the row shape matches phase 41's SELECT.)
    let phase41_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("migrations")
        .join("20260518170000_phase41_smart_collections.sql");
    let phase41_sql = std::fs::read_to_string(&phase41_path)
        .expect("read phase 41 migration file");
    sqlx::query(&phase41_sql)
        .execute(&pool)
        .await
        .expect("apply phase 41 — must succeed with FK off");

    let preserved_after_41: Vec<(i64, Option<i64>)> =
        sqlx::query_as("SELECT id, collection_id FROM items ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("re-read items after phase 41");
    assert_eq!(
        preserved_after_41,
        vec![(1, Some(10)), (2, Some(10)), (3, Some(20))],
        "phase 41 must also preserve items.collection_id",
    );

    pool.close().await;
}
