-- Phase 93: track furthest-watched position separately from resume position.
--
-- `play_state.position_ms` is the *resume* point — the literal place the
-- user last stopped, so seeking backward and quitting resumes there. But
-- that makes it wrong for the progress bar: a backward seek shrinks the
-- red bar even though the user already watched further, and skipping
-- around made episodes look un-watched (the symptom reported on the
-- "I Parry Everything" season page).
--
-- `max_position_ms` records the FURTHEST point ever reached. The progress
-- bar and the "X min left" label read this (monotonic — never shrinks on a
-- backward seek), while resume keeps reading `position_ms`. Two columns,
-- two concerns:
--   position_ms      -> resume / seek-to-where-I-stopped
--   max_position_ms  -> progress bar / completion display
--
-- Backfill existing rows from position_ms so progress bars don't reset to
-- zero on upgrade. Watched rows already carry position_ms == duration_ms
-- (set_watched writes the full duration), so they backfill to a full bar.

ALTER TABLE play_state
    ADD COLUMN max_position_ms INTEGER NOT NULL DEFAULT 0;

UPDATE play_state SET max_position_ms = position_ms;
