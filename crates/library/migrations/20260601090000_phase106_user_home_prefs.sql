-- Per-user home-page customization preferences (Feature: home customization).
--
-- home_rails_json: a sparse overlay describing the user's home rail layout,
-- stored as a JSON array of {"rail_id": "<id>", "enabled": <bool>} entries.
-- Overlay/sparse semantics: any rail NOT present in this array keeps its
-- default position + enabled state, so an empty array '[]' = stock home.
-- The stable rail-id catalogue is defined in
-- crates/library/src/models.rs::HOME_RAIL_CATALOGUE (derived from the rails
-- actually assembled in web/src/app/page.tsx). Validated as parseable JSON
-- at /auth/me; the array order is the user's desired rail order, and the
-- frontend (Feature 3) merges it over the default catalogue.
ALTER TABLE users ADD COLUMN home_rails_json TEXT NOT NULL DEFAULT '[]';

-- hide_watched_cw: when 1, additionally suppress fully-watched/completed
-- titles from the Continue Watching rail. The on-deck query already excludes
-- watched + >=90%-progress titles; this opt-in toggle hides anything the
-- user has explicitly marked watched even if it slipped through. Default 0
-- preserves the historical Continue-Watching behaviour for every row.
ALTER TABLE users ADD COLUMN hide_watched_cw INTEGER NOT NULL DEFAULT 0;

-- kids_safe: when 1, restrict home + main browse to titles whose
-- certification (items.rating_age) is in a kid-safe allow-set (G/PG/TV-Y/
-- TV-Y7/TV-G/TV-PG and equivalents); unrated/NULL titles are excluded to
-- fail safe. NOTE: items.rating_age is currently only populated via manual
-- metadata edits (no scanner/agent backfill yet), so live coverage is ~0% —
-- the toggle is persisted and the filter applies wherever cert data exists,
-- and becomes effective as ratings are populated. Default 0 = no filter.
ALTER TABLE users ADD COLUMN kids_safe INTEGER NOT NULL DEFAULT 0;
