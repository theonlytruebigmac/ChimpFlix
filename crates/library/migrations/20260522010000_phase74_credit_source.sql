-- Phase 74 — Per-credit source attribution.
--
-- Prep work for multi-agent cast/crew. Today only TMDB's
-- `enrich_credits_and_extras` writes to `item_credits`; other agents
-- (TVDB, OMDb, AniList) have `cast: false` in their capability matrix
-- so they never write credits. Once a second agent does (e.g. TVDB v4
-- characters via `/series/extended`), the apply layer needs to scope
-- DELETE by source so a TVDB pass doesn't wipe TMDB's rows out from
-- under it.
--
-- Existing rows are backfilled to `source = 'tmdb'` since that's where
-- they came from. New writes set the source explicitly via
-- `apply_item_credits_for_source` (see queries.rs).

ALTER TABLE item_credits
    ADD COLUMN source TEXT NOT NULL DEFAULT 'tmdb';

CREATE INDEX idx_credits_item_source ON item_credits(item_id, source);
