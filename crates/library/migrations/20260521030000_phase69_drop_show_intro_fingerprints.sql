-- Phase 69: drop the legacy chromaprint `show_intro_fingerprints`
-- table.
--
-- The phase-56 system captured a chromaprint signature when an
-- operator saved a manual intro marker on E01 of a show. Nothing
-- ever read it for detection — that's tacet's job now via
-- `show_season_intro_refs` (constellation hashes, per-season).
-- The table accumulated rows but had no operational effect, and the
-- admin UI surface that listed them was dead too.
--
-- All callers (API endpoints, queries helpers, MarkerEditor's
-- "Fingerprint captured" badge, the `chimpflix_transcoder::fingerprint`
-- module that produced the blobs) were removed in the same commit
-- that ships this migration, so DROP is safe — no live code path
-- references the table or its data.
DROP INDEX IF EXISTS idx_show_intro_fingerprints_scope;
DROP TABLE IF EXISTS show_intro_fingerprints;
