-- Phase 58 — operator-configurable background job worker count.
--
-- Previously hardcoded to 2 in `jobs::start`. Two workers + per-kind
-- limit of 1 for the ffmpeg-heavy kinds meant a backlog of newly-added
-- files could pin both workers on a single file's detect_markers +
-- generate_preview_sprite passes for many minutes, with no headroom for
-- chapter_thumbs / loudness to make progress concurrently. The
-- operator-controllable knob lets a beefier host run more parallel
-- decoders; the default stays 2 so existing installs see no behaviour
-- change until they opt in.
--
-- Read at startup; updating it surfaces the standard "restart pending"
-- badge because workers are spawned once and not re-spun on the fly.
-- Bounded to [1, 16] to keep an honest typo from forking dozens of
-- ffmpeg children.

ALTER TABLE server_settings
    ADD COLUMN job_workers INTEGER NOT NULL DEFAULT 2
        CHECK (job_workers >= 1 AND job_workers <= 16);
