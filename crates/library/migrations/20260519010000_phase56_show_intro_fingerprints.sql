-- Phase 56 — per-show intro audio fingerprints.
--
-- Powers the chromaprint-based intro detector. Each row stores the
-- canonical fingerprint of a show's intro/theme song, captured from
-- a trusted source (typically the first episode after an operator
-- saves a manual intro marker on it). The detect_markers task uses
-- this fingerprint to anchor intro start/end on the show's other
-- episodes — much more reliable than blackdetect alone, which only
-- catches cold-open → theme fades.
--
-- One row per (show_id, season_id) — NULL season_id means a
-- show-wide fingerprint. We keep `season_id` for forward-compat:
-- many shows change theme music across seasons (e.g. anime OPs),
-- and per-season fingerprints capture that. The current capture
-- path writes show-wide rows; per-season is future polish.
--
-- The fingerprint BLOB is a packed little-endian array of u32 hashes
-- produced by chromaprint's Fingerprinter at ~124ms per u32 entry.
-- We also store the captured audio duration so the match path can
-- compute an end_ms relative to the matched start offset.

CREATE TABLE show_intro_fingerprints (
    id                        INTEGER PRIMARY KEY,
    show_id                   INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    season_id                 INTEGER          REFERENCES seasons(id) ON DELETE CASCADE,
    fingerprint               BLOB NOT NULL,
    -- Length of the source audio range the fingerprint was computed
    -- from, in milliseconds. The match path adds this to the matched
    -- start position to produce the resulting marker's end_ms.
    duration_ms               INTEGER NOT NULL,
    captured_from_media_file_id INTEGER REFERENCES media_files(id) ON DELETE SET NULL,
    captured_at               INTEGER NOT NULL,
    -- 'manual' when captured after an operator-saved intro marker;
    -- 'auto' if a future code path captures from a high-confidence
    -- auto-detection. We never overwrite a 'manual' row with an
    -- 'auto' one (operators own the truth).
    captured_by               TEXT NOT NULL
);

-- We allow at most one fingerprint per scope. Re-capturing replaces
-- the existing row via ON CONFLICT in the upsert query. The COALESCE
-- pattern handles NULL season_id correctly — without it, SQLite's
-- NULL-treats-as-distinct rule would let duplicate show-wide rows
-- accumulate.
CREATE UNIQUE INDEX idx_show_intro_fingerprints_scope
    ON show_intro_fingerprints(show_id, COALESCE(season_id, -1));
