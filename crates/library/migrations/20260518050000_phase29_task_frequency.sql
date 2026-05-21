-- Phase 29: friendly schedule model for background tasks.
--
-- The original cron-string-per-task surface was operator-hostile — most
-- admins shouldn't have to know what `0 30 2 * * 0` means. Plex models
-- the same problem as a *frequency* enum (hourly / daily / weekly / ...)
-- plus an optional *maintenance window* during which the heavy tasks
-- (full scans, deep media analysis) are allowed to run.
--
-- This migration adds:
--   - `scheduled_tasks.frequency`: the canonical schedule when it's
--     not `custom`. Values: manual | hourly | every_3_hours |
--     every_6_hours | every_12_hours | daily | every_3_days |
--     weekly | monthly | on_change | custom.
--   - `scheduled_tasks.requires_maintenance_window`: when true, the
--     computed `next_run_at` is snapped forward to the next opening
--     of the maintenance window. Used to keep heavy scans off
--     prime-time playback hours.
--   - `server_settings.maintenance_window_start` / `_end`: HH:MM
--     strings interpreted in server-local time. Defaults 02:00 → 09:00
--     match Plex's defaults.
--
-- `cron_expr` is kept for the `custom` mode and as a back-compat shim;
-- when `frequency != 'custom'`, the value is ignored by the scheduler
-- but preserved so toggling to/from custom doesn't lose the operator's
-- last hand-written expression.

ALTER TABLE scheduled_tasks
    ADD COLUMN frequency TEXT NOT NULL DEFAULT 'custom';

ALTER TABLE scheduled_tasks
    ADD COLUMN requires_maintenance_window INTEGER NOT NULL DEFAULT 0;

ALTER TABLE server_settings
    ADD COLUMN maintenance_window_start TEXT NOT NULL DEFAULT '02:00';

ALTER TABLE server_settings
    ADD COLUMN maintenance_window_end TEXT NOT NULL DEFAULT '09:00';

-- Backfill: infer frequency from the existing cron_expr for the
-- seed-default rows. Anything we can't recognize stays `custom` so the
-- operator's prior schedule is preserved verbatim.
--
-- Recognised seed patterns (`scheduler::seed_defaults`):
--   `0 0 * * * *`  → hourly       (prune_sessions)
--   `0 0 3 * * *`  → daily        (backup_db)
--   `0 0 4 * * *`  → daily        (refresh_trending)
--   `0 30 2 * * 0` → weekly       (verify_libraries)
--   `0 30 3 * * *` → daily        (purge_removed_files)
--   `0 30 4 * * *` → daily        (cleanup_audit_log)
UPDATE scheduled_tasks
   SET frequency = 'hourly'
 WHERE cron_expr = '0 0 * * * *';

UPDATE scheduled_tasks
   SET frequency = 'daily'
 WHERE cron_expr IN (
   '0 0 3 * * *',
   '0 0 4 * * *',
   '0 30 3 * * *',
   '0 30 4 * * *'
 );

UPDATE scheduled_tasks
   SET frequency = 'weekly'
 WHERE cron_expr = '0 30 2 * * 0';

-- Window-eligibility for the seeded heavy tasks. Anything that walks
-- the whole library or rewrites the DB should be deferred to the
-- maintenance window by default.
UPDATE scheduled_tasks
   SET requires_maintenance_window = 1
 WHERE kind IN (
   'backup_db',
   'verify_libraries',
   'purge_removed_files',
   'cleanup_audit_log',
   'scan_library',
   'refresh_metadata',
   'refresh_logos',
   'refresh_trending',
   'generate_previews',
   'fetch_subtitles',
   'optimize_versions'
 );
