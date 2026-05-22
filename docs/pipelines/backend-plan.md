# Scheduled-task backend rebuild — game plan

Companion to the [pipeline diagrams](index.html) and [frontend mockup](tasks-ui.html). This is the implementation plan, not a design exploration — every section maps to code we'll change.

## North-star constraints

The user's three watchwords: **fast, accurate, efficient**. Concretely:

- **Fast.** A 10k-file initial scan must dispatch its job fan-out in under 10s. Job pickup latency under 500ms. Admin pages render in under 300ms.
- **Accurate.** One source of truth for gating. Idempotency keys per kind so retries are no-ops. Safety-net sweeps converge on the same state the on-add path would have produced.
- **Efficient.** File watcher idle CPU at 0%. Per-kind concurrency caps so loudness analysis doesn't starve marker detection. Live metrics in memory, not via heavy SQL.

## Non-goals (drawing the box)

- No per-library gate overrides yet. Gates are server-wide per kind. *(Confirmed in the frontend mockup planning.)*
- No realtime websockets — 5s polling on the activity screen is enough.
- No external job runner (Sidekiq / Celery shape). The existing in-process queue stays.
- No backwards-compat shims for the current `scheduled_tasks` shape — we own this end-to-end.

---

## 1. Task-kind registry (the new core abstraction)

Today, "kinds" are stringly-typed: scheduler dispatches by matching `kind` against a giant `match` in [scheduler/mod.rs](../../crates/server/src/scheduler/mod.rs). Handlers register themselves in [jobs/handlers/mod.rs](../../crates/server/src/jobs/handlers/mod.rs) the same way. Adding a kind means editing both files.

Replace with a compile-time registry: one trait, one entry per kind, registered via `inventory` or a `Lazy<HashMap>`.

```rust
// crates/server/src/tasks/kind.rs (new)
pub enum TaskMode { Automatic, Gated, Periodic }
pub enum TaskScope { PerFile, PerItem, Global }

#[async_trait]
pub trait TaskKind: Send + Sync + 'static {
    const NAME: &'static str;
    const MODE: TaskMode;
    const SCOPE: TaskScope;

    /// Setting key that gates this kind. None for Automatic kinds.
    fn gate_setting_key(&self) -> Option<&'static str> { None }

    /// External services / paths this kind requires (TMDB, ffmpeg, OpenSubtitles creds, …)
    fn dependencies(&self) -> Vec<Dependency> { vec![] }

    /// Hard cap on simultaneous workers for this kind.
    fn concurrency(&self) -> u32 { 1 }

    /// True iff the work is already done for this target — checked at enqueue + execute time.
    async fn already_done(&self, ctx: &Ctx, target: TargetId) -> Result<bool>;

    /// Enqueue from FileAdded / ItemMatched events. May skip via already_done().
    async fn dispatch_from_event(&self, ctx: &Ctx, evt: PipelineEvent) -> Result<()>;

    /// Enqueue from the safety-net cron. Returns N jobs queued.
    async fn dispatch_from_sweep(&self, ctx: &Ctx, batch_size: u32) -> Result<usize>;

    /// The actual job body.
    async fn execute(&self, ctx: &Ctx, job: &JobRow) -> Result<JobOutcome>;
}
```

Each existing handler ([detect_markers_file.rs](../../crates/server/src/jobs/handlers/detect_markers_file.rs), [analyze_loudness.rs](../../crates/server/src/jobs/handlers/analyze_loudness.rs), …) wraps in a struct that impls `TaskKind`. The scheduler and worker both drive everything through the trait — no more giant matches.

**Why a trait, not just config rows?** Handlers are code, dependencies are code, idempotency checks are code. Pretending the registry is data-driven creates the illusion of runtime extensibility we don't actually have.

---

## 2. Gate enforcement — the correctness fix

This is the [open memory item](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_pipeline_scheduled_task_gating.md): today [enqueue_pipeline()](../../crates/server/src/jobs/pipeline.rs) fans out all 4 per-file kinds on every `FileAdded` regardless of `scheduled_tasks.enabled`. Only the cron sweep respects the toggle.

After the rebuild, both paths funnel through one function:

