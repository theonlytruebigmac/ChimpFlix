-- Title-treatment logo art (transparent PNG of the title typography)
-- so the modal hero can render the title as art instead of plain text.
-- Sourced from TMDB's /movie/{id}/images and /tv/{id}/images endpoints
-- under `images.logos` and stored as a relative path the frontend
-- joins against the TMDB image base URL.
--
-- Nullable: items without a TMDB id or without an English logo just
-- continue to render the existing H1 fallback. The column is populated
-- by the normal enrichment flow (scan / refresh_metadata) as well as
-- a dedicated `refresh_logos` backfill task for already-imported items.

ALTER TABLE items ADD COLUMN logo_path TEXT;
