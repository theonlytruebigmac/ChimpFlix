-- Phase 44 — operator-selectable GPU device for transcode sessions.
--
-- Multi-GPU hosts can now pin transcoding to a specific card. The
-- value is one of:
--   * "auto" (default): existing behaviour — driver picks.
--   * "<index>": NVENC `-gpu <index>` (0 = first card).
--   * "/dev/dri/renderD<NNN>": VAAPI device override.
--
-- We don't try to enforce that the chosen value matches the active
-- hwaccel — operator picks both knobs and we honor them. Bad
-- combinations (e.g. picking renderD129 with NVENC) silently fall back
-- to the driver default at session-spawn time.

ALTER TABLE server_settings ADD COLUMN transcoder_gpu_device TEXT NOT NULL DEFAULT 'auto';
