# Admin guide

Operator-facing reference for the settings exposed in `/settings/admin`. The
in-app forms carry their own short help text; this doc is the longer "why does
this exist / what range is sane / what happens if I set it wrong" reference.

If you're a developer adding a new setting: declare its bounds in
[`crates/server/src/api/admin/settings.rs`](../crates/server/src/api/admin/settings.rs)
in the `validate()` function so the PATCH endpoint rejects bad values before
they ever land in the DB. Add a row to the matrix below in the same change.

---

## Settings matrix

<!-- markdownlint-disable MD060 -->
<!-- The Notes column has variable-width content; column-style alignment
     fights us more than it helps when adding new rows. -->

| Setting key                                | Type      | Bounds              | Default          | Notes                                                                                                                          |
| ------------------------------------------ | --------- | ------------------- | ---------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `preroll_path`                             | string    | single filename     | unset            | Single filename component only — no `..`, no slashes, no leading dot. Resolved under `data/preroll/`.                          |
| `secure_connections`                       | enum      | `required`/`preferred`/`disabled` | `preferred` | `required` blocks all plain HTTP. `preferred` allows but warns. `disabled` permits everything (LAN-only setups).               |
| `transcoder_hw_accel`                      | enum      | `auto`/`none`/`vaapi`/`nvenc`/`qsv`/`videotoolbox`/`amf` | `auto` | `auto` probes for a hwaccel; falls back to software. Pin explicitly if you have multiple GPUs.                                  |
| `transcoder_encoder_preset`                | enum      | `speed`/`balanced`/`quality` | `balanced` | Used by on-the-fly streams. Background optimization uses `transcoder_background_preset` separately.                            |
| `transcoder_hw_strictness`                 | enum      | `auto`/`prefer_hw`/`require_hw` | `auto` | `require_hw` fails the session if hwaccel can't satisfy. Use when you want to detect hwaccel breakage instead of silent fallback. |
| `transcoder_background_preset`             | enum      | libx264 preset name | `medium`         | Slower = smaller files, longer optimize jobs. `ultrafast`/`superfast` are bad ideas for retained files.                        |
| `transcoder_max_background_concurrent`     | int       | 1–16                | 1                | Background optimize/loudness/marker jobs. Hot-reloaded. Don't exceed physical cores.                                            |
| `transcoder_max_concurrent`                | int       | 1–64                | 8                | Live transcode session cap (per-server). Each session = one ffmpeg process. Increase if you have lots of concurrent viewers.   |
| `transcoder_quality_ceiling_kbps`          | int?      | 100–200000          | unset            | When set, no session goes above this bitrate. Use to throttle WAN bandwidth.                                                    |
| `transcoder_hdr_tonemap_algo`              | enum      | `hable`/`reinhard`/`mobius`/`bt2390`/`clip`/`linear` | `hable` | Algorithm for SDR fallback from HDR sources. `hable` is the safe default.                                                       |
| `job_workers`                              | int       | 1–16                | 2                | Background job worker count. Hot-reloaded (no restart). More workers = faster queue drain, more SQLite contention.             |
| `job_kind_concurrency`                     | JSON map  | per-kind 1–32       | `{}`             | Per-kind caps. Example: `{"detect_markers_file":4}` runs up to 4 marker jobs in parallel. Hot-reloaded.                        |
| `cors_origins`                             | JSON array | http(s) origins only | `[]`             | Used for both CORS allow + CSRF origin validation. `*` is rejected at write time (credentialled-wildcard is a misconfig).      |
| `extras_json`                              | JSON obj  | object              | `{}`             | Free-form key-value bag for experimental settings. Don't depend on its shape in code.                                          |
| `public_url`                               | URL       | http(s)             | unset            | Origin used for absolute URLs in webhooks, share links, emails. Must include scheme.                                            |
| `email_smtp_security`                      | enum      | `starttls`/`tls`/`none` | `starttls`    | `none` should only be used for `localhost` relay.                                                                                |
| `email_smtp_port`                          | int       | 1–65535             | unset            | Convention: 465 = TLS, 587 = STARTTLS, 25 = plain.                                                                              |
| `email_smtp_host`                          | string    | ≤253 chars, no WS   | unset            |                                                                                                                                |
| `email_smtp_username`                      | string    | ≤256 chars          | unset            | Leave blank for anonymous relay.                                                                                                |
| `email_from_address`                       | string    | local@domain ≤320   | unset            | Light email-shape validation; lettre re-validates at send time.                                                                 |
| `email_from_name`                          | string    | ≤128 chars          | unset            | Display name shown alongside the from address.                                                                                  |
| `totp_enforcement`                         | enum      | `disabled`/`optional`/`required` | `optional` | `required` forces every login to have a TOTP setup. Lock yourself out → recover via env-var owner reset.                       |
| `maintenance_window_start`                 | HH:MM     | 00:00–23:59         | `02:00`          | 24-hour local time. Heavy maintenance jobs only run inside the window.                                                          |
| `maintenance_window_end`                   | HH:MM     | 00:00–23:59         | `06:00`          | Set start == end to disable the window entirely.                                                                                |
| `continue_watching_max_items`              | int       | 1–200               | 30               | Items shown on the home-page CW rail.                                                                                           |
| `continue_watching_max_age_weeks`          | int       | 0–520               | 12               | Drop CW entries older than this. 0 = never drop.                                                                                |
| `video_played_threshold_pct`               | int       | 50–99               | 90               | Auto-mark watched when playback reaches this % of duration.                                                                     |
| `database_cache_size_mb`                   | int       | 0–4096              | 0 (SQLite default) | SQLite page cache. Higher = fewer disk hits, more RAM use. ~128 MiB is a good default for medium libraries.                    |
| `transcoder_reaper_idle_threshold_ms`      | int       | 5000–3600000        | 90000            | Idle session age before the reaper kills it. 5s minimum prevents race with the 60s keepalive. **Restart required.**            |
| `max_remote_streams_per_user`              | int       | 0–64                | 0 (unlimited)    | Counts streams from outside `lan_networks`. Set non-zero to throttle remote bandwidth per user.                                 |
| `lan_networks`                             | CIDR list | comma-separated     | unset            | Example: `192.168.0.0/16, 10.0.0.0/8`. Validated at write; bad input rejected before save.                                      |
| `auth_bypass_cidrs`                        | CIDR list | comma-separated     | unset            | Matching IPs skip cookie auth and run as the server owner. Use sparingly — only for trusted LAN automation.                    |
| `bind_interface`                           | socket    | host:port           | env `BIND_ADDR`  | Pin the listener to a NIC. Empty honors env. **Restart required.**                                                              |
| `backup_retention_count`                   | int       | 0–365               | 7                | Daily snapshots kept on disk. 0 disables retention pruning entirely.                                                            |

