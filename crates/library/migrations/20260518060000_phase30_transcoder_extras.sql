-- Phase 30: rest of the Plex-parity transcoder settings.
--
-- Plex's Settings → Transcoder page exposes more knobs than we had
-- after phase 18/20. This migration adds the ones that map to code we
-- can actually wire today; toggles for features we don't ship yet
-- (HEVC output, throttle buffer, multi-GPU selection) are deferred
-- so we don't ship dead settings.
--
-- Added:
--   - `transcoder_background_preset` — x264 preset for the
--     background optimize-versions encoder. Previously hard-coded to
--     `veryfast`; surface it so operators can trade CPU time for
--     smaller optimized files.
--   - `transcoder_max_background_concurrent` — cap on how many
--     optimize_versions jobs the scheduler runs per tick. Default 1
--     keeps background work from starving live transcodes on a small
--     box.
--   - `transcoder_hdr_tonemap_enabled` — opt out of the HDR → SDR
--     tonemap. Off means HDR sources play in the SDR pipeline
--     without the libzimg/tonemap filter chain (washed-out look but
--     lower CPU). Default on.
--   - `transcoder_hdr_tonemap_algo` — algorithm string passed to the
--     `tonemap=tonemap=` argument. Default `hable` matches the
--     previous hard-coded value.

ALTER TABLE server_settings
    ADD COLUMN transcoder_background_preset TEXT NOT NULL DEFAULT 'veryfast';

ALTER TABLE server_settings
    ADD COLUMN transcoder_max_background_concurrent INTEGER NOT NULL DEFAULT 1;

ALTER TABLE server_settings
    ADD COLUMN transcoder_hdr_tonemap_enabled INTEGER NOT NULL DEFAULT 1;

ALTER TABLE server_settings
    ADD COLUMN transcoder_hdr_tonemap_algo TEXT NOT NULL DEFAULT 'hable';
