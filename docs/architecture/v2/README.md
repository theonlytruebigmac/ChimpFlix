# ChimpFlix V2 Architecture

This folder holds the planning surface for ChimpFlix V2 — a from-
scratch rebuild on a new branch that preserves the product V1 has
shaped while replacing the foundation.

**Status:** Planning. No V2 code exists yet. The strategic decisions
in [`OVERVIEW.md`](./OVERVIEW.md) gate every RFC here, and the RFCs
gate the actual code.

## Read order

1. **[`OVERVIEW.md`](./OVERVIEW.md)** — start here. Why V2, what's in
   scope, what's not, the architectural bets, phasing, risks,
   success criteria.
2. **[`REQUIREMENTS.md`](./REQUIREMENTS.md)** — V1 feature inventory.
   The bar V2 must clear, organized by user-facing surface. Anything
   V2 doesn't deliver must be moved to the explicit "Cut" section.

## Foundation RFCs (full depth)

These are the architectural decisions V2 is built on. Decide these
before writing code.

3. **[`STORAGE.md`](./STORAGE.md)** — Turso adoption, concurrent-
   write model, repository layer, schema migration story, risk
   management for the pre-1.0 dependency.
4. **[`SCHEMA.md`](./SCHEMA.md)** — Greenfield schema design
   principles, identity model, naming conventions, index strategy.
5. **[`SCANNER.md`](./SCANNER.md)** — Ingest pipeline. Stage-based
   architecture, foreground-pressure backpressure, two-phase walk,
   what V1 got right and wrong.
6. **[`JOBS.md`](./JOBS.md)** — Background work subsystem. Priority
   classes, concurrency keys, event-driven pipeline, scheduler.

## Subsystem RFCs (skeleton)

These are mostly carry-forward from V1 with refactoring. Expand as
Phase 1+ work demands.

7. **[`METADATA.md`](./METADATA.md)** — Five-agent chain.
8. **[`TRANSCODING.md`](./TRANSCODING.md)** — ffmpeg pipeline.
9. **[`AUTH.md`](./AUTH.md)** — Identity, sessions, roles, OAuth.
10. **[`REALTIME.md`](./REALTIME.md)** — Event hub + SSE.

## Decisions in flight

11. **[`API.md`](./API.md)** — REST vs. typed RPC vs. GraphQL.
    Recommendation: typed RPC with codegen.
12. **[`FRONTEND.md`](./FRONTEND.md)** — Framework, state model,
    SSR strategy. Recommendation: stay on Next.js, redesign the
    architecture inside it.

## How to use these docs

- **Adding a feature?** Check `REQUIREMENTS.md` first. If it's not
  there and V1 doesn't have it, propose adding it before
  implementing.
- **Disagreeing with a decision?** Edit the RFC. These are not
  immutable — but every change should leave the "why" trail intact.
- **Filling in a TBD?** Update the relevant RFC's open-questions
  section. Don't proliferate ad-hoc notes elsewhere.
- **Cutting a V1 feature?** Move the entry from `REQUIREMENTS.md`'s
  inventory into the "Explicitly cut" section with a one-line
  rationale.

## Where this lives in git

These docs are committed to `main` so V1 work can reference them.
V2 implementation lives on a separate branch (name TBD by maintainer,
likely `v2`). The branch can rebase against `main` as planning docs
evolve.

## Not in this folder

- V1 architecture: see [`../ARCHITECTURE.md`](../ARCHITECTURE.md).
- V1 perf plan: see [`../PERF_PLAN.md`](../PERF_PLAN.md).
- V1 schema: see [`../SCHEMA.md`](../SCHEMA.md).
- V1 Plex parity audit: see
  [`../PLEX_PARITY_PLAN.md`](../PLEX_PARITY_PLAN.md).

Those documents stay frozen as the V1 record. They are not edited
during V2 planning.
