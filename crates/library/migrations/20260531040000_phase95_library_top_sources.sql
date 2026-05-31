-- Phase 95: per-library, type-aware "Top 10" rails.
--
-- The home page already has a global "Top 10" rail backed by
-- `trending_cache` (TMDB weekly *trending*, source='tmdb', matched to
-- local items by tmdb_id). This phase adds a per-library "Top 10" rail
-- whose SOURCE depends on the library's kind:
--   Movies / Shows -> TMDB *top-rated* (source='tmdb_top_rated')
--   Anime          -> MyAnimeList ranking (source='mal_ranking', Phase 2)
-- blended with that library's local top-watched so sparse libraries
-- still fill ten slots.
--
-- The external ranked lists are GLOBAL per (source, media_kind) — the
-- TMDB top-rated movie list is the same for every movie library; only
-- the *intersection* with a given library differs, which the read query
-- (`list_library_top`) handles by scoping the JOIN to one library_id.
-- So we reuse `trending_cache` rather than duplicate a list per library.
--
-- New id columns: non-TMDB sources (MAL) can't be matched by tmdb_id.
-- MAL ranking resolves each entry through an anime-id mapping file to
-- whatever cross-ids it has, so the cache row can carry tvdb_id /
-- anilist_id / mal_id and the read query matches local items on ANY of
-- them (the same multi-id trick the Trakt history mirror uses). All
-- nullable; existing trending rows (source='tmdb') leave them NULL and
-- keep matching on tmdb_id exactly as before.

ALTER TABLE trending_cache ADD COLUMN tvdb_id INTEGER;
ALTER TABLE trending_cache ADD COLUMN anilist_id INTEGER;
ALTER TABLE trending_cache ADD COLUMN mal_id INTEGER;
