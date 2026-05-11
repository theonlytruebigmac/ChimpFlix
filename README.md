# ChimpFlix

A self-hosted, open-source media server with a Netflix-style web UI.
Apache 2.0 licensed.

> **Status: pre-v0.1 active development.** The Rust backend was scaffolded
> on 2026-05-11 and replaces an earlier Plex-frontend incarnation of this
> repo. The four MVP pillars (library scan, direct-play + watch state,
> HLS transcoding, multi-user auth) are not yet wired up — see
> [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the plan.

## What ChimpFlix is (and is not)

ChimpFlix is a brand-new media server: net-new code, no API compatibility
or shared lineage with Plex, Jellyfin, Emby, or any other media server.
The goal is a fast, efficient, Rust-based backend with a polished Netflix-
style web UI, packaged for Docker, and easy to self-host.

**v0.1 scope:**

- Library scan + TMDB metadata for movies and TV shows.
- Direct-play streaming + per-user watch state.
- HLS transcoding via ffmpeg.
- Multi-user authentication and per-library access control.

**Not in v0.1:** music libraries, photos, Live TV / DVR, plugin system,
mobile apps, sync-to-device, remote relay streaming, hardware-accelerated
transcoding (detected but not yet used).

## Repo layout

```text
chimpflix/
├── Cargo.toml              # Rust workspace
├── crates/
│   ├── server/             # axum HTTP + WebSocket server (the binary)
│   ├── library/            # FS scanner, SQLite schema, library DB
│   ├── metadata/           # TMDB and future metadata agents
│   ├── transcoder/         # ffmpeg/ffprobe orchestration
│   └── common/             # shared types and helpers
├── web/                    # Next.js frontend (the Netflix-style UI)
├── docker/                 # Dockerfile(s)
├── docs/                   # ARCHITECTURE, SCHEMA, API
└── docker-compose.yml      # two-service compose: server + web
```

## Quick start (local dev)

You'll need Rust (stable, picked up automatically via `rust-toolchain.toml`),
Node 22, and `ffmpeg` + `ffprobe` on `PATH` (only needed once transcoding
lands in Phase 4).

```bash
# Backend
cargo run -p chimpflix-server
# → listening on 0.0.0.0:8080
# → curl http://127.0.0.1:8080/health
#   { "status": "ok", "version": "0.1.0-dev", "uptime_s": 3 }

# Frontend (separate terminal)
cd web
npm install
npm run dev
# → open http://localhost:3000
```

The backend creates `./data/chimpflix.db` on first run and applies all
migrations. Delete the `data/` directory to start fresh.

## Docker

```bash
mkdir -p ./data
docker compose up -d --build
open http://localhost:3000
```

[docker-compose.yml](docker-compose.yml) builds two images from the same
multi-stage [docker/Dockerfile](docker/Dockerfile): `chimpflix-server` (the
Rust binary + ffmpeg, on :8080) and `chimpflix-web` (the Next.js standalone
build, on :3000). They share a compose network; only `:3000` is exposed
to the host by default.

## Configuration

The backend reads a small set of environment variables:

| Variable | Default | Purpose |
| --- | --- | --- |
| `BIND_ADDR` | `0.0.0.0:8080` | Listening address. |
| `DATA_DIR` | `./data` | Where `chimpflix.db` and caches live. |
| `RUST_LOG` | `info,sqlx=warn` | `tracing-subscriber` filter. |

Configuration for the (currently Plex-pointing) web frontend lives in
[web/.env.example](web/.env.example) — it will be reworked when Phase 2
swaps the frontend over to the new backend.

## Development

```bash
cargo build --workspace           # all crates
cargo run -p chimpflix-server     # the binary
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

CI runs `fmt`, `clippy`, `cargo test`, the Next.js build, and both Docker
image builds on every PR. See [.github/workflows/ci.yml](.github/workflows/ci.yml).

## Documentation

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — system shape, crate
  boundaries, process model, request lifecycles.
- [docs/SCHEMA.md](docs/SCHEMA.md) — SQLite schema for v0.1.
- [docs/API.md](docs/API.md) — REST endpoints and WebSocket event catalog.
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to build, style, PR process.

## License

[Apache 2.0](LICENSE). See [NOTICE](NOTICE) for attribution requirements.
