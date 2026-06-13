# ChimpFlix Architecture (v0.1 draft)

> Status: **design draft**. This describes the target shape for v0.1, not what
> exists today. The current repo is the Next.js frontend that talks to Plex.
> The pivot replaces "Plex" with a new Rust backend that lives in the same
> repo.

## Goal

An open-source, self-hosted media server. Single Docker image, one process
tree, manages a video library on disk, serves it to a Netflix-style web UI
shipped in the same image. Permissive license (Apache 2.0). No relationship
to Plex, Jellyfin, or Emby — net-new code and API.

**v0.1 in one sentence:** a user can drop video files in a folder, invite a
friend, and both of them can browse, play (direct or transcoded), and resume
playback on any browser.

## Process model

One Docker container, one supervised process tree:

```text
┌─ chimpflix-server (Rust, axum) ──────────────────────────────┐
│  • HTTP API + WebSocket on :8080                             │
│  • Serves the bundled Next.js standalone build              │
│  • Spawns ffmpeg / ffprobe subprocesses for transcoding     │
│  • Owns the SQLite library DB                                │
└──────────────────────────────────────────────────────────────┘
         │
         ├─► ffmpeg (subprocess, per active HLS session)
         ├─► ffprobe (subprocess, on demand during library scan)
         └─► TMDB API (HTTPS, metadata)
```

The Next.js app is built as a static `out/` (or `standalone/` server). The
Rust server either serves the static files directly or reverse-proxies to a
sidecar `node` process — decided in Phase 2. Default plan: **static export**,
served by axum directly, no Node.js at runtime. The Next.js app uses client
components and talks to `/api/v1` for everything dynamic, so a static export
is feasible.

If a feature later requires Next.js server components (e.g. server-side
TMDB-side rendering for SEO of public catalog pages), we revisit and run
`next start` as a sidecar.

## Crate layout (cargo workspace)

```text
chimpflix/
├── Cargo.toml                 # workspace root
├── crates/
│   ├── server/                # axum app, routes, auth, sessions, WS hub
│   ├── library/               # FS scanner, file→DB ingest, watcher, schema
│   ├── metadata/              # TMDB agent + trait for future agents
│   ├── transcoder/            # ffprobe analysis, ffmpeg HLS supervisor
│   └── common/                # shared types, error, config, telemetry
├── web/                       # the existing Next.js app
├── docker/
│   ├── Dockerfile             # multi-stage: rust+node builder → runtime
│   └── entrypoint.sh
├── docs/
└── docker-compose.yml
```

**Why these crate boundaries:**

- `server` is the only crate that knows about HTTP/WS. Everything else is a
  library of pure functions and async services it can drive.
- `library` and `transcoder` are the two most likely places to grow heavy
  logic and the most likely places contributors will work in isolation —
  separating them keeps PRs reviewable.
- `metadata` is its own crate because v0.2+ will add TVDB, AniDB,
  MusicBrainz behind the same trait. Keeping it isolated forces the trait
  abstraction from the start.
- `common` exists to break circular dep risk (errors, config, IDs).

**What deliberately doesn't get its own crate yet:**

- No separate `auth` crate — it's small and lives in `server`.
- No `api-types` crate — for v0.1, types live in `server` and the frontend
  duplicates the small set it needs. Splitting out a shared types crate is
  premature; revisit when third-party clients show up.

## Runtime model

- Single `tokio` multi-thread runtime, default worker count.
- HTTP request handlers are pure async.
- Blocking work (sqlx queries, file I/O for streaming) uses sqlx's async
  drivers and `tokio::fs` — no explicit `spawn_blocking` in hot paths.
- Long-running jobs (library scan, transcode session) run as spawned tokio
  tasks owned by a "supervisor" struct that lives for the process lifetime.
- ffmpeg/ffprobe are **OS subprocesses** managed via `tokio::process::Command`.
  No FFI to libav — too painful, breaks on every distro upgrade, and we don't
  need the perf.

## State

- **SQLite** is the source of truth for everything: users, sessions,
  libraries, items, files, play state, markers.
  - Single file: `${DATA_DIR}/chimpflix.db`
  - WAL mode, `synchronous=NORMAL`, `foreign_keys=ON`.
- **Filesystem**: media lives wherever the user mounted it; we never move or
  rewrite original files. Generated artifacts (extracted subtitles, scaled
  posters, HLS segment cache) live under `${DATA_DIR}/cache/`.
- **No external Redis / Postgres / message broker.** A self-hosted media
  server should run from a single binary + a data dir.

