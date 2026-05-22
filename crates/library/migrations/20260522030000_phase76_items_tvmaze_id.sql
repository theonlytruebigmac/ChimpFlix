-- Phase 76 — Surface TVMaze id on items.
--
-- The metadata-agent framework's `ShowData.tvmaze_id` field (and the
-- matching column in `apply_show_data`) expects an `items.tvmaze_id`
-- column that the original schema never added. Every show-level write
-- from `TvMazeAgent::fetch_show` hit `no such column: tvmaze_id` and
-- failed the entire refresh — including TVDB / AniList writes
-- bundled in the same transaction.
--
-- Mirrors the shape of `anilist_id` (phase 13c): nullable INTEGER,
-- partial index for non-null values.

ALTER TABLE items ADD COLUMN tvmaze_id INTEGER;

CREATE INDEX idx_items_tvmaze ON items(tvmaze_id) WHERE tvmaze_id IS NOT NULL;
