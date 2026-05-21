-- Phase 55 — indexes for the periodic-background-task "needs work" probes.
--
-- The loudness analyser and chapter-thumb generator both run on a fixed
-- schedule and ask SQLite "find me up to N files that still need
-- processing, joined into items/episodes for library scoping". The
-- columns those queries filter on were added by phase 38 / phase 39
-- but never indexed, so each tick was a full table scan of media_files
-- (10k+ rows on a mature library). During the scheduled tick that
-- would steal disk reads from any live playback session.
--
-- These are PARTIAL indexes — the predicate matches the WHERE clauses
-- in `list_media_files_needing_loudness` and
-- `list_media_files_needing_chapter_thumbs`, so the index holds only
-- the rows we ever ask about. Once a file's been processed it falls
-- out of the index, which keeps it tiny on a steady-state library.
-- The columns indexed (`item_id`, `episode_id`) are the join keys the
-- queries use to scope by library.

CREATE INDEX IF NOT EXISTS idx_media_files_loudness_pending
    ON media_files (item_id, episode_id)
    WHERE loudnorm_analyzed_at IS NULL
      AND removed_at IS NULL
      AND duration_ms IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_media_files_chapter_thumbs_pending
    ON media_files (item_id, episode_id)
    WHERE chapter_thumbs_generated_at IS NULL
      AND removed_at IS NULL;
