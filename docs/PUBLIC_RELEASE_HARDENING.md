<!-- markdownlint-disable MD024 -->
<!-- Tier sections (BLOCK / WEEK 1 / MONTH 1) deliberately share parallel
     "Issue / Why / Fix / Effort" sub-headings so each item reads the
     same way. MD024 (no duplicate headings) is disabled for the whole
     doc. -->

# Public-release hardening plan

Created 2026-05-24 from a three-agent audit (security, resource
exhaustion, operational) covering "what blows up if this ships to
real users on a public IP tomorrow." Findings ranked by likelihood ×
severity and grouped into three tiers by when they need to be done.

This document is the source of truth for the hardening work. When an
item ships, append a "shipped" note in-place rather than rewriting it,
so the reasoning trail stays visible (same pattern as
[PERF_PLAN.md](./PERF_PLAN.md)).

---

## Threat model

"Public release" here means a self-hosted instance behind a reverse
proxy, on a public DNS hostname, with invite-only registration. The
realistic threat profile:

- **Bot probes** within hours of DNS propagation — login brute force,
  registration spam, known-CVE scanners, exposed-admin probes.
- **Invited users** sharing the URL with extended friends/family
  beyond the invite recipient — session-URL leaks via screen-share,
  Discord, browser referrer.
- **Single bad media file** triggering an unbounded ffmpeg
  allocation — most common DoS vector for transcoder-fronted apps.
- **Operational drift** over weeks: disk fill, expired Trakt tokens,
  vault key forgotten before the first restore attempt.

NOT in scope (yet):

- Hostile authenticated user with admin role — admin-vs-owner
  hierarchy is already enforced ([[project-admin-role-tier]]).
- DDoS amplification — fronted by a CDN or reverse proxy with its
  own rate limit; we focus on what the app itself can do.
- Compliance frameworks (SOC 2, GDPR data export, etc.) — separate
  project.

---

## Tier 0: BEFORE the world finds the repo (paperwork + self-rescue)

Eight items, ~5.5 hours total. These need to land before the repo URL
is shared anywhere — they shape what a newcomer sees when they look
at the project for the first time, and what an operator can do when
something breaks. Tier 0 is "paperwork the technical audit didn't
catch"; the BLOCK tier below is "code changes for things that crash
on day 1."

### 0.1 No SECURITY.md / vuln disclosure path

**File:** `SECURITY.md` (new)

