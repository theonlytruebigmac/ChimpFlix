# V2 Storage — Turso Adoption + Concurrency Model

> Status: **planning draft.** Locks how V2 talks to disk.

## Decision

V2 uses [Turso] (embedded, Rust-native) as its storage engine from
day one. The V1 SQLite pool model — single shared pool, busy_timeout
30s, every connection a potential writer — is the root cause of V1's
single biggest user-visible failure mode (scan-induced playback
stall). V2 is designed around Turso's concurrent-write model from the
schema up, not retrofitted onto it.

[Turso]: https://github.com/tursodatabase/turso

## Why Turso, not SQLite

The V1 incident: a 1 TB watcher-triggered ingest held the single
SQLite writer lock for minutes. Every concurrent write (play_state
upserts during playback, scheduler ticks, job claims, webhook
retries) queued behind it. 30s busy_timeout meant 30s pool-acquire
times, then cascading 500s.

SQLite's `BEGIN CONCURRENT` extension exists but is not enabled in
the standard build, and getting it into a Rust+sqlx stack is not a
small lift. Turso's headline feature is exactly this: concurrent
writes without the single-writer lock. The same workload that broke
V1 should run on V2 without any contention strategy beyond schema
hygiene.

The risk is real: Turso is pre-1.0 as of plan date. We are explicitly
betting that a maturing engine designed for our exact problem is a
better foundation than a 25-year-old engine fighting our exact
problem. The mitigation plan is in [Risk management](#risk-management)
below.

## Storage architecture

```
                       ┌─────────────────────────────────┐
                       │       application code          │
                       │  (no direct SQL outside repo/*) │
                       └─────────────┬───────────────────┘
                                     │
                  ┌──────────────────┴──────────────────┐
                  │      repository layer (typed)        │
                  │  movie_repo, show_repo, play_repo… │
                  └──────────────────┬──────────────────┘
                                     │
                  ┌──────────────────┴──────────────────┐
                  │   storage trait — Db / DbReader     │
                  │  (abstracts Turso for testability + │
                  │   escape hatch if Turso falls over) │
                  └──────────────────┬──────────────────┘
                                     │
                              ┌──────┴──────┐
                              │    Turso    │
                              │  (embedded) │
                              └─────────────┘
```

The repository layer is the only thing that knows about SQL. Routes,
services, and workers consume typed repository methods. This is
deliberately the boundary V1 doesn't have — V1's `queries.rs` is
14k lines and grew organically with every feature, so any change risks
side effects three subsystems away.

## Concurrency model

Turso supports concurrent writers. The model V2 codifies on top:

### Write path isolation

Each foreground write category gets its own conceptual "writer
affinity" — not necessarily separate connections, but separate logical
queues at the application layer:

- **Playback writes** (play_state upserts, scrobble events): highest
  priority, smallest transactions, never batched with anything else.
- **User writes** (ratings, list adds, watchlist toggles): foreground
  priority. Small transactions.
- **Admin writes** (settings PATCH, schedule changes): foreground
  priority. Small transactions.
- **Background writes** (scanner ingest, metadata enrichment, job
  state updates): background priority. May batch.

The point is that no single background transaction blocks the
foreground queue from making progress. With Turso's concurrent
writes, conflicts are *resolved* (retry / abort) rather than
*serialized at the file lock*. The application gets to express
priority via the order it submits work, not via lock contention.

### Read path

Reads use connection pooling with no special priority. Turso's MVCC
gives readers a consistent snapshot regardless of in-flight writes.
The read pool size is tuned for concurrent user load (browse,
modal, search), separate from any write concerns.

### Repository contract

Each repository method declares its category:

```rust
// Sketch — final API TBD in Phase 1.
impl PlayStateRepo {
    /// Foreground priority. Never batched.
    pub async fn upsert(&self, ...) -> Result<()>;
}

impl ScannerRepo {
    /// Background priority. May coalesce within a tx window.
    pub async fn upsert_media_file_batch(
        &self,
        files: &[NewMediaFile],
    ) -> Result<Vec<MediaFileId>>;
}
```

The category isn't enforced by the type system in the initial cut;
it's a convention enforced by code review. If it proves load-bearing,
the trait can require it explicitly.

## Read/write split — still relevant?

In V1's model, the read/write split is the canonical fix for SQLite
contention: one dedicated writer connection + a pool of readers,
because SQLite serializes writers at the file lock anyway. In V2's
Turso model, the engine itself doesn't have that constraint, so the
split's value is different — it's still useful for resource budgeting
(read pool sized for user load vs. write pool sized for known-small
write rate) but not load-bearing for correctness.

V2 keeps the split as a soft convention via the repository layer: a
`DbReader` trait for read-only repos, a `Db` trait for repos that
also write. Implementations may share connections or not; the
abstraction lets us change that later.

## Schema migration story

V2 ships with a greenfield schema. There is **no** V1→V2 migration
for the media catalog — operators point V2 at their media library
and it rebuilds. Rationale:

- V1 has 91 phases of schema scars. Inheriting them defeats the
  rewrite.
- The media catalog is fully reproducible from on-disk files +
  metadata API calls. Rebuilding is slow but correct.
- The "rebuild from scratch" path is also V2's recovery story for any
  catastrophic data loss — making it the *primary* import path
  exercises the recovery code on every install.

User-generated data (watch history, ratings, watchlists, custom
collections) is **not** reproducible. V2 may ship an opt-in importer
for these:

- Reads the V1 SQLite file directly (read-only).
- Resolves V1 row IDs to V2 row IDs via on-disk file paths + IMDB/
  TMDB/AniList external IDs.
- Skips entries where the resolution fails and reports them to the
  operator.

This decision is deferred to `SCHEMA.md` — depends on how cleanly the
V1 ID universe maps onto V2's external-ID-first identity model.

## Migrations within V2

Turso supports SQL migrations the same way SQLite does. V2's
migrations directory starts at `0001_initial.sql` with the full
schema, not 91 incremental phases.

Migration tooling: a thin in-house runner (V1 has this; carry the
pattern forward) or an off-the-shelf tool if there's a Turso-friendly
one by then. Decide in Phase 1.

The discipline: V2 migrations are forward-only and small. No
`legacy_alter_table` dances. If a column needs to change shape in a
way SQLite/Turso can't do in-place, the migration is "create
new_table, copy data, drop old, rename." That's a long-standing
SQLite pattern; just be explicit about it.

## Backup + restore

- Online backup via Turso's snapshot API (whatever it provides at
  the time of build).
