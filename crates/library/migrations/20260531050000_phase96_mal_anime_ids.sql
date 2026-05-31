-- Phase 96: MyAnimeList anime Top-10 support (id-mapping + items.mal_id).
--
-- MAL's ranking endpoint returns only MAL ids; local anime items are
-- matched on tvdb_id / tmdb_id / anilist_id (anime libraries default to
-- TVDB-primary, so tvdb_id is the usual hook). `anime_id_map` is a local
-- mirror of a community anime-id mapping file (mal <-> tvdb <-> tmdb <->
-- anilist), refreshed on a schedule. The MAL ranking refresh resolves
-- each mal_id through this map and writes the cross-ids into
-- `trending_cache` (source='mal_ranking'), which `list_library_top` then
-- matches to local items by ANY id.
--
-- `items.mal_id` is added for completeness + future Feature A (MAL list
-- import); the Top-10 rail itself works purely off the cache + map, so
-- this column is not load-bearing for Phase 2.

ALTER TABLE items ADD COLUMN mal_id INTEGER;
CREATE INDEX idx_items_mal_id ON items(mal_id) WHERE mal_id IS NOT NULL;

CREATE TABLE anime_id_map (
    mal_id      INTEGER PRIMARY KEY,
    anilist_id  INTEGER,
    tvdb_id     INTEGER,
    tmdb_id     INTEGER,
    updated_at  INTEGER NOT NULL
);
-- Reverse-lookup indexes: the ranking refresh resolves mal_id -> others
-- (PK covers that); these help any future "what's the mal_id for this
-- tvdb/anilist item" backfill.
CREATE INDEX idx_anime_id_map_tvdb ON anime_id_map(tvdb_id) WHERE tvdb_id IS NOT NULL;
CREATE INDEX idx_anime_id_map_anilist ON anime_id_map(anilist_id) WHERE anilist_id IS NOT NULL;
