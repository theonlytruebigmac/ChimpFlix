-- Phase 61 — Embedded subtitle extraction
--
-- New per-file column + a separate gate setting from the external
-- subtitle fetch. Operators may want embedded extract (cheap, no
-- network) but not external fetch (rate-limited, OpenSubtitles
-- account required), or vice versa. Two gates lets them.
--
-- The handler `extract_embedded_subs` stamps the column on success;
-- a sweep filters on `embedded_subs_extracted_at IS NULL`. The
-- scanner does NOT clear this on file updates — if a user re-encodes
-- a source to add tracks, they'd run "Process all pending" or wait
-- for the next sweep tick.

ALTER TABLE media_files ADD COLUMN embedded_subs_extracted_at INTEGER;
ALTER TABLE server_settings ADD COLUMN embedded_subs_extract_enabled INTEGER NOT NULL DEFAULT 0;
