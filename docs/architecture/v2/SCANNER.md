# V2 Scanner — Ingest Pipeline

> Status: **planning draft.** Defines V2's library scanner.

## What V1 got right

- **Parallel candidate processing.** V1's `buffer_unordered(N)` model
  over per-file tasks is the right shape. ffprobe is subprocess work,
  metadata is HTTP work, both are I/O-bound; parallelism over them
  hides latency. V2 keeps this.
- **Per-scan provider caches.** TMDB season cache, AniList show /
  episode / season-id caches. These exist because anime libraries
  with 12-episode seasons would otherwise call TMDB 12 times for the
  same season. Carry forward.
- **AniList split-cour season mapping.** V1's `resolve_season_anilist_id`
  is non-obvious and correct. Critical: returns `None` when no
  distinct entry, never falls back to the primary (which would
  mis-assign S1 titles to S2 files). Carry forward exactly.
- **Soft-delete reconciliation scoped to reachable roots.** A partial
  unmount must not eat the offline catalog. Carry forward.
- **Unmatched-files visibility.** V1 Phase 71 ish: failed-regex files
  get stub rows with `auto_matched=false` so they appear in the
  browse grid. V2 carries this forward — the 89%-of-files-invisible
  bug is a permanent lesson.
- **AgentChain primary semantics.** First agent in the chain owns
  canonical fields; later agents fill nulls. Same in V2.
- **Tacet integration.** Single-decode fan-out for intro / credits /
  loudness via tacet. V1 Phase A through deferrals-shipped. V2 keeps
  the architecture; might trim what the scanner triggers vs. what
  the post-scan pipeline triggers.

## What V1 got wrong

1. **Every file write fights every other writer.** N parallel scanner
   tasks each acquire pool connections, each does multiple writes per
   file, all competing with job workers, scheduler, playback,
   webhooks. Storage-level fix in `STORAGE.md`; scanner needs to
   participate in the new contract.
2. **Scanner is monolithic.** `scanner.rs` is 2582 lines mixing file
   discovery, ffprobe orchestration, metadata agent dispatch, DB
   upserts, soft-delete reconciliation, and progress reporting.
   Reading any one of these requires understanding all of them. V2
   splits these along clear seams.
3. **Counters update via the pool.** V1's `update_scan_counters` call
   inside the streaming loop was the source of the "scan stops at
   775/1090, database is locked" bug. Currently mitigated by
   warn-and-continue, but the underlying coupling is wrong — UI
   progress should not block ingest, and ingest should not be able to
   poison the scan from a progress-counter error.
4. **No backpressure from the foreground.** When users are actively
   watching, the scanner doesn't know. It hammers as if it's the only
   process on the box.
5. **Discovery and processing are interleaved.** V1 walks + processes
   in one streaming pass. This makes "scan progress: 23/?" — total
   unknown until the walk completes. V2 should consider two-phase:
   walk first (cheap), enumerate candidates, then process (with a
   known denominator).

## V2 design

### Pipeline shape

```
   roots ──► discover ──► candidates ──► classify ──► match ──► enrich ──► upsert
                │                                                              │
                └────── reconcile ◄─── existing files ◄────────────────────────┘
```

Each stage is a separate module with a typed input + output:

- **discover** — walks roots via `WalkDir`, filters by extension,
  emits `Candidate { root, path, mtime, size }`. Pure I/O, no DB
  access, no provider calls.
- **classify** — applies regex parsers to the path, emits
  `ClassifiedCandidate { kind: Movie | Episode { show_name, season,
  episode }, raw_meta }`. Pure computation, fast.
- **match** — looks up local identities matching the classified
  candidate. Read-only DB access via repository layer. Emits
  `MatchResult::{ Existing(MediaFileId), New(NewMediaFile),
  Unmatched(Reason) }`.
- **enrich** — for `New` matches: calls metadata agents to fetch
  titles, summaries, artwork. Uses per-scan caches. Emits
  `EnrichedFile`.
- **upsert** — single sink that writes `EnrichedFile` to the DB.
  Batches where safe. Background priority writes per `STORAGE.md`.
- **reconcile** — at end of scan, compares existing+reachable files
  against seen set, soft-deletes the absent.

Each stage is an `async fn` taking a stream and yielding a stream.
Parallelism happens at the stage level (e.g. `enrich` can run N
agents concurrently against M candidates) and not at the per-file
level monolithically.

### Backpressure from foreground

The scanner subscribes to a foreground-pressure signal:

```rust
pub struct ForegroundPressure {
    pub active_play_sessions: usize,
    // …possibly more later: pending interactive requests, etc.
}
```

The scanner uses this to scale its parallelism in real time:

- `active_play_sessions == 0` → full speed (e.g. parallelism = 4).
- `active_play_sessions ≥ 1` → reduced parallelism (e.g. 1 or 2).
- `active_play_sessions ≥ N` (operator-tunable) → pause ingest
  entirely until the load drops; resume from where it paused.

Critically, this is not just throttling write rate — it's throttling
the *enrichment* stage too, so we don't queue 1000 metadata fetches
that'll then race to write when the user finishes a show.

### Counters out-of-band

Progress counters are published to the realtime hub directly, not
written to the DB on every progress interval. The final tally lands
in `scan_jobs` at scan completion. The activity feed reads the
realtime stream; the DB has the durable end-state. Two consumers,
two write rates.

