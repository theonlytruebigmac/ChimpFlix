-- Phase 60 — Item extras (filesystem discovery)
--
-- Adds the per-item watermark columns the new detect_extras_item
-- handler uses to dedup filesystem walks. Reuses the existing
-- `item_extras` table from the metadata overhaul migration —
-- locally-discovered extras land there as `source = 'local'` rows
-- alongside the TMDB-fetched YouTube ones, so the UI sees a single
-- list per item.

ALTER TABLE items ADD COLUMN extras_scanned_at INTEGER;

ALTER TABLE items ADD COLUMN extras_dir_mtime INTEGER;
