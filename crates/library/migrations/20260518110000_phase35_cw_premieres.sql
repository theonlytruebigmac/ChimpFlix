-- Phase 35: include season premieres in Continue Watching.
--
-- Plex parity: when a user has watched (or is watching) any episode of
-- a show AND a new season's first episode exists they haven't started,
-- surface S(N+1)E01 in the Continue Watching rail. Without this, a
-- user who finished S2 of a show and just got S3 has no path back to
-- the show from CW until they click through Browse → Show → Season 3.
--
-- Setting is on by default — matches Plex's default behavior. Off
-- skips the premiere augmentation entirely (`on_deck` returns only
-- actively-watching items).

ALTER TABLE server_settings
    ADD COLUMN continue_watching_include_premieres INTEGER NOT NULL DEFAULT 1;
