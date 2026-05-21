# Contributing to ChimpFlix

ChimpFlix is an open-source, self-hosted media server. The project is in
**pre-v0.1 active development** — the API, schema, and module boundaries are
still moving. Please open an issue or discussion before starting a large
contribution so we can align on direction.

## Code of conduct

This project follows the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/).
Be excellent to each other.

## Repo layout

```text
chimpflix/
├── Cargo.toml          # Rust workspace root
├── crates/             # Rust backend crates
│   ├── server/         # axum HTTP + WebSocket server (the binary)
│   ├── library/        # FS scanner, SQLite schema, library DB accessors
│   ├── metadata/       # TMDB (and future) metadata agents
│   ├── transcoder/     # ffmpeg/ffprobe orchestration
│   └── common/         # shared types and helpers
├── web/                # Next.js frontend (Netflix-style UI)
├── docker/             # Dockerfile(s)
├── docs/               # design docs (ARCHITECTURE, SCHEMA, API)
└── docker-compose.yml  # two-service compose: server + web
```

## Toolchain

- **Rust**: pinned via `rust-toolchain.toml` — `rustup` will pick up the
  right channel automatically. Stable, with `rustfmt` and `clippy`.
- **Node**: 22.x for the `web/` frontend.
- **ffmpeg / ffprobe**: required at runtime for the transcoder crate.
  Tests that touch transcoding require both to be on `PATH`.

## Building & running

```bash
# Rust workspace
cargo build --workspace
cargo run -p chimpflix-server          # listens on :8080

# Web frontend (separate)
cd web
npm install
npm run dev                            # listens on :3000

# Full stack via Docker
docker compose up --build
```

The server creates `./data/chimpflix.db` on first run and runs migrations.
Delete the directory to start fresh.

## Style

- **Rust**: `cargo fmt` is non-negotiable in CI; `cargo clippy --all-targets
  -- -D warnings` must pass. No `unwrap()` / `expect()` in non-test code
  unless the panic is genuinely impossible and documented.
- **TypeScript**: `npm run lint` must pass. Prefer typed errors at API
  boundaries; client components should be small and serializable-prop.
- **SQL**: schema changes go in a new timestamped migration. Never edit a
  past migration that has shipped in a release; migrations are append-only
  after v0.1.0.
- **Commits**: small, focused. Imperative subject ("add scan progress
  events"), wrapped at 72 chars, body explains *why* not *what*.

## Tests

- **Unit tests**: `cargo test --workspace`.
- **Integration tests** for the server use a temporary SQLite file under
  `TMPDIR` and run migrations fresh per test. Avoid sharing state.
- **Frontend tests** are not set up yet (v0.2). Manual testing is the
  current baseline.

## Pull requests

- Open a PR against `main`. CI must be green.
- For non-trivial changes, link the issue or discussion you opened first.
- Include a short test plan in the PR body — what you verified locally.
- Squash before merge unless the commit history is genuinely useful.

## Security

If you find a security issue, do **not** open a public issue. Email the
maintainers (see SECURITY.md, when added) or use GitHub's private
security advisories.

## License

By contributing you agree your work will be licensed under Apache 2.0
(see [LICENSE](LICENSE)). No CLA — the Developer Certificate of Origin
implicit in the Apache 2.0 contribution clause is sufficient.
