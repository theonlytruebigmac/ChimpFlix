-- Phase 86 — `last_activities_seen_at` cursor on `user_trakt_tokens`
-- so the hourly pull can short-circuit when Trakt's `/sync/last_activities`
-- reports the user's data is unchanged since the previous sync. Stored
-- as the raw ISO-8601 string Trakt returns under the response's `all`
-- field; an equality comparison is enough (we never need to parse it
-- back into a timestamp). NULL means "never checked" which forces the
-- next sync to do the full pull.
ALTER TABLE user_trakt_tokens ADD COLUMN last_activities_seen_at TEXT;

-- Backfill: also seed a row for the daily collection-push task if the
-- install bootstrapped before this seed list included it. Same pattern
-- as phase85 for trakt_pull.
INSERT INTO scheduled_tasks (
    kind, name, cron_expr, frequency, requires_maintenance_window,
    params_json, enabled, next_run_at, created_at, updated_at
)
SELECT
    'trakt_collection_push',
    'Trakt: push collection ("I own this")',
    '0 0 5 * * *',
    'daily',
    1,
    '{}',
    1,
    (CAST(strftime('%s', 'now') AS INTEGER) * 1000) + 300000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    CAST(strftime('%s', 'now') AS INTEGER) * 1000
WHERE NOT EXISTS (
    SELECT 1 FROM scheduled_tasks WHERE kind = 'trakt_collection_push'
);
