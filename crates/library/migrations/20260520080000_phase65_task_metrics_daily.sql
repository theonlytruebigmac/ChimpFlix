-- Phase 65 — Daily metrics rollup for the admin detail screen
--
-- One row per (UTC midnight day, kind). Populated by the nightly
-- `rollup_task_metrics` scheduled task which aggregates the
-- previous day's finished jobs from `jobs`. Backs the 30-day
-- chart on the per-task detail page; reading 30 rows × N kinds is
-- sub-millisecond compared to the alternative of bucketing the
-- jobs table on the fly per render.
--
-- Why a separate table vs. live-querying jobs:
--   - The cleanup_jobs sweep trims `jobs` (succeeded 7d, dead 30d
--     by default), so historical rollups depend on a parallel
--     store that survives the trim.
--   - The detail page renders fast even on instances with millions
--     of historical jobs because the aggregation cost is paid
--     once per day, not per page-load.

CREATE TABLE task_kind_metrics_daily (
    day               INTEGER NOT NULL,
    kind              TEXT NOT NULL,
    success_count     INTEGER NOT NULL DEFAULT 0,
    failure_count     INTEGER NOT NULL DEFAULT 0,
    p50_duration_ms   INTEGER,
    p95_duration_ms   INTEGER,
    targets_processed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, kind)
);

CREATE INDEX idx_task_metrics_daily_kind ON task_kind_metrics_daily(kind, day DESC);
