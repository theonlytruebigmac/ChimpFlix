-- Phase 78 — Per-library primary metadata source.
--
-- Before this migration the library's "agent chain" was stored as a
-- flat list of rows in `library_agents` with a priority column, and
-- the UI offered a drag-to-reorder interface to set Primary. That
-- proved confusing in practice (operators not realising Primary mode
-- overwrites where FillNulls cannot displace, leading to the
-- AniList-Japanese-titles incident) and overpowered for the actual
-- operator decision, which is simply "TMDB or TVDB?".
--
-- This phase adds an explicit per-library `primary_metadata_agent`
-- column. Allowed values: 'tmdb' or 'tvdb'. The rest of the chain
-- (TVMaze / AniList / OMDb) still runs after the primary in
-- FillNulls mode; their priorities in `library_agents` are interpreted
-- as fallback order only.
--
-- Defaults:
--   * Movies / Shows libraries → 'tmdb'
--   * Anime libraries          → 'tvdb' (English titles, TheTVDB has
--                                 best anime coverage among the agents
--                                 ChimpFlix supports)
--
-- The `library_agents` table is *kept* — it still tracks which agents
-- run for this library and in what fallback order. What's gone is the
-- "any agent can be primary" affordance; the dropdown gives you a
-- choice of two, the chain logic does the rest.

ALTER TABLE libraries
    ADD COLUMN primary_metadata_agent TEXT
    NOT NULL DEFAULT 'tmdb'
    CHECK (primary_metadata_agent IN ('tmdb', 'tvdb'));

-- Backfill anime libraries to TVDB primary. New anime libraries will
-- get this default automatically (see queries::create_library).
UPDATE libraries SET primary_metadata_agent = 'tvdb' WHERE kind = 'anime';
