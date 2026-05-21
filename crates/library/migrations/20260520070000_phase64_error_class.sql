-- Phase 64 — Error classification on the job queue
--
-- Adds a denormalized `error_class` column to `jobs` so the worker
-- can record *why* a job failed (rate-limited external API, auth
-- problem, timeout, ffmpeg refused, etc.) without parsing the
-- free-text `last_error` field at read time. Drives:
--
--   - Per-class backoff curves in the worker (rate-limited jobs
--     wait much longer than transient network blips).
--   - The activity-screen "3 jobs rate-limited" grouping.
--
-- Backwards-compatible: NULL means the job failed before this
-- column was tracked or with an unclassified error. The worker
-- treats NULL as `transient` for the retry math.

ALTER TABLE jobs ADD COLUMN error_class TEXT;

-- Index for the dashboard query "give me the last N dead jobs of
-- class X". The composite (error_class, finished_at) means a
-- typical filter ("fetch_external_ratings, rate_limited, last 24h")
-- avoids a full scan.
CREATE INDEX idx_jobs_error_class ON jobs(error_class, finished_at DESC)
    WHERE error_class IS NOT NULL;
