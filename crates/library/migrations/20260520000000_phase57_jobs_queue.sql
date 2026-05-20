-- Phase 57: durable job queue.
--
-- The pre-existing pattern for background work was `tokio::spawn`
-- inside the request handler. That works for fire-and-forget, but if
-- the server restarts mid-run the in-flight work vanishes — and for
-- batches that run ffmpeg per file (detect_markers, generate_previews)
-- that means re-doing whatever was already partially done on the
-- next manual trigger, since there's no progress checkpoint.
--
-- This table backs a real queue. Handlers (`crates/server/src/jobs/`)
-- register per `kind`, the worker pool polls atomically via the
-- claim query, runs the handler, and updates status. Crash recovery
-- is `reclaim_orphan_jobs` on startup — any row left as `running`
-- whose `locked_at` is older than the lease ttl gets bumped back to
-- `queued` so the next worker picks it up.

CREATE TABLE jobs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    kind          TEXT    NOT NULL,
    payload       TEXT    NOT NULL,              -- JSON-encoded
    status        TEXT    NOT NULL DEFAULT 'queued',
                                                 -- queued | running | succeeded
                                                 --   | failed (retry pending)
                                                 --   | dead (max attempts exhausted)
    priority      INTEGER NOT NULL DEFAULT 0,    -- higher runs first
    attempts      INTEGER NOT NULL DEFAULT 0,
    max_attempts  INTEGER NOT NULL DEFAULT 3,
    run_after     INTEGER NOT NULL DEFAULT 0,    -- epoch ms; row eligible when now >= run_after
    locked_at     INTEGER,                       -- epoch ms when the current worker claimed it
    last_error    TEXT,
    created_at    INTEGER NOT NULL,
    started_at    INTEGER,
    finished_at   INTEGER
);

-- Claim index. The worker query is
--   SELECT … WHERE status = 'queued' AND run_after <= ?
--   ORDER BY priority DESC, id ASC LIMIT 1
-- so the index leads with (status, run_after) for the filter and
-- includes (priority DESC, id) for the order. SQLite can use this
-- without a temp B-tree sort.
CREATE INDEX idx_jobs_claim ON jobs(status, run_after, priority DESC, id);

-- Admin filters by kind + status (e.g. "show me all failed
-- detect_markers jobs").
CREATE INDEX idx_jobs_kind_status ON jobs(kind, status);

-- For listing the most recent jobs in admin UI without a temp sort.
CREATE INDEX idx_jobs_created_at ON jobs(created_at DESC);