(V1's mid-scan counter writes were cosmetic and triggered real bugs.
Removing them entirely from the hot path is a clean win.)

### Two-phase: walk, then process

V2 walks roots first to enumerate candidates, then processes them.
Walk is cheap (just FS metadata). The benefit: progress UI shows
"23/1090 (2%)" with a real denominator. Operators stop asking "how
much longer."

The walk itself is fast enough on NFS that the round-trip overhead
isn't worth optimizing — even at 1 TB scale, the walk completes in
seconds; the slow part is per-file processing. Memory cost of holding
the candidate list in RAM is bounded (~100 bytes per file × 100k
files = 10 MB).

### Watcher integration

V1 file watcher queues a `scan_library` job that runs the full scan
pipeline. V2 keeps this design — the watcher is just a trigger for
the same scan pipeline; there is no separate "watcher-triggered fast
path." Justification: any optimization for "watcher saw 1 file" gets
defeated the moment the watcher sees 100 files (a season pack
landing).

What V2 changes: the watcher's debounce window and the scan
pipeline's two-phase walk combine to make watcher-triggered scans
naturally efficient — only files newer than the last scan's
high-water-mark need full processing, and the walk's metadata
filter is enough to identify them.

### Idempotency

Every stage is idempotent on retry. A scan that crashes mid-way and
restarts produces the same result as one that ran cleanly. Specifically:

- `discover` is deterministic given a snapshot of disk state.
- `classify` is pure.
- `match` is idempotent (existing identity won't be re-created).
- `enrich` may re-fetch from agents on retry; per-scan caches make
  this cheap on a restart-within-window.
- `upsert` uses ON CONFLICT to coalesce.

### Cancellation

Scans support cancellation via `CancellationToken` propagated through
every stage. Cancelling closes the candidate channel cleanly, lets
in-flight ffprobe/HTTP calls drain, releases the scan-job lock, and
marks the job aborted. (V1 has cancellation; V2 ports the pattern.)

## Subtitle pre-warming

V1 pre-warms WebVTT during scan via ffmpeg subprocess fan-out, bounded
by a global semaphore (`SUBTITLE_PREWARM_LIMIT = 4`). V2 keeps this
but moves it out of the scanner: completed scans emit a "FileAdded"
event, the post-scan pipeline schedules subtitle pre-warm as a job,
gated by the same semaphore. Keeps the scanner focused on identity
resolution, not transcoder warmup.

## Intro / credits / loudness detection

V1's [Phase A → D + deferrals] established the model:
- Chapter-first short-circuit: embedded chapter labels bypass tacet.
- Bootstrap season detection: tacet runs once per season at
  `bootstrap_season_refs` time.
- Tacet's `fused_decode` runs one symphonia pass feeding ebur128 +
  intro buffer + credits buffer.

V2 keeps this architecture. The scanner triggers the chapter-first
short-circuit during enrich; the deeper detection runs as a
background job (`bootstrap_season_refs` equivalent), gated by the
foreground-pressure signal.

[Phase A → D + deferrals]: ../../docs/PERF_PLAN.md

## Mount-aware logging

V1 file watcher learned to demote "library path missing" WARNs after
the first occurrence per mount transition. V2 scanner inherits the
discipline:

- WARN once per fresh missing-mount transition.
- DEBUG on retries while still missing.
- INFO "path recovered" when the mount returns.

Same throttling pattern. The shared infrastructure should live in a
small `mount_watch` module both scanner + watcher consume.

## Observability

- Scan stages emit OpenTelemetry spans.
- Per-stage histograms: time per candidate in discover, classify,
  match, enrich, upsert.
- Per-agent histograms in enrich (TMDB call time, AniList call time).
- Surfaced in admin via Prometheus scrape + a dashboard JSON we
  ship with the binary.

## Open questions

- **Walk-then-process vs. interleaved.** Two-phase is cleaner for
  progress but blocks first-write until walk completes. For a 1 TB
  NFS library the walk is still fast (no `stat -L`, just `readdir`),
  but worth measuring in Phase 2.
- **Should match be DB-only or can it also probe disk?** V1 hashes
  on first-match for fingerprinting. V2 should decide: keep hash at
  match time (slow) or defer to enrich (correct for "is this the
  same file we matched last time" questions).
- **Watcher event coalescing window.** V1's 5s debounce works in
  practice. V2 starts at the same value; tune from observed behavior.
- **Per-library scan_lock semantics.** Concurrent scans against the
  same library must serialize. Across libraries can run in parallel.
  V1 has a per-library gate; V2 keeps it; the gate primitive belongs
  in the jobs subsystem (`JOBS.md`), not the scanner.

## Cut list

- **Chromaprint fingerprint intro detection.** V1 Phase 71 retired
  this. V2 does not restore it. The chapter-first + tacet path is
  what the product needs.
- **Scrub-preview sprite generation.** V1 retired sprites + chapter
  thumbs in Phase 71. V2 does not bring them back.
- **In-scanner progress writes to DB.** Moved to realtime channel
  only.
- **Watcher fast-path that bypasses metadata enrichment.** Considered
  for "huh, just one file appeared" — rejected because correctness
  requires the same enrichment regardless of trigger source.
