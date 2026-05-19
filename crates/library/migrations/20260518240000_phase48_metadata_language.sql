-- Phase 48 — operator-configurable metadata language.
--
-- TMDB returns localised text (overview, tagline, even some titles)
-- based on the `language` query parameter, falling back to the
-- original language when no translation exists. Until now the
-- TmdbClient hard-coded `en-US`, which meant an admin who'd rather
-- see Spanish (or Japanese for an anime-heavy library) had no way to
-- override that without forking the binary.
--
-- The value is a BCP-47 tag (`en-US`, `ja-JP`, `de-DE`, etc.). We
-- bound the length so a typo can't silently encode a megabyte of
-- garbage into the query string. The change is read at startup —
-- updating it surfaces a "restart pending" badge in the admin UI
-- because the TmdbClient is a long-lived process-wide singleton.

ALTER TABLE server_settings
    ADD COLUMN metadata_language TEXT NOT NULL DEFAULT 'en-US'
        CHECK (length(metadata_language) > 0 AND length(metadata_language) <= 12);
