-- Phase 103: live progress + cancellation for Optimized Versions.
--
-- The admin Optimized-versions table previously showed only a static
-- status badge (queued | running | success | failed) with no progress
-- and no way to stop an in-flight (or backlogged) re-encode. This
-- migration adds the two columns that back the redesigned UI:
--
--   - `progress_permille` — re-encode progress in tenths of a percent
--     (0..=1000), so the UI can render a determinate bar without the
--     float-rounding fuzz of a 0..1 REAL. NULL while queued (and on
--     pre-existing rows, since the worker only stamps it once running)
--     so the UI falls back to an indeterminate "running" bar until the
--     first real measurement lands. The `optimize_versions` task fills
--     this from ffmpeg's `-progress` `out_time_ms` divided by the
--     source `media_files.duration_ms`, then 1000 on success.
--
--   - status gains a fifth value, `cancelled`. Set by the admin
--     POST /admin/versions/{id}/cancel route: a queued row is flipped
--     directly to `cancelled` (the worker's claim query only picks up
--     `queued`, so it's skipped); a running row is flipped to
--     `cancelled` AND its id is dropped into an in-memory cancel set
--     that the worker polls between progress reads so it can kill the
--     ffmpeg child and clean up the partial output file.
--
-- No CHECK constraint on status is added (the column never had one —
-- the set of valid values is enforced in application code), so this is
-- a pure additive ALTER and stays idempotent-friendly.

ALTER TABLE optimized_versions
    ADD COLUMN progress_permille INTEGER;
