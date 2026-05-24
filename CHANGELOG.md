<!-- markdownlint-disable MD024 MD004 -->
<!-- MD024 disabled: keep-a-changelog uses repeated "Added/Changed/..."
     section headings per release. MD004 disabled: nested sub-bullets
     under top-level "- " items render the same way regardless of the
     marker style; enforcing a single style across levels just churns
     the file. -->

# Changelog

All notable changes to ChimpFlix are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once it reaches v1.0. Before then, breaking changes can land in any
minor release (`0.y.z`); the changelog will call them out explicitly.

## [Unreleased]

### Added

- **Public-release hardening pass.** See
  [docs/PUBLIC_RELEASE_HARDENING.md](docs/PUBLIC_RELEASE_HARDENING.md)
  for the full plan and per-item shipped notes.

#### Operator-facing surface

- New CLI subcommands on the server binary for incident recovery:
  - `chimpflix-server owner-password-reset --email <addr> [--clear-2fa]`
  - `chimpflix-server owner-2fa-reset --email <addr>`
  - `chimpflix-server vault-rotate --old-key-env <NAME> --new-key-env <NAME>`
- New environment variables (all optional):
  - `CHIMPFLIX_SETUP_TOKEN` — first-run setup gate; REQUIRED when
    `APP_PUBLIC_ORIGIN` starts with `https://`.
  - `CHIMPFLIX_TRANSCODER_MEMORY_MB` — per-session ffmpeg
    `RLIMIT_AS` cap (default 2048, 0 to disable).
- New `server_settings.backup_retention_count` (default 14, range
  0..=365) — auto-prunes `<data_dir>/backups/auto/` after each
  daily snapshot. 0 disables pruning.
- New HTTP endpoints:
  - `GET /api/v1/ready` — deep readiness (DB + ffmpeg + vault).
    Pointed at by the docker-compose healthcheck.
  - `GET /metrics` — Prometheus exposition format (unauth; gate at
    the reverse proxy).
- New admin diagnostic on `/admin/server/network` — surfaces
  `TRUSTED_PROXIES`, the detected peer IP, and a "your proxy config
  is broken" banner when the peer is in RFC1918 but not covered by
  the trusted list.
- `/admin/backups` UI now shows a vault-key-required banner when
  encrypted secrets are present, plus "N of M retained" so
  operators see retention pressure.
- Settings → Integrations now warns when Trakt link is within 10
  days of expiry or already expired.

#### Internal hardening (no operator action required)

- Boot-time vault decoupling check refuses to start when a restored
  DB cannot be decrypted with the current key (paired with a
  `chimpflix.db.pre-restore-*.db` sibling).
- ffmpeg session subprocesses can run under `setrlimit(RLIMIT_AS)`
  on Unix to bound runaway allocations. **Default is disabled (0)**
  because CUDA's `cuInit` reserves a huge virtual address range up
  front; CPU-only operators can opt in via
  `CHIMPFLIX_TRANSCODER_MEMORY_MB=2048`.
- Login brute-force tightened: per-IP burst 5 → 2; per-username
  lockout schedule rewritten to 3 failures → 60 min, 6 → 6 h, 10 →
  24 h.
- SQLite pool 8 → 24 connections.
- Transcode session-start race fixed via a `start_gate` mutex on
  `TranscodeManager`; cap is now enforced atomically.
- WebSocket upgrades pinned to 64 KiB max message / 16 KiB max
  frame; per-user connection cap of 5.
- Session-fixation defense: `issue_session` invalidates any
  pre-existing cookie before minting the new one.
- Sole-owner self-revoke guard prevents the only owner from
  revoking their own current session.
- Password-reset returns a 400 validation error when SMTP isn't
  configured instead of silently no-opping.
- Disk-full pre-check on `POST /admin/backups` returns 507 when
  the data partition has less than 1.2× the current DB size free.
- OpenSubtitles download switched to a streaming bounded read
  that aborts mid-stream if the 10 MiB cap is exceeded.
- Person filmography paginated (default 50, max 200 per page).
- Search `COUNT(*)` capped at 10 000 rows (FTS bm25 no longer
  walks the full virtual table on huge libraries).
- `subtitle_fetch` scheduled task now caps per-run enqueue at 500
  items and cursor-paginates across ticks.

## [0.1.0] — 2026-05-18

First feature-complete release of the v0.1 scope. Pre-1.0, so
operators should treat the upgrade path as best-effort and back up
before pulling new images.

### Added

- **Library scan** for movies, TV, and anime, with a multi-agent
  metadata pipeline (TMDB, TVDB, TVMaze, AniList, OMDb) and a
  capability matrix for each agent.
- **HLS transcoding** via ffmpeg, including HEVC, ABR ladders,
  hardware encode/decode (NVENC, VAAPI, QSV, VideoToolbox, AMF),
  HDR→SDR tonemap, two-pass loudness normalization, and burned
  or sidecar subtitle paths.
- **Multi-user authentication** with optional 2FA (TOTP + recovery
  codes), per-library access control, and a three-tier role
  hierarchy (Owner > Admin > User).
- **Netflix-style web frontend** with continue-watching, on-deck,
  collections, smart collections, person/cast pages, search with
  FTS5 bm25 ranking, history, and a polished player (resume pill,
  skip-intro, chapter menu, audio/subtitle selection).
- **Admin surface** with onboarding wizard, scheduled-task scheduler
  + maintenance windows, webhooks, audit log, encrypted backup +
  restore, library health dashboard, bulk operations, and per-kind
  job concurrency caps.
- **Trakt two-way integration:** scrobble (start/pause/stop),
  watchlist sync, collection push, ratings push, Coming Soon and
  Upcoming Movies rails, recommendations, calendar variants, lists
  as rails, and a Favorites read-only rail.
- **Plex OAuth** for invite-only signup and account linking
  (password-less Plex-only users supported).
- **Intro / credits detection** via the in-house `tacet-core` crate
  (audio fingerprint matching across episodes in a season), plus
  embedded-chapter short-circuit, blackframe fallback, and operator
  marker editor.
- **First-scan exclusivity gate** to avoid SQLite contention from
  drowning the initial scan of a large library.

### Known limitations

- No music libraries, photos, Live TV / DVR, plugin system, mobile
  apps, sync-to-device, or remote-relay streaming.
- No GDPR-style user data export endpoint (planned for v0.2).
- HLS playback in older Safari is best-effort; see "Supported
  browsers" in the README.

[Unreleased]: https://github.com/soybigmac/ChimpFlix/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/soybigmac/ChimpFlix/releases/tag/v0.1.0
