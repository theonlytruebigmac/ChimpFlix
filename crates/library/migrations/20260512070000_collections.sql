-- Movie collections (franchises): "John Wick Collection", "Studio Ghibli",
-- "The Lord of the Rings", etc. TMDB exposes these via `belongs_to_collection`
-- on each movie. Plex's UI surfaces them as a row of related items in the
-- title modal plus a dedicated collection page — we'll match both.
--
-- One row per TMDB collection. Items reference their collection via
-- `items.collection_id`. Only movies belong to TMDB collections; shows do
-- not, so we leave the FK nullable.

CREATE TABLE collections (
    id              INTEGER PRIMARY KEY,
    tmdb_id         INTEGER NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    overview        TEXT,
    poster_path     TEXT,
    backdrop_path   TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX idx_collections_name ON collections(name COLLATE NOCASE);

ALTER TABLE items ADD COLUMN collection_id INTEGER
    REFERENCES collections(id) ON DELETE SET NULL;
CREATE INDEX idx_items_collection ON items(collection_id) WHERE collection_id IS NOT NULL;
