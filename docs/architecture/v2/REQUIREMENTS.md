# V2 Requirements — V1 Feature Inventory

> Status: **planning draft.** This is the bar V2 must clear before
> merging back to `main`. Any item here that V2 does not deliver must
> be moved to the "Explicitly cut" section with a one-line rationale.

The inventory is grouped by user-facing surface, not by code module.
That keeps the conversation about what the *product* does, not how V1
happened to implement it.

## Library & catalog

- Libraries are typed: Movies, Shows, Anime, with kind-specific
  matching rules and metadata-agent defaults.
- Multiple roots per library (`library_paths`); nested roots
  attribute to the deepest matching root.
- Per-library hidden flag (operator can hide a library from non-owner
  users).
- Per-library metadata-agent chain ordering, with an explicit primary
  agent that owns canonical fields.
- Operator-configurable metadata language (BCP-47), threaded through
  every agent.
- Per-library "scan automatically" toggle that gates the file watcher
  but never blocks manual or scheduled scans.

## Scanning & ingest

- Manual scan trigger per library.
- Scheduled scan (Plex-style frequency + maintenance-window UX, not
  raw cron).
- File-watcher-triggered scan with both inotify and PollWatcher
  backends (NFS/SMB compatibility).
- Watcher dedupe / debounce so a `cp -r` doesn't fan into dozens of
  scans.
- Watcher events that arrive during an in-flight scan must be picked
  up by a subsequent scan, not dropped. (V1 bug closed 2026-05-27.)