```rust
// crates/server/src/tasks/gates.rs (new)
pub fn is_kind_allowed(ctx: &Ctx, kind: &str) -> GateState {
    let k = TASK_KINDS.get(kind).expect("registered kind");

    // Automatic kinds: only blocked by missing dependencies
    if matches!(k.mode(), TaskMode::Automatic) {
        return check_deps(ctx, k.dependencies());
    }

    // Gated kinds: setting key + deps
    let key = k.gate_setting_key().expect("gated kind must have gate key");
    if !ctx.settings.get_bool(key).unwrap_or(false) {
        return GateState::DisabledByAdmin;
    }
    check_deps(ctx, k.dependencies())
}

pub enum GateState {
    Allowed,
    DisabledByAdmin,
    MissingDependency(String),
}
```

`enqueue_pipeline()` and the scheduler tick both call `is_kind_allowed()`. A gate flip is instantly honored by both.

**Caching.** Settings reads are hot — calling `settings.get_bool()` once per `FileAdded` is fine but a 10k-file scan calling it 50k times is wasteful. Cache gate state in a `RwLock<HashMap<&str, GateState>>` invalidated by settings writes.

---

## 3. Data model changes

Keep [scheduled_tasks](../../crates/library/migrations/20260513020000_phase4_scheduled_tasks.sql) for sweep schedules — it's well-shaped already (cron + freq + window + backoff). Add columns + a metrics rollup table.

```sql
-- Migration: phase60_task_gating
ALTER TABLE scheduled_tasks ADD COLUMN gate_setting_key TEXT;
ALTER TABLE scheduled_tasks ADD COLUMN mode TEXT NOT NULL DEFAULT 'periodic';
-- backfill from registry on startup; values: 'automatic' | 'gated' | 'periodic'

-- Failure categorization
ALTER TABLE job_queue ADD COLUMN error_class TEXT;
-- values: external_rate_limit | external_auth | timeout | transient | permanent

-- Daily metrics rollup (backs the 30-day chart in the detail screen)
CREATE TABLE task_kind_metrics_daily (
    day INTEGER NOT NULL,           -- unix epoch midnight UTC
    kind TEXT NOT NULL,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    p50_duration_ms INTEGER,
    p95_duration_ms INTEGER,
    targets_processed INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (day, kind)
);
CREATE INDEX idx_task_kind_metrics_daily_kind ON task_kind_metrics_daily(kind, day DESC);

-- Per-item extras (for detect_extras_item)
CREATE TABLE item_extras (
    id INTEGER PRIMARY KEY,
    item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,             -- trailer | featurette | bts | deleted_scene
    path TEXT NOT NULL,
    duration_ms INTEGER,
    thumb_path TEXT,
    discovered_at INTEGER NOT NULL,
    UNIQUE (item_id, path)
);

-- External ratings on items (for fetch_external_ratings)
ALTER TABLE items ADD COLUMN ratings_json TEXT;        -- {"omdb_imdb": 8.4, "rt_critics": 92, "mpaa": "PG-13"}
ALTER TABLE items ADD COLUMN ratings_updated_at INTEGER;
```

Remember [the SQLite FK gotcha](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_sqlite_migration_fk_gotcha.md) — `PRAGMA foreign_keys` toggles must happen at the pool level via the two-pool startup, not inside migration bodies.

---

## 4. Pipeline restructure — Automatic vs Gated handlers

### 4.1 New handlers

| Handler | Mode | Scope | Where |
|---|---|---|---|
| `detect_extras_item` | Automatic | per-item | new — [crates/server/src/jobs/handlers/detect_extras_item.rs](../../crates/server/src/jobs/handlers/detect_extras_item.rs) |
| `extract_embedded_subs` | Gated | per-file | new — [crates/server/src/jobs/handlers/extract_embedded_subs.rs](../../crates/server/src/jobs/handlers/extract_embedded_subs.rs) |
| `fetch_external_ratings` | Gated | per-item | new — [crates/server/src/jobs/handlers/fetch_external_ratings.rs](../../crates/server/src/jobs/handlers/fetch_external_ratings.rs) |
| `refresh_logos_item` | Automatic | per-item | extracted from [scheduler/mod.rs:1578](../../crates/server/src/scheduler/mod.rs) |
| `fetch_subtitles_item` | Gated | per-item | already exists — extend to be pipeline-enqueueable |

