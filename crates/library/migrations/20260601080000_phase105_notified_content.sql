-- Ledger of content already announced via the new-content notification
-- pipeline (KIND_NEW_MOVIE / KIND_NEW_EPISODE). Written ONLY by the
-- `notify_new_content` background job handler
-- (crates/server/src/jobs/handlers/notify_new_content.rs).
--
-- Purpose: de-dupe so a re-scan that re-persists an already-announced
-- movie or episode never re-notifies. The scan hot-path never reads or
-- writes this table — the scan only enqueues the job; the handler is the
-- single reader/writer.
--
-- `kind`     — 'content.new_movie' or 'content.new_episode' (matches the
--              notifier KIND_* discriminators; stored so the same item id
--              space can't collide across kinds, though in practice a
--              movie item id and a show item id are distinct rows anyway).
-- `ref_id`   — the announced content's stable id:
--                * new_movie   → items.id of the movie.
--                * new_episode → episodes.id of the episode.
--              We key episodes at the episode grain (not the show) so a
--              show's later seasons still announce, while a re-scan of an
--              already-announced episode is suppressed. The handler then
--              GROUPS the un-announced episodes per show for the actual
--              fan-out (one "N new episodes of <Show>" notification).
-- `library_id` — denormalized owning library, for cheap cleanup if a
--              library is deleted (FK cascade) and for diagnostics.
-- `notified_at` — when the announcement was recorded (epoch ms).
CREATE TABLE notified_content (
    kind         TEXT    NOT NULL,
    ref_id       INTEGER NOT NULL,
    library_id   INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    notified_at  INTEGER NOT NULL,
    PRIMARY KEY (kind, ref_id)
);

CREATE INDEX idx_notified_content_library
    ON notified_content(library_id);

-- Seed the ledger with ALL content that already exists at the moment this
-- feature is deployed, so the FIRST post-deploy scan of a pre-existing
-- (non-greenfield) library does NOT treat the entire back-catalogue as
-- "new" and blast every user with thousands of notifications. The two
-- SELECTs mirror the exact predicates of list_unannounced_movies /
-- list_unannounced_episodes (movie items / episodes with a live media_file)
-- so every currently-announceable row is pre-marked. Content added AFTER
-- this migration runs is absent here and so still notifies normally.
INSERT OR IGNORE INTO notified_content (kind, ref_id, library_id, notified_at)
SELECT 'content.new_movie', i.id, i.library_id, (CAST(strftime('%s','now') AS INTEGER) * 1000)
  FROM items i
 WHERE i.kind = 'movie'
   AND EXISTS (SELECT 1 FROM media_files mf WHERE mf.item_id = i.id AND mf.removed_at IS NULL);

INSERT OR IGNORE INTO notified_content (kind, ref_id, library_id, notified_at)
SELECT 'content.new_episode', e.id, sh.library_id, (CAST(strftime('%s','now') AS INTEGER) * 1000)
  FROM episodes e
  JOIN seasons s ON s.id = e.season_id
  JOIN items sh ON sh.id = s.show_id
 WHERE EXISTS (SELECT 1 FROM media_files mf WHERE mf.episode_id = e.id AND mf.removed_at IS NULL);