- Soft-delete reconciliation: files missing on disk get marked
  removed, scoped to roots that were actually reachable this scan
  (partial unmounts don't eat the offline catalog).
- Unmatched files don't get silently dropped; they appear in the
  library browse grid with an "Unmatched" chip and a fix-matching
  affordance.
- First-scan of a new library runs with worker-pool + scheduler
  paused (the [first-scan exclusivity gate]).
- Scan progress is visible in the activity feed in real time.

[first-scan exclusivity gate]: ../../crates/server/src/jobs/scan_gate.rs

## Metadata

- Five metadata agents: TMDB, TVDB, AniList, TVMaze, OMDb. Per-library
  enable/disable + priority order.
- Capability matrix (which agent provides which fields) surfaced in
  admin UI.
- Anime-specific: AniList split-cour season mapping
  (`resolve_season_anilist_id`). TMDB absolute-episode fallback is
  deliberately NOT done in V1 and remains a non-goal in V2 unless
  operators specifically request it.
- Per-scan caches for all agents (success / missing / errored) to
  avoid duplicate provider calls within a single scan.
- AniList rate-limit retry with floor + exponential backoff.
- Episode-level enrichment (AniList `streamingEpisodes`, TVDB
  episodes, TVMaze episodes).
- Cast + crew with click-through to person detail pages.
- Show-level + season-level + episode-level artwork.

## Playback

- HLS adaptive bitrate (multiple renditions per session).
- Hardware acceleration: NVENC primary, CUDA decode, VAAPI/QSV
  capability-probed and gracefully demoted if smoke test fails.
- Direct play when codec + container compatible.
- HEVC end-to-end (Phase 43+).
- HDR tonemap config.
- Per-user max-resolution + max-bitrate cap; operator ceiling applies
  on top.
- Per-user remote stream cap; LAN bypass via CIDR list.
- Subtitle burn-in when required; otherwise WebVTT delivery with
  pre-warmed cache.
- Operator-wide subtitle default offset (compensates fansub re-encode
  drift).
- Per-file player subtitle offset stepper (additive with operator
  default).
- Subtitle style preferences (Phase 89).
- Audio loudness normalization (two-pass loudnorm, optional).
- Chapter menu (skip intro / outro / etc.).
- Pre-roll content.
- Hotkeys: standard playback (space, arrows, m, f), plus 0–9 for
  chapter seek and `n` for next.
- Resume pill on the modal with "Start over" option.
- Mobile scrubber time bubble.
- Skip-intro / skip-credits UI with chromaprint fingerprint detection
  (V1 had this; reassess in `SCANNER.md` whether V2 keeps it after
  the chapter-first short-circuit work).
- iOS native fullscreen via `webkitbeginfullscreen` sync.
- Page Lifecycle resume handler.
- Casting (Phase TBD; V1 has device detection and improved UI feedback).

## Auth & users

- Three-tier role hierarchy: Owner > Admin > User. `AdminAuth`
  extractor + `can_act_on` hierarchy guard + last-owner safety check.
- Owner-only routes for credentials, library mounts, destructive ops.
- Username/password auth with strong-password requirements.
- 2FA (TOTP) with recovery codes.
- Plex OAuth (invite-only signup; password-less Plex-only users
  supported). Provider-agnostic `user_auth_providers` table to allow
  future Google OAuth, etc.
- Cookie sessions, HMAC-signed.
- Email confirmation flow.
- Invite-only signup. Invite codes managed by owner.
- Onboarding wizard for first-run.

## UX surfaces

- Netflix-style home page: hero + multiple rails (Continue Watching,
  Up Next, Recently Added, Recommendations, Trakt watchlist, custom
  collections, etc.).
- Title modal: artwork, summary, cast, more-like-this, file info,
  Up Next chip for shows, remaining-episodes label, Trakt sync badge.
- Browse by library with filter chips (status, decade, codec,
  resolution, HDR), sort options (last played, duration, size, random
  seeded), pagination.
- Genre pages, collection pages (auto from TMDB + manual operator-
  curated).
- Search via FTS5 with bm25 ranking and column weights (title=10,
  original=5, cast=3, summary=1), kind chips, pagination, aria-live.
- Person detail pages with full filmography.
- History page (watch history with pagination).
- My List (user-curated watchlist).
- New & Popular page (server-side dedup).
- Coming Soon page (Trakt-driven calendar variants: premieres, new,
  movies).
- "Up Next" semantics: bulk watched/unwatched recalculates correctly
  (V1 bug closed 2026-05-27).
- Recently Added badge (operator-configurable window).
- Ratings (Like / dislike with cross-component event bus).
- Reviews (paginated).
- Mark-watched at episode / season / show level. Trakt push on all three.
- Auto-mark-watched scrobble at threshold (Trakt scrobble lifecycle).

## Trakt integration (two-way)

- OAuth login.
- Scrobble lifecycle (start / pause / stop) including direct-play stop.
- Watchlist two-way sync (push + pull, phase 88 state table).
- Collection diff-based push (phase 87 state table).
- Last-activities cursor for incremental pull.
- Hidden-items filter on recommendations.
- Personal lists as rails on home.
- Favorites rail (read-only).
- User stats.
- Recommendations rail (movies + shows).
- Coming Soon calendar variants.

## Plex compatibility (read-only)

- Plex OAuth for sign-in (invite flow polished 2026-05-27).
- Plex client identifier rotation in admin.
- Plex remains a one-way auth provider, not a metadata source.

## Admin surfaces

- Plex-style consolidated `/admin/library` page with scan settings,
  watcher polling, recently-added window, scanner ffmpeg nice level.
- Library health page.
- Per-library agent configuration.
- `/admin/playback` (Phase 3).
- Job queue / activity feed with kind-specific ETAs.
- Scheduled tasks simple-view + advanced editor.
- Audit log (narrow, deliberate scope).
- Users page (paginated).
- Logs page.
- Notifications page (webhooks).
- Server credentials vault (encrypted at rest).
- Backup + verify task (Phase 47).
- Backup retention policy (Phase 90).
- Manual collections CRUD.
- Marker editor (per-show intro/outro overrides).
- Maintenance window UX.
- Hot-reload of worker pool size via settings PATCH.
- Per-kind transcoder concurrency overrides.

## Real-time / observability

- Server-sent events for: scan progress, job state changes, activity
  events, marker detection completion.
- Activity feed in admin showing recent events + alerts.
- Disk-space alerts per library root.
- Slow-query warnings (sqlx alert thresholds).

## Operational

- Single Docker image, single supervised process.
- Operator-configured PRAGMA cache_size (V1; V2 equivalent in
  `STORAGE.md`).
- Credential vault with encrypted-at-rest secrets.
- Scheduled `scan_library` task with both `on_change` and periodic
  modes.
- Bulk admin operations.
- TMDB API key configured via admin UI.
- ffmpeg version + capability probe on startup.

## Frontend specifics

- Responsive layout (mobile + desktop).
- iOS safe-area support.
- Accessibility: focus trap on dialogs (`useFocusTrap`), arrow nav in
  menus, Card focus rings, aria-live regions.
- Image onError fallbacks.
- ConfirmDialog primitive used consistently for destructive ops.
- ServerSettings design-system primitives: Pill, SettingsCard,
  SaveBar, Drawer, HeroCard, FilterChip, AdminTabBar.
- Error boundaries on rails.
- Skeleton loaders.
- Not-found page (404).
- Empty states (history, search).
- 500 error visibility (regression-tested 2026-05-22).

---

## Explicitly cut from V2

> Add entries here when V2 deliberately drops a V1 feature, with a
> one-line rationale.

*(empty as of plan date — to be populated as RFC work surfaces cuts)*

## Open questions for requirements

- Should V2 keep chromaprint fingerprint-based intro detection, or is
  the chapter-first short-circuit + tacet audio analysis (V1 Phase A)
  sufficient? Decide in `SCANNER.md`.
- Does V2 ship with watch history import from V1, or fresh-start
  only? See `SCHEMA.md` for the user-data migration question.
- Casting feature surface — V1 has device detection + improved UI;
  worth re-evaluating fundamentals (Cast SDK vs. native picker)
  before porting. Decide in `FRONTEND.md`.
- Email delivery (transactional, for confirmations + invites) — V1
  uses an external SMTP config. Carry forward as-is or consider an
  embedded delivery solution. Likely carry forward; flag in `AUTH.md`.
