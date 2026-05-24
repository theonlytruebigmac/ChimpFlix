-- Phase 89 — server-synced per-user subtitle styling.
--
-- Pre-phase: subtitle appearance lived in two parallel localStorage layers
-- on the client (`cf_prefs_v1.subtitle*` and `chimpflix:subtitle:appearance`)
-- with different field names + units. Settings changed in one place didn't
-- show up in the other, and nothing followed users across devices.
--
-- This phase puts the canonical six-field model on the `users` row so the
-- player and the settings page consume from the same source, the change
-- syncs across devices, and operator-burned ASS subtitles match what the
-- browser renders via ::cue. All columns are nullable — NULL means
-- "use client defaults" so a freshly-created user isn't locked into a
-- specific look chosen at signup.
--
-- Validation of the enum-shaped columns (subtitle_font_family,
-- subtitle_edge) is intentionally server-side rather than via CHECK
-- constraints — ALTER TABLE ADD COLUMN with CHECK against existing rows
-- needs the legacy_alter_table dance and the enum lists are short enough
-- that handler-level validation is clearer.

ALTER TABLE users ADD COLUMN subtitle_font_size_px       INTEGER;
ALTER TABLE users ADD COLUMN subtitle_text_color         TEXT;
ALTER TABLE users ADD COLUMN subtitle_background_color   TEXT;
ALTER TABLE users ADD COLUMN subtitle_font_family        TEXT;
ALTER TABLE users ADD COLUMN subtitle_edge               TEXT;
ALTER TABLE users ADD COLUMN subtitle_bottom_inset_pct   INTEGER;
