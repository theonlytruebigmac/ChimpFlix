-- Phase 41 — smart collections (rule-based, dynamically evaluated).
--
-- Third `kind` for the `collections` table: 'smart'. Members aren't
-- stored anywhere — they're computed at read time from `rule_json`
-- (a small DSL: field + op + value, AND/OR-composed).
--
-- The DSL is translated into a parameterised SQL WHERE clause inside
-- `list_items_in_collection`; no raw user input is ever interpolated
-- into the query string, only the literal field/op tokens we
-- whitelist on the server side.
--
-- Same rebuild dance as phase 36 — see that file's preamble for the
-- `legacy_alter_table=ON` rationale and the connection-level FK-off
-- handling in `db::open_with`.

PRAGMA legacy_alter_table = ON;

-- Drop the CHECK constraint by rebuilding (same dance as phase 36).
CREATE TABLE collections_new (
    id                   INTEGER PRIMARY KEY,
    tmdb_id              INTEGER UNIQUE,
    kind                 TEXT NOT NULL DEFAULT 'auto'
                            CHECK (kind IN ('auto', 'manual', 'smart')),
    name                 TEXT NOT NULL,
    sort_title           TEXT,
    overview             TEXT,
    description          TEXT,
    poster_path          TEXT,
    backdrop_path        TEXT,
    created_by_user_id   INTEGER REFERENCES users(id) ON DELETE SET NULL,
    rule_json            TEXT,
    created_at           INTEGER NOT NULL,
    updated_at           INTEGER NOT NULL
);

INSERT INTO collections_new (id, tmdb_id, kind, name, sort_title, overview,
                             description, poster_path, backdrop_path,
                             created_by_user_id, created_at, updated_at)
SELECT id, tmdb_id, kind, name, sort_title, overview, description,
       poster_path, backdrop_path, created_by_user_id, created_at, updated_at
FROM collections;

DROP TABLE collections;
ALTER TABLE collections_new RENAME TO collections;

CREATE INDEX idx_collections_name ON collections(name COLLATE NOCASE);
CREATE INDEX idx_collections_kind ON collections(kind);

PRAGMA legacy_alter_table = OFF;
