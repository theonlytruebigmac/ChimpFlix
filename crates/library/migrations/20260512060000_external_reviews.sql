-- Pivot reviews from user-authored to read-only external (TMDB) reviews.
--
-- The first cut of `item_reviews` modelled local user reviews; per a UX
-- decision we don't want a "leave a review" composer in the app. The
-- reviews section now surfaces public reviews from the metadata provider
-- (TMDB to start). That changes the row shape:
--   * no user_id; authors are free-text names from the source
--   * source + source_id let us deduplicate across re-enrichments and
--     later mix providers (TMDB + Trakt + etc.)
--   * avatar_url stores the provider's poster for the author
--
-- The previous table is dropped outright because no reviews were ever
-- persisted in production — the table was created hours earlier in the
-- same dev session.

DROP TABLE IF EXISTS item_reviews;

CREATE TABLE item_reviews (
    id              INTEGER PRIMARY KEY,
    item_id         INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    source          TEXT NOT NULL,        -- 'tmdb'
    source_id       TEXT,                 -- provider review id; nullable for non-stable sources
    author          TEXT NOT NULL,
    author_url      TEXT,
    avatar_url      TEXT,
    rating          INTEGER,              -- 1-10 scale; NULL = author didn't rate
    body            TEXT,
    created_at      INTEGER NOT NULL,     -- epoch ms (provider's createdAt, not our ingest time)
    UNIQUE(item_id, source, source_id)
);
CREATE INDEX idx_reviews_item ON item_reviews(item_id, created_at DESC);