**Issue:** [CONTRIBUTING.md:87-88](../CONTRIBUTING.md#L87-L88) already
says "see SECURITY.md, when added" but the file doesn't exist. The
first person to find a vulnerability has nowhere to report it
privately and will open a public issue.

**Why pre-release:** disclosure path matters from the moment the repo
is reachable, not from the moment someone uses the running server.

**Fix:** Add `SECURITY.md` with:

- A non-public contact (email + optional PGP key).
- Supported-versions table.
- Response SLA ("acknowledged within 7 days").
- Explicit "no telemetry / no phone-home" statement.

**Effort:** ~30m.

**Shipped 2026-05-24:** [SECURITY.md](../SECURITY.md) covers GitHub
private advisories as primary channel, supported-versions table,
response SLA, no-telemetry commitment, scope exclusions.

### 0.2 No CODE_OF_CONDUCT.md (referenced, not present)

**File:** `CODE_OF_CONDUCT.md` (new)

**Issue:** `CONTRIBUTING.md` references Contributor Covenant 2.1 but
the file isn't checked in.

**Fix:** Add Contributor Covenant 2.1 verbatim with the project's
contact email in the enforcement section.

**Effort:** ~10m.

**Shipped 2026-05-24:** [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md)
verbatim Contributor Covenant 2.1 with GitHub PSA as the
enforcement-report channel. CONTRIBUTING.md updated to link to the
local file.

### 0.3 No CHANGELOG.md

**File:** `CHANGELOG.md` (new)

**Issue:** semantic-release is wired in CI but produces no human-
readable changelog at the root. Day-1 users can't see what shipped
in v0.1 versus what's still in flight.

**Fix:** Initial `CHANGELOG.md` in keep-a-changelog format capturing
v0.1 scope; document that subsequent releases should append.

**Effort:** ~20m.

**Shipped 2026-05-24:** [CHANGELOG.md](../CHANGELOG.md) initial entry
captures v0.1 scope; `[Unreleased]` section documents this hardening
pass. Format is keep-a-changelog 1.1.0.

### 0.4 No GitHub issue / PR templates

**File:** `.github/ISSUE_TEMPLATE/`, `.github/PULL_REQUEST_TEMPLATE.md`
(new)

**Issue:** Bug reports come in without playback environment, server
version, browser, or logs. Every triage round-trips with "what
version are you on?"

**Fix:** Bug report + feature request issue forms (YAML); PR
template covering tests/types/lint/screenshot.

**Effort:** ~30m.

**Shipped 2026-05-24:** issue forms (`config.yml`, `bug_report.yml`,
`feature_request.yml`) under `.github/ISSUE_TEMPLATE/` and
`.github/PULL_REQUEST_TEMPLATE.md`. Blank issues disabled; security
reports routed to PSA. Bug template enforces version / deployment /
browser / logs fields.

### 0.5 No deployment runbook

**File:** `docs/DEPLOYMENT.md` (new)

**Issue:** `README.md` covers local dev + docker compose, but
operators exposing the server to the public internet have to figure
out reverse proxy + trusted-proxy + TLS themselves. This is exactly
the WEEK 1 #8 silent-misconfig trap.

**Fix:** `docs/DEPLOYMENT.md` with worked examples for Caddy, nginx,
Traefik; trusted-proxy CIDR guidance; TLS + HSTS notes; "before
exposing publicly" checklist that links back to this hardening doc.

**Effort:** ~1h.

**Shipped 2026-05-24:** [docs/DEPLOYMENT.md](DEPLOYMENT.md) — preflight
checklist, Caddy / nginx / Traefik recipes (with WebSocket + HLS
timeout caveats), trusted-proxy anti-patterns, Cloudflare Tunnel +
Tailscale + bare-metal notes, upgrade procedure.

### 0.6 No operator self-rescue CLI

**File:** [crates/server/src/main.rs](../crates/server/src/main.rs)
(subcommand parsing)

**Issue:** If the sole owner loses their password, loses 2FA, or
rotates the vault key incorrectly, the only path back today is direct
SQLite edits. These pair with BLOCK #1 (vault decoupling) as the most
likely panic scenarios. When the operator is mid-incident is exactly
the wrong time to teach them the schema.

**Fix:** Add subcommands to the server binary:

- `chimpflix-server owner-password-reset --email <addr>` — prompts
  for new password, sets it, optionally clears 2FA.
- `chimpflix-server owner-2fa-reset --email <addr>` — clears TOTP
  secret + recovery codes for that owner.
- `chimpflix-server vault-rotate --old-key-env <NAME> --new-key-env <NAME>`
  — decrypt with old, re-encrypt with new, write back.

All three require the live server to be stopped (file lock on the DB
dir) so they don't race.

**Effort:** ~2h.

**Shipped 2026-05-24:** [crates/server/src/cli/](../crates/server/src/cli/)
adds dispatch in `mod.rs` + three subcommand handlers
(`owner_password.rs`, `owner_twofa.rs`, `vault_rotate.rs`). Wired
into `main.rs` before tracing init. Reads passwords via `rpassword`
(no echo) and keys from named env vars (not flag values). README
documents the three commands.

### 0.7 No telemetry / browser-support statement in README

**File:** `README.md`

**Issue:** Privacy-minded self-hosters can't tell whether the server
phones home without `grep`-ing the codebase. Mobile users opening
issues against unsupported browsers eat triage time.

**Fix:** Add two short sections to README:

- "Privacy" — server makes outbound calls only to TMDB / AniList /
  Trakt / OpenSubtitles when the operator configures them; no
  analytics, no auto-update check.
- "Supported browsers" — Chromium 120+, Firefox 120+, Safari 17+
  (desktop + iOS). HLS playback in older Safari is best-effort.

**Effort:** ~15m.

**Shipped 2026-05-24:** README adds three sections — Privacy,
Supported browsers, Operator self-rescue — plus links to SECURITY,
DEPLOYMENT, CHANGELOG, and this hardening doc from the Documentation
section.

### 0.8 No release-time smoke + migration matrix

**File:** `.github/workflows/ci.yml` (extend)

**Issue:** CI runs unit tests + clippy + the Docker image build, but
never proves the image *boots* and applies migrations cleanly from a
prior schema. A bad migration ships green.

**Fix:** Add a job that, for each tagged release:

1. Boots the new image against a fresh empty `data/`.
2. Boots against a `data/` snapshot of the *previous* release's
   migrated DB.
3. Hits `/api/v1/ready` (BLOCK #2) and a few GET endpoints.

**Effort:** ~1h.

**Shipped 2026-05-24:** `.github/workflows/ci.yml` adds a `smoke` job
that boots the freshly-built server image against a temp data dir
and waits for `/health` (switch to `/ready` once BLOCK #2 lands).
Tagged-release runs additionally replay the previous release's image
against a shared data dir, then boot the new image to prove
migrations apply cleanly. First release skips gracefully (no prior
tag).

---

## Tier 1: BLOCK before public release

Five items, ~5.5 hours total. Without these, day-1 footguns are real.

### 1. Backup/vault decoupling silently bricks restores

**File:** `crates/server/src/api/admin/backup.rs`

**Issue:** `VACUUM INTO` snapshots the encrypted `chimpflix.db` but
not the vault key (`CHIMPFLIX_SECRET_KEY` env var). An operator who
restores from backup without preserving the key gets a database
where every encrypted secret — SMTP password, Trakt tokens, session
HMAC, TOTP secrets — is unrecoverable. The server boots without an
error but every secret-dependent flow silently fails.

**Why blocking:** restore-from-backup is exactly the path operators
take when something has gone wrong. They will be panicking. The time
to discover the vault-key coupling is NOT mid-incident.

**Fix:**

- Boot-time check: try decrypting one known encrypted column. If it
  fails AND the column was non-empty in the backup, exit with a
  loud "vault key mismatch — restore needs the same
  CHIMPFLIX_SECRET_KEY as the source server" message.
- Surface the backup-vs-vault coupling in `/admin/backups` UI:
  banner above the backup list, doc link in the row-level Restore
  dialog.
- Verify-backups task ([[project_finish_pass_2026_05_18]]) should
  attempt a sample decrypt against the operator's current vault
  key and flag mismatches in the task log.

**Effort:** ~1h.

**Shipped 2026-05-24:** new `queries::vault_self_test` samples one
encrypted row (across `secrets` → `webhooks` → `user_totp` in that
order) and returns one of `NoEncryptedRows` / `Ok` / `Mismatch`.
Boot path in `main.rs` calls it after `ensure_default_user`: a
mismatch combined with a `chimpflix.db.pre-restore-*.db` sibling
exits with `EX_CONFIG (78)` and a recovery script; without the
sibling it WARNs and continues so a rotated key doesn't wedge the
server. `verify_backups_task` runs the same check per backup file,
surfacing per-file mismatches as informational log entries (does
not fail the task). `/admin/backups` list response gains
`vault_key_required: bool`; `AdminBackupRestoreClient` renders an
amber banner when true. CLI tooling for the recovery path (key
rotation) lives in Tier 0.6.

### 2. `/health` is shallow and routes traffic to broken servers

**File:** [crates/server/src/api/health.rs:26-35](../crates/server/src/api/health.rs#L26-L35)

**Issue:** Returns `{"status":"ok"}` after 1ms boot. Doesn't probe
the DB pool, ffmpeg availability, or vault decryption. Docker
compose's healthcheck currently points at `/health`, so a server
with a broken DB or missing ffmpeg binary will be marked healthy
and the proxy will route real traffic to it.

**Why blocking:** the whole point of a healthcheck is to fail
closed during a partial outage. An always-green check is worse
than no check at all — operators trust it and skip deeper
debugging.

**Fix:**

- Add `GET /api/v1/ready` that runs:
  - `SELECT 1` against the pool (DB up + pool not exhausted)
  - `ffmpeg -version` (binary present + executable)
  - Decrypt one sample secret (vault key valid)
- Update `docker-compose.yml` healthcheck to hit `/ready`, not
  `/health`. Keep `/health` as the cheap "process is alive" probe
  for upstream load balancers that need a sub-millisecond check.

**Effort:** ~30m.

**Shipped 2026-05-24:** `GET /api/v1/ready` returns a 200 with per-
component status (`database` via `SELECT 1`, `ffmpeg` via
`{bin} -version`, `vault` via `vault_self_test`) or 503 when any
check fails. `DegradedReason::NoEncryptedRows` is treated as ready,
not failed — fresh installs without secrets still answer 200. The
Dockerfile + docker-compose healthcheck now point at
`/api/v1/ready` with `start_period: 30s` to absorb migration time;
`/health` stays at the root for upstream LBs. CI smoke + migration
matrix already poll `/api/v1/ready` (Tier 0 item 0.8).

### 3. No ffmpeg per-session resource limits

**File:** `crates/transcoder/src/session.rs` (Command spawn site)

**Issue:** ffmpeg subprocesses are spawned with bare
`Command::new(ffmpeg)` — no `RLIMIT_AS`, no cgroup, no
memory cap. A single malformed media file can trigger an unbounded
allocation; three concurrent encodes of pathological content can
OOM the host. The operator's `transcoder_max_concurrent` setting
caps process count but not per-process resource use.

**Why blocking:** this is the most common DoS vector for any
transcoder-fronted service. A friend uploads a corrupted episode
and the box dies. No active attacker required.

**Fix:**

- Wrap spawn with `systemd-run --scope --property MemoryMax=2G
  --property CPUQuota=200%` when running under systemd.
- Fallback for non-systemd hosts: `Command::pre_exec` setting
  `setrlimit(RLIMIT_AS, 2 * 1024^3)` so the kernel kills the
  ffmpeg subprocess before it gets the whole box.
- Operator setting `transcoder_per_session_memory_mb` (default
  2048) so high-end deployments can raise it for 4K HEVC.

**Effort:** ~2h.

**Shipped 2026-05-24:** new `crates/transcoder/src/rlimit.rs`
applies `setrlimit(RLIMIT_AS, ...)` via the `pre_exec` hook of the
spawned ffmpeg `Command`. Wired into the long-running session spawn
in `spawn_ffmpeg` (`session.rs`); short-lived probes intentionally
skip the cap. `FfmpegConfig::session_memory_mb` reads
`CHIMPFLIX_TRANSCODER_MEMORY_MB` at boot. Documented in
`.env.example`. The doc's `systemd-run` option was dropped in
favour of in-process `setrlimit` — works the same on systemd hosts
and on plain `docker run`, no host-dependency on systemd
availability. Typed `server_settings` field deferred until
operators ask for runtime tuning.

**Default changed 2026-05-24 (same-day revision):** initial default
of 2048 MiB broke every GPU-accelerated session — `cuInit()` mmaps
tens of GiB of unified-memory virtual address range *before*
allocating anything, so a 2 GiB `RLIMIT_AS` makes libcuda return
`CUDA_ERROR_OUT_OF_MEMORY` immediately. New default is **0
(disabled)**, with documentation that CPU-only operators can opt
in (2048 / 4096 depending on workload) and GPU operators should
rely on cgroup memory limits (`docker compose mem_limit:`) for
the same defense. Lesson: `RLIMIT_AS` is too blunt for GPU
workloads because virtual-address-space reservations are an
order of magnitude larger than actual residency.

### 4. Backup directory grows unbounded

**File:** [crates/server/src/api/admin/backup.rs](../crates/server/src/api/admin/backup.rs)

**Issue:** Daily backup task writes to `<data_dir>/backups/auto/`
with no retention cap. Over weeks the directory fills the
partition. When the disk hits 100%, live transcodes start writing
truncated segments and the SQLite WAL stops checkpointing —
catastrophic compound failure.

**Why blocking:** "the disk filled because backups never get
pruned" is the canonical operations footgun. Days-or-weeks-to-fail
on a typical install, but the failure cascades into data
corruption rather than a clean error.

**Fix:**

- Add `backup_retention_count` server setting (default 14).
- Daily backup task, after writing the new snapshot, list
  `auto/`, sort by mtime descending, delete entries past
  the retention count.
- Surface remaining count + total bytes in `/admin/backups` so
  the operator notices retention pressure.

**Effort:** ~1h.

**Shipped 2026-05-24:** new migration `phase90_backup_retention.sql`
adds `server_settings.backup_retention_count` (default 14, clamped
to 0..=365 in the PATCH handler). `backup_db` scheduled task calls
new `prune_old_backups` after each VACUUM INTO — lists `chimpflix-*.db`
under `backups/auto/`, sorts newest-first by mtime, deletes the
tail past the cap. 0 disables pruning (loudly documented in the
field doc). `/admin/backups` response gains `retention_count` and
the UI renders "N of M retained" / "(retention disabled)" so
operators see pressure before the next prune. UI editor (input +
Save button next to the retention counter) added to
`AdminBackupRestoreClient`. The transcoder per-session memory cap
stays env-var only (`CHIMPFLIX_TRANSCODER_MEMORY_MB`) because
`RLIMIT_AS` is applied at subprocess spawn — a typed runtime
setting would have the same restart-required UX as the env var.

### 5. Setup-mode CSRF bypass window

**File:** [crates/server/src/api/auth.rs:116-127](../crates/server/src/api/auth.rs#L116-L127)

**Issue:** `POST /auth/setup` skips Origin/Referer validation while
`is_in_setup_mode()` returns true. Between server boot and the
operator completing first-run setup, anyone reachable on the
network can claim the owner account with attacker-controlled
credentials. If DNS propagates before the operator logs in, an
internet bot scanning common admin paths could win the race.

**Why blocking:** ownership of the owner account is total
ownership of the system. The setup window is short but the cost
of losing the race is unbounded.

**Fix:** pick the approach that fits your deployment model:

- **Bind to 127.0.0.1** until setup completes. Operator does
  first-run setup over SSH tunnel or local browser; server only
  starts listening on `0.0.0.0` after the owner exists.
- **Setup token env var** (`CHIMPFLIX_SETUP_TOKEN=<random>`): the
  `/auth/setup` request must include a header matching the env
  var, which the operator generated when standing up the server.

**Effort:** ~1h.

**Shipped 2026-05-24:** `enforce_setup_token` runs at the top of
the `setup` handler. Three branches: token set → require constant-
time-compared `X-Setup-Token` header; token unset on
`APP_PUBLIC_ORIGIN=https://...` → refuse with an actionable
validation error (mirrors the plaintext-vault refusal in
`load_vault`); token unset on LAN/dev → allow (current behaviour
preserved). Documented in `.env.example`; the
[`docs/DEPLOYMENT.md`](DEPLOYMENT.md) preflight section already
calls out the recommended approach.

---

## Tier 2: WEEK 1 (visible defense, before traffic grows)

Five items, ~8 hours total. These keep the surface honest as real
users + sustained bot probes show up.

### 6. Login brute-force across many IPs

**File:** [crates/server/src/api/rate_limit.rs:47-50](../crates/server/src/api/rate_limit.rs#L47-L50)

**Issue:** Per-IP cap is 10 req/min + 5 burst. A 100-IP botnet
gets 500 free password guesses per minute against any single
username. Per-username `AttemptTracker` only activates after 5
failures so the first 4 per IP are free.

**Fix:** Lower per-IP login burst from 5 to 2. Lower per-username
activation threshold from 5 to 3. Lengthen lockout from 30s to
60min.

**Effort:** ~2h (mostly testing).

**Shipped 2026-05-24:** `auth_limiter` burst 5→2. `AttemptTracker::backoff_for`
schedule rewritten: 0..=2 → no lock, 3..=5 → **60min**, 6..=9 →
6h, 10+ → 24h (was 0..=4 → 0, 5..=7 → 30s, 8..=11 → 5min, 12+ →
30min). Updated unit tests cover both the lowered activation
threshold and the 24h escalation; `cargo test -p chimpflix-server
rate_limit` is green.

### 7. SQLite pool size 8 too small for public load

**File:** `crates/library/src/db.rs` (pool config)

**Issue:** Pool capped at 8 connections. Combined with 30s
`busy_timeout`, 50 concurrent users running normal mixed read
traffic exhaust the pool and queue behind it. Result: every
queued request waits up to 30s before timing out.

**Fix:** Bump to 24-32. Profile actual p95 connection hold time
under load before going higher (over-pooling hurts SQLite write
contention).

**Effort:** ~30m.

**Shipped 2026-05-24:** [crates/library/src/db.rs](../crates/library/src/db.rs)
pool `max_connections` 8 → 24. The migrate pool stays at 1
(single-connection migration is intentional). 32 considered but
the WAL writer serialises so the marginal cost of going higher
hurts write contention more than it helps reads. Bumping further
should be motivated by actual /metrics observation once that
endpoint ships (WEEK 1 #10).

### 8. Trusted-proxy misconfig is silent

**File:** [crates/server/src/main.rs:350-362](../crates/server/src/main.rs#L350-L362)

**Issue:** If the operator forgets `TRUSTED_PROXIES`, the server
uses the Docker bridge IP as the "client IP" for rate limiting +
audit logging. All users share one rate-limit bucket; audit logs
attribute everything to the proxy. Boot warning exists but is
easy to miss.

**Fix:** Add `/admin/network` diagnostic showing currently-trusted
CIDRs + the detected peer IP. Banner on the admin home if
PEER ∈ RFC1918 — gives the operator a "your proxy config is
broken" signal they actually see.

**Effort:** ~1.5h.

**Shipped 2026-05-24:** `NetworkResponse` gains a `proxy_diagnostic`
block with `trusted_proxies` (CIDR strings as parsed), `peer_ip`
(the immediate TCP peer from `ConnectInfo`), `peer_is_private`
(RFC1918 / RFC4193 / loopback / link-local), and `looks_misconfigured`
(`peer_is_private && !trusted_proxies.contains(peer)`).
`AdminNetworkClient` renders an amber banner above the form when
`looks_misconfigured` is true, with the actual peer IP, the current
TRUSTED_PROXIES value, and a link to `docs/DEPLOYMENT.md#trusted-proxy-anti-patterns`.

### 9. Transcode session capacity check race

**File:** `crates/server/src/api/stream.rs` (`create_session`)

**Issue:** Check `current >= max_concurrent` then call `start()`
is two non-atomic operations. Under burst load, N simultaneous
session requests all see `current < max` and each start. The
operator's cap is silently violated.

**Fix:** Replace counter-and-check with `tokio::sync::Semaphore`
sized to `transcoder_max_concurrent`. Acquire a permit before
spawning ffmpeg; drop on session end.

**Effort:** ~1h.

**Shipped 2026-05-24:** `TranscodeManager` gains a `start_gate:
tokio::sync::Mutex<()>` plus `lock_start_gate()` accessor. The
`PlayMode::Transcode` arm of `create_session_impl` holds the gate
across "read max → check current → call `start`," making the
check+spawn atomic. Chosen over a true `Semaphore` because the
operator-tunable cap is hot-reloaded (a `Semaphore::add_permits` /
shrink dance is awkward); a brief serialising mutex on the
session-start hot path is both simpler and sufficient — sessions
start at single-digit-per-second rates even in heavy use, so the
serialisation cost is negligible. Keepalive / cancel / status
paths do not contend on the gate.

### 10. No metrics endpoint

**Issue:** Post-launch the operator has zero visibility into job
counts, active sessions, request latency, DB pool stats. First
incident, they're flying blind.

**Fix:** Add `GET /metrics` (Prometheus exposition format):

- Job counts by `kind` × `status`
- Active streaming sessions (gauge)
- HTTP request count + latency histogram by route
- DB pool: active connections, idle, waiting
- SQLite WAL size
- Backup retention status (count, oldest age, total bytes)

**Effort:** ~3h.

**Shipped 2026-05-24:** new `crates/server/src/api/metrics.rs`
hand-rolls the Prometheus exposition format (no extra crate
dep) and exposes uptime, pool size + idle, active sessions,
jobs grouped by kind × status, backup count + bytes + oldest-
age, and SQLite WAL bytes. Mounted at `GET /metrics` (root,
unauth — operators gate at the reverse proxy). Per-route HTTP
request counters + latency histograms deferred: they need a
tower middleware that records every request, and operators
without a Prometheus scraper get no value out of the cost.
Add once the first deployment asks for it.

### 11. External-fetch response sizes unbounded

**Files:** `crates/server/src/subtitles_lookup.rs`,
`crates/metadata/src/**`, anywhere `reqwest::get(...).bytes()` is
called against TMDB/AniList/Trakt/OpenSubtitles.

**Issue:** Upstream metadata + subtitle responses are read into memory
with no upper bound. A hostile or broken upstream returning a 10 GB
body hangs the worker and exhausts memory. Pairs with the discovery
pipeline silent-drops item (Tier 3) — both are "we trusted the
outside world too much."

**Fix:** A small `bounded_fetch` helper that caps response size
(default 4 MiB for metadata, 8 MiB for subtitles) + a 30s read
timeout. Route every outbound call site through it.

**Effort:** ~1.5h.

**Shipped 2026-05-24 (partial):** OpenSubtitles subtitle download
upgraded from a buffer-then-check (which let the full body land
in memory before bailing) to a streaming `bytes_stream()` loop
that aborts the moment the accumulator exceeds the 10 MiB cap.
Metadata-API JSON reads (TMDB, AniList, Trakt, OMDb, TVDB,
TVMaze) keep their existing 15–20s per-request timeouts; those
upstream APIs return tiny payloads so the hostile-large-body
threat is theoretical without a confirmed exploit path. Migrating
every `.json()` / `.text()` call site through a uniform
`bounded_fetch` helper is tracked as a MONTH 1 follow-up.

### 12. WebSocket message size + per-connection memory unbounded

**File:** [crates/server/src/api/ws.rs](../crates/server/src/api/ws.rs)

**Issue:** Server reads and broadcasts inbound WS messages with no
per-message size or per-connection memory cap. One client can send a
multi-megabyte frame, or thousands of small frames faster than the
broadcaster drains. The Tier 3 per-user WS connection cap addresses
*how many*, not *how big*.

**Fix:** Configure the axum WebSocket upgrade with
`max_message_size = 64 KiB`, `max_frame_size = 16 KiB`, and a bounded
per-connection send buffer (drop the connection when full rather than
queuing unboundedly in the broadcaster).

**Effort:** ~1h.

**Shipped 2026-05-24:** axum `WebSocketUpgrade::max_message_size(64
KiB)` and `max_frame_size(16 KiB)` pinned in the upgrade handler.
axum 0.8 defaults are 16 MiB / 16 MiB. The broadcaster's per-conn
buffer is already bounded by the tokio broadcast channel's
`Lagged` semantics (slow consumer gets `RecvError::Lagged(n)` and
warned rather than backing up unboundedly), so no further change
needed on the outbound side.

### 13. Session ID not regenerated on login (fixation defense)

**File:** [crates/server/src/api/auth.rs](../crates/server/src/api/auth.rs)
(login handler)

**Issue:** Explore-agent audit (2026-05-24) found session-token
issuance on successful login but no explicit invalidation of any
pre-existing session cookie. Classic session fixation: an attacker
plants a known session cookie on a victim's browser, waits for them
to log in, then reuses the cookie.

**Fix:** In the login handler, before issuing the new session,
unconditionally clear any existing session cookie + invalidate the
matching row server-side if one exists. Verify by booting + curling
with a fabricated cookie pre-login.

**Effort:** ~30m.

**Shipped 2026-05-24:** new `invalidate_inbound_session_if_any`
helper parses the inbound session cookie via
`crate::auth::cookie::parse_value`, and when the HMAC matches a
real session id, deletes the row via `queries::delete_session`.
Centralised inside `issue_session` so every login-success path
(`login`, `oauth_complete`, `accept_invite`,
`confirm_password_reset`, `complete_setup`) gets the defense
without each handler having to remember. Best-effort failures
(bad HMAC, expired session, missing cookie) silently no-op so a
first-time login isn't blocked.

---

## Tier 3: MONTH 1 (hardening, when there's time)

Items below were drained as part of the 2026-05-24 hardening pass.
Status notes inline; "shipped" entries follow the same convention
as the higher tiers.

- **Per-user WebSocket connection cap (5 max).** A malicious or
  broken client can open 100+ WS connections; each one fans out
  events.
  **Shipped 2026-05-24:** `AppState::try_acquire_ws_connection` /
  `release_ws_connection` track per-user counts in a
  `RwLock<HashMap>`; ws upgrade handler claims a slot (cap 5)
  before accepting and a `ConnCountGuard` RAII releases on every
  exit path. Over-cap upgrades return 429.

- **Person filmography pagination.** `list_items_for_person`
  returns all items, no LIMIT.
  **Shipped 2026-05-24:** `list_items_for_person` now takes
  `limit` (default 50, clamped 1..=200) + `offset`. New
  `count_items_for_person` for total. `PersonDetail` response
  carries `total` / `page` / `page_size` for the UI.

- **Search query result LIMIT.** FTS5 bm25 with no upper bound
  scans the whole `items_fts`.
  **Shipped 2026-05-24:** COUNT(*) wrapped in `SELECT 1 ... LIMIT
  10000` subquery — FTS bm25 walks at most 10k matches and the UI
  shows "10000+" when capped, which is all pagination needs.

- **`subtitle_fetch_task` enqueues all items every run.**
  **Shipped 2026-05-24:** per-run cap of 500 items, cursor
  persisted in `secrets` table per library (`subtitle_fetch_cursor_library_{id}`
  / `subtitle_fetch_cursor_all`). Wraps back to 0 when the table
  exhausts so successive runs walk the whole library.

- **Trakt refresh-token expiration UI warning.**
  **Shipped 2026-05-24:** Trakt status response gains
  `expiring_soon` (within 10 days of `expires_at`) and `expired`
  (already past). `SettingsIntegrationsClient` renders an amber
  "your link is about to silently expire" / red "your link has
  expired" message.

- **Disk-full backup pre-check.** Before `VACUUM INTO`, require
  1.2× current DB size free on the partition.
  **Shipped 2026-05-24:** `preflight_disk_space` uses `statvfs`
  on Unix to check free bytes ≥ 1.2× current DB size, returning
  the new `ApiError::InsufficientStorage` (HTTP 507) on shortfall.
  No-op on non-Unix; let the OS surface the error there.

- **Owner self-session-revoke guard.** Sole owner using "revoke
  all my sessions" locks themselves out.
  **Shipped 2026-05-24:** `revoke_my_session` refuses the call
  when `session_id == user.session_id && user.role == Owner &&
  count_owners() <= 1`, with a message pointing the operator at
  the role-promotion path or the new
  `chimpflix-server owner-password-reset` CLI (Tier 0.6).

- **Password-reset SMTP-unconfigured loud failure.**
  **Shipped 2026-05-24:** `request_password_reset` runs the SMTP
  build before any other work and returns a 400 validation error
  ("Email isn't configured on this server. Ask the administrator
  to set up SMTP under Admin -> Server -> Email, then try again.")
  when no mailer is available.

- **HLS session URLs bound to user token.** Today HLS segment
  URLs (`/api/v1/stream/sessions/{id}/{variant}/{name}`) are
  position-only scoped — sharing a player URL via Discord
  paste-with-referrer leaks the session.
  **Confirmed fine 2026-05-24:** re-audit found
  `ensure_session_accessible` already rejects
  `session.user_id != user.id && !user.role.is_admin_or_owner()`,
  so segments require a valid session cookie matching the
  originating user. The `__Host-` cookie is HttpOnly + SameSite
  and isn't sent via Discord paste-with-referrer / open-graph
  scrapers, so a leaked URL alone doesn't grant access. The
  per-session token variant in the original doc would have been
  redundant.

- **Discovery pipeline silent drops on overflow.** Channel
  capacity bounded but drops only log at DEBUG.
  **Confirmed fine 2026-05-24:** `jobs/pipeline.rs:104` already
  logs at `warn!` with both `media_file_id` and `capacity`
  context. The original doc was based on an older revision; no
  change needed.

---

## Confirmed fine (do not re-flag in future audits)

Items the audit looked at and found solid. Listed here so the next
pass doesn't waste cycles re-investigating:

- **Password-reset tokens are atomic single-use.** [`find_active_password_reset_token`](../crates/library/src/queries.rs#L3106) filters `consumed_at IS NULL`; [`consume_password_reset`](../crates/library/src/queries.rs#L3130) updates with `WHERE consumed_at IS NULL` inside a transaction and bails on `rows_affected() == 0`.
- **`/admin/users` is OwnerAuth-gated.** Handler has `_owner: OwnerAuth` extractor in [crates/server/src/api/auth.rs](../crates/server/src/api/auth.rs).
- **Body size limits exist.** [crates/server/src/api/mod.rs:90](../crates/server/src/api/mod.rs#L90) (auth: 16KB) + [crates/server/src/api/mod.rs:629](../crates/server/src/api/mod.rs#L629) (default: 16MB).
- **Pagination is clamped.** `page_size.clamp(1, 200)` + `history limit.clamp(1, 500)`.
- **CSRF is comprehensive.** SameSite + Origin/Referer + double-submit tokens.
- **SSRF defense for webhooks** blocks RFC 1918, loopback, link-local, cloud metadata IPs.
- **Security headers present:** HSTS (when HTTPS), CSP, X-Frame-Options, CORP, COEP.
- **Invite codes are cryptographically sound** (`getrandom`-backed, SHA-256 hashed).
- **Per-kind job concurrency caps** prevent runaway backfill from starving live transcodes.
- **First-scan exclusivity gate** prevents SQLite contention from drowning a fresh library import.
- **Password hashing is Argon2id with OWASP 2024 params** (`m=64MiB, t=3, p=1`, OsRng-backed salt). See [crates/server/src/auth/password.rs](../crates/server/src/auth/password.rs).
- **Session tokens are HMAC over a 32-byte `OsRng` nonce** (not the row id), so tokens are unforgeable without the server's session secret.
- **HTTP read + body-size timeouts present:** 60s `TimeoutLayer` on non-streaming routes (slowloris defense), body size limits already listed above.
- **Graceful shutdown is wired** to SIGTERM via axum `with_graceful_shutdown` (though in-flight transcodes are not drained — see Tier 3).
- **No telemetry / phone-home in the codebase.** Only outbound calls are operator-configured (TMDB, AniList, Trakt, OpenSubtitles).
- **Session cookie flags survey** (verified 2026-05-24, [crates/server/src/auth/cookie.rs](../crates/server/src/auth/cookie.rs)):
  - Session cookie: `__Host-cf_session` over HTTPS (forces `Path=/` + `Secure` at the browser level), `cf_session` over plain HTTP. Always `Path=/`, `HttpOnly`, `SameSite=Lax`, `Max-Age=…`; `Secure` is added when HTTPS is detected.
  - CSRF double-submit companion (`__Host-cf_csrf` / `cf_csrf`): same scope + `Secure` policy, intentionally **not** `HttpOnly` (the client must read it and echo in `X-CSRF-Token`).
  - HMAC signature over `{session_id}:{nonce_hex}` keyed by `session_secret` — tampered values fail to parse.
