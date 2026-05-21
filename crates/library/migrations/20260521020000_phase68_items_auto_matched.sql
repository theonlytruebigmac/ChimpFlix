-- Phase 68: track whether an item was auto-matched by the parser or
-- was created as an "unmatched stub" because the filename didn't fit
-- any known pattern.
--
-- Pre-this-phase the scanner silently dropped any video file whose
-- name didn't match the SxxExx / anime / movie-year regexes. An
-- operator with a 1584-file anime library (Sonarr-organized but
-- with release-name suffixes the parser didn't recognise) saw only
-- ~175 files land in `media_files` and had no idea where the rest
-- went. Worse, the dropped files were invisible everywhere — no
-- "fix this" affordance, just gone.
--
-- The new flow always inserts a stub Item + Episode/Movie row and
-- the matching media_file, with `auto_matched = 0`. The catalog
-- shows the file immediately ("link first, normalize later"); the
-- existing Fix Match dialog lets the operator correct the metadata
-- without having to rename files on disk first.
--
-- Backfill: existing rows are all auto-matched (the parser succeeded
-- for them; that's why they exist). Default 1 captures that.
ALTER TABLE items ADD COLUMN auto_matched INTEGER NOT NULL DEFAULT 1;

-- Partial index over the unmatched rows only — most installs will
-- have very few of them, so the index is tiny and lookups for the
-- "unmatched files" UI surface are O(1) on it instead of a full
-- table scan.
CREATE INDEX idx_items_auto_matched_false
    ON items(auto_matched) WHERE auto_matched = 0;