## Request lifecycles

### 1. Browsing API request

```text
Browser ──HTTP──► axum router
                    │
                    ├─ middleware: session cookie → user_id
                    ├─ middleware: rate limit (per-IP + per-user)
                    ├─ handler: SQL query (sqlx)
                    └─ response: JSON
```

Typical latency budget: **< 20ms** for cached library queries. SQLite
queries dominate; aim for indexed lookups everywhere on the browse path.

### 2. Direct play

```text
Browser ──HTTP Range──► /api/v1/stream/:file_id/direct
                          │
                          ├─ auth + access check
                          ├─ resolve file_id → absolute path
                          └─ tokio::fs::File + Range header
                              → streamed via axum's body stream
```

No transcoding, no buffering in our process. Bytes go disk → kernel → socket.

### 3. HLS transcode session

```text
Browser ──POST /sessions──► start session (returns session_id + variants)
        ──GET master.m3u8─►
        ──GET variant.m3u8─► (server writes manifest pointing at upcoming segs)
        ──GET seg-N.ts────►
                            │
                            └─ session supervisor
                                ├─ ffmpeg subprocess writes segments
                                │   to ${DATA_DIR}/cache/sessions/<id>/
                                ├─ stream the requested seg from disk
                                ├─ broadcast progress over WS
                                └─ GC sliding window: drop segments
                                    < (currentSeg - 30) to bound disk use
```

A session has:

- A media file ID + target ladder rung (resolution/bitrate).
- A starting time offset (changes on seek; the supervisor restarts ffmpeg
  with `-ss` and renumbers segments).
- A live keepalive — if no segment fetched for 60s, session is reaped.

Concurrency: each user can have multiple concurrent sessions (one per active
device). Server-wide cap configurable, default = number of CPU cores for SW
transcode, higher with hardware accel.

### 4. WebSocket

```text
Browser ──WS upgrade──► /api/v1/ws
                          │
                          ├─ auth from cookie (no token in query)
                          ├─ register with central Hub
                          ├─ client sends subscribe {topic: ...}
                          └─ Hub fans out events from publishers
                              (library scanner, transcode supervisor, etc.)
```

Topics are scoped per user (so a user only sees events relevant to their
play sessions, their library access, etc.). A small set of global topics
(server health, owner-only scan progress) exist for admin UI.

## Security model (v0.1)

- All `/api/v1` routes require a valid session cookie except:
  `POST /auth/setup` (only if no users exist), `POST /auth/login`,
  `POST /auth/register` (only with valid invite code).
- Cookies: httpOnly, Secure (when behind TLS), SameSite=Lax, signed with
  HMAC over a stable secret.
- CSRF: same Origin-check middleware the current proxy uses, ported to Rust.
- Subprocess hardening: ffmpeg invoked with arg vectors only (no shell), no
  user-controllable strings in command line. Filenames are passed by file
  descriptor where possible.
- Password hashing: argon2id, sane defaults.
- Rate limiting: per-IP for unauthenticated routes, per-user for auth'd.

## Out of scope for v0.1

Listed so reviewers can stop suggesting them in PRs:

- Music library, photo library.
- Live TV / DVR.
- Mobile apps (the web UI is responsive; native apps are post-v1).
- Plugin system.
- Sync-to-device / offline downloads.
- Federated / remote streaming over the public internet (LAN + reverse-proxy
  works; we don't ship a relay).
- Hardware-accelerated transcoding (Phase 4 will detect it; first cut is
  software transcode only).
- Public catalog SEO / non-authenticated browsing.

## Open design questions

These are deliberately unresolved and will be decided in implementation:

1. **HLS session cleanup on browser close.** Browsers don't reliably send
   close events. Plan: keepalive heartbeats over the WS connection; reap
   sessions whose owning WS has been gone > 60s. Fallback: idle timeout.
2. **Hardware accel detection.** Plan: run `ffmpeg -hwaccels` at startup,
   probe each with a tiny test transcode, cache the result. Phase 4.
3. **Image pipeline.** Posters from TMDB come in fixed sizes; we want
   on-the-fly resize for the various card sizes the frontend uses. Likely
   `image` crate + an LRU disk cache.
4. **Search.** SQLite FTS5 is sufficient for v0.1 (titles, summaries, cast).
   No external search index needed.
5. **Schema migrations.** Use `sqlx::migrate!` with timestamped SQL files
   in `crates/library/migrations/`. Lock file-format compatibility before
   v0.1.0 tag.
