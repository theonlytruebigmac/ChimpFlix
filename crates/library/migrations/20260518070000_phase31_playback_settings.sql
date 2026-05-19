-- Phase 31: playback / Continue Watching dials, plus a database cache
-- knob. Surfaces the values that were previously hard-coded constants
-- in `queries::on_deck`, the client-side scrobble threshold, and the
-- SQLite cache size — all things Plex exposes under Settings → Library
-- that an operator may want to tune on a busy / cold server.
--
-- Added:
--   - `continue_watching_max_items` — cap on the on-deck rail. Was
--     hard-coded 20; default 40 matches Plex's default.
--   - `continue_watching_max_age_weeks` — drop in-progress items
--     last touched more than N weeks ago. Stops months-stale entries
--     from sticking around forever. Default 16 weeks.
--   - `video_played_threshold_pct` — single source of truth for
--     "this counts as watched". Used by both the on-deck upper-
--     bound filter and the client-side auto-mark. Was split between
--     95% (on-deck) and 90% (client); unifying at 90% means the
--     tile disappears the moment we scrobble, so the user doesn't
--     see a phantom "just watched" entry.
--   - `database_cache_size_mb` — applied via `PRAGMA cache_size` at
--     pool open. Default 64 MB matches SQLite's modern recommended
--     baseline (the default is ~2 MB which is hostile on a big
--     library).

ALTER TABLE server_settings
    ADD COLUMN continue_watching_max_items INTEGER NOT NULL DEFAULT 40;

ALTER TABLE server_settings
    ADD COLUMN continue_watching_max_age_weeks INTEGER NOT NULL DEFAULT 16;

ALTER TABLE server_settings
    ADD COLUMN video_played_threshold_pct INTEGER NOT NULL DEFAULT 90;

ALTER TABLE server_settings
    ADD COLUMN database_cache_size_mb INTEGER NOT NULL DEFAULT 64;
