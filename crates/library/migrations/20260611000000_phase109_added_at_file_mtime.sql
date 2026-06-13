-- Phase 109: make "date added" reflect file acquisition time.
--
-- items.added_at / episodes.added_at used to be stamped with the
-- scanner's wall clock at first insert. For a library ingested in one
-- bulk scan that collapses every row to the same instant, so the
-- Recently Added sort degenerates into directory-walk order. The
-- scanner now seeds added_at from the backing file's mtime on first
-- insert; this backfill heals rows created before that change by
-- pulling each one back to the earliest mtime of its surviving files.
--
-- added_at only ever DECREASES here: a row moves iff at least one
-- file's mtime predates the stored value (so MIN(mtime) is guaranteed
-- below it, and bogus future mtimes can't drag anything forward).
-- Removed file rows count too — a soft-deleted row is still evidence
-- the item was in the collection at that mtime; the common case is a
-- quality upgrade where the original (old-mtime) file got replaced and
-- only the new file is live. Rows without files — placeholders,
-- agent-created shells — are untouched, as are mtime_ms = 0 rows
-- (stat() failed).
--
-- Known one-time side effects, accepted deliberately:
--   * Smart-collection rules filtering on added_at with absolute
--     epoch-ms thresholds (smart_rule.rs `added_at` arm) re-evaluate
--     dynamically, so their membership changes the moment this runs.
--   * Episodes promoted from placeholders BEFORE the upsert_episode
--     promotion fix keep the placeholder's creation wall-clock, which
--     is EARLIER than any file mtime — the lower-only guard cannot
--     raise them, and raising on mtime > added_at would corrupt
--     legitimately old episodes whose mtimes were bumped (touch/copy).
--     The class shrinks to zero going forward.
--   * Rails that use added_at as a tie-break (Top 10, library top
--     sources) reorder their low-signal tail once.

-- Movies (files attach directly to the item).
UPDATE items
SET added_at = (
    SELECT MIN(mf.mtime_ms) FROM media_files mf
    WHERE mf.item_id = items.id
      AND mf.mtime_ms > 0
)
WHERE EXISTS (
    SELECT 1 FROM media_files mf
    WHERE mf.item_id = items.id
      AND mf.mtime_ms > 0
      AND mf.mtime_ms < items.added_at
);

-- Episodes.
UPDATE episodes
SET added_at = (
    SELECT MIN(mf.mtime_ms) FROM media_files mf
    WHERE mf.episode_id = episodes.id
      AND mf.mtime_ms > 0
)
WHERE EXISTS (
    SELECT 1 FROM media_files mf
    WHERE mf.episode_id = episodes.id
      AND mf.mtime_ms > 0
      AND mf.mtime_ms < episodes.added_at
);

-- Shows: earliest file mtime across every episode of the show.
UPDATE items
SET added_at = (
    SELECT MIN(mf.mtime_ms)
    FROM media_files mf
    JOIN episodes e ON mf.episode_id = e.id
    JOIN seasons s ON e.season_id = s.id
    WHERE s.show_id = items.id
      AND mf.mtime_ms > 0
)
WHERE items.kind = 'show'
  AND EXISTS (
    SELECT 1
    FROM media_files mf
    JOIN episodes e ON mf.episode_id = e.id
    JOIN seasons s ON e.season_id = s.id
    WHERE s.show_id = items.id
      AND mf.mtime_ms > 0
      AND mf.mtime_ms < items.added_at
);
