-- Phase 36 — manual (user-created) collections.
--
-- Existing `collections` table holds TMDB-franchise rows (kind = 'auto'),
-- discovered during scan via `belongs_to_collection`. Items linked via
-- `items.collection_id` (one item → at most one auto collection).
--
-- This phase adds user-curated collections (kind = 'manual'): admin-created
-- named groupings, many-to-many with items via a junction table.
--
-- Schema dance (SQLite textbook): rebuild collections to make tmdb_id
-- nullable and add new columns, preserving ids so items.collection_id
-- continues to resolve. `legacy_alter_table=ON` keeps the FK reference
-- text in items pointing at the literal name "collections" through the
-- rename — otherwise modern SQLite would helpfully rewrite refs and
-- defeat the dance.
--
-- FK enforcement must be OFF for the DROP step: with FK ON,
-- `DROP TABLE collections` would fire the ON DELETE SET NULL cascade on
-- items.collection_id and wipe every existing franchise link. `PRAGMA
-- foreign_keys` is a no-op inside a transaction and sqlx-sqlite wraps
-- migrations in one, so the toggle here is handled at the *connection*
-- level instead — see `db::open_with`, which runs migrations on a
-- dedicated pool with FK off before the app pool (FK on) is opened.

PRAGMA legacy_alter_table = ON;

CREATE TABLE collections_new (
    id                   INTEGER PRIMARY KEY,
    tmdb_id              INTEGER UNIQUE,                       -- NULL for manual
    kind                 TEXT NOT NULL DEFAULT 'auto'
                            CHECK (kind IN ('auto', 'manual')),
    name                 TEXT NOT NULL,
    sort_title           TEXT,                                  -- manual collections only
    overview             TEXT,
    description          TEXT,                                  -- manual collections only (admin-authored)
    poster_path          TEXT,
    backdrop_path        TEXT,
    created_by_user_id   INTEGER REFERENCES users(id) ON DELETE SET NULL,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

INSERT INTO collections_new (id, tmdb_id, kind, name, overview, poster_path, backdrop_path, created_at, updated_at)
SELECT id, tmdb_id, 'auto', name, overview, poster_path, backdrop_path, created_at, updated_at
FROM collections;

DROP TABLE collections;
ALTER TABLE collections_new RENAME TO collections;

CREATE INDEX idx_collections_name ON collections(name COLLATE NOCASE);
CREATE INDEX idx_collections_kind ON collections(kind);

-- Many-to-many junction for manual collections. Auto collections still
-- use `items.collection_id` (denormalized 1:N from TMDB scan). A given
-- item can be in any number of manual collections without affecting its
-- auto franchise membership.
CREATE TABLE collection_items (
    collection_id   INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    item_id         INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    added_at        INTEGER NOT NULL,
    PRIMARY KEY (collection_id, item_id)
);
CREATE INDEX idx_collection_items_order ON collection_items(collection_id, sort_order);
CREATE INDEX idx_collection_items_item ON collection_items(item_id);

PRAGMA legacy_alter_table = OFF;