### 4.2 Per-handler notes

**`detect_extras_item`** — walks the item's directory for `*-trailer.*`, `Extras/`, `Featurettes/`, `Behind The Scenes/`, `Deleted Scenes/`. Coalesces by `extras_dir_mtime` so a Sonarr drop of 12 extras doesn't fire 12 jobs. Output: rows in `item_extras`. Idempotency: skip if `extras_dir_mtime` hasn't changed since last run.

**`extract_embedded_subs`** — `ffprobe` for subtitle streams; for each `(stream_index, language)` pair, run `ffmpeg -map 0:s:N -c:s webvtt` → `.{lang}.vtt` sidecar. PGS bitmap subs need OCR — punt to a per-track concurrency of 1 with explicit 300s timeout. Idempotency: skip languages already present as `.vtt` next to source. Plays nicely with `fetch_subtitles_item` — extract first (free), fetch external only if no embedded track for the requested language.

**`fetch_external_ratings`** — OMDb (IMDb mirror, free tier 1000/day), Rotten Tomatoes, MPAA via TMDB releases endpoint. Backoff curve for 429: `5min → 15min → 1hr → 6hr → 24hr` then mark target as backoff-shelved. 401 → mark the dependency as broken on the kind so the UI shows "credentials invalid" instead of "task failing". Idempotency: skip if `ratings_updated_at > now - 30d`.

**`refresh_logos_item`** — extract per-item logic from the current sweep loop. Keep the weekly sweep as a safety-net that enqueues per-item jobs. Today the sweep is monolithic; splitting lets the on-add path produce logos immediately for new items.