<!-- markdownlint-enable MD060 -->

---

## Endpoints worth alerting on

The server exposes machine-readable health/metrics for upstream monitoring.

- **`/api/v1/health`** — process uptime. 200 = process alive. Bare-minimum docker healthcheck. Unauthenticated.
- **`/api/v1/ready`** — deep readiness. Returns 503 if **any** of: DB unreachable, ffmpeg missing, vault self-test fails, or **any library_paths root is unreadable**. Point your load balancer's drain probe here. Unauthenticated.
- **`/metrics`** — Prometheus exposition format. Includes:
  - `chimpflix_uptime_seconds`
  - `chimpflix_db_pool{state="size|idle"}`
  - `chimpflix_active_sessions`
  - `chimpflix_jobs{kind, status}` — queue depths by kind/status
  - `chimpflix_backups{stat="count|bytes|oldest_age_seconds"}`
  - `chimpflix_sqlite_wal_bytes` and `chimpflix_sqlite_db_bytes` — DB growth
  - `chimpflix_disk_bytes{state="total|used|free"}` — filesystem hosting the data dir
  - `chimpflix_http_requests_total{route, method, status}` — request counters
  - `chimpflix_http_request_duration_seconds_sum{route, method, status}` — cumulative latency
- The metrics endpoint is unauthenticated by design; gate it at your reverse proxy if the host is internet-exposed.

### Suggested alerts

- `chimpflix_disk_bytes{state="free"} < 10 GiB` — disk fill catches mid-transcode.
- `chimpflix_sqlite_wal_bytes > 256 MiB` — WAL bloat (long-running reader blocking checkpoint).
- `up{job="chimpflix"} == 0 for 2m` — process down (paired with `/ready`).
- `chimpflix_jobs{status="dead"} > 0` — operator intervention needed; owners also get an inbox notification per kind.

---

## Operator notifications

Server owners (role = `Owner`) receive inbox notifications on:

- `user.registered` — invitee finished signup
- `user.2fa.disabled` — a user turned off their own 2FA
- `user.2fa.reset` — an admin reset another user's 2FA
- `job.failed` — a background job exhausted retries and went terminal

`job.failed` only fires on the dead-transition edge — retried failures stay
quiet. The notification includes the job kind, last error, and a link to the
activity page. Mute per-kind from **Settings → Account → Notifications**.

---

## Backup + restore

- Snapshots live under `<data_dir>/backups/auto/`. The `backup_db` scheduled
  task writes one daily during the maintenance window; retention pruning runs
  after each snapshot. Keep the matching `CHIMPFLIX_SECRET_KEY` backed up off-box
  — restoring against a different key bricks every encrypted credential.
- The Restore button stages the chosen snapshot as the next-boot database. The
  actual restore happens on server restart; your current DB is preserved as
  `chimpflix.db.pre-restore-<timestamp>.db`. The admin UI polls `/healthz` after
  staging and tells you the moment the server comes back online.

---

## When changing a setting needs a restart

Settings that affect _process startup_ rather than per-request behaviour are
flagged in the admin UI with a "Restart pending" pill. The two currently are:

- `transcoder_reaper_idle_threshold_ms` (read at process start)
- `bind_interface` (the listener socket is bound once during boot)

Every other setting is hot-applied — including `job_workers`,
`job_kind_concurrency`, and `transcoder_max_background_concurrent`.
