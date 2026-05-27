# ChimpFlix V2 — Architecture Overview

> Status: **planning draft.** No V2 code exists yet. This document
> captures the strategic direction agreed on 2026-05-27 and gates every
> downstream RFC in this folder.

## Why V2

V1 is a working Netflix-style self-hosted media server with a strong
feature set: scanning, metadata enrichment across 5 agents, HLS
transcoding with HDR + intro/credits detection, Trakt and Plex OAuth
integration, owner/admin/user role hierarchy, a polished Netflix-clone
frontend, and a real-time job pipeline. It is feature-solid.

It is **not** structurally solid for the workloads it now runs against.
The defining incident: a watcher-triggered scan of ~1 TB of newly-added
anime saturated the single SQLite writer lock for minutes, blocking
live playback's `play_state` writes and cascading 500s across the API
surface. That is not a tuning problem. It is the natural endpoint of
1) layering 91 schema migrations on a schema that was first sketched
for a much smaller product, 2) treating one shared connection pool as
the only storage primitive, and 3) growing the job/scanner/scheduler
subsystems organically without ever rethinking the contention model.

V1 has taught us what the product looks like, how operators use it,
which features matter, and where the polish lives. V2's purpose is to
take that hard-won product clarity and rebuild the foundation around
the actual usage patterns we now understand, not the ones we guessed
at in phase 1.

This is not a refactor. V1 stays on `main` as the working production
line. V2 is built in a separate branch from a clean slate and merges
back when complete.

## Scope (locked)

- **Full stack.** Backend (server, storage, scanner, jobs, transcoder,
  metadata, real-time) AND frontend (web UI) are both in scope. The
  goal is one coherent V2, not a backend rewrite stapled to a legacy
  frontend.
- **Turso from day one.** V2 commits to [Turso] as the storage engine
  and embraces its concurrent-write model. We accept the cost of
  riding a pre-1.0 dependency. See `STORAGE.md` for the rationale and
  risk-management plan.
- **Clean break schema.** V2 ships with a greenfield schema designed
  from the patterns V1 made visible. Operators point V2 at their
  media library and it rebuilds. No V1→V2 import path for the media
  catalog itself. (User-generated data — watch history, ratings,
  watchlists — *may* have an opt-in importer; see `SCHEMA.md`.)
- **Parallel run, then merge.** V1 keeps running on `main` for
  operators using ChimpFlix today. V2 develops on its own branch with
  its own data directory layout. When V2 is feature-complete and
  validated, the branches merge and V2 becomes the new `main`.

[Turso]: https://github.com/tursodatabase/turso

## Non-goals

Things V2 explicitly is NOT trying to be:

- **Plex feature-parity for its own sake.** V1 already closed every
  audited Plex gap. V2 inherits that target list (see `REQUIREMENTS.md`)
  but does not add new "because Plex has it" features.
- **Mobile or TV apps.** Web-first remains the bet. See the V1 scope
  exclusions; they carry forward.
- **Live TV / DVR.** Same.
- **A general-purpose media tool.** ChimpFlix is a curated, owner-
  operated server for a known invite list. The V2 schema, auth model,
  and scaling targets reflect that — not a multi-tenant SaaS.
- **Cluster mode.** Single-node remains the deployment model. Turso's
  embedded library is fine; we are not chasing libSQL's remote / sync
  mode unless a concrete operator pain point demands it.

## Goals

In order of weight:

1. **Background work never disrupts foreground experience.** A 1 TB
   ingest happening in the background must leave playback, browse,
   modal, and search latencies indistinguishable from idle. This is
   the single biggest failure of V1 and the foundational requirement
   for V2.
2. **Schema legibility.** A new contributor can read the schema and
   understand what each table is for in one pass. V1's 91 migration
   layers make that impossible today.
3. **Subsystem isolation.** Scanning, metadata enrichment, transcoding,
   real-time, and frontend should be independently understandable and
   independently changeable. V1 has crept into a state where a scanner
   tweak risks the playback path. V2 codifies the seams.
4. **Operability.** V2 ships with structured metrics + traces from
   day one. V1 has solid logging and an activity feed; V2 expands to
   proper observability primitives so operators can see and reason
   about what the server is doing.
