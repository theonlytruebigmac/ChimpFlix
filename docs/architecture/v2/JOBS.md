# V2 Jobs — Background Work Subsystem

> Status: **planning draft.** Defines V2's background work model:
> scanner jobs, metadata refresh, transcoder-related precompute,
> scheduled maintenance, etc.

## What V1 got right

- **One queue, many kinds.** V1 has a single `jobs` table with a
  `kind` column dispatching to typed handlers. Carry forward.
- **Worker pool with operator-tunable size.** V1 has hot-reload via
  `WorkerPoolHandle::resize`. Carry forward.
- **Per-kind concurrency caps.** Some kinds (transcoder background
  precompute) want strict concurrency limits independent of pool size.
  V1 has `transcoder_max_background_concurrent`; V2 keeps the model
  but generalizes — every kind can have its own cap.
- **Retry with backoff.** V1 jobs retry on timeout/transient errors
  with exponential backoff. Carry forward.
- **Scan-job specialization.** Scan progress + state has its own
  table because operators care about it as a first-class object.
  Carry forward.
- **Pipeline-emitter wrapper.** V1's `wrap_emitter_for_pipeline`
  fans scanner FileAdded events into downstream discovery jobs.
  Carry forward as a clean module.
- **Scheduled tasks (cron-equivalent with maintenance window UX).**
  Plex-style frequency + window, not raw cron. Carry forward.
- **TaskMode::Gated.** Per-kind enable/disable that both scheduled
  task and immediate pipeline trigger respect. Carry forward.

## What V1 got wrong

1. **Job claims compete with everything else for the writer lock.**
   `claim_next_job` uses `BEGIN IMMEDIATE` and fails under contention.
   V1 logs `claim_next_job failed; backing off` and retries. V2's
   concurrent-write model makes this irrelevant at the engine level,
   but the *pattern* — every worker polling the DB for work — is
   still wasteful. V2 considers a notify-based queue.
2. **Job table grows unbounded.** Completed jobs accumulate. V1 has
   pagination on the activity feed but no retention policy. V2 ships
   with retention from day one.
3. **Pipeline wrapper is implicit.** The fact that FileAdded triggers
   N downstream jobs is hidden in a closure. V2 surfaces this as a
   declarative "on event, enqueue jobs" config.
4. **First-scan exclusivity is bolted on.** V1 has `LibraryScanGate`
   as a separate primitive that pauses workers + scheduler. V2 makes
   this a first-class concept: jobs declare their priority class
   (foreground / normal / bulk), and the scheduler honors the
   priority order without a special gate.

## V2 design

### Job lifecycle

```
   enqueue ─► pending ─► running ─► completed
                 │           │           │
                 │           └─► failed (retry/dead-letter)
                 └─► cancelled
```

Job rows in `jobs` table:

```sql
CREATE TABLE jobs (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,                -- JSON, kind-specific
    priority INTEGER NOT NULL DEFAULT 0,  -- 0 normal, 1 foreground, -1 bulk
    status TEXT NOT NULL,                 -- pending|running|completed|failed|cancelled
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    scheduled_at INTEGER NOT NULL,        -- earliest run time
    started_at INTEGER,
    finished_at INTEGER,
    created_at INTEGER NOT NULL,
    -- For per-kind concurrency control:
    concurrency_key TEXT,                 -- e.g. "transcoder_bg" or "scan:library_id=1"
    CHECK (status IN ('pending','running','completed','failed','cancelled'))
);

CREATE INDEX idx_jobs_claim ON jobs(status, priority DESC, scheduled_at)
    WHERE status = 'pending';
CREATE INDEX idx_jobs_concurrency ON jobs(concurrency_key, status)
    WHERE status = 'running';
```

`concurrency_key` is a string namespace. Any number of jobs sharing
the same key can be configured to cap at N concurrent runners:

```rust
let limits = ConcurrencyLimits::default()
    .with("transcoder_bg", 2)
    .with("metadata_refresh", 4)
    .with_per_kind("scan_library", 1);
```

Per-library scan exclusivity is just a concurrency_key of
`"scan:library_id=N"` with cap 1. The `LibraryScanGate` primitive
becomes a derived view, not a separate piece of code.

### Priority classes

- **+1 Foreground** — work a user is waiting on. E.g. subtitle
  pre-warm initiated by a play_start, on-demand metadata refresh
  triggered by an admin clicking "Re-match." Always preempts bulk.
- **0 Normal** — standard background work. Scheduled scans, metadata
  refreshes from scheduled tasks, periodic maintenance.
- **−1 Bulk** — heavy ingest. Watcher-triggered scans on first hit,
  bootstrap_season_refs, transcoder precompute. Yields to foreground
  pressure (scanner already participates via `SCANNER.md`; jobs
  inherit the same signal).

