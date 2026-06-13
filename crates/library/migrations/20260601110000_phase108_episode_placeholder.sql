-- Phase 108: placeholder-episode marker.
--
-- The scanner now materializes PLACEHOLDER `episodes` rows for every
-- episode the metadata agent knows about, even ones with no downloaded
-- file yet (in-progress / future seasons). A placeholder is an episode
-- row with NO `media_files` row — informational only (it makes a season
-- complete so the finale flag is correct and the calendar has air dates)
-- but never "content you have".
--
-- Problem this column solves: the orphan-cleanup purge
-- (`purge_removed_media_files` / `..._for_library`) deletes
-- `episodes WHERE NOT EXISTS (media_files)` to reap rows left behind when
-- a file is removed. Placeholders match that predicate by construction
-- (they intentionally have no file), so without a way to tell the two
-- apart the purge would wipe every placeholder on each "empty trash"
-- run — undoing the feature. This flag is that discriminator:
--
--   * `is_placeholder = 1` — agent-materialized, never had a file. The
--     purge MUST keep it (it's not an orphan, it's a roadmap row).
--   * `is_placeholder = 0` — a real episode. When its files are removed
--     and the grace window lapses, the purge reaps it as a true orphan,
--     exactly as before.
--
-- `upsert_episode_placeholder` sets it to 1 on insert (and leaves it
-- untouched on conflict so a real row never silently becomes a
-- placeholder). The file-backed `upsert_episode` clears it to 0 on every
-- write, so the instant a file arrives for a placeholder slot the row is
-- promoted to a real episode and re-enters the normal orphan lifecycle.

ALTER TABLE episodes
    ADD COLUMN is_placeholder INTEGER NOT NULL DEFAULT 0;
    -- 1 = agent-materialized placeholder (no file, keep through purge).
    -- 0 = real episode (default; reaped as an orphan when fileless).

-- Partial index: the purge's "keep placeholders" carve-out and any
-- placeholder-only maintenance only ever scan the small set of flagged
-- rows, not the whole episodes table.
CREATE INDEX idx_episodes_is_placeholder
    ON episodes(is_placeholder)
    WHERE is_placeholder = 1;