- Verified-restore task as a scheduled job (V1 has this; port to V2).
- Backup retention policy as a typed setting (V1 Phase 90 — carry
  forward).
- Offsite backup is operator's responsibility; ChimpFlix produces the
  artifact and stops there. (Same as V1.)

## Observability

- Slow-query logging via Turso's instrumentation, surfaced in admin
  logs page.
- Per-repository-method timing histograms exported as Prometheus
  metrics (Phase 1 of V2 wires this).
- Active connection / pool metrics.
- Migration-run audit log on startup.

## Risk management

Turso pre-1.0 risk mitigation:

1. **Storage trait** — repositories depend on the trait, not Turso
   directly. If Turso becomes untenable mid-build, we swap the
   implementation behind the trait. The trait is shaped to be
   implementable by sqlx::sqlite as a fallback.
2. **Pinned dependency.** No `*` or `^` ranges. Bump cadence is
   deliberate, with a check-in session per bump.
3. **Integration test suite** runs the full V2 workload (scan +
   playback writes + background jobs) on every Turso bump. Pinned
   benchmarks catch regression.
4. **Bug-report relationship.** We file issues against Turso for any
   blocker we hit. Their alpha-stage maintainers care about feedback;
   we benefit from the early-adopter influence.
5. **Escape hatch documented from day one.** A short note here:
   if Turso goes unmaintained, V2's storage trait + V1's existing
   sqlx::sqlite expertise mean reverting to SQLite is a ~2-week
   project, not a rewrite. Cost-of-rollback is bounded.

## Open questions

- **Repository code generation vs. hand-written?** sqlx's
  `query_as!` macro gives compile-time SQL verification. Does Turso
  have an equivalent at planning time? If not, hand-written repos
  with integration test coverage are the fallback.
- **Connection pooling primitive.** Turso's own pool, deadpool, or
  custom? Probably whichever ships with the Turso crate at build
  time.
- **Cross-version V1 read access.** If we ship an opt-in V1→V2
  user-data importer, V2 needs a way to open V1's SQLite file
  read-only. This means a sqlx::sqlite dependency in the importer
  binary, separate from V2's runtime Turso dependency. Decide:
  separate binary, separate crate, or runtime dependency?

## Cut list

> Things considered and explicitly not in V2.

- **libSQL remote / sync mode.** ChimpFlix is single-node. Remote DB
  adds an operator dependency we don't need.
- **Read replicas.** Same — single node.
- **Sharding by library.** Each library's catalog in its own DB file
  would isolate write contention more thoroughly, but Turso's
  concurrent writes make this unnecessary, and cross-library queries
  (search, home rails) get harder. Reject.
- **Postgres backend.** Briefly considered as the safe alternative to
  Turso. Adds an operator dependency (separate process), drops the
  single-binary deploy story, and the V1 product never needed
  Postgres-only features. Reject.
