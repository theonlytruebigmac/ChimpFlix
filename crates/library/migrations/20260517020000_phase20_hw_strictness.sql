-- Phase 20: hardware-acceleration strictness setting.
--
-- Operator picks one of three modes:
--
--   * 'auto'        — current behavior. Hardware where possible, software
--                     fallback per stage (decode / filter / encode). Mixed
--                     pipelines (e.g. SW decode + HW encode) are allowed.
--
--   * 'prefer_hw'   — same effective behavior as auto BUT we log a warn
--                     to the admin Logs page every time a session falls
--                     back to software for any stage. Useful for tracking
--                     down "my GPU isn't being used" hunches.
--
--   * 'require_hw'  — sessions that can't run end-to-end on hardware are
--                     refused. The player gets a clean 409 with the
--                     specific reason ("source codec AV1 isn't decodable
--                     on your GPU"). Operators who want guaranteed-low
--                     CPU usage pick this; trades broader compatibility
--                     for predictable resource usage.
--
-- Default 'auto' preserves existing behavior on upgrade.

ALTER TABLE server_settings
    ADD COLUMN transcoder_hw_strictness TEXT NOT NULL DEFAULT 'auto';
