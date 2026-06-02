-- Tri-state library access (Feature: none / view / full).
--
-- Until now access was BINARY: a row in `library_access` (or a group grant
-- via `access_group_libraries`) meant "allowed" (full browse + playback);
-- the absence of any grant meant "no access" (library + items hidden).
--
-- This migration introduces an explicit per-grant level so an operator can
-- grant a user the ability to BROWSE a library's metadata without being
-- able to PLAY (start a stream/transcode) from it. The three tiers are:
--
--   * none — no grant at all (the absence of a row; HIDDEN). There is no
--            stored 'none' level — "none" is simply the lack of a grant.
--   * view — can browse/see the library + item metadata, but CANNOT play.
--   * full — can browse AND play (the prior binary "allowed" behaviour).
--
-- Every EXISTING grant row predates the level concept and represented the
-- prior "allowed" = full behaviour, so the column defaults to 'full'. This
-- means every current user keeps both browse + playback with no regression:
-- the backfill from 20260512040000 inserted a `library_access` row per
-- (user, library) for everyone, and each of those rows is now 'full'.
--
-- When a user has MULTIPLE grants for one library (a direct row plus one or
-- more group grants), effective access is the HIGHEST level: full > view.
-- That resolution lives in queries::user_effective_access_level (and the
-- browse filter unions both view + full). Owners always implicitly have
-- full and never appear in these tables.
--
-- The column is constrained to the two stored levels so a malformed write
-- can't smuggle in an unknown level that the resolver would treat as 'none'
-- and silently hide a library.

ALTER TABLE library_access
    ADD COLUMN access_level TEXT NOT NULL DEFAULT 'full'
    CHECK (access_level IN ('view', 'full'));

ALTER TABLE access_group_libraries
    ADD COLUMN access_level TEXT NOT NULL DEFAULT 'full'
    CHECK (access_level IN ('view', 'full'));
