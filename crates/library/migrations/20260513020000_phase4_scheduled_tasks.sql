-- Phase 4: cron-driven scheduled tasks.
--
-- Two tables — `scheduled_tasks` is the user-editable schedule, `task_runs`
-- is an append-only history of executions used for the admin UI's
-- run history drawer and for diagnostics.
--
-- `kind` is the task class (one of a small enum baked into the server's
-- TaskRegistry); `params_json` carries kind-specific arguments (e.g.
-- `{"library_id": 7}` for a per-library scan). The runtime validates that
-- both fields make sense before scheduling.
--
-- We do not seed any default tasks in the migration; the server seeds them
-- on first run when the table is empty (see scheduler::seed_defaults), so
-- existing installations don't get surprised on upgrade.

CREATE TABLE scheduled_tasks (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    kind                TEXT NOT NULL,                -- 'scan_library' | 'refresh_metadata' | 'detect_markers' | 'prune_sessions' | 'backup_db' | 'optimize_versions'
    name                TEXT NOT NULL,
    cron_expr           TEXT NOT NULL,
    params_json         TEXT NOT NULL DEFAULT '{}',
    enabled             INTEGER NOT NULL DEFAULT 1,
    last_run_at         INTEGER,
    last_status         TEXT,                         -- 'success' | 'failed' | 'running'
    last_error          TEXT,
    last_duration_ms    INTEGER,
    next_run_at         INTEGER NOT NULL,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);
CREATE INDEX idx_scheduled_tasks_due ON scheduled_tasks(next_run_at) WHERE enabled = 1;

CREATE TABLE task_runs (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id         INTEGER NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE,
    started_at      INTEGER NOT NULL,
    finished_at     INTEGER,
    status          TEXT NOT NULL,                    -- 'running' | 'success' | 'failed'
    error           TEXT,
    log             TEXT
);
CREATE INDEX idx_task_runs_task ON task_runs(task_id, started_at DESC);
