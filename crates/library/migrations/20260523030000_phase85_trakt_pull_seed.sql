-- Phase 85 — backfill the `trakt_pull` scheduled task row for any
-- install that bootstrapped before the seed list included it. Without
-- this row the hourly Trakt pull never runs, so a user who linked
-- Trakt after the initial deploy never sees their history sync (the
-- only sync that happens is the one-shot push the Link UI fires).
--
-- next_run_at: now + 5 minutes (epoch ms) so the next scheduler tick
-- picks it up quickly without overlapping any other startup work.
INSERT INTO scheduled_tasks (
    kind, name, cron_expr, frequency, requires_maintenance_window,
    params_json, enabled, next_run_at, created_at, updated_at
)
SELECT
    'trakt_pull',
    'Trakt: pull history + playback',
    '0 0 * * * *',
    'hourly',
    0,
    '{}',
    1,
    (CAST(strftime('%s', 'now') AS INTEGER) * 1000) + 300000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE NOT EXISTS (
    SELECT 1 FROM scheduled_tasks WHERE kind = 'trakt_pull'
);
