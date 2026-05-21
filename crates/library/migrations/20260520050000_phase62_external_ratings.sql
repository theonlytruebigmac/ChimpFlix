-- Phase 62 — External ratings on items
--
-- Backs the new `fetch_external_ratings` per-item handler. We store
-- the full provider response as JSON in `ratings_json` so future
-- additions (Metacritic, Letterboxd, …) don't need new columns.
-- The watermark `ratings_updated_at` powers per-item idempotency:
-- the sweep skips items refreshed within the last 30 days.

ALTER TABLE items ADD COLUMN ratings_json TEXT;
ALTER TABLE items ADD COLUMN ratings_updated_at INTEGER;

ALTER TABLE server_settings ADD COLUMN external_ratings_enabled INTEGER NOT NULL DEFAULT 0;
