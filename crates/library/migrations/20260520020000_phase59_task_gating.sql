-- Phase 59 — Task gating
--
-- Foundation for the scheduled-task rebuild. Two related changes:
--
-- 1. Per-feature gate settings on `server_settings`. Three pipeline kinds
--    move to opt-in (off by default):
--      - chapter_thumbs_enabled
--      - loudness_analysis_enabled
--      - subtitle_fetch_enabled
--    These gate BOTH the on-add discovery pipeline AND the safety-net
--    cron sweep. The corresponding handlers used to run unconditionally
--    on FileAdded; that was the bug fix described in
--    docs/pipelines/backend-plan.md §2.
--
--    Existing installs that want loudness / chapter thumbs to keep
--    running must flip the toggle in admin → tasks. Documented as
--    breaking-change in the phase notes.
--
-- 2. Two columns on `scheduled_tasks`:
--      - mode TEXT      (automatic | gated | periodic) — sourced from
--                       the in-code registry; informs the UI grouping.
--      - gate_setting_key TEXT — the `server_settings` boolean that
--                       gates this kind. Mirrors the registry so a row
--                       inspection alone tells you which switch flips
--                       the kind off without needing to read code.
--
--    Both columns are denormalized convenience for the admin UI; the
--    in-code registry remains the source of truth.

ALTER TABLE server_settings ADD COLUMN chapter_thumbs_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE server_settings ADD COLUMN loudness_analysis_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE server_settings ADD COLUMN subtitle_fetch_enabled INTEGER NOT NULL DEFAULT 0;

ALTER TABLE scheduled_tasks ADD COLUMN mode TEXT NOT NULL DEFAULT 'periodic';
ALTER TABLE scheduled_tasks ADD COLUMN gate_setting_key TEXT;

-- Backfill mode + gate_setting_key for kinds that ship in the binary.
-- New scheduled-task rows added by future operators / migrations should
-- set these explicitly; this UPDATE just catches the existing fleet.

UPDATE scheduled_tasks SET mode = 'automatic' WHERE kind IN (
    'detect_markers',
    'generate_previews'
);

UPDATE scheduled_tasks SET mode = 'gated', gate_setting_key = 'chapter_thumbs_enabled'
    WHERE kind = 'generate_chapter_thumbs';

UPDATE scheduled_tasks SET mode = 'gated', gate_setting_key = 'loudness_analysis_enabled'
    WHERE kind = 'analyze_loudness';

UPDATE scheduled_tasks SET mode = 'gated', gate_setting_key = 'subtitle_fetch_enabled'
    WHERE kind = 'fetch_subtitles';
