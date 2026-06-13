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

use chimpflix_library::{ServerSettingsUpdate, db, queries};
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
    let phase36_sql = std::fs::read_to_string(&phase36_path).expect("read phase 36 migration file");
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
    let phase41_sql = std::fs::read_to_string(&phase41_path).expect("read phase 41 migration file");
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

/// Library-browse "Unwatched / In progress / Watched" status filters
/// must agree with the Continue Watching rail about what those states
/// mean for SHOWS. Before this regression test landed, the filter
/// joined `play_state` directly on the show's item_id — but progress
/// for a show always lives on its episode rows (the `play_state` CHECK
/// constraint enforces exactly one of `item_id` / `episode_id`). The
/// result was:
///   * "In progress" returned zero shows even when CW had them.
///   * "Watched" returned zero shows even when every episode was
///     marked watched (the show-level Mark-watched toggle on the
///     title page agreed it was finished).
///   * "Unwatched" returned every show, including ones the user had
///     fully finished — because the show's own `play_state` row never
///     exists, the LEFT JOIN gives NULL, and `(NULL OR watched=0)` is
///     true.
///
/// This test builds four shows in four distinct states and asserts
/// each filter returns exactly the shows it should.
#[tokio::test]
async fn status_filters_aggregate_show_state_from_episodes() {
    use chimpflix_library::{ItemFilter, ItemKind, queries};

    let data_dir = fresh_data_dir();
    let _cleanup = Cleanup(data_dir.clone());
    let pool = db::open(&data_dir).await.expect("db::open succeeds");

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    // One user. The status filters key on user_id, but a single user
    // is enough — each show is its own state under that user.
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) \
         VALUES (1, 'tester', '', 'owner', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert user");

    sqlx::query(
        "INSERT INTO libraries (id, name, kind, created_at, updated_at) \
         VALUES (1, 'Shows', 'tv', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert library");

    // Four shows, each with two episodes and one file per episode.
    // ids: show=100+N, season=200+N, episodes=300+N0/N1, files=400+N0/N1.
    let shows = [
        (101, "Untouched"),     // no play_state at all
        (102, "MidEpisode"),    // ep0 has position > 0, watched = 0
        (103, "PartialWatch"),  // ep0 watched=1, ep1 untouched
        (104, "FullyWatched"),  // both episodes watched=1
    ];
    for (show_id, title) in shows {
        sqlx::query(
            "INSERT INTO items (id, library_id, kind, title, sort_title, added_at, updated_at) \
             VALUES (?, 1, 'show', ?, ?, ?, ?)",
        )
        .bind(show_id)
        .bind(title)
        .bind(title.to_lowercase())
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert show");

        let season_id = show_id + 100;
        sqlx::query(
            "INSERT INTO seasons (id, show_id, season_number) VALUES (?, ?, 1)",
        )
        .bind(season_id)
        .bind(show_id)
        .execute(&pool)
        .await
        .expect("insert season");

        for ep_idx in 0..2_i64 {
            let episode_id = show_id * 10 + ep_idx;
            sqlx::query(
                "INSERT INTO episodes (id, season_id, episode_number, title, added_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(episode_id)
            .bind(season_id)
            .bind(ep_idx + 1)
            .bind(format!("E{}", ep_idx + 1))
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .expect("insert episode");

            sqlx::query(
                "INSERT INTO media_files (episode_id, path, size_bytes, mtime_ms, scanned_at) \
                 VALUES (?, ?, 1, ?, ?)",
            )
            .bind(episode_id)
            .bind(format!("/tmp/show{show_id}/e{ep_idx}.mkv"))
            .bind(now)
            .bind(now)
            .execute(&pool)
            .await
            .expect("insert media file");
        }
    }

    // Now wire play_state per show, matching each state above.
    // MidEpisode: ep0 has position_ms > 0, watched=0.
    sqlx::query(
        "INSERT INTO play_state \
            (user_id, episode_id, position_ms, watched, last_played_at) \
         VALUES (1, ?, 60000, 0, ?)",
    )
    .bind(102 * 10) // MidEpisode ep0
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert mid-episode play_state");

    // PartialWatch: ep0 watched=1, ep1 untouched.
    sqlx::query(
        "INSERT INTO play_state \
            (user_id, episode_id, position_ms, watched, last_played_at) \
         VALUES (1, ?, 0, 1, ?)",
    )
    .bind(103 * 10)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert partial-watch ep0 play_state");

    // FullyWatched: both episodes watched=1.
    for ep_idx in 0..2_i64 {
        sqlx::query(
            "INSERT INTO play_state \
                (user_id, episode_id, position_ms, watched, last_played_at) \
             VALUES (1, ?, 0, 1, ?)",
        )
        .bind(104 * 10 + ep_idx)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert fully-watched play_state");
    }

    // Helper: run list_items with one status flag set, return matched
    // show ids sorted for stable comparison.
    async fn run_filter(
        pool: &SqlitePool,
        kind: ItemKind,
        unwatched: bool,
        in_progress: bool,
        watched: bool,
    ) -> Vec<i64> {
        let filter = ItemFilter {
            kind: Some(kind),
            unwatched_only: if unwatched { Some(true) } else { None },
            in_progress_only: if in_progress { Some(true) } else { None },
            watched_only: if watched { Some(true) } else { None },
            page_size: Some(200),
            ..Default::default()
        };
        let page = queries::list_items(pool, filter, 1, None)
            .await
            .expect("list_items");
        let mut ids: Vec<i64> = page.items.iter().map(|li| li.item.id).collect();
        ids.sort();
        ids
    }

    // Unwatched (shows): only "Untouched" — no episode has any
    // play_state activity for any of the others.
    let got = run_filter(&pool, ItemKind::Show, true, false, false).await;
    assert_eq!(got, vec![101], "unwatched: only Untouched should match");

    // In progress (shows): MidEpisode (one episode mid-position) AND
    // PartialWatch (one watched, one still unwatched). FullyWatched
    // must NOT show up (every active episode is watched). Untouched
    // must NOT show up (no activity at all).
    let got = run_filter(&pool, ItemKind::Show, false, true, false).await;
    assert_eq!(
        got,
        vec![102, 103],
        "in_progress: MidEpisode + PartialWatch should match; \
         FullyWatched + Untouched should not",
    );

    // Watched (shows): only "FullyWatched" — every active episode is
    // marked watched. The PartialWatch case has an unwatched episode
    // left so it must NOT be Watched.
    let got = run_filter(&pool, ItemKind::Show, false, false, true).await;
    assert_eq!(got, vec![104], "watched: only FullyWatched should match");

    // Sanity: with no status filter at all, all four shows should come
    // back. Confirms the OR branches didn't accidentally suppress the
    // baseline list-all path.
    let all = run_filter(&pool, ItemKind::Show, false, false, false).await;
    assert_eq!(all, vec![101, 102, 103, 104], "no-filter baseline");
}

/// Regression: the per-library Top 10 (`list_library_top`) must surface
/// each library item AT MOST ONCE, even when several `trending_cache`
/// rows of the same source resolve to the same item. This is the real
/// anime/MAL case: MAL ranks each cour as a separate `mal_id`, the id map
/// collapses them onto ONE `tvdb_id`, and the local series carries that
/// single `tvdb_id` — so a naive `OR`-JOIN would emit the show once per
/// cour. The fix uses a `MIN(rank)` scalar subquery (best cour wins).
#[tokio::test]
async fn library_top_dedupes_one_item_matching_multiple_ranked_rows() {
    use chimpflix_library::{db, queries};

    let data_dir = fresh_data_dir();
    let _cleanup = Cleanup(data_dir.clone());
    let pool = db::open(&data_dir).await.expect("db::open succeeds");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    sqlx::query(
        "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) \
         VALUES (1, 'tester', '', 'owner', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert user");
    sqlx::query(
        "INSERT INTO libraries (id, name, kind, created_at, updated_at) \
         VALUES (7, 'Anime', 'anime', ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert anime library");

    // One local anime series, matched via tvdb_id (the anime default).
    sqlx::query(
        "INSERT INTO items (id, library_id, kind, title, sort_title, tvdb_id, added_at, updated_at) \
         VALUES (900, 7, 'show', 'My Anime', 'my anime', 555, ?, ?)",
    )
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert anime show");

    // Two MAL-ranking cache rows (different ranks, distinct mal_ids) that
    // BOTH resolve to the same tvdb_id 555 — exactly the per-cour collision.
    for (rank, mal_id) in [(3_i64, 1001_i64), (7, 1002)] {
        sqlx::query(
            "INSERT INTO trending_cache \
                (source, media_kind, rank, tmdb_id, tvdb_id, mal_id, title, fetched_at) \
             VALUES ('mal_ranking', 'show', ?, 0, 555, ?, 'My Anime', ?)",
        )
        .bind(rank)
        .bind(mal_id)
        .bind(now)
        .execute(&pool)
        .await
        .expect("insert mal_ranking cache row");
    }

    let rows = queries::list_library_top(&pool, 7, "show", "mal_ranking", 1, 10, None)
        .await
        .expect("list_library_top");

    // The show must appear exactly once (best rank wins), not twice.
    let ids: Vec<i64> = rows.iter().map(|(_, li)| li.item.id).collect();
    assert_eq!(
        ids,
        vec![900],
        "anime show matching two MAL cours must appear exactly once"
    );
    assert_eq!(rows[0].0, 1, "displayed rank is the row position (1-based)");
}

/// Phase 109 backfill semantics. The boot test above only proves the
/// SQL executes on an EMPTY database — every UPDATE matches zero rows.
/// This exercises the invariants against populated data, and matters
/// because sqlx's checksum makes the migration file immutable once it
/// has been applied anywhere: regressions must be caught before the
/// first deploy, not after.
///
/// Invariants under test:
///   * added_at only ever DECREASES (strict `<` guard) — a row whose
///     files are all NEWER than its stamp is untouched.
///   * soft-removed file rows count as acquisition evidence.
///   * mtime_ms = 0 rows (stat() failed) are ignored.
///   * rows without any files are untouched.
///   * a show takes MIN(mtime) across all episodes' files (3-way join).
///   * running the backfill twice changes nothing (idempotency).
#[tokio::test]
async fn phase109_backfill_added_at_from_file_mtime() {
    let data_dir = fresh_data_dir();
    let _cleanup = Cleanup(data_dir.clone());
    let pool = db::open(&data_dir).await.expect("db::open succeeds");

    const SCAN: i64 = 2_000_000; // the bulk-scan wall-clock stamp

    sqlx::query(
        "INSERT INTO libraries (id, name, kind, created_at, updated_at) \
         VALUES (1, 'Mixed', 'movies', ?, ?)",
    )
    .bind(SCAN)
    .bind(SCAN)
    .execute(&pool)
    .await
    .expect("insert library");

    // Movies covering each invariant. (id, title, file specs as
    // (mtime, removed_at)). All items start stamped at SCAN.
    // 1: plain old file            → drops to 500_000
    // 2: only-newer file           → unchanged (strict <)
    // 3: removed older + live new  → drops to 400_000 (removed counts)
    // 4: only mtime_ms = 0 file    → unchanged (stat-failure sentinel)
    // 5: no files at all           → unchanged
    type FileSpec = (i64, Option<i64>);
    let movies: [(i64, &str, &[FileSpec]); 5] = [
        (1, "OldFile", &[(500_000, None)]),
        (2, "NewerFileOnly", &[(3_000_000, None)]),
        (3, "UpgradedKeepsHistory", &[(400_000, Some(SCAN)), (900_000, None)]),
        (4, "StatFailed", &[(0, None)]),
        (5, "NoFiles", &[]),
    ];
    for (id, title, files) in movies {
        sqlx::query(
            "INSERT INTO items (id, library_id, kind, title, sort_title, added_at, updated_at) \
             VALUES (?, 1, 'movie', ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(title)
        .bind(title.to_lowercase())
        .bind(SCAN)
        .bind(SCAN)
        .execute(&pool)
        .await
        .expect("insert movie");
        for (idx, (mtime, removed_at)) in files.iter().enumerate() {
            sqlx::query(
                "INSERT INTO media_files (item_id, path, size_bytes, mtime_ms, scanned_at, removed_at) \
                 VALUES (?, ?, 1, ?, ?, ?)",
            )
            .bind(id)
            .bind(format!("/m/{id}/v{idx}.mkv"))
            .bind(mtime)
            .bind(SCAN)
            .bind(removed_at)
            .execute(&pool)
            .await
            .expect("insert movie file");
        }
    }

    // One show, two episodes: files at 700_000 and 600_000. The show
    // must land on the MIN across episodes (600_000) via the 3-way
    // join; each episode takes its own file's mtime.
    sqlx::query(
        "INSERT INTO items (id, library_id, kind, title, sort_title, added_at, updated_at) \
         VALUES (10, 1, 'show', 'Show', 'show', ?, ?)",
    )
    .bind(SCAN)
    .bind(SCAN)
    .execute(&pool)
    .await
    .expect("insert show");
    sqlx::query("INSERT INTO seasons (id, show_id, season_number) VALUES (11, 10, 1)")
        .execute(&pool)
        .await
        .expect("insert season");
    for (ep_id, ep_no, mtime) in [(21_i64, 1_i64, 700_000_i64), (22, 2, 600_000)] {
        sqlx::query(
            "INSERT INTO episodes (id, season_id, episode_number, title, added_at, updated_at) \
             VALUES (?, 11, ?, ?, ?, ?)",
        )
        .bind(ep_id)
        .bind(ep_no)
        .bind(format!("E{ep_no}"))
        .bind(SCAN)
        .bind(SCAN)
        .execute(&pool)
        .await
        .expect("insert episode");
        sqlx::query(
            "INSERT INTO media_files (episode_id, path, size_bytes, mtime_ms, scanned_at) \
             VALUES (?, ?, 1, ?, ?)",
        )
        .bind(ep_id)
        .bind(format!("/s/e{ep_no}.mkv"))
        .bind(mtime)
        .bind(SCAN)
        .execute(&pool)
        .await
        .expect("insert episode file");
    }

    // Execute the checked-in migration SQL against the populated DB —
    // same read-from-disk pattern as the phase 36 test above, so future
    // edits to the file are tested by these assertions.
    let phase109_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("migrations")
        .join("20260611000000_phase109_added_at_file_mtime.sql");
    let phase109_sql =
        std::fs::read_to_string(&phase109_path).expect("read phase 109 migration file");

    async fn snapshot(pool: &SqlitePool) -> (Vec<(i64, i64)>, Vec<(i64, i64)>) {
        let items: Vec<(i64, i64)> =
            sqlx::query_as("SELECT id, added_at FROM items ORDER BY id")
                .fetch_all(pool)
                .await
                .expect("read items");
        let eps: Vec<(i64, i64)> =
            sqlx::query_as("SELECT id, added_at FROM episodes ORDER BY id")
                .fetch_all(pool)
                .await
                .expect("read episodes");
        (items, eps)
    }

    sqlx::query(&phase109_sql)
        .execute(&pool)
        .await
        .expect("apply phase 109 backfill to populated rows");

    let (items, eps) = snapshot(&pool).await;
    assert_eq!(
        items,
        vec![
            (1, 500_000),  // old file wins
            (2, SCAN),     // strictly-newer file must not raise or move it
            (3, 400_000),  // soft-removed row is acquisition evidence
            (4, SCAN),     // mtime 0 ignored
            (5, SCAN),     // no files — untouched
            (10, 600_000), // show = MIN across episode files
        ],
        "items backfill invariants",
    );
    assert_eq!(
        eps,
        vec![(21, 700_000), (22, 600_000)],
        "episodes take their own file's mtime",
    );

    // Idempotency: a second run must be a no-op.
    sqlx::query(&phase109_sql)
        .execute(&pool)
        .await
        .expect("re-apply phase 109");
    let again = snapshot(&pool).await;
    assert_eq!(again, (items, eps), "second run must change nothing");
}
