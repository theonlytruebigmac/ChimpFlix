# Plex Parity Plan

Captured 2026-05-18 after a full app sweep + comparison against Plex's
Settings → {Scheduled Tasks, Transcoder, Library, Network} screens.
Companion to [ARCHITECTURE.md](ARCHITECTURE.md) and the project roadmap
memory; this doc is the concrete settings-parity backlog.

## Context: what just shipped (Option A "tighten" pass)

These were the bugs/quality issues identified by the parallel-agent
audit before this plan was written. All resolved before drafting the
parity work below:

### Player

- **MediaSession lock-screen seek** now routes through `seekBy` (was
  mutating `video.currentTime` directly, which ignored the source-time
  offset on transcode sessions — back-10s could land on source-time 0
  instead of the user's intended position). Uses a late-bound
  `seekByRef` to avoid the TDZ that a direct reference would cause.
- **Stall watchdog** no longer fires `currentTime += 0.001` while
  the user is mid-scrub or `video.seeking` — prevents the visible
  yank during a manual drag.
- **Auto-skip-intro** suppressed during scrub — dragging through an
  intro region no longer pulls the playhead to the credits-end mark.
- **Triple-tap suppression** now timeouts after 500 ms so a held
  `suppressNextClickRef` can't accidentally swallow an unrelated
  later tap.
- **Mobile tap UX**: single tap on the video toggles the controls
  overlay instead of pausing (industry-standard mobile behavior).
  Mouse-click still toggles play/pause. Detected via `pointerType`.
- **WakeLock + MediaSession API** wired so the screen stays awake
  during playback and lock-screen controls appear.
- **`-nostdin` added to the ffmpeg cmdline** — root cause of the
  "ffmpeg silently dies after ~50 s with stderr_tail showing only
  init warnings" bug. With stdin=/dev/null and no `-nostdin`, ffmpeg
  was treating the EOF on stdin as a user-quit signal.

### Server

- **Reaper window** shrunk from 5 min to **90 s** (polled every 15 s).
  Paused sessions now reap based on `last_seen` freshness; the
  previous "skip paused sessions entirely" policy leaked GPU resources
  when mobile users backgrounded the app.
- **Session struct `Drop` impl** logs `"session struct dropped"` so
  operator can correlate ffmpeg deaths with our own kill paths.
- **`waitpid(WNOHANG)`** in the stderr-drain task captures ffmpeg's
  exit code/signal — fixes the "ffmpeg died but we don't know why"
  diagnostic gap.
- **`sendBeacon` for unload teardown** (with `POST /stream/sessions/
  {id}/close` route as a DELETE alias) — fires reliably during PWA
  force-close, where `fetch+keepalive` can be dropped.
- **`pagehide` DELETE** restored on mobile but only when
  `event.persisted === false`. The earlier blanket-disable was over-
  cautious — Chrome on mobile fires pagehide only on real unload, not
  on app-switch.
- **`Mutex::lock().unwrap()`** in `trakt.rs` (3 sites) and unsafe
  cookie-header construction in `auth.rs` replaced with graceful
  recovery (`unwrap_or_else(|e| e.into_inner())` and `if let Ok(hv)`
  respectively).
- **File watcher MPSC channel** bounded to 4096 (was unbounded) so a
  rsync sweep or mass-rename can't OOM the server. Periodic re-sync
  every 30 s recovers any dropped events.

### Frontend

- **React purity violations** removed: `Math.random()` in `useRef`
  init → `useId()`; `Date.now()` in `useState` init → 0 (replaced on
  first dashboard response).
- **`setState-in-effect`** patterns fixed across `AdminMobileNav`,
  `Login`, `prefs.ts` (now uses `useSyncExternalStore`),
  `EditMetadataDialog` (added cancellation guard), and the player's
  subtitle hydration (documented exception — localStorage as
  external store).
- **Missing useCallback dep** `subtitleOffsetMs` added to the prewarm
  callback.
- **TitleModalShell** got a focus trap (Tab/Shift+Tab cycle through
  focusable elements inside the modal), focus restoration on close,
  and `role="dialog" aria-modal="true"`. Keyboard users can no longer
  tab into the background.
- **Error boundaries** wrap every home-page rail. One broken rail
  data source no longer collapses the whole page.
- **`AdminLogsClient`** ref-assignment-in-render moved into a
  `useEffect`.

### Server: Continue Watching dedupe

`on_deck` now collapses multiple in-progress episodes of the same show
into one tile — the most recently played episode. Was showing N tiles
of the same series; Netflix-style behavior is one tile per show.

---

## Plex parity plan (4 phases)

User specifically flagged Scheduled Tasks as the worst current UX
("not intuitive and hard to follow"). The rest of this doc maps each
Plex settings surface against what ChimpFlix has today and what's
missing, organized as discrete shippable phases.

### Phase 1 — Scheduled Tasks rework ✅ SHIPPED (2026-05-18)

**Why first:** explicitly user-flagged. Cron expressions in the admin
UI are operator-hostile and most ops users will never customize them.

**What landed:**

- Migration `20260518050000_phase29_task_frequency.sql` adds
  `scheduled_tasks.frequency` (enum) + `requires_maintenance_window`
  (bool) and `server_settings.maintenance_window_start` / `_end`
  (HH:MM, server-local). Backfills existing cron rows to the closest
  frequency enum value and flags the heavy seed tasks as
  window-eligible.
- `scheduler::compute_next_run` is the new single source of truth for
  next-firing math (frequency + cron + window). `manual` and
  `on_change` return a 2100 sentinel so they never auto-fire.
  `snap_to_maintenance_window` handles wraparound windows
  (`22:00→06:00`). 12 unit tests cover all the branches.
- `TaskKindInfo` registry now carries `default_frequency` and
  `default_requires_maintenance_window` so the "New task" form
  pre-fills sensibly per kind.
- `AdminTasksClient.tsx` rewritten: cards show a friendly schedule
  summary instead of a raw cron string, the edit drawer is a
  frequency dropdown + window checkbox + (advanced) cron field.
  Window editor lives at the top of the page with an "Active now"
  badge.
- Validation: `/admin/tasks` rejects unknown frequencies;
  `/admin/settings` validates HH:MM strings.

**Plex's model:**

- One **maintenance window** (start time + end time, e.g. 02:00–09:00)
  where background tasks are allowed to run.
- Each task has a simple **frequency** label ("every 3 days",
  "weekly", "as a scheduled task", "when media is added") and a
  toggle for whether it runs during the maintenance window vs
  immediately on demand.
- No cron strings exposed.

**ChimpFlix today:** every task has its own cron expression. Powerful
but unfriendly.

**Plan:**

- Add `maintenance_window_start: text` and `maintenance_window_end: text`
  (HH:MM strings) to `server_settings`. Default 02:00 → 09:00 like Plex.
- Replace per-task cron with two new columns on `scheduled_tasks`:
  - `frequency: text` enum — `manual | hourly | daily | every_3_days | weekly | monthly | on_change`
  - `requires_maintenance_window: bool` (default `false` for fast
    tasks, `true` for the expensive ones like full library scan or
    deep media analysis)
- Scheduler computes `next_run_at` from `frequency + last_run_at`,
  then if `requires_maintenance_window` is true, snaps the next run
  forward to the next window opening.
- Keep raw cron as an "Advanced (custom schedule)" toggle for power
  users — most ops won't see it.
- Admin UI overhaul:
  - One card per task with: name, description, frequency dropdown,
    "in maintenance window" toggle, **last run** (success/fail badge
    + elapsed-since), **next run** (computed), **Run now** button.
  - Top of page: window picker + a single "active now / next window
    in 4h" indicator.
- Migration: convert existing cron expressions to the closest
  frequency enum value; preserve original cron in a `custom_cron`
  column for the advanced opt-in.

**Estimate:** ~1 day.

### Phase 2 — Transcoder settings completeness ✅ SHIPPED (2026-05-18)

**What landed:**

- Migration `20260518060000_phase30_transcoder_extras.sql` adds 4
  new `server_settings` columns:
  - `transcoder_background_preset` (libx264 preset enum, default
    `veryfast`) — wired to `optimize_one`, replacing the hard-coded
    value.
  - `transcoder_max_background_concurrent` (default 1) — gates the
    `optimize_versions` task's effective batch size so heavy
    background re-encodes can't starve live transcodes.
  - `transcoder_hdr_tonemap_enabled` (default true) — operator opt-out
    of HDR → SDR tonemap during reencode.
  - `transcoder_hdr_tonemap_algo` (default `hable`) — algorithm
    passed to ffmpeg's `tonemap=tonemap=<algo>` filter.
- New `TonemapConfig` struct in the transcoder crate is the single
  builder for the HDR filter chain. Threaded through
  `TranscodeManager::start` → `spawn_ffmpeg`; the hard-coded chain
  in `session.rs` is gone. 4 new unit tests cover the chain (SDR
  bypass, disabled bypass, algorithm injection, legacy-default
  byte-for-byte regression guard).
- `GET /admin/transcoder/capabilities` now also returns
  `cache_root`, surfaced in the Engine "Capability detail" drawer
  as a read-only path (with a hint that it's bound by
  `TRANSCODER_CACHE_DIR` env and needs a restart to change).
- `AdminTranscoderClient` gains two new sections: **Background
  transcoding** (x264 preset dropdown + concurrency cap) and **HDR
  tone mapping** (enable toggle + algorithm picker with hints per
  algorithm). Both flow through the existing `/admin/settings`
  PATCH path.
- `/admin/settings` validates: preset name ∈ {ultrafast, …, slower};
  concurrency ∈ [1, 16]; algo ∈ {hable, reinhard, mobius, bt2390,
  clip, linear}.

**Follow-on phases shipped (2026-05-18):**

- HEVC encoding mode — Phase 43. Full end-to-end pipeline:
  per-codec encoder selection across all 6 hwaccel variants,
  forced fMP4 container, `-tag:v hvc1`, client-cap fallback. Modes:
  `off` / `when_client_supports` / `always`.
- Hardware device dropdown — Phase 44. `nvidia-smi` + `/dev/dri/
  renderD*` enumeration at startup; setting whitelisted to
  `auto` / digit / render-path; NVENC `-gpu N` and VAAPI
  `-vaapi_device` honored at session spawn.
- Split GPU vs CPU max concurrent — Phase 45. New
  `transcoder_max_cpu_concurrent` fires only when the resolved
  hwaccel is software; rejection message names the right cap.

**Dropped (won't ship):**

- Throttle buffer seconds — the SIGSTOP/SIGCONT pause path was
  removed for the PWA freeze fix; nothing to throttle against
  today. Restoring would require re-architecting pause-on-buffer-
  ahead logic, substantial work for marginal value over the
  existing rate-control caps.
- Disable video stream transcoding — would force every session to
  direct-play or remux, silently failing for clients whose
  container/codec combo we know will break. The Netflix-clone use
  case has no audio-only mode that would benefit.

| Plex setting | ChimpFlix status | Notes |
|---|---|---|
| Transcoder quality preset | ✅ Phase 18 | `transcoder_encoder_preset` (speed / balanced / quality) |
| Transcoder temp directory | ✅ (`cache_root` env) | Surfaced read-only under Engine → Capability detail |
| Downloads temp directory | N/A | No download/sync feature |
| Throttle buffer (seconds) | 🚫 Dropped | Pause path was removed for PWA freeze fix; nothing to throttle |
| Background transcoding x264 preset | ✅ Phase 30 | `transcoder_background_preset` (ultrafast → slower) |
| HDR tone mapping toggle | ✅ Phase 30 | `transcoder_hdr_tonemap_enabled` |
| Tonemap algorithm | ✅ Phase 30 | `transcoder_hdr_tonemap_algo` (hable / reinhard / mobius / bt2390 / clip / linear) |
| Disable video stream transcoding | 🚫 Dropped | Would silently break playback for clients that need transcode |
| HW accel when available | ✅ | — |
| HW encoding when available | ✅ | — |
| Enable HEVC video encoding | ✅ Phase 43 | `transcoder_hevc_encoding_mode` (off / when_client_supports / always) |
| HEVC optimization | ✅ Phase 43 | `-tag:v hvc1` + forced fMP4 container for Safari compat |
| HEVC ABR fallback variant | ✅ Phase 45 follow-on | Enabled on hardware encoders; disabled on software libx265 (dual-context issues) |
| Hardware transcoding device (Auto / GPU0 / GPU1) | ✅ Phase 44 | `nvidia-smi` + `/dev/dri/renderD*` enumeration; `-gpu N` / `-vaapi_device` honored |
| Max simultaneous GPU transcodes | ✅ Phase 45 | Combined `transcoder_max_concurrent` + sub-cap `transcoder_max_cpu_concurrent` |
| Max simultaneous background video transcode | ✅ Phase 30 | `transcoder_max_background_concurrent` (default 1) |

**Plan:** all of these are server_settings fields + admin UI in
`/admin/server/transcoder`. Hardware device enumeration extends the
existing capability probe at startup; the probe already returns a
`hwaccels` list — extend to include per-device info.

**Estimate:** ~1 day.

### Phase 3 — Library settings completeness ✅ SHIPPED (2026-05-18)

**What landed:**

- Migration `20260518070000_phase31_playback_settings.sql` adds
  `continue_watching_max_items` (40), `continue_watching_max_age_weeks`
  (16; 0 = disable), `video_played_threshold_pct` (90), and
  `database_cache_size_mb` (64) to `server_settings`.
- `queries::on_deck` now takes an `OnDeckOptions` struct so the rail
  cap, the upper-bound "watched" threshold, and the time-window
  filter are all driven by the operator. The query over-fetches 2x
  the user-visible cap so the show-dedup post-step still has rows to
  fill the rail. `GET /play-state/on-deck` pulls the options from the
  settings cache on every request.
- New `GET /play-state/config` endpoint (authed, not admin) returns
  just the `played_threshold_pct` so the player can stay in sync
  with the operator's value. Watch page fetches it server-side and
  threads it into `<ChimpFlixPlayer playedThresholdPct={…} />`. The
  player normalizes + clamps to a [0.5, 0.99] band before using it
  so a misbehaving API can't cause runaway scrobble at 1% or
  prevent scrobble entirely.
- `database_cache_size_mb` is baked into `SqliteConnectOptions`
  via a new `open_with(data_dir, cache_size_mb)` helper. Main does a
  two-pass open: probe the DB with defaults, read the setting, close
  + reopen with the value pinned via `PRAGMA cache_size = -<KiB>`.
  Admin UI surfaces a "Restart pending" badge when the operator
  changes the value, since per-connection PRAGMAs only apply to
  connections opened *after* the change.
- New admin page `/settings/admin/library/playback` with sections
  for Watched threshold, Continue Watching, and Database. Added to
  the AdminNav under Library.
- Single threshold unified: same `video_played_threshold_pct` value
  is used by the client scrobble + the on-deck filter. Before,
  these were split (90% client / 95% server), so the user could see
  a "Continue Watching" tile for a few seconds AFTER the player
  scrobbled — that phantom entry is gone.

**Deferred (would need bigger refactors, not just settings):**

- ~~Allow media deletion per-library + Delete UI on item detail~~ —
  **shipped 2026-05-18 as a follow-up phase.** Migration
  `20260518090000_phase33_allow_media_deletion.sql` adds
  `libraries.allow_media_deletion` (default false). New
  `queries::delete_media_files_force` reuses the cascade logic from
  `purge_removed_media_files`. `DELETE /v1/items/{id}/media` and
  `DELETE /v1/episodes/{id}/media` are owner-gated, library-gated,
  audit-logged, fire background tokio tasks to unlink files +
  evict transcoder cache. Admin libraries page gains a Danger zone
  checkbox. Item modal AdminActions menu gains a destructive
  "Delete from disk…" item that opens a confirm dialog requiring
  the operator to type the title (Plex-style). Modal auto-pops
  when the cascade reaches the item itself.
- Generate chapter thumbnails — new task + ffmpeg orchestration.
- Loudness analysis — needs per-item audio_normalize tracking.
- Include season premieres in Continue Watching — requires
  cross-show query restructuring.
- "Run scanner tasks at lower priority" via `nice()` — one-line
  syscall but cross-platform testing matters; deferred until we
  have a clear lower-priority backlog item driving it.
- Marker generation "when media is added" trigger — `detect_markers`
  is still a stub (Phase 9); blocks adding the file_watcher hook.

| Plex setting | ChimpFlix status | Notes |
| --- | --- | --- |
| Scan automatically | ✅ Phase 14 | `file_watcher` crate; toggle in admin settings (Phase 34) |
| Partial scan on change | ✅ | — |
| Periodic scan + interval | ✅ | `scan_library` scheduled task; per-library `library_scans` history table |
| Empty trash after scan | ✅ | `purge_removed_files` scheduled task |
| Allow media deletion | ✅ Phase 33 | Per-library `allow_media_deletion`; type-to-confirm Delete UI on item modal |
| Weeks to consider for Continue Watching | ✅ Phase 31 | `continue_watching_max_age_weeks` (default 16) |
| Maximum Continue Watching items | ✅ Phase 31 | `continue_watching_max_items` (default 40) |
| Include season premieres in CW | ✅ Phase 35 | `continue_watching_include_premieres` toggle |
| Video played threshold | ✅ Phase 31 | `video_played_threshold_pct` (50-99) |
| Video play completion behaviour | ✅ Phase 46 | `video_completion_behaviour` (threshold_pct / first_credits_marker / earliest_of_both) |
| Marker generation: "when added" trigger | ✅ Phase 37 | `detect_markers_on_add` setting; file_watcher post-scan hook |
| Generate chapter thumbnails | ✅ Phase 38 | `generate_chapter_thumbs` task + `/media-files/{id}/chapters[/{i}/thumb]` API |
| Loudness analysis | ✅ Phase 39 | `analyze_loudness` task; two-pass loudnorm with stored measurements |
| Run scanner tasks at lower priority | ✅ Phase 40 | `scanner_nice_level` (0-19) wraps ffmpeg/ffprobe in `nice -n N` |
| Database cache size | ✅ Phase 31 | `database_cache_size_mb` via `PRAGMA cache_size` |
| Location visibility | N/A | No geo metadata in our model |

**Plan:** most are new `server_settings` columns + a few new
scheduled tasks. The Continue Watching parameters dovetail with the
just-shipped dedup work — surface them in the same admin section.
"When media is added" marker trigger requires file_watcher to enqueue
a marker job, not just a scan.

**Estimate:** ~2 days.

### Phase 4 — Network settings ✅ SHIPPED (2026-05-18)

**What landed:**

- Migration `20260518080000_phase32_network_settings.sql` adds
  `transcoder_reaper_idle_threshold_ms` (default 90_000),
  `max_remote_streams_per_user` (default 0 = unlimited),
  `lan_networks` (CSV CIDR list), and `auth_bypass_cidrs` (CSV CIDR
  list).
- New `crates/server/src/net.rs` module with `parse_cidr_list`,
  `validate_cidr_list`, and `ip_in_list` helpers. Single place we
  parse operator-entered CIDR strings so garbage can't poison
  multiple call sites. 5 unit tests cover IPv4/IPv6/bare-IP/empty
  paths.
- `ipnet = "2"` added to workspace deps.
- `main.rs` reads `transcoder_reaper_idle_threshold_ms` from the
  loaded settings and feeds it to `spawn_reaper` — no more
  hard-coded 90_000.
- `AuthUser` extractor now checks the bypass CIDR list before the
  cookie path. Matching IPs are mapped to the first owner user
  (`queries::find_first_owner`) and return a synthesised AuthUser
  whose `session_id` is the new `BYPASS_SESSION_ID` sentinel
  (negative i64) so session-rotation code can't accidentally
  invalidate a real session row.
- `POST /stream/sessions` now applies the per-user remote-streams
  cap. `is_remote_request` classifies the request via X-Forwarded-
  For / X-Real-IP intersected with the LAN CIDR list; matching LAN
  requests bypass the cap. Hot-reloaded from the settings cache.
- `/admin/network` endpoint extended with all four new fields plus
  validation. Admin UI gains a "LAN policy" section (LAN networks
  + remote-stream cap + auth bypass CIDRs) and a "Session cleanup"
  section with the reaper threshold (flagged "Restart pending"
  when changed since the value is read at spawn time).

**Deferred (out of project scope or no concrete need):**

- Preferred network interface — `BIND_ADDR` env already handles
  the multi-NIC case; full-blown interface picker would mean
  enumerating NICs at startup.
- GDM / Relay / strict TLS / custom certificate location — Plex
  cloud-only or upstream-terminated; explicitly out of scope.
- Multi-URL "Custom server access URLs" — single `public_url`
  covers the deployment models we care about.
- "Treat WAN IP as LAN bandwidth" — orthogonal to current behavior
  since we deferred the aggressive bandwidth-aware downgrade
  (HLS.js handles in-session adaptation).

| Plex setting | ChimpFlix status | Notes |
| --- | --- | --- |
| Client Network (IPv4/IPv6) | ✅ | axum binds both by default; bind_interface override available |
| Secure connections (Preferred/Required) | N/A | TLS terminated upstream by reverse proxy |
| Custom certificate location | N/A | Reverse proxy concern |
| Preferred network interface | ✅ Phase 47 | `bind_interface` setting; restart-pending UX badge |
| Strict TLS configuration | N/A | Reverse proxy concern |
| GDM (local network discovery) | N/A | Out of scope (no TV/DLNA clients) |
| Remote streams per user limit | ✅ Phase 32 | `max_remote_streams_per_user` cap; LAN bypass via `lan_networks` |
| LAN networks (CIDR list) | ✅ Phase 32 | `lan_networks` CSV CIDR list |
| Terminate paused sessions threshold | ✅ Phase 32 | `transcoder_reaper_idle_threshold_ms` (restart-pending) |
| Treat WAN IP as LAN bandwidth | 🚫 Dropped | Bandwidth-aware auto-downgrade was removed; HLS.js ABR handles in-session adaptation natively |
| Enable Relay | N/A | No Plex relay equivalent for self-hosted |
| Custom server access URLs | ✅ | Single canonical `public_url` for outgoing links + `cors_origins` CSV for additional trusted URLs (CSRF + CORS) |
| List of IP addresses allowed without auth | ✅ Phase 32 | `auth_bypass_cidrs` CSV; matching IPs run as the first owner |
| Webhooks toggle | ✅ | — |

**Plan:** all settings fields, most affect existing middleware
(session reaper, auth extractor) which already read from
server_settings. The IP whitelist is useful for LAN automation
(Home Assistant calling our API without logging in).

**Estimate:** ~0.5 day.

---

## Recommended sequence

1. **Phase 1 (Scheduled Tasks)** — biggest immediate UX win, user explicitly flagged.
2. **Phase 2 (Transcoder)** — we just shipped the `-nostdin` fix; settings parity is the natural follow-up.
3. **Phase 3 (Library)** — incremental additions that compound.
4. **Phase 4 (Network)** — quickest, lowest immediate impact.

**Total ~4–5 days** for full Plex-settings-parity.

---

## Open items NOT covered by parity work

These came out of the audit but are independent of Plex parity:

- **Remaining lint** — 9 `set-state-in-effect` warnings in admin
  client components (audit, libraries, invites, two-factor). All
  follow the same documented "fetch on prop change + cancellation
  guard" pattern. Either add eslint-disable-with-justification on
  each, or migrate to `useEffectEvent` once it's out of experimental.
- **Bandwidth-aware quality downgrade** can still restart sessions
  if HLS.js's bandwidth estimate fluctuates. With the multi-variant
  ABR fallback now shipped, the downgrade rarely needs to fire —
  consider removing the auto-downgrade entirely and letting HLS.js
  handle it natively.
- **iOS PWA fullscreen** — `requestFullscreen()` on container
  doesn't work the same on iOS Safari. Verify and add
  `webkitEnterFullscreen()` fallback on the video element if needed.
- **Page Lifecycle freeze/resume** — Chrome PWAs can be frozen
  after long backgrounding; setIntervals stop. Adding a
  `freeze`/`resume` event handler that refreshes the keepalive on
  resume would close that gap. Acceptable to defer.
- **Bulk operations in admin** — multi-select items to apply tags,
  refresh metadata, delete. Quality-of-life win for operators
  managing larger libraries.
- **Restore-from-backup UI** — we create backups but no restore
  flow; operator must drop to `sqlite3` to recover.
