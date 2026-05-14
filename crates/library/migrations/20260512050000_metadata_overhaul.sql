-- Metadata overhaul: per-field user overrides, richer people, extras, reviews.
--
-- This migration is the foundation for Fix Match, Edit Metadata, multi-source
-- metadata providers, and the richer item-detail UI (cast headshots with
-- character names, Extras rail with trailers/featurettes, user reviews).
--
-- Design notes:
--   * `locked_fields` stores a JSON array of field names that the user has
--     manually edited. The enrichment pipeline must skip these fields when
--     applying provider data. Empty string '[]' is the unlocked default.
--   * `people` gains bio/dates so we can render Plex-style person pages.
--     `imdb_id` lets us round-trip people across providers (TMDB ↔ Wikidata).
--   * `item_extras` stores trailers/featurettes/etc. Sources are typically
--     YouTube via TMDB's /videos endpoint, but we keep the schema source-
--     agnostic so we can later add local-file extras (Plex's `Extras` folder).
--   * `item_reviews` is for local users only — outside review aggregation
--     (Rotten Tomatoes / IMDb) would belong on the item row, not here.

-- ─── Field-lock overrides ───────────────────────────────────────────────────
ALTER TABLE items ADD COLUMN locked_fields TEXT NOT NULL DEFAULT '[]';
ALTER TABLE episodes ADD COLUMN locked_fields TEXT NOT NULL DEFAULT '[]';

-- ─── People: extended biographical fields ──────────────────────────────────
ALTER TABLE people ADD COLUMN imdb_id TEXT;
ALTER TABLE people ADD COLUMN biography TEXT;
ALTER TABLE people ADD COLUMN birthday INTEGER;        -- epoch ms
ALTER TABLE people ADD COLUMN deathday INTEGER;        -- epoch ms
ALTER TABLE people ADD COLUMN place_of_birth TEXT;
ALTER TABLE people ADD COLUMN known_for_department TEXT;

CREATE INDEX IF NOT EXISTS idx_people_imdb ON people(imdb_id) WHERE imdb_id IS NOT NULL;

-- Tag credits with their kind so the modal can split cast vs. crew without
-- hand-parsing the `role` string.
ALTER TABLE item_credits ADD COLUMN role_kind TEXT NOT NULL DEFAULT 'cast';
CREATE INDEX IF NOT EXISTS idx_credits_kind ON item_credits(item_id, role_kind, sort_order);

-- ─── Extras (trailers, featurettes, behind-the-scenes, clips, deleted) ─────
CREATE TABLE item_extras (
    id              INTEGER PRIMARY KEY,
    item_id         INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,        -- 'trailer' | 'teaser' | 'featurette' | 'behind_the_scenes' | 'clip' | 'deleted_scene'
    title           TEXT NOT NULL,
    source          TEXT NOT NULL,        -- 'youtube' | 'local'
    source_id       TEXT NOT NULL,        -- YouTube video id or local file path
    thumb_url       TEXT,
    duration_ms     INTEGER,
    published_at    INTEGER,              -- epoch ms; for sorting newest-first
    sort_order      INTEGER NOT NULL DEFAULT 0,
    UNIQUE(item_id, source, source_id)
);
CREATE INDEX idx_extras_item ON item_extras(item_id, sort_order);

-- ─── User reviews ──────────────────────────────────────────────────────────
CREATE TABLE item_reviews (
    id              INTEGER PRIMARY KEY,
    item_id         INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    rating          INTEGER,              -- 1-10 scale, NULL = no star rating
    body            TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE(item_id, user_id)              -- one review per user per item
);
CREATE INDEX idx_reviews_item ON item_reviews(item_id, created_at DESC);
CREATE INDEX idx_reviews_user ON item_reviews(user_id, created_at DESC);