5. **Honest typing.** TypeScript and Rust types are the contract. V1
   leaks `unknown`/`any` in a few API edges and has Rust struct fields
   that have shifted purpose without renames. V2 holds the line.

## Technical bets (locked)

| Area | V1 | V2 |
|---|---|---|
| Storage engine | SQLite via sqlx | Turso (current) |
| Concurrency model | Single pool, busy_timeout 30s | Concurrent writers, read pool unbounded |
| Schema | 91 accumulated migrations | Greenfield |
| Job system | `jobs` table + worker pool | TBD — see `JOBS.md` |
| API shape | REST under `/api/v1`, hand-typed | TBD — see `API.md` |
| Frontend | Next.js 16 app router | TBD — see `FRONTEND.md` |
| Real-time | SSE via event hub | TBD — see `REALTIME.md` |
| Auth | Cookie sessions + Plex OAuth | Carry forward (see `AUTH.md`) |
| Transcoder | ffmpeg+ffprobe subprocesses | Carry forward (see `TRANSCODING.md`) |
| Metadata | 5-agent chain | Carry forward (see `METADATA.md`) |

Cells marked **TBD** have RFCs in this folder where the decision gets
made. Cells marked "carry forward" are V1 subsystems considered to be
in their right shape; V2 will port them with refactoring as the new
boundaries demand but not redesign them.

## High-level architecture sketch

```
                      ┌──────────────────────────┐
                      │   web (TBD framework)    │
                      └────────────┬─────────────┘
                                   │ HTTP + real-time
                      ┌────────────▼─────────────┐
                      │     api gateway          │  ← thin; auth + routing
                      └─┬────────┬────────┬──────┘
                        │        │        │
            ┌───────────▼─┐  ┌───▼────┐  ┌▼────────────┐
            │  read       │  │ write  │  │ realtime    │
            │  service    │  │ svc    │  │ hub         │
            └─────┬───────┘  └───┬────┘  └──────┬──────┘
                  │              │               │
                  │ (read pool)  │ (write conn)  │ (in-memory)
                  ▼              ▼               │
              ┌──────────────────────┐           │
              │   Turso (embedded)   │◄──────────┘
              └──────────────────────┘
                        ▲
                        │
            ┌───────────┴──────────────┐
            │      ingest workers      │  ← scanner, metadata, transcode jobs
            └──────────────────────────┘
```

The shape that matters: read service, write service, and ingest
workers are *separate write paths* into Turso. Background ingest can
saturate its own writer affinity without blocking the foreground write
service that's persisting `play_state` and ratings. See `STORAGE.md`
for how this maps to Turso's concurrency model.

## Phasing

Each phase ends with something demoable, even if not user-facing.

- **Phase 0 — Planning** (this folder). All RFCs at decision-quality.
  Exit criterion: every open question in the RFCs has an answer or an
  explicit "decide later" with the decision-point named.
- **Phase 1 — Foundation.** Cargo workspace skeleton on the v2 branch.
  Turso embedded + a stub schema + the read/write service split, with
  one tracer-bullet endpoint (`GET /api/v2/health` returning the Turso
  version). Migrations runner. Logging + tracing wired.
- **Phase 2 — Storage + scanner.** Greenfield schema landed. Scanner
  redesigned per `SCANNER.md`. End-to-end: drop files in a folder,
  they get scanned + matched, queryable via a CLI dump tool. No HTTP
  surface yet. Validates the concurrency model — a scan running while
  a synthetic write workload hits play_state must not slow the
  workload.
- **Phase 3 — Metadata + transcoder.** Port the 5-agent chain and the
  ffmpeg transcoder behind their new boundaries. End-to-end: from
  scanned file to playable HLS session.
- **Phase 4 — API surface.** Bring up enough of the V2 API to support
  the new frontend. Auth in place. Real-time hub running.
- **Phase 5 — Frontend.** Build the new web UI against the V2 API.
  This is where most contributor-time lands; frontend work is parallel
  with later backend hardening.
- **Phase 6 — Feature parity check.** Walk `REQUIREMENTS.md` end-to-
  end. Anything V1 has that V2 doesn't gets ported or explicitly cut
  with rationale.