The claim query orders by `(priority DESC, scheduled_at ASC)`. A
foreground job sitting next to 100 bulk jobs runs first.

### Notify-based claim

V1's workers poll the DB. V2 considers:

- Workers wait on a `tokio::sync::Notify` plus a periodic poll
  fallback.
- Producers (e.g. scanner emitting FileAdded events) call
  `notify_one()` after enqueueing.
- Scheduler calls `notify_one()` when ticking off a scheduled task.

This dramatically reduces idle DB load. The poll fallback (every N
seconds) catches startup races and ensures eventual progress if the
notify is missed.

Whether this is a net win depends on Turso's claim semantics at
build time. V1's polling approach is fine for a small number of
workers; V2 with a clean slate can choose either. Decide in Phase 1.

### Event-driven pipeline

V1's `wrap_emitter_for_pipeline` fans scanner FileAdded events into
downstream jobs (probe, detect_markers, etc.). V2 makes this a
declarative config:

```rust
pub struct EventPipeline {
    pub on_file_added: Vec<JobKind>,
    pub on_show_added: Vec<JobKind>,
    pub on_season_added: Vec<JobKind>,
    // …
}
```

Operators can disable specific stages via the settings UI (V1's
`TaskMode::Gated` per-kind toggle). The pipeline reads the gate
config at enqueue time, not run time, so a disabled kind never
hits the queue.

### Scheduled tasks

V2's scheduled-task surface is the same Plex-style frequency +
maintenance window UX V1 settled on. The simple view (flat checkbox
list) is the default; advanced editor is behind a toggle. Carry
forward.

What V2 adds: scheduled tasks can target a priority class. "Verify
backups" runs at +1 foreground because operators want feedback;
"Refresh metadata for movies older than a year" runs at −1 bulk.

### Per-job progress + per-stage timing

V1 added live progress + per-stage timing in the perf-plan deferrals
session. V2 ships this from day one — every job gets a progress
channel + a stage-timing accumulator persisted at completion.

This unlocks the activity feed's "23/1090 (2%) — currently in
metadata" UX without bolting on extra infrastructure.

### Retention

Completed and cancelled jobs older than 7 days get pruned by a
maintenance task. Failed jobs that exhausted retries stay around for
30 days for forensics. Configurable. Defaults shippable.

### Dead-letter

Jobs that fail after `max_attempts` move to status='failed' and stay
there. Admin UI surfaces them with a "retry" affordance. No separate
dead_letter table — the status column is enough.

### Observability

- Per-kind histograms: time-in-queue, time-to-run, success rate.
- Per-concurrency-key gauges: current running count.
- Worker pool utilization gauge.
- Surfaced via Prometheus.

## Worker pool

Pool size is hot-reloadable (V1 `WorkerPoolHandle::resize`). V2 keeps
this. Default = `num_cpus`, capped at 16 (single-node server, no
need to schedule against more than that).

Workers are async tasks. Long-running jobs (scan, transcode) run
inside the task; ffmpeg/ffprobe subprocesses are still subprocesses
spawned via `tokio::process`.

Cancellation propagates via `CancellationToken` from worker → job
handler → subprocesses.

## Tasks-UI surface

Carry forward V1's consolidated `/admin/tasks` page. Simple view for
the 90% case, Advanced editor for the rest. Per-kind concurrency caps
exposed in advanced.

## Open questions

- **Notify vs. poll.** Depends on Turso claim semantics. Default to
  poll (simpler) unless measurement shows it's a real cost.
- **Should `scan_jobs` stay separate from `jobs`?** V1 keeps them
  separate because scan progress is a first-class operator concept.
  V2 leans the same way — splitting keeps the `jobs` table small
  and the scan-job UI focused. Confirm in Phase 1.
- **Cron-style schedules vs. frequency-window UX.** V1's UX is
  frequency + maintenance window; that's what operators want.
  Advanced editor uses a structured representation (interval +
  bounds), not raw cron strings. Keep the same model in V2.
- **Distributed jobs.** V2 is single-node; the `jobs` table doesn't
  need fencing tokens or lease renewals. If we ever multi-node,
  revisit then.

## Cut list

- **Raw cron string scheduling.** Frequency + window is the right UX.
  Raw cron in the advanced editor stays *behind* the structured form,
  not as the primary representation.
- **Per-job notification webhooks.** V1 has webhooks for events but
  not per-job. Operators who want per-job hooks can subscribe to the
  realtime stream. No separate webhook-per-job feature.
- **Priority preemption mid-run.** A running bulk job does not get
  killed when a foreground job arrives; foreground jobs use a
  separate concurrency-key with its own slot. Preemption is expensive
  and the foreground-pressure signal in the scanner already covers
  the worst case.
