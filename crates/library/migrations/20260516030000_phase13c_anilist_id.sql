-- Phase 13c: AniList cross-reference on items.
--
-- Stored alongside tmdb_id / imdb_id / tvdb_id so anime items can carry
-- a stable AniList identifier for re-enrichment and to power AniList
-- deep-links from the UI.

ALTER TABLE items ADD COLUMN anilist_id INTEGER;
CREATE INDEX idx_items_anilist ON items(anilist_id) WHERE anilist_id IS NOT NULL;
