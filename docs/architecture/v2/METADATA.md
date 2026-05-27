# V2 Metadata — Agent Chain Port

> Status: **RFC skeleton.** Carries forward from V1 with refactoring,
> not redesign. Lift-and-shift target.

## Scope

How V2 enriches identities (movies, shows, episodes, people) with
external metadata from TMDB, TVDB, AniList, TVMaze, and OMDb.

## Carry forward from V1

V1's metadata-agent framework (the 10-slice refactor shipped 2026-05-
21) is one of V1's cleanest subsystems. V2 ports it largely
unchanged:

- The `MetadataAgent` trait + the 5 agents.
- Per-library priority order with `is_primary` semantics: primary
  owns canonical fields; later agents fill nulls without clobbering.
- Per-scan provider caches (success / missing / errored) — explicitly
  required for the AniList rate-limit problem.
- AniList split-cour season mapping (`resolve_season_anilist_id`).
- AniList retry hardening (`MIN_RETRY_AFTER_S` floor, exponential
  backoff).
- TMDB season cache that stores 404s.
- Capability matrix UI in admin.
- Operator-configurable metadata language (BCP-47).

## What changes

- **Repository layer.** Agents no longer touch SQL directly. They
  consume the typed repository methods defined in `SCHEMA.md`.
- **Single write sink.** Agents emit "metadata patches" against a
  typed in-memory model; the scanner's upsert stage flushes them.
  Same idempotency guarantees.
- **Async-by-default.** All HTTP calls go through one `reqwest`
  client shared across agents, with per-host concurrency caps.

## Open questions

- **OMDb full agent.** V1 shipped OMDb as a metadata agent (Phase
  TBD). V2 confirms ratings-only or full-metadata depending on what
  OMDb's API surface usefully fills.
- **AniDB.** V1 considered and skipped. Re-evaluate for V2 if
  AniList's coverage gaps come up.
- **MusicBrainz.** ChimpFlix is video-only; no music libraries.
  MusicBrainz not in scope.
- **Episode crew vs. show crew.** V1 stores some at show-level.
  Schema RFC decides; metadata pulls into the chosen shape.

## Cut list

- **Embedded metadata files (`.nfo`).** V1 doesn't read them; V2
  doesn't either. Plex / Kodi compatibility is not a goal.
- **Custom metadata agents at runtime.** Plug-in agents would be
  nice but the maintenance surface isn't worth it for this product.
  Five built-ins is the line.
