-- Cached "global trending" list for the Top 10 rail. Populated by the
-- `refresh_trending` scheduled task from TMDB (and later Trakt as a
-- fallback). The homepage joins this against `items` by tmdb_id to
-- produce a Top 10 row of titles that are *in* the local library AND
-- trending globally — so the result is empty until the library covers
-- some of what the world is currently watching.

CREATE TABLE trending_cache (
    id              INTEGER PRIMARY KEY,
    source          TEXT NOT NULL,        -- 'tmdb' | 'trakt'
    media_kind      TEXT NOT NULL,        -- 'movie' | 'show'
    rank            INTEGER NOT NULL,     -- 1-based, lower = trending higher
    tmdb_id         INTEGER NOT NULL,
    title           TEXT,                 -- fallback display when we don't have it locally yet
    poster_path     TEXT,                 -- TMDB-relative poster path
    fetched_at      INTEGER NOT NULL,     -- epoch ms
    UNIQUE(source, media_kind, rank)
);

CREATE INDEX idx_trending_cache_tmdb
    ON trending_cache(media_kind, tmdb_id);
