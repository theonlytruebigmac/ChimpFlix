-- Speed up `has_active_files_clause()` from list_items WHERE.
--
-- That clause is now on every items.list query (shipped 2026-05-23 with
-- the scanner removal-reconciliation work — see
-- crates/library/src/queries.rs `has_active_files_clause`). It evaluates
-- `EXISTS (SELECT 1 FROM media_files mf ... WHERE COALESCE(s.show_id,
-- mf.item_id) = i.id AND mf.removed_at IS NULL)` for every candidate
-- row in the COUNT path.
--
-- Without partial coverage, the existing `idx_media_files_item` and
-- `idx_media_files_episode` indexes still match every row regardless of
-- `removed_at`, so the EXISTS subquery has to read the row from the
-- table to apply the `removed_at IS NULL` predicate. Over a 10k-item
-- library this surfaces as a 1-2s delay on the home-page empty-Home
-- probe — the user sees a blank hero zone until the COUNT resolves.
--
-- Partial indexes on `WHERE removed_at IS NULL` cover the hot path
-- (active files only) at a tiny storage cost. The CHECK constraint on
-- media_files enforces exactly one of (item_id, episode_id) is NOT
-- NULL, so the IS NOT NULL filter on the indexed column is essentially
-- free.

CREATE INDEX IF NOT EXISTS idx_media_files_item_active
    ON media_files(item_id)
    WHERE removed_at IS NULL AND item_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_media_files_episode_active
    ON media_files(episode_id)
    WHERE removed_at IS NULL AND episode_id IS NOT NULL;