**`fetch_subtitles_item`** — already a per-item handler in [scheduler/mod.rs:1704](../../crates/server/src/scheduler/mod.rs). Audit it for direct pipeline enqueueability (today it's only reachable from sweep dispatch). Should compose with `extract_embedded_subs` so we don't fetch externally for languages already extracted.

### 4.3 tacet integration

User's `tacet` crate at `/mnt/github/tacet`. Three options:

| Option | How | Pros | Cons |
|---|---|---|---|
| **A** workspace dep | move `tacet` into `crates/tacet/` | single repo · CI trivial | tacet can't easily be reused for non-ChimpFlix projects |
| **B** git dep | `tacet = { git = "…", branch = "main" }` | tacet stays standalone · clean ownership | CI needs network · pins to commit |
| **C** path dep | `tacet = { path = "../../tacet" }` | simplest locally | broken outside the user's machine |

**Recommendation:** B for prod, C during the dev cycle until tacet's API stabilizes. The handler change is identical either way:

```rust
// crates/server/src/jobs/handlers/detect_markers_file.rs
async fn execute(&self, ctx: &Ctx, job: &JobRow) -> Result<JobOutcome> {
    let file = queries::load_media_file(&ctx.db, job.target_id).await?;
    let cfg = tacet::AnalysisConfig::from_settings(&ctx.settings)?;
    let markers = tacet::detect_intro_credits(&file.path, &cfg).await?;
    queries::replace_auto_markers(&ctx.db, file.id, markers.into()).await?;
    Ok(JobOutcome::Success { targets: 1 })
}
```

**Open question:** confirm tacet's public API surface before Phase 4. Aligning `tacet::MarkerSet` with our [`auto_markers`](../../crates/library/src/queries.rs) row shape needs one short spec.

---

## 5. Job queue improvements

### 5.1 Per-kind concurrency

Today, the worker pool reads from `job_queue` without per-kind awareness; concurrency is hardcoded inside handlers. Move concurrency caps to the registry, enforce with semaphores in the dispatch loop:

```rust
// crates/server/src/jobs/worker.rs
let kind_semaphores: HashMap<&str, Arc<Semaphore>> = TASK_KINDS
    .iter()
    .map(|(name, k)| (*name, Arc::new(Semaphore::new(k.concurrency() as usize))))
    .collect();

loop {
    let job = next_job(&db).await?;
    let sem = kind_semaphores[&job.kind].clone();
    let permit = sem.acquire_owned().await?;
    tokio::spawn(async move {
        let _permit = permit;  // released on drop
        execute_with_metrics(job).await;
    });
}
```

Recommended caps per kind:

| Kind | Cap | Why |
|---|--:|---|
| `detect_markers_file` | 1 | CPU + ffmpeg blackdetect, eventually tacet |
| `analyze_loudness` | 1 | ffmpeg loudnorm |
| `extract_embedded_subs` | 1 | PGS OCR is the worst case |
| `refresh_logos_item` | 4 | network-bound, TMDB |
| `fetch_subtitles_item` | 4 | network-bound, OpenSubtitles |
| `fetch_external_ratings` | 2 | network-bound, OMDb rate limits dominate |
| `detect_extras_item` | 2 | filesystem walk + ffprobe |

All caps tunable via `server_settings` (e.g. `task_concurrency.analyze_loudness = 1`); registry defaults are floors.

### 5.2 Failure categorization

Add an `error_class` column to `job_queue`. The execute path classifies on failure:

```rust
match err {
    e if e.is_http_status(429) => ErrorClass::ExternalRateLimit,
    e if e.is_http_status_in(&[401, 403]) => ErrorClass::ExternalAuth,
    e if e.is_timeout() => ErrorClass::Timeout,
    e if e.is_transient() => ErrorClass::Transient,
    _ => ErrorClass::Permanent,
}
```

Backoff per class:

| Class | Curve | Max attempts |
|---|---|--:|
| `ExternalRateLimit` | 5m → 15m → 1h → 6h → 24h | 5 |
| `ExternalAuth` | — (dead-letter; flag dependency) | 1 |
| `Timeout` | 30s → 2m → 5m | 3 |
| `Transient` | 5s → 30s → 2m → 10m → 30m | 5 |
| `Permanent` | — (dead-letter immediately) | 1 |

The activity screen groups failures by class — "3 jobs rate-limited" reads cleaner than "3 unrelated failures."

### 5.3 Idempotency

Today most handlers check via a column on the target row (`loudnorm_analyzed_at`, `markers_detected_at`). Formalize that as the `already_done()` method on each kind. Called twice:

1. **At enqueue time** — `enqueue_pipeline()` skips inserting if already done. Cheap rejection of duplicate work.
2. **At execute time** — handler bails before doing real work in case state changed between enqueue and pickup.

Per-kind idempotency keys:

| Kind | Key |
|---|---|
| `detect_markers_file` | `media_files.markers_detected_at IS NOT NULL` |
| `analyze_loudness` | `media_files.loudnorm_analyzed_at IS NOT NULL` |
| `extract_embedded_subs` | per-(file, lang) — sidecar `.vtt` exists |
| `refresh_logos_item` | `items.logo_url IS NOT NULL OR logo_attempted_at > now-7d` |
| `fetch_subtitles_item` | per-(item, lang) — sidecar exists |
| `fetch_external_ratings` | `items.ratings_updated_at > now - 30d` |
| `detect_extras_item` | `items.extras_scanned_mtime = current dir mtime` |

### 5.4 Batched dispatch

`enqueue_pipeline()` today does separate INSERTs per kind per file. A 10k-file scan → 50k inserts serialized.

Fix at the scanner layer:

```rust
// scanner emits ScanEvents in batches now, not one at a time
while let Some(batch) = scan_event_rx.recv_many(1000).await {
    enqueue_pipeline_batch(&ctx, &batch).await?;
}

// enqueue_pipeline_batch does one tx per batch, one multi-row insert per kind
async fn enqueue_pipeline_batch(ctx: &Ctx, events: &[ScanEvent]) -> Result<()> {
    let mut tx = ctx.db.begin().await?;
    for kind in DISCOVERY_KINDS {
        if !gates::is_kind_allowed(ctx, kind).is_allowed() { continue; }
        let rows = events.iter()
            .filter(|e| !kind.already_done(&ctx, e.target).await.unwrap_or(false))
            .collect::<Vec<_>>();
        sqlx::query("INSERT INTO job_queue (kind, target_id, payload, run_at) VALUES " /* multi-row */)
            .bind_batch(rows)
            .execute(&mut tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
```

Target: 10k files dispatched in <1s of DB time (10 batches × 100ms each).

---

## 6. File watcher hardening

Already always-on via [file_watcher.rs:225](../../crates/server/src/file_watcher.rs). Three changes:

### 6.1 Debounce
Sonarr / Radarr writes are noisy — they write to `.partial`, then rename, then sometimes touch mtime again. Coalesce events with the same final path within a 2-second window. Use a `HashMap<PathBuf, Timer>` + tokio sleep.

### 6.2 Hot-reload `scan_automatically`
Today read once at startup ([memory: restart req'd](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/feedback_preserve_frontend.md)). Wrap in `tokio::sync::watch::channel`; settings-write path broadcasts the new value; the watcher's event-handling loop checks it on every event. Admin toggles take effect instantly without restart.

### 6.3 Per-item coalescing
When Sonarr drops 24 episodes for a season in one burst, per-item kinds (logos, ratings, extras) should fire **once per item**, not 24×. Implement: bucket events by `item_id` (resolved post-scanner) in a 5s window; emit one per-item dispatch per bucket. Per-file kinds (markers, loudness) still fire per file.

---

## 7. Metrics & observability

### 7.1 Live counters (in-memory)

```rust
// crates/server/src/tasks/metrics.rs (new)
pub struct LiveMetrics {
    in_flight: DashMap<&'static str, AtomicU32>,
    queued:    DashMap<&'static str, AtomicU32>,
    // Last N run results per kind, lock-free ring
    recent:    DashMap<&'static str, RingBuffer<RunResult>>,
}
```

Worker increments `in_flight` on pickup, decrements on completion. `queued` updated by enqueue + dequeue. `recent` writes on completion. All reads are atomic; no SQL hit on the activity screen.

Reset on restart — that's fine, the screen is "what's happening now."

### 7.2 Historical rollup

A nightly task (`rollup_task_metrics`) reads `job_queue` for the previous day, computes p50/p95/throughput per kind, writes one row per (day, kind) to `task_kind_metrics_daily`. The detail screen's 30-day chart reads 30 rows × N kinds — sub-millisecond.

### 7.3 Dependency health checks

Each `TaskKind::dependencies()` returns a list of probes:

```rust
fn dependencies(&self) -> Vec<Dependency> {
    vec![
        Dependency::Binary("ffmpeg", &["-filter:a", "loudnorm", "-version"]),
        Dependency::WritableDir("<data>/analytics/loudness/"),
    ]
}
```

Probes run every 60s on a background heartbeat task; results cached in `LiveMetrics`. The detail screen's "Gate dependencies" panel reads from the cache.

---

## 8. API surface — keyed to UI screens

Each UI screen maps to a single endpoint family:

| Screen | Endpoint | Notes |
|---|---|---|
| **Overview** hero | `GET /api/admin/tasks/summary` | running, queued, succeeded_24h, failed_24h, next_window_at |
| **Overview** list | `GET /api/admin/tasks` | grouped by section (media/watch/system); kinds + schedule + live stats |
| **Overview** toggle | `PATCH /api/admin/tasks/{kind}/gate` | flip gate_enabled |
| **Activity** | `GET /api/admin/tasks/activity` | per-kind health + recent runs + failed jobs |
| **Activity** stream | `GET /api/admin/tasks/activity/stream` | SSE (defer to Phase 7 — polling works first) |
| **Detail** | `GET /api/admin/tasks/{kind}` | TaskKindDetail with stats + recent runs + 30d history |
| **Detail** save | `PATCH /api/admin/tasks/{kind}` | frequency, batch_size, concurrency, timeout |
| **Detail** force-run | `POST /api/admin/tasks/{kind}/run` | enqueues sweep immediately |
| **Activity** retry | `POST /api/admin/jobs/{id}/retry` | re-enqueue one failed job |
| **Activity** retry-all | `POST /api/admin/jobs/retry-failed?kind=…` | re-enqueue all failed of a kind |
| **Flow** | `GET /api/admin/tasks/pipeline` | nodes + edges + live counts (computed server-side for one network round-trip) |

Response shape for the overview list:

```jsonc
{
  "groups": [
    {
      "name": "Media ingest pipeline",
      "sections": [
        {
          "label": "Automatic",
          "kinds": [
            {
              "name": "detect_markers_file",
              "mode": "automatic",
              "scope": "per_file",
              "gate": { "enabled": true, "locked": true /* automatic, can't disable */ },
              "schedule": { "frequency": "weekly", "next_at": 1716696000, "window": "02:00-09:00" },
              "live": { "in_flight": 3, "queued": 214, "last_run_at": 1716595200, "last_status": "ok" },
              "deps": [{ "name": "ffmpeg", "ok": true }, { "name": "tacet", "ok": true }]
            }
          ]
        },
        { "label": "Gated", "kinds": [/* ... */] }
      ]
    },
    { "name": "Watch state & housekeeping", "sections": [/* ... */] },
    { "name": "System tasks",              "sections": [/* ... */] }
  ]
}
```

Designed to be one fetch → render. No N+1 per kind.

---

## 9. Performance budget

| Operation | Target | Mechanism |
|---|--:|---|
| FileAdded → all jobs queued | **<10ms** | Batched INSERT in caller's batch; in-memory gate cache; no fanout-per-row tx |
| 10k-file scan dispatched | **<10s** | 10 batches × 1k events, one tx each |
| Scheduler tick | **<100ms** | Single SELECT for due tasks; pre-cached gate state |
| Worker pickup latency | **<500ms** | Short SELECT poll on `job_queue` w/ index on `(state, run_at, kind)`; LISTEN/NOTIFY later if needed |
| Overview screen load | **<200ms** | All counters in-memory; one SQL hit for schedule rows |
| Activity screen load | **<200ms** | All counters in-memory; recent_runs from ring buffer |
| Detail screen load | **<300ms** | One SELECT schedule, one SELECT 30 daily rows, one SELECT last 20 runs |
| File watcher idle | **0% CPU** | inotify; debounce uses a single tokio timer per active path |

Indexes to add:

```sql
CREATE INDEX idx_job_queue_pickup ON job_queue(state, run_at, kind) WHERE state IN ('queued','failed_retry');
CREATE INDEX idx_job_queue_kind_completed ON job_queue(kind, completed_at DESC) WHERE state IN ('succeeded','failed_dead');
```

---

## 10. Migration phases

Each phase ships independently. No phase requires the next.

### Phase 1 — Registry + gate enforcement *(foundation)*
- [ ] `TaskKind` trait + registry
- [ ] Migrate all existing handlers behind the trait
- [ ] `gates::is_kind_allowed()` consulted by both `enqueue_pipeline()` and scheduler tick
- [ ] Migration: add `gate_setting_key` + `mode` columns; backfill from registry
- [ ] Integration test: toggling each gate stops both entry points

### Phase 2 — Handler refactors *(no behaviour change)*
- [ ] Extract `refresh_logos_item` from scheduler sweep into a handler; sweep becomes "enqueue per-item jobs"
- [ ] Make `fetch_subtitles_item` enqueueable from pipeline (currently sweep-only)
- [ ] Per-handler integration tests for both entry points

### Phase 3 — New handlers
- [ ] `detect_extras_item` + `item_extras` table
- [ ] `extract_embedded_subs` + sidecar idempotency
- [ ] `fetch_external_ratings` + OMDb client + 429 backoff
- [ ] Per-handler integration tests with fixtures

### Phase 4 — tacet integration
- [ ] Confirm tacet API surface (separate spec)
- [ ] Add tacet dep (Option C locally, Option B in CI)
- [ ] Replace `detect_markers_file` body
- [ ] Migration: clear `markers_detected_at` so safety-net re-processes everything once
- [ ] Snapshot test against current marker output on a fixture set

### Phase 5 — Job queue & file watcher hardening
- [ ] Per-kind semaphores in worker pool
- [ ] `error_class` column + classification + per-class backoff
- [ ] Batched dispatch in scanner
- [ ] File watcher debounce + hot-reload `scan_automatically`
- [ ] Per-item coalescing window
- [ ] Load test: 10k-file scan asserts <10s budget

### Phase 6 — Metrics infra
- [ ] `LiveMetrics` in-memory layer
- [ ] `task_kind_metrics_daily` rollup + nightly flush
- [ ] Dependency-health heartbeat task
- [ ] Counter-accuracy test under concurrent workers

### Phase 7 — API + UI
- [ ] New endpoints (one fetch per screen)
- [ ] React screens from `tasks-ui.html` mockup
- [ ] Polling at 5s for activity (defer SSE)
- [ ] Contract tests for endpoint shapes

Rough estimate: ~14-23 days of focused work. Each phase deployable on its own; can pause between any two.

---

## 11. Testing strategy

The non-obvious bits worth explicit tests:

- **Gate enforcement.** For each kind, an integration test that flips the gate off and verifies *both* `enqueue_pipeline()` and the scheduler sweep skip dispatching. The whole point of the rebuild — has to be locked down.
- **Idempotency under race.** Property test: enqueue the same target N times across M workers; assert `execute()` runs at most once.
- **Failure categorization.** Parametric test feeding each error class through the worker; assert correct backoff curve + dead-letter behaviour.
- **Live-metrics accuracy.** Under sustained worker load, in-memory counters should match `SELECT count(*)` from `job_queue` (within 1s drift).
- **Batched dispatch performance.** CI load test: 10k FileAdded events; assert p95 dispatch latency < 1ms per event.
- **tacet output stability.** Snapshot the marker output from a fixture file set; alert on regression.
- **File watcher debounce.** Simulate Sonarr write-rename-touch sequence; assert exactly one FileAdded.
- **Hot-reload `scan_automatically`.** Toggle the setting while the watcher is running; assert it pauses without restart.

---

## 12. Open questions

These need decisions before the corresponding phase starts:

- **tacet API shape.** Confirm public types — `detect_intro_credits()` signature, `MarkerSet` fields — before Phase 4. Currently this plan writes against a hypothesis.
- **OMDb daily quota strategy.** Free tier 1000/day. A 1000+ item library blows the quota on initial backfill. Options: (a) rate-limit our end to 800/day, spread backfill over weeks via sweep batches; (b) require user to bring their own paid key; (c) skip OMDb, use only RT + MPAA via TMDB. **Recommend (a)** with a "fetching ratings (X / Y items)" indicator on the gated kind.
- **Embedded subtitle PGS OCR.** Which engine? `tesseract` direct is the lowest-friction; `vobsub2srt` is dated; bitmap → tesseract pipeline works but eats CPU. May need a Phase 3.5 spike before committing.
- **Per-library gate overrides.** Deferred per the mockup planning. Worth designing forward-compat into the gate API now: `is_kind_allowed_for_library(kind, library_id)` rather than `is_kind_allowed(kind)`. Server-wide is the default; per-library is a future override layer on top.
- **LiveMetrics persistence.** Counters are in-memory only; reset on restart. Snapshot to disk every 60s for survivability across restarts? Probably not worth it — admins viewing the activity screen care about *now*, and the daily rollup captures history.

---

## Appendix: structural inventory of changes

What lands in which crate:

```
crates/server/src/tasks/                    NEW
  ├── mod.rs              registry exports
  ├── kind.rs             TaskKind trait + enums
  ├── registry.rs         static KIND map
  ├── gates.rs            is_kind_allowed + cache
  └── metrics.rs          LiveMetrics

crates/server/src/jobs/
  ├── pipeline.rs         enqueue_pipeline_batch() rewrite
  ├── worker.rs           per-kind semaphores
  └── handlers/
      ├── detect_markers_file.rs       tacet swap
      ├── refresh_logos_item.rs        NEW (extracted)
      ├── detect_extras_item.rs        NEW
      ├── extract_embedded_subs.rs     NEW
      └── fetch_external_ratings.rs    NEW

crates/server/src/scheduler/mod.rs      gate consultation; remove sweep loops where extracted

crates/server/src/file_watcher.rs       debounce + hot-reload + per-item coalesce

crates/server/src/api/admin/tasks.rs    NEW endpoint family (replaces current jobs.rs surface)

crates/library/migrations/
  └── 20260520020000_phase60_task_gating.sql
  └── 20260520020001_phase60_item_extras.sql
  └── 20260520020002_phase60_task_metrics.sql

web/src/components/admin/
  └── AdminTasksClient.tsx              rewrite from tasks-ui.html mockup
  └── AdminTasksActivityClient.tsx      NEW
  └── AdminTaskDetailClient.tsx         NEW
  └── AdminTaskFlowClient.tsx           NEW

[external] /mnt/github/tacet            ChimpFlix becomes its first consumer
```