- **Phase 7 — Hardening + operator polish.** Backup, restore,
  upgrade story, deployment docs. Performance budget enforced (see
  Success criteria).
- **Phase 8 — Branch merge.** V2 becomes the new `main`. V1 archived
  in a tag for historical reference.

Phases are sequential at the *foundation* level but parallel within
each. Frontend can iterate on Phase 5 while the backend continues
hardening in Phase 7.

## Risks

- **Turso churn.** Pre-1.0 dependency. Mitigation: storage trait
  abstraction (`STORAGE.md`), pinned commit with deliberate bump
  cadence, integration test suite that proves regressions on bump.
  Escape hatch: revert to sqlx::sqlite behind the trait if Turso
  becomes untenable.
- **Schema-from-scratch regression risk.** Easy to drop a V1 behavior
  in the rewrite. Mitigation: `REQUIREMENTS.md` as the gating
  checklist, plus the parallel-run period gives operators time to
  flag missing behaviors before V1 is retired.
- **Frontend scope inflation.** A from-scratch frontend can absorb
  unlimited time. Mitigation: V1's UI is the visual + interaction
  spec; V2 frontend ports look + feel verbatim unless the V2 backend
  *requires* a UX change.
- **Time-to-merge.** A long-running parallel branch drifts from
  `main`. Mitigation: V2's data layer is independent (separate data
  dir, separate Turso file), so the branches genuinely don't share
  runtime state. Drift is contained to docs + shared crates. Periodic
  rebase + sync sessions on a fixed cadence (operator-driven).
- **The "while we're at it" trap.** A clean-slate rewrite invites
  every long-deferred wish. Each RFC ends with a "Cut list" — things
  we considered and chose not to do in V2. The cut list is binding.

## Success criteria

V2 is ready to merge when all of:

1. A 1 TB watcher-triggered ingest leaves p95 `play_state` write
   latency under 50 ms throughout the ingest. (V1: 30,000 ms.)
2. p95 home-page paint, modal open, and search response under 200 ms
   while a scan is running.
3. Every V1 feature in `REQUIREMENTS.md` has a V2 implementation or
   an explicit cut entry.
4. A new contributor can read `OVERVIEW.md` + one RFC and ship a
   meaningful change to that subsystem without needing the
   maintainer's help to find the relevant code.
5. Operator upgrade path from V1 documented end-to-end including
   data-directory rebuild expectations.

## Index of RFCs

- [`REQUIREMENTS.md`](./REQUIREMENTS.md) — V1 feature inventory V2
  must match or explicitly cut.
- [`STORAGE.md`](./STORAGE.md) — Turso adoption, concurrency model,
  read/write split, abstraction layer.
- [`SCHEMA.md`](./SCHEMA.md) — Greenfield schema design principles,
  table-by-table sketch, naming conventions.
- [`SCANNER.md`](./SCANNER.md) — Ingest pipeline redesign.
- [`JOBS.md`](./JOBS.md) — Job/queue subsystem redesign.
- [`METADATA.md`](./METADATA.md) — Agent chain port.
- [`TRANSCODING.md`](./TRANSCODING.md) — Transcoder port.
- [`FRONTEND.md`](./FRONTEND.md) — Web UI rebuild.
- [`API.md`](./API.md) — API shape decisions (REST vs. typed RPC vs.
  GraphQL).
- [`AUTH.md`](./AUTH.md) — Auth + identity port.
- [`REALTIME.md`](./REALTIME.md) — Real-time hub redesign.

## Glossary

- **V1** — the current `main` branch as of 2026-05-27. Feature-solid,
  contention-troubled.
- **V2** — the planned rebuild. Lives on a separate branch (name TBD
  by user — likely `v2` or `next`).
- **Turso** — Rust-native SQLite-compatible engine with concurrent
  writes. Pre-1.0 as of plan date.
- **Foreground / background** — foreground = anything a logged-in user
  is waiting on (browse, modal, play_state writes during playback).
  Background = scanner, metadata enrichment, transcoder thumbnails,
  scheduled tasks. V2's first promise: foreground latency is
  independent of background load.
