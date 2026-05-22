-- Phase 73 — Episode numbering mode + absolute episode capture.
--
-- Anime libraries frequently use *absolute* episode numbering in
-- filenames ("[Subs] Show - 013.mkv") rather than season-relative
-- (S02E01). Today the parser puts every absolute-numbered file in
-- season 1 because there's no S/E tag to extract a season from, and
-- the episode_number column gets the absolute value (13 in the
-- example). Metadata agents that key off (season_number, episode_number)
-- then 404 on TMDB's S01E13 because S1 has 12 episodes, and the
-- episode row stays at the raw filename stem.
--
-- This migration prepares the schema to handle that case:
--
--   * `items.episode_numbering_mode` — 'season_relative' (default,
--     correct for live-action and S/E-tagged anime) or 'absolute'
--     (anime with bare-number filenames). The scanner sets it per
--     show once it has enough signal to know.
--
--   * `episodes.absolute_number` — the original on-disk number when
--     the file was absolute-numbered. Preserved alongside the
--     remapped (season_number, episode_number) so an absolute-aware
--     agent like AniList can look the episode up by its native
--     numbering, while season-relative agents (TMDB, TVDB, TVMaze)
--     get the remapped values.
--
-- No data backfill: existing rows default to 'season_relative' /
-- NULL. On the next scan of an anime library with absolute-numbered
-- files, the scanner will detect the mode and update; the metadata
-- enrichment pipeline will then re-resolve episodes against the
-- correct numbering.

ALTER TABLE items
    ADD COLUMN episode_numbering_mode TEXT
    NOT NULL DEFAULT 'season_relative'
    CHECK (episode_numbering_mode IN ('season_relative', 'absolute'));

ALTER TABLE episodes
    ADD COLUMN absolute_number INTEGER;

-- Partial index — most episode rows won't have an absolute_number
-- (live-action shows + S/E-tagged anime). Sparse index keeps lookups
-- by absolute number cheap without bloating the dense common case.
CREATE INDEX idx_episodes_absolute_number
    ON episodes(absolute_number)
    WHERE absolute_number IS NOT NULL;
