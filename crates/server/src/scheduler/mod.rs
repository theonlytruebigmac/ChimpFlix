//! Cron-driven background task runner.
//!
//! The runner is a single tokio task spawned at startup. Every tick (30s)
//! it polls `scheduled_tasks` for rows whose `next_run_at` is due, marks
//! each running, dispatches to the right handler, then writes the
//! outcome + the next firing time back to the DB.
//!
//! Crash safety: on startup we flip any rows left in `last_status='running'`
//! to `failed/interrupted` so a hard kill mid-run doesn't permanently
//! freeze a task in "running" forever.
//!
//! Cron parsing: we use the `cron` crate's 7-field schedule (sec min hour
//! dom mon dow year). 5-field expressions ("0 */4 * * *") are accepted by
//! normalizing to the 7-field form (prepending "0 " and appending " *").

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chimpflix_common::now_ms;
use chimpflix_library::queries;
use chimpflix_library::scanner;
use chimpflix_library::{NewScheduledTask, ScheduledTask};
use chrono::{Local, NaiveTime, TimeZone, Utc};
use cron::Schedule;
use sqlx::SqlitePool;
use tracing::{debug, error, info, warn};

use crate::state::AppState;

const TICK_INTERVAL_S: u64 = 30;

/// Validate and parse a cron expression. Accepts both the standard 5-field
/// form and the cron-crate's 7-field form.
pub fn parse_cron(expr: &str) -> Result<Schedule> {
    let trimmed = expr.trim();
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    let normalized = match fields.len() {
        5 => format!("0 {trimmed} *"),
        6 => format!("0 {trimmed}"),
        7 => trimmed.to_string(),
        n => bail!("cron expression must have 5, 6, or 7 fields (got {n})"),
    };
    Schedule::from_str(&normalized).with_context(|| format!("invalid cron `{trimmed}`"))
}

pub fn next_after(expr: &str, after_ms: i64) -> Result<i64> {
    let schedule = parse_cron(expr)?;
    let after = Utc.timestamp_millis_opt(after_ms).single().unwrap_or_else(Utc::now);
    let next = schedule
        .after(&after)
        .next()
        .context("cron schedule produced no future firing")?;
    Ok(next.timestamp_millis())
}

/// Sentinel future timestamp for tasks that should never auto-fire
/// (`manual`, `on_change`). Picking a value 100 years out keeps any
/// `next_run_at <= now` queries cheap and avoids reserving i64::MAX
/// (which some downstream Date constructors handle poorly).
const NEVER_RUN_AT_MS: i64 = 4_102_444_800_000; // 2100-01-01 UTC

/// Convert a `frequency` enum value to its fixed interval in
/// milliseconds. Returns `None` for non-interval frequencies
/// (`manual`, `on_change`, `custom`).
pub fn frequency_interval_ms(frequency: &str) -> Option<i64> {
    let hour: i64 = 60 * 60 * 1000;
    let day: i64 = 24 * hour;
    match frequency {
        "hourly" => Some(hour),
        "every_3_hours" => Some(3 * hour),
        "every_6_hours" => Some(6 * hour),
        "every_12_hours" => Some(12 * hour),
        "daily" => Some(day),
        "every_3_days" => Some(3 * day),
        "weekly" => Some(7 * day),
        // Monthly is approximated as 30 days — Plex's UI does the same.
        // Real calendar-month scheduling would require timezone-aware
        // anchor dates and isn't worth the surface area for "roughly
        // monthly housekeeping" semantics.
        "monthly" => Some(30 * day),
        _ => None,
    }
}

/// Parse an `HH:MM` (24-hour) string into a NaiveTime, falling back
/// to 02:00 if the value is garbage so a misconfigured row can't
/// crash the scheduler.
fn parse_hhmm(s: &str, fallback_h: u32, fallback_m: u32) -> NaiveTime {
    let trimmed = s.trim();
    let parts: Vec<&str> = trimmed.split(':').collect();
    if parts.len() == 2 {
        if let (Ok(h), Ok(m)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
            if h < 24 && m < 60 {
                if let Some(t) = NaiveTime::from_hms_opt(h, m, 0) {
                    return t;
                }
            }
        }
    }
    NaiveTime::from_hms_opt(fallback_h, fallback_m, 0).expect("valid fallback")
}

/// Snap `t_ms` forward to the next moment that falls within the
/// `[start..end]` maintenance window (server-local time). When
/// `end <= start` the window wraps midnight (e.g. 22:00→06:00 means
/// "from 22:00 today to 06:00 tomorrow").
///
/// If `t_ms` is already inside the current window, it's returned
/// unchanged. Otherwise we snap forward to the next start time.
pub fn snap_to_maintenance_window(t_ms: i64, win_start: &str, win_end: &str) -> i64 {
    let start = parse_hhmm(win_start, 2, 0);
    let end = parse_hhmm(win_end, 9, 0);
    let t = match Local.timestamp_millis_opt(t_ms).single() {
        Some(d) => d,
        // Out-of-range millisecond — bail and return input unchanged
        // rather than risk panicking the scheduler.
        None => return t_ms,
    };
    let today = t.date_naive();
    let start_today = match Local.from_local_datetime(&today.and_time(start)).single() {
        Some(d) => d,
        None => return t_ms,
    };
    let wraps = end <= start;
    let end_today_or_tomorrow = if wraps {
        // End time is tomorrow morning.
        let tomorrow = today.succ_opt().unwrap_or(today);
        match Local
            .from_local_datetime(&tomorrow.and_time(end))
            .single()
        {
            Some(d) => d,
            None => return t_ms,
        }
    } else {
        match Local.from_local_datetime(&today.and_time(end)).single() {
            Some(d) => d,
            None => return t_ms,
        }
    };
    // Already inside today's window.
    if t >= start_today && t < end_today_or_tomorrow {
        return t_ms;
    }
    // For wrapping windows, also check whether `t` is in yesterday's
    // window that hasn't closed yet (e.g. window 22:00→06:00 and t at
    // 03:00 — that's inside the window that opened the previous day).
    if wraps {
        if let Some(yesterday) = today.pred_opt() {
            let start_y = Local.from_local_datetime(&yesterday.and_time(start)).single();
            let end_y = Local.from_local_datetime(&today.and_time(end)).single();
            if let (Some(s), Some(e)) = (start_y, end_y) {
                if t >= s && t < e {
                    return t_ms;
                }
            }
        }
    }
    // Snap forward: today's start if still ahead, else tomorrow's.
    let snap = if t < start_today {
        start_today
    } else {
        let tomorrow = today.succ_opt().unwrap_or(today);
        match Local
            .from_local_datetime(&tomorrow.and_time(start))
            .single()
        {
            Some(d) => d,
            None => return t_ms,
        }
    };
    snap.timestamp_millis()
}

/// Compute the next firing for a task using its frequency + custom-
/// cron + maintenance-window settings. Single source of truth for
/// scheduling math.
///
/// - `manual` / `on_change`: returns `NEVER_RUN_AT_MS`. The scheduler
///   tick treats this as "don't ever auto-fire" — these only run via
///   the `Run Now` button or event-driven hooks (file_watcher, etc.).
/// - `custom`: parses `cron_expr` and uses its `after(after_ms)`.
/// - any interval frequency: returns `after_ms + interval`, then
///   snaps forward into the next maintenance-window opening when
///   `requires_window` is true.
pub fn compute_next_run(
    frequency: &str,
    cron_expr: &str,
    after_ms: i64,
    requires_window: bool,
    win_start: &str,
    win_end: &str,
) -> Result<i64> {
    let base = match frequency {
        "manual" | "on_change" => return Ok(NEVER_RUN_AT_MS),
        "custom" => next_after(cron_expr, after_ms)?,
        other => match frequency_interval_ms(other) {
            Some(interval) => after_ms + interval,
            None => bail!("unknown frequency `{other}`"),
        },
    };
    if requires_window {
        Ok(snap_to_maintenance_window(base, win_start, win_end))
    } else {
        Ok(base)
    }
}

/// Convenience: read the current settings cache and call
/// `compute_next_run` with whatever window the operator configured.
/// Used by every code path that needs to recompute `next_run_at`.
pub async fn compute_next_run_with_settings(
    state: &AppState,
    frequency: &str,
    cron_expr: &str,
    after_ms: i64,
    requires_window: bool,
) -> Result<i64> {
    let (win_start, win_end) = {
        let s = state.settings.read().await;
        (
            s.maintenance_window_start.clone(),
            s.maintenance_window_end.clone(),
        )
    };
    compute_next_run(
        frequency,
        cron_expr,
        after_ms,
        requires_window,
        &win_start,
        &win_end,
    )
}

/// Seed the default task set on first run. Idempotent — if any tasks
/// exist (created by a previous boot or by the user) this does nothing.
///
/// New tasks use the frequency model directly (no cron strings); the
/// `cron_expr` field is set to a sensible placeholder so toggling to
/// "Custom (advanced)" later starts from something reasonable.
pub async fn seed_defaults(pool: &SqlitePool) -> Result<()> {
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_tasks")
        .fetch_one(pool)
        .await?;
    if existing > 0 {
        return Ok(());
    }
    struct Seed {
        kind: &'static str,
        name: &'static str,
        frequency: &'static str,
        requires_window: bool,
        params_json: &'static str,
        /// Placeholder cron — only consulted if the operator flips this
        /// row to `frequency = 'custom'` later. Reasonable default per
        /// task kind so the advanced editor doesn't start blank.
        cron_placeholder: &'static str,
    }
    let defaults: &[Seed] = &[
        Seed {
            kind: "prune_sessions",
            name: "Prune expired sessions",
            frequency: "hourly",
            // Cheap (one DELETE) — fine to run during prime time.
            requires_window: false,
            params_json: "{}",
            cron_placeholder: "0 0 * * * *",
        },
        Seed {
            kind: "backup_db",
            name: "Backup database",
            frequency: "daily",
            requires_window: true,
            params_json: "{}",
            cron_placeholder: "0 0 3 * * *",
        },
        Seed {
            kind: "refresh_trending",
            name: "Refresh global trending (Top 10)",
            // External-API call + small upsert; not heavy but no point
            // hitting TMDB during peak hours either.
            frequency: "daily",
            requires_window: true,
            params_json: "{}",
            cron_placeholder: "0 0 4 * * *",
        },
        Seed {
            kind: "verify_libraries",
            name: "Verify libraries (find missing files)",
            // Walks every file's stat() — heavy on cold caches; weekly
            // keeps the IO cost predictable.
            frequency: "weekly",
            requires_window: true,
            params_json: "{}",
            cron_placeholder: "0 30 2 * * 0",
        },
        Seed {
            kind: "purge_removed_files",
            name: "Purge files removed for > 7 days",
            frequency: "daily",
            requires_window: true,
            params_json: "{\"grace_days\":7}",
            cron_placeholder: "0 30 3 * * *",
        },
        Seed {
            kind: "cleanup_audit_log",
            name: "Trim audit log (>90 days)",
            frequency: "daily",
            requires_window: true,
            params_json: "{\"retention_days\":90}",
            cron_placeholder: "0 30 4 * * *",
        },
        // Seed order matters: rollup must come before cleanup_jobs.
        // When two daily+requires_window tasks land in the same
        // maintenance window, the scheduler picks them in
        // (next_run_at ASC, id ASC) order — and the id falls back
        // to insertion order on a tie. Putting rollup first means
        // it processes yesterday's `succeeded` / `dead` rows before
        // cleanup_jobs trims them. With the default retention (7d)
        // there's a comfortable margin anyway; only an operator who
        // tightens succeeded_retention_days to <2 risks losing a
        // day's rollup. Documented at
        // [docs/pipelines/backend-plan.md] §6.
        Seed {
            kind: "rollup_task_metrics",
            name: "Daily metrics rollup",
            // Cheap (aggregates < 100k rows in milliseconds for
            // any realistic backlog) — runs first in the window.
            frequency: "daily",
            requires_window: true,
            params_json: "{}",
            cron_placeholder: "0 0 2 * * *",
        },
        Seed {
            kind: "cleanup_jobs",
            name: "Trim job queue history",
            // Daily is plenty for queue cleanup — `succeeded` rows
            // accumulate steadily during a backfill and then the
            // table stays mostly flat. Window-gated because it can
            // delete tens of thousands of rows in one shot when
            // a fresh backfill ages out.
            frequency: "daily",
            requires_window: true,
            // Defaults: keep succeeded for 7 days (long enough to
            // diagnose a recent regression and to give the daily
            // rollup a comfortable read window), dead for 30 days
            // (long enough for an operator to notice and decide
            // whether to requeue).
            params_json: "{\"succeeded_retention_days\":7,\"dead_retention_days\":30}",
            cron_placeholder: "0 30 4 * * *",
        },
    ];
    // Window snap requires the actual operator-configured window which
    // lives on the (yet-to-be-populated) settings cache. At seed time
    // it's safe to use the migration defaults (02:00 → 09:00) because
    // that's what the row was initialized with.
    let win_start = "02:00";
    let win_end = "09:00";
    for s in defaults {
        let next = compute_next_run(
            s.frequency,
            s.cron_placeholder,
            now_ms(),
            s.requires_window,
            win_start,
            win_end,
        )?;
        let _ = queries::create_scheduled_task(
            pool,
            NewScheduledTask {
                kind: s.kind.into(),
                name: s.name.into(),
                cron_expr: s.cron_placeholder.into(),
                frequency: s.frequency.into(),
                requires_maintenance_window: s.requires_window,
                params_json: s.params_json.into(),
                enabled: true,
            },
            next,
        )
        .await?;
    }
    info!("seeded default scheduled tasks");
    Ok(())
}

/// Spawn the runner. Returns immediately; the loop runs until the server
/// process exits.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(TICK_INTERVAL_S));
        tick.tick().await; // skip the immediate first tick
        loop {
            tick.tick().await;
            if let Err(e) = run_once(&state).await {
                error!(error = %format!("{e:#}"), "scheduler tick failed");
            }
        }
    });
}

async fn run_once(state: &AppState) -> Result<()> {
    let now = now_ms();
    let due = queries::claim_due_tasks(&state.pool, now).await?;
    for task in due {
        let st = state.clone();
        tokio::spawn(async move {
            execute(st, task).await;
        });
    }
    Ok(())
}

/// Public for the `POST /admin/tasks/{id}/run` route — fires the handler
/// out-of-band of the schedule. Rejects the call if the task is already
/// running so a double-click in the admin UI (or two operators hitting
/// the button at the same time) can't spawn duplicate executions —
/// previously `mark_task_running` would succeed for both, scan/marker
/// jobs would race on the same files, and the second `mark_task_finished`
/// would overwrite the first's status/next-run bookkeeping.
pub async fn run_now(state: AppState, task_id: i64) -> Result<()> {
    let Some(task) = queries::get_scheduled_task(&state.pool, task_id).await? else {
        bail!("task {task_id} not found");
    };
    if task.last_status.as_deref() == Some("running") {
        bail!("task {} is already running", task.name);
    }
    tokio::spawn(async move { execute(state, task).await });
    Ok(())
}

async fn execute(state: AppState, task: ScheduledTask) {
    let started_at = now_ms();
    let run_id = match queries::mark_task_running(&state.pool, task.id, started_at).await {
        Ok(id) => id,
        Err(e) => {
            warn!(task_id = task.id, error = %format!("{e:#}"), "could not mark task running");
            return;
        }
    };
    let log_buf: Arc<std::sync::Mutex<String>> = Arc::new(std::sync::Mutex::new(String::new()));
    let result = dispatch(&state, &task, &log_buf).await;
    let finished_at = now_ms();
    let duration = (finished_at - started_at).max(0);
    let (status, error_msg) = match &result {
        Ok(()) => ("success", None),
        Err(e) => ("failed", Some(format!("{e:#}"))),
    };
    let log = log_buf.lock().ok().map(|s| s.clone()).filter(|s| !s.is_empty());

    let next = match compute_next_run_with_settings(
        &state,
        &task.frequency,
        &task.cron_expr,
        finished_at,
        task.requires_maintenance_window,
    )
    .await
    {
        Ok(n) => n,
        Err(e) => {
            warn!(task_id = task.id, error = %format!("{e:#}"), "next firing computation failed; deferring 1h");
            finished_at + 3_600_000
        }
    };

    // Exponential backoff on consecutive failures. A failing task
    // (e.g. refresh_metadata while TMDB is down) at hourly cadence
    // would otherwise retry every hour and spam the logs. With
    // backoff: 1st failure → +5 min, 2nd → +10 min, 3rd → +20 min,
    // capped at 6 hours. The backoff is layered on top of the normal
    // schedule via `max`, so a normally-daily task whose backoff is
    // only 20 minutes still waits the full day.
    //
    // We count *including this run* — the query reads task_runs after
    // mark_task_finished below, so we read before-the-finish and
    // include this run's outcome by checking `status` locally. This
    // avoids a second round-trip after the UPDATE.
    let next = if status == "failed" {
        let prior_failures = queries::count_consecutive_task_failures(&state.pool, task.id)
            .await
            .unwrap_or(0);
        // +1 for this run itself (which mark_task_finished is about to record).
        let n = (prior_failures + 1).clamp(1, 16) as u32;
        let base_ms: i64 = 5 * 60 * 1000;
        let cap_ms: i64 = 6 * 60 * 60 * 1000;
        let backoff = base_ms
            .saturating_mul(1_i64.checked_shl(n.saturating_sub(1)).unwrap_or(i64::MAX))
            .min(cap_ms);
        let backoff_until = finished_at.saturating_add(backoff);
        debug!(
            task_id = task.id,
            consecutive_failures = n,
            backoff_ms = backoff,
            "applying failure backoff"
        );
        next.max(backoff_until)
    } else {
        next
    };

    if let Err(e) = queries::mark_task_finished(
        &state.pool,
        task.id,
        run_id,
        finished_at,
        duration,
        next,
        status,
        error_msg.as_deref(),
        log.as_deref(),
    )
    .await
    {
        error!(task_id = task.id, error = %format!("{e:#}"), "failed to persist task outcome");
    }

    debug!(
        task_id = task.id,
        kind = %task.kind,
        status,
        duration_ms = duration,
        "task finished"
    );
}

/// Dispatch to the matching handler. Each handler returns `Result<()>` and
/// is responsible for its own internal logging via `tracing` plus
/// optional structured logs appended to `log` for the history viewer.
async fn dispatch(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    // Gate check — runs before any kind-specific work. If admin has
    // turned the gate off (e.g. chapter thumbs), the sweep is a no-op:
    // we log a one-liner so the run is visible in history with "skipped
    // — gated off" rather than vanishing silently. Unknown kinds fall
    // through to the legacy dispatch below — they pre-date the
    // registry, so we don't gatekeep them yet.
    let gate = crate::tasks::is_kind_allowed(state, &task.kind).await;
    if matches!(gate, crate::tasks::GateState::DisabledByAdmin) {
        append_log(
            log,
            format!("skipped: kind `{}` is gated off in server settings", task.kind),
        );
        return Ok(());
    }

    match task.kind.as_str() {
        "prune_sessions" => {
            let removed = queries::cleanup_expired_sessions(&state.pool).await?;
            append_log(log, format!("pruned {removed} expired sessions"));
            Ok(())
        }
        "cleanup_jobs" => {
            // Trim succeeded + dead rows from the `jobs` table.
            // params.succeeded_retention_days / dead_retention_days
            // override the defaults (7 / 30).
            let params: serde_json::Value =
                serde_json::from_str(&task.params_json).unwrap_or_default();
            let succ_days = params
                .get("succeeded_retention_days")
                .and_then(|v| v.as_i64())
                .unwrap_or(7)
                .clamp(1, 3650);
            let dead_days = params
                .get("dead_retention_days")
                .and_then(|v| v.as_i64())
                .unwrap_or(30)
                .clamp(1, 3650);
            let day_ms: i64 = 24 * 60 * 60 * 1000;
            let (succ_removed, dead_removed) = queries::cleanup_old_jobs(
                &state.pool,
                succ_days * day_ms,
                dead_days * day_ms,
            )
            .await?;
            append_log(
                log,
                format!(
                    "trimmed {succ_removed} succeeded rows (>{succ_days}d), \
                     {dead_removed} dead rows (>{dead_days}d)"
                ),
            );
            Ok(())
        }
        "cleanup_audit_log" => {
            // Retention is configurable via params_json.retention_days
            // (default 90). The audit log is append-only and grows
            // unbounded otherwise; 90 days is enough to investigate
            // most incidents while keeping the table size sane.
            let params: serde_json::Value = serde_json::from_str(&task.params_json)
                .unwrap_or_else(|_| serde_json::json!({}));
            let retention_days = params
                .get("retention_days")
                .and_then(|v| v.as_i64())
                .unwrap_or(90)
                .clamp(1, 3650);
            let cutoff = now_ms() - retention_days * 24 * 60 * 60 * 1000;
            let removed = queries::cleanup_old_audit_log(&state.pool, cutoff).await?;
            let pwreset_removed =
                queries::cleanup_expired_password_reset_tokens(&state.pool).await?;
            let echange_removed =
                queries::cleanup_expired_email_change_tokens(&state.pool).await?;
            append_log(
                log,
                format!(
                    "trimmed {removed} audit rows older than {retention_days}d; \
                     also dropped {pwreset_removed} expired password-reset tokens \
                     and {echange_removed} expired email-change tokens"
                ),
            );
            Ok(())
        }
        "backup_db" => {
            // Re-uses the manual-backup helper's VACUUM INTO; we don't
            // stream it anywhere — the file is dropped under
            // DATA_DIR/backups/auto/ and the maintenance UI lists them.
            let dir = state.data_dir.join("backups/auto");
            tokio::fs::create_dir_all(&dir).await?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let path = dir.join(format!("chimpflix-{stamp}.db"));
            if path.exists() {
                tokio::fs::remove_file(&path).await.ok();
            }
            let target = path.to_string_lossy().replace('\'', "''");
            use sqlx::Executor;
            state
                .pool
                .execute(format!("VACUUM INTO '{target}'").as_str())
                .await?;
            append_log(log, format!("wrote {}", path.display()));
            Ok(())
        }
        "scan_library" => {
            let params: serde_json::Value = serde_json::from_str(&task.params_json)
                .context("parse params_json")?;
            let library_id = params
                .get("library_id")
                .and_then(|v| v.as_i64())
                .context("scan_library requires params.library_id")?;
            // Per-library scan lock: prevents the scheduled scan, an
            // operator-triggered manual scan, and the filesystem
            // watcher from running concurrent ffmpeg/IO work on the
            // same library. The dominant symptom of overlap was live
            // playback stalling during maintenance windows because all
            // three pathways were hammering the same disk holding the
            // transcoder cache. Bail out cleanly when the lock is
            // already held — the in-flight scan will pick up whatever
            // this task would have noticed.
            if !state.try_acquire_library_scan(library_id).await {
                append_log(
                    log,
                    format!(
                        "skipped: a scan for library {library_id} is already in progress"
                    ),
                );
                return Ok(());
            }
            // Reuse the existing trigger-scan flow: create a scan_job row
            // and spawn the scanner. The handler returns immediately —
            // long-running scan progress is tracked in scan_jobs, not
            // task_runs.
            let job = queries::create_scan_job(&state.pool, library_id).await?;
            let pool = state.pool.clone();
            let ffmpeg = state.ffmpeg.clone();
            let tmdb = state.tmdb_snapshot().await;
            let tvdb = state.tvdb_snapshot().await;
            let anilist = state.anilist_snapshot().await;
            let tvmaze = state.tvmaze.clone();
            let job_id = job.id;
            let hub = state.hub.clone();
            let cache_root = state.transcoder.cache_root().to_path_buf();
            let release_state = state.clone();
            let pipeline_state = state.clone();
            tokio::spawn(async move {
                // RAII guard: release the scan lock on every exit path
                // including a panic inside `run_scan`. Without it, a
                // panic between the acquire above and the release below
                // would leak the entry and the library would be stuck
                // until a process restart. Spawned in its own thread so
                // the destructor runs in `tokio::spawn`'s catch_unwind
                // boundary even when the future panics.
                struct ScanLockGuard {
                    state: AppState,
                    library_id: i64,
                }
                impl Drop for ScanLockGuard {
                    fn drop(&mut self) {
                        let st = self.state.clone();
                        let lib = self.library_id;
                        tokio::spawn(async move {
                            st.release_library_scan(lib).await;
                        });
                    }
                }
                let _guard = ScanLockGuard { state: release_state, library_id };
                let inner_emitter: chimpflix_library::ScanEmitter =
                    Arc::new(move |evt| {
                        hub.publish(crate::events::Event::Scan(evt));
                    });
                let emitter = crate::jobs::pipeline::wrap_emitter_for_pipeline(
                    pipeline_state,
                    inner_emitter,
                );
                if let Err(e) = scanner::run_scan(
                    pool, ffmpeg, tmdb, tvdb, anilist, tvmaze, library_id, job_id,
                    Some(cache_root),
                    emitter,
                )
                .await
                {
                    warn!(library_id, job_id, error = %format!("{e:#}"), "scheduled scan failed");
                }
            });
            append_log(log, format!("queued scan job #{job_id}"));
            Ok(())
        }
        "refresh_metadata" => {
            // Refresh every item in the library (or the whole DB if no
            // params.library_id is given). Best-effort; per-item failures
            // do not fail the task.
            let params: serde_json::Value = serde_json::from_str(&task.params_json)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            let library_id = params.get("library_id").and_then(|v| v.as_i64());
            let Some(tmdb) = state.tmdb_snapshot().await else {
                append_log(log, "TMDB disabled — skipping refresh");
                return Ok(());
            };
            let tvdb = state.tvdb_snapshot().await;
            let tvmaze = state.tvmaze.clone();
            let rows = if let Some(lid) = library_id {
                sqlx::query_scalar::<_, i64>("SELECT id FROM items WHERE library_id = ?")
                    .bind(lid)
                    .fetch_all(&state.pool)
                    .await?
            } else {
                sqlx::query_scalar::<_, i64>("SELECT id FROM items")
                    .fetch_all(&state.pool)
                    .await?
            };
            let total = rows.len();
            let mut ok = 0usize;
            let mut err = 0usize;
            for item_id in rows {
                match scanner::refresh_item_metadata(
                    &state.pool,
                    &tmdb,
                    tvdb.as_ref(),
                    tvmaze.as_ref(),
                    item_id,
                    None,
                )
                .await
                {
                    Ok(()) => ok += 1,
                    Err(e) => {
                        err += 1;
                        warn!(item_id, error = %format!("{e:#}"), "refresh_metadata failed for item");
                    }
                }
            }
            append_log(log, format!("refreshed {ok}/{total} items, {err} failed"));
            Ok(())
        }
        "detect_markers" => {
            // Safety-net sweep: finds files whose discovery-pipeline
            // job never ran (server was down during scan, or row
            // pre-dates the pipeline migration) and enqueues
            // `detect_markers_file` jobs for them. The job queue
            // worker pool does the actual ffmpeg work — this task
            // no longer runs detection inline.
            //
            // Per-library cap protects the queue from a fresh
            // 10k-file library piling up everything in one tick.
            // Subsequent ticks drain whatever the queue worker
            // hasn't picked up yet.
            let params: serde_json::Value =
                serde_json::from_str(&task.params_json).unwrap_or_default();
            let scoped_library_id = params.get("library_id").and_then(|v| v.as_i64());
            let per_library_cap = params
                .get("batch_size")
                .and_then(|v| v.as_i64())
                .unwrap_or(500)
                .max(1);

            let library_ids: Vec<i64> = match scoped_library_id {
                Some(lid) => vec![lid],
                None => queries::list_libraries(&state.pool, None)
                    .await?
                    .into_iter()
                    .map(|l| l.id)
                    .collect(),
            };

            let mut total_enqueued = 0usize;
            for lib_id in &library_ids {
                let pending = queries::list_media_files_needing_markers(
                    &state.pool,
                    *lib_id,
                    per_library_cap,
                )
                .await?;
                let file_ids: Vec<i64> = pending.iter().map(|(id, _, _)| *id).collect();
                let n = crate::jobs::handlers::detect_markers_file::enqueue_for_files(
                    &state.pool,
                    &file_ids,
                )
                .await?;
                total_enqueued += n;
            }
            if total_enqueued == 0 {
                append_log(log, "no files needing markers — queue is the active path");
            } else {
                append_log(
                    log,
                    format!(
                        "enqueued {total_enqueued} detect_markers_file jobs across {} libraries",
                        library_ids.len()
                    ),
                );
            }
            Ok(())
        }
        "fetch_subtitles" => fetch_subtitles_task(state, task, log).await,
        "generate_previews" => generate_previews_task(state, task, log).await,
        "generate_chapter_thumbs" => generate_chapter_thumbs_task(state, task, log).await,
        "analyze_loudness" => analyze_loudness_task(state, task, log).await,
        "verify_backups" => verify_backups_task(state, log).await,
        "trakt_pull" => trakt_pull_task(state, log).await,
        "refresh_trending" => refresh_trending_task(state, log).await,
        "refresh_logos" => refresh_logos_task(state, task, log).await,
        "scan_extras" => scan_extras_task(state, task, log).await,
        "extract_subs_sweep" => extract_subs_sweep_task(state, task, log).await,
        "refresh_ratings" => refresh_ratings_task(state, task, log).await,
        "rollup_task_metrics" => rollup_task_metrics_task(state, log).await,
        "verify_libraries" => verify_libraries_task(state, task, log).await,
        "purge_removed_files" => purge_removed_files_task(state, task, log).await,
        "optimize_versions" => {
            // Process up to `batch_size` queued rows. Per-row failures are
            // captured in the optimized_versions table, not in the task
            // outcome — the task itself succeeds whenever it ran to
            // completion.
            //
            // The operator can cap concurrency via server_settings
            // `transcoder_max_background_concurrent` (default 1) so a
            // big backlog doesn't starve live transcodes; we take the
            // tighter of "explicit params.batch_size" and "settings
            // ceiling".
            let params: serde_json::Value =
                serde_json::from_str(&task.params_json).unwrap_or_default();
            let param_batch = params
                .get("batch_size")
                .and_then(|v| v.as_i64())
                .unwrap_or(2);
            let settings_cap = state
                .settings
                .read()
                .await
                .transcoder_max_background_concurrent;
            let batch = param_batch.min(settings_cap).max(1);
            let pending = queries::claim_queued_optimized(&state.pool, batch).await?;
            let mut ok = 0usize;
            let mut failed = 0usize;
            for row in &pending {
                match optimize_one(state, row).await {
                    Ok(()) => ok += 1,
                    Err(e) => {
                        failed += 1;
                        tracing::warn!(
                            id = row.id,
                            error = %format!("{e:#}"),
                            "optimized version failed"
                        );
                    }
                }
            }
            append_log(
                log,
                format!("processed {} ({} ok, {} failed)", pending.len(), ok, failed),
            );
            Ok(())
        }
        other => bail!("unknown task kind `{other}`"),
    }
}

/// One optimization pass: re-encode a source file to a preset's bitrate/
/// resolution constraints, write to `<DATA_DIR>/optimized/{src}-{preset}.mp4`,
/// then record success/failure on the row.
async fn optimize_one(
    state: &AppState,
    row: &chimpflix_library::OptimizedVersion,
) -> anyhow::Result<()> {
    // Resolve source path + preset config.
    use sqlx::Row;
    let source_row = sqlx::query("SELECT path FROM media_files WHERE id = ?")
        .bind(row.source_file_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source media_file {} missing", row.source_file_id))?;
    let source_path: String = source_row.try_get("path")?;

    let preset = queries::get_transcoder_preset(&state.pool, row.preset_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("preset {} missing", row.preset_id))?;

    // libx264 preset for background re-encodes. Operator-configurable
    // via server_settings.transcoder_background_preset; default
    // `veryfast` matches the value this was hard-coded to before
    // phase 30.
    let background_preset = state
        .settings
        .read()
        .await
        .transcoder_background_preset
        .clone();

    let dir = state.data_dir.join("optimized");
    tokio::fs::create_dir_all(&dir).await?;
    let output = dir.join(format!("{}-{}.mp4", row.source_file_id, row.preset_id));
    let output_str = output.to_string_lossy().into_owned();

    queries::mark_optimized_running(&state.pool, row.id, &output_str).await?;

    let started = std::time::Instant::now();
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        source_path.clone(),
    ];
    // Video: cap height and bitrate per preset. max_height=0 means
    // passthrough resolution; the same goes for bitrate=0.
    if preset.max_height > 0 {
        args.push("-vf".into());
        args.push(format!("scale=-2:'min({},ih)'", preset.max_height));
    }
    args.push("-c:v".into());
    args.push("libx264".into());
    args.push("-preset".into());
    args.push(background_preset);
    if preset.max_video_bitrate_kbps > 0 {
        args.push("-b:v".into());
        args.push(format!("{}k", preset.max_video_bitrate_kbps));
        args.push("-maxrate".into());
        args.push(format!("{}k", preset.max_video_bitrate_kbps));
        args.push("-bufsize".into());
        args.push(format!("{}k", preset.max_video_bitrate_kbps * 2));
    }
    args.push("-c:a".into());
    args.push(preset.audio_codec.clone());
    args.push("-b:a".into());
    args.push(format!("{}k", preset.audio_bitrate_kbps));
    args.push("-movflags".into());
    args.push("+faststart".into());
    args.push(output_str.clone());

    let out = tokio::process::Command::new(&state.ffmpeg.ffmpeg)
        .args(&args)
        .output()
        .await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let _ = queries::mark_optimized_finished(
            &state.pool,
            row.id,
            false,
            None,
            None,
            Some(&stderr.chars().take(1024).collect::<String>()),
        )
        .await;
        let _ = tokio::fs::remove_file(&output).await;
        anyhow::bail!("ffmpeg exited {}", out.status);
    }

    let meta = tokio::fs::metadata(&output).await?;
    queries::mark_optimized_finished(
        &state.pool,
        row.id,
        true,
        Some(meta.len() as i64),
        Some(started.elapsed().as_millis() as i64),
        None,
    )
    .await?;
    Ok(())
}

fn append_log(buf: &Arc<std::sync::Mutex<String>>, line: impl Into<String>) {
    if let Ok(mut s) = buf.lock() {
        if !s.is_empty() {
            s.push('\n');
        }
        s.push_str(&line.into());
    }
}

/// Task-kind catalog for the admin UI. Each entry describes the kind name,
/// human label, which JSON params it accepts, and the recommended
/// schedule (frequency + maintenance-window eligibility). The UI
/// pre-fills these when the operator creates a new task of the kind so
/// the typical case is "click Create" with no schedule tweaks.
/// Keep this in sync with the `dispatch` match arms.
pub fn registry() -> Vec<TaskKindInfo> {
    vec![
        TaskKindInfo {
            kind: "prune_sessions",
            display_name: "Prune expired sessions",
            description: "Delete session rows past their expires_at.",
            params_schema: r#"{}"#,
            default_frequency: "hourly",
            default_requires_maintenance_window: false,
        },
        TaskKindInfo {
            kind: "backup_db",
            display_name: "Backup database",
            description: "VACUUM INTO snapshot under data/backups/auto/.",
            params_schema: r#"{}"#,
            default_frequency: "daily",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "scan_library",
            display_name: "Scan library",
            description: "Walk a library's paths and refresh media_files.",
            params_schema: r#"{"library_id": "number (required)"}"#,
            // File watcher fires on-change scans; operator can still set
            // a periodic safety-net schedule if they want one.
            default_frequency: "on_change",
            default_requires_maintenance_window: false,
        },
        TaskKindInfo {
            kind: "refresh_metadata",
            display_name: "Refresh metadata",
            description: "Re-pull metadata for every item (optionally restricted to one library).",
            params_schema: r#"{"library_id": "number (optional)"}"#,
            default_frequency: "weekly",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "detect_markers",
            display_name: "Detect markers (intros/credits)",
            description: "Walk files in a library that don't yet have \
                          auto markers and run chapter + blackdetect \
                          analysis. Idempotent — files with existing \
                          auto markers are skipped. Heavy: capped per \
                          tick via params.batch_size (default 32).",
            params_schema:
                r#"{"library_id": "number (required)", "batch_size": "number (optional; default 32)"}"#,
            default_frequency: "weekly",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "fetch_subtitles",
            display_name: "Fetch external subtitles",
            description: "Pull subtitles from OpenSubtitles for items \
                          missing them, in the configured languages.",
            params_schema:
                r#"{"library_id": "number (optional)", "languages": "string[] (optional; default ['en'])"}"#,
            default_frequency: "weekly",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "generate_previews",
            display_name: "Generate scrub-preview sprites",
            description: "Build per-file thumbnail sprites the player uses \
                          for hover/scrub previews. Idempotent — files with \
                          a sprite already are skipped.",
            params_schema:
                r#"{"library_id": "number (optional)", "batch_size": "number (optional; default 4)", "interval_s": "number (optional; default 10)"}"#,
            default_frequency: "daily",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "generate_chapter_thumbs",
            display_name: "Generate chapter thumbnails",
            description: "Extract one thumbnail per container chapter so \
                          the seek menu can show a poster for each act. \
                          Files without chapter metadata are marked \
                          processed on first pass so they don't get \
                          re-probed.",
            params_schema:
                r#"{"library_id": "number (optional)", "batch_size": "number (optional; default 8)"}"#,
            default_frequency: "weekly",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "verify_backups",
            display_name: "Verify auto-backups",
            description: "Open every snapshot under `<data_dir>/backups/auto/` \
                          read-only, run `PRAGMA integrity_check` and \
                          `PRAGMA foreign_key_check`. A broken backup that \
                          nobody discovers until restore is the worst case; \
                          this catches corruption proactively. Failures \
                          surface in the task log.",
            params_schema: r#"{}"#,
            default_frequency: "weekly",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "analyze_loudness",
            display_name: "Analyze loudness (EBU R 128)",
            description: "Run ffmpeg's loudnorm filter on every media \
                          file to measure integrated loudness, true peak, \
                          loudness range, and noise floor. The transcoder \
                          uses these for precise per-file normalization \
                          when audio_normalize is enabled. Slow — ~2 min \
                          per 45-min episode.",
            params_schema:
                r#"{"library_id": "number (optional)", "batch_size": "number (optional; default 4)"}"#,
            default_frequency: "weekly",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "trakt_pull",
            display_name: "Trakt: pull history + playback",
            description: "Import recent Trakt watch history and resume \
                          points for every linked user. Runs incrementally \
                          using each user's last_synced_at cursor.",
            params_schema: r#"{}"#,
            default_frequency: "hourly",
            default_requires_maintenance_window: false,
        },
        TaskKindInfo {
            kind: "refresh_trending",
            display_name: "Refresh trending (Top 10)",
            description: "Pull the weekly global trending list from TMDB \
                          (movies + shows). The home page intersects this \
                          with the local library to render a Top 10 rail.",
            params_schema: r#"{}"#,
            default_frequency: "daily",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "refresh_logos",
            display_name: "Refresh title-treatment logos",
            description: "Backfill the transparent title logo art used \
                          by the modal hero for items that don't have \
                          one yet. Idempotent — only items with a \
                          tmdb_id and no logo_path are touched.",
            params_schema: r#"{"batch_size": "number (optional; default 50)"}"#,
            default_frequency: "weekly",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "optimize_versions",
            display_name: "Optimize versions",
            description: "Process the next batch of queued optimized-version \
                          jobs (re-encodes to operator-chosen presets).",
            params_schema: r#"{"batch_size": "number (optional; default 2)"}"#,
            default_frequency: "daily",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "verify_libraries",
            display_name: "Verify libraries (find missing files)",
            description: "Stat() every media_file and soft-delete missing ones.",
            params_schema: r#"{}"#,
            default_frequency: "weekly",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "purge_removed_files",
            display_name: "Purge files removed past the grace window",
            description: "Hard-delete media_files whose removed_at is older \
                          than `grace_days`. Cascade-cleans orphan episodes, \
                          seasons, and items.",
            params_schema: r#"{"grace_days": "number (optional; default 7)"}"#,
            default_frequency: "daily",
            default_requires_maintenance_window: true,
        },
        TaskKindInfo {
            kind: "cleanup_audit_log",
            display_name: "Trim audit log",
            description: "Delete audit_log rows older than `retention_days` \
                          and sweep expired password-reset tokens.",
            params_schema: r#"{"retention_days": "number (optional; default 90)"}"#,
            default_frequency: "daily",
            default_requires_maintenance_window: true,
        },
    ]
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskKindInfo {
    pub kind: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub params_schema: &'static str,
    /// Frequency the admin UI pre-selects when an operator creates a
    /// task of this kind. One of the values accepted by
    /// `frequency_interval_ms` plus `manual`, `on_change`, `custom`.
    pub default_frequency: &'static str,
    /// Pre-checked window toggle. Heavy tasks (scans, full refreshes,
    /// backup) default true so they don't compete with playback.
    pub default_requires_maintenance_window: bool,
}

/// Walk every item lacking a stored subtitle in each requested language,
/// search OpenSubtitles by tmdb/imdb id (+ season/episode for shows),
/// and download the top hit. Best-effort; per-item failures do not fail
/// the whole task.
/// Safety-net sweep that enqueues `generate_preview_sprite` jobs
/// for files lacking a sprite. The actual ffmpeg work happens in
/// the queue worker, not inline here. This catches files that
/// either pre-date the discovery-pipeline migration or whose
/// on-discovery job failed past max_attempts.
async fn generate_previews_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let library_id = params.get("library_id").and_then(|v| v.as_i64());
    let per_library_cap = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(500)
        .max(1);

    let candidates = queries::list_media_files_needing_previews(
        &state.pool,
        library_id,
        per_library_cap,
    )
    .await?;
    if candidates.is_empty() {
        append_log(log, "no files need previews — queue is the active path");
        return Ok(());
    }
    let file_ids: Vec<i64> = candidates.iter().map(|c| c.id).collect();
    let enqueued = crate::jobs::handlers::generate_preview_sprite::enqueue_for_files(
        &state.pool,
        &file_ids,
    )
    .await?;
    append_log(
        log,
        format!("enqueued {enqueued} generate_preview_sprite jobs"),
    );
    Ok(())
}

/// Safety-net sweep that enqueues `build_chapter_thumbs` jobs for
/// files that haven't been chapter-probed yet. Same shape as
/// generate_previews_task — the queue worker does the work.
async fn generate_chapter_thumbs_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let library_id = params.get("library_id").and_then(|v| v.as_i64());
    let per_library_cap = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(500)
        .max(1);

    let candidates = queries::list_media_files_needing_chapter_thumbs(
        &state.pool,
        library_id,
        per_library_cap,
    )
    .await?;
    if candidates.is_empty() {
        append_log(log, "no files need chapter thumbs — queue is the active path");
        return Ok(());
    }
    let file_ids: Vec<i64> = candidates.iter().map(|c| c.id).collect();
    let enqueued = crate::jobs::handlers::build_chapter_thumbs::enqueue_for_files(
        &state.pool,
        &file_ids,
    )
    .await?;
    append_log(
        log,
        format!("enqueued {enqueued} build_chapter_thumbs jobs"),
    );
    Ok(())
}

/// Safety-net sweep that enqueues `analyze_loudness` jobs for
/// files whose audio hasn't been measured yet. The queue worker
/// does the ffmpeg loudnorm pass.
async fn analyze_loudness_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let library_id = params.get("library_id").and_then(|v| v.as_i64());
    let per_library_cap = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(500)
        .max(1);

    let candidates = queries::list_media_files_needing_loudness(
        &state.pool,
        library_id,
        per_library_cap,
    )
    .await?;
    if candidates.is_empty() {
        append_log(log, "no files need loudness — queue is the active path");
        return Ok(());
    }
    let file_ids: Vec<i64> = candidates.iter().map(|c| c.id).collect();
    let enqueued = crate::jobs::handlers::analyze_loudness::enqueue_for_files(
        &state.pool,
        &file_ids,
    )
    .await?;
    append_log(log, format!("enqueued {enqueued} analyze_loudness jobs"));
    Ok(())
}

async fn verify_backups_task(
    state: &AppState,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::ConnectOptions;
    use std::str::FromStr;

    let dir = state
        .data_dir
        .join(crate::api::admin::backup::AUTO_BACKUP_SUBDIR);
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            append_log(log, "no backups dir yet — nothing to verify");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    while let Ok(Some(ent)) = entries.next_entry().await {
        let path = ent.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("chimpflix-") && n.ends_with(".db"))
        {
            files.push(path);
        }
    }
    if files.is_empty() {
        append_log(log, "no backup files found under <data_dir>/backups/auto/");
        return Ok(());
    }

    let mut ok = 0usize;
    let mut bad: Vec<String> = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        // Quick header check first — a malformed SQLite file fails
        // here before we even spawn a pool.
        match tokio::fs::read(&path).await {
            Ok(bytes) if bytes.len() >= 16 && &bytes[..16] == b"SQLite format 3\0" => {}
            Ok(_) => {
                bad.push(format!("{name}: not a SQLite file (bad header)"));
                continue;
            }
            Err(e) => {
                bad.push(format!("{name}: read error: {e}"));
                continue;
            }
        }
        // Open read-only + run integrity check. Read-only prevents a
        // verification from accidentally journaling and bumping the
        // backup's mtime / inode.
        let url = format!("sqlite://{}?mode=ro", path.display());
        let opts = match sqlx::sqlite::SqliteConnectOptions::from_str(&url) {
            Ok(o) => o.disable_statement_logging(),
            Err(e) => {
                bad.push(format!("{name}: parse url: {e}"));
                continue;
            }
        };
        let pool = match SqlitePoolOptions::new()
            .max_connections(1)
            .acquire_timeout(std::time::Duration::from_secs(10))
            .connect_with(opts)
            .await
        {
            Ok(p) => p,
            Err(e) => {
                bad.push(format!("{name}: open: {e}"));
                continue;
            }
        };
        let integrity: Result<String, _> =
            sqlx::query_scalar("PRAGMA integrity_check").fetch_one(&pool).await;
        match integrity {
            Ok(s) if s == "ok" => ok += 1,
            Ok(s) => bad.push(format!("{name}: integrity_check returned `{s}`")),
            Err(e) => bad.push(format!("{name}: integrity_check failed: {e}")),
        }
        pool.close().await;
    }

    if bad.is_empty() {
        append_log(log, format!("verified {ok} backup file(s), all OK"));
    } else {
        for b in &bad {
            append_log(log, b);
        }
        append_log(
            log,
            format!("{ok} ok, {} corrupted/unreadable", bad.len()),
        );
        // Surface as a task error so the operator's alerts panel
        // flags it — silent log entries would be too easy to miss.
        anyhow::bail!("{} of {} backups failed verification", bad.len(), ok + bad.len());
    }
    Ok(())
}

/// Pull TMDB's weekly trending movies + shows and upsert into the
/// `trending_cache` table. Skips silently when TMDB isn't configured —
/// the rail just renders empty until the operator adds a token.
async fn refresh_trending_task(
    state: &AppState,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let Some(tmdb) = state.tmdb_snapshot().await else {
        append_log(log, "TMDB disabled — skipping trending refresh");
        return Ok(());
    };
    let movies = tmdb.trending_movies().await?;
    let shows = tmdb.trending_shows().await?;
    let m_count = queries::replace_trending(
        &state.pool,
        "tmdb",
        "movie",
        &movies
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, m)| chimpflix_library::TrendingEntry {
                rank: (i as i64) + 1,
                tmdb_id: m.tmdb_id,
                title: Some(m.title.clone()),
                poster_path: m.poster_path.clone(),
            })
            .collect::<Vec<_>>(),
    )
    .await?;
    let s_count = queries::replace_trending(
        &state.pool,
        "tmdb",
        "show",
        &shows
            .iter()
            .take(10)
            .enumerate()
            .map(|(i, s)| chimpflix_library::TrendingEntry {
                rank: (i as i64) + 1,
                tmdb_id: s.tmdb_id,
                title: Some(s.title.clone()),
                poster_path: s.poster_path.clone(),
            })
            .collect::<Vec<_>>(),
    )
    .await?;
    append_log(
        log,
        format!("cached {m_count} trending movies, {s_count} trending shows"),
    );
    Ok(())
}

/// Walk every library and verify each media_file's underlying path
/// still exists on disk. Files that have gone missing are soft-deleted
/// (`removed_at` timestamped) — they keep existing for a grace window
/// so a temporary unmount doesn't immediately destroy associated play
/// state / markers / preview sprites. A separate purge task hard-
/// deletes them once the window expires.
///
/// This task aggregates across every library and emits one summary
/// log line; per-library counts are still surfaced via the admin
/// "Verify now" button which calls the underlying function directly
/// and returns a structured report.
async fn verify_libraries_task(
    state: &AppState,
    _task: &chimpflix_library::ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let libraries = queries::list_libraries(&state.pool, None).await?;
    let mut totals = (0usize, 0u64, 0usize, 0usize);
    // (libs_processed, newly_marked, still_missing, returned)
    for lib in &libraries {
        match queries::verify_library(&state.pool, lib.id).await {
            Ok(report) => {
                totals.0 += 1;
                totals.1 += report.newly_marked_removed;
                totals.2 += report.still_missing;
                totals.3 += report.returned_files;
                if report.newly_marked_removed > 0
                    || report.still_missing > 0
                    || report.returned_files > 0
                {
                    append_log(
                        log,
                        format!(
                            "library {} ({}): checked={} missing={} new_removed={} still_missing={} returned={}",
                            lib.id,
                            lib.name,
                            report.files_checked,
                            report.files_missing,
                            report.newly_marked_removed,
                            report.still_missing,
                            report.returned_files
                        ),
                    );
                }
            }
            Err(e) => {
                warn!(
                    library_id = lib.id,
                    error = %format!("{e:#}"),
                    "library verify failed"
                );
                append_log(log, format!("library {}: error: {e:#}", lib.id));
            }
        }
    }
    append_log(
        log,
        format!(
            "verified {} libraries — {} newly marked removed, {} still missing, {} returned",
            totals.0, totals.1, totals.2, totals.3
        ),
    );
    Ok(())
}

/// Hard-delete media_files whose `removed_at` is older than the
/// configured grace window, then cascade-sweep orphaned episodes /
/// seasons / items. Grace window comes from `params_json`:
/// `{"grace_days": N}`. The cascade order matters and lives inside
/// [`queries::purge_removed_media_files`].
async fn purge_removed_files_task(
    state: &AppState,
    task: &chimpflix_library::ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let grace_days = params
        .get("grace_days")
        .and_then(|v| v.as_i64())
        .unwrap_or(7)
        .max(0);
    let cutoff_ms = now_ms() - grace_days * 86_400_000;
    let report = queries::purge_removed_media_files(&state.pool, cutoff_ms).await?;
    // Evict per-file WebVTT cache for any path we just hard-deleted.
    // Spawned so a bulk purge doesn't stall the scheduler tick.
    if !report.purged_paths.is_empty() {
        let cache_root = state.transcoder.cache_root().to_path_buf();
        let paths = report.purged_paths.clone();
        tokio::spawn(async move {
            for p in paths {
                let _ = chimpflix_transcoder::evict_text_subs_cache(
                    &cache_root,
                    std::path::Path::new(&p),
                )
                .await;
            }
        });
    }
    if report.files_purged > 0
        || report.episodes_purged > 0
        || report.seasons_purged > 0
        || report.items_purged > 0
    {
        append_log(
            log,
            format!(
                "grace={}d cutoff={} purged: files={} episodes={} seasons={} items={}",
                grace_days,
                cutoff_ms,
                report.files_purged,
                report.episodes_purged,
                report.seasons_purged,
                report.items_purged
            ),
        );
    } else {
        append_log(log, format!("grace={}d nothing to purge", grace_days));
    }
    Ok(())
}

/// Fetch the TMDB title-treatment logo for items that don't have one
/// yet. Skips items without a tmdb_id (we can't look anything up) and
/// items that already have a `logo_path`. Caps per-run at `batch_size`
/// so a fresh server with thousands of items doesn't hammer TMDB in
/// one go — operators can re-run until the backlog drains.
/// Daily rollup: bucket the previous UTC day's finished jobs by
/// kind, compute success/failure counts + p50/p95 duration, upsert
/// into `task_kind_metrics_daily`.
///
/// Reads from `jobs`, so this must run *before* `cleanup_jobs`
/// trims old terminal rows — otherwise yesterday's succeeded jobs
/// would already be gone (cleanup defaults: succeeded 7d, dead 30d,
/// which gives the rollup plenty of overlap, but the ordering is
/// still worth respecting via maintenance-window slot allocation).
async fn rollup_task_metrics_task(
    state: &AppState,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    use std::collections::HashMap;

    // Yesterday's UTC midnight bounds, computed from now() truncated
    // to the day. `chrono` is already in the workspace.
    let now = chimpflix_common::now_ms();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let today_midnight = (now / day_ms) * day_ms;
    let yesterday_midnight = today_midnight - day_ms;
    let day_key = yesterday_midnight; // store epoch-ms of the day's UTC start

    let rows = queries::list_finished_jobs_in_window(
        &state.pool,
        yesterday_midnight,
        today_midnight,
    )
    .await?;

    // Bucket per kind, accumulating durations + status counts.
    struct Bucket {
        success: i64,
        failure: i64,
        durations: Vec<i64>,
    }
    let mut buckets: HashMap<String, Bucket> = HashMap::new();
    for r in &rows {
        let entry = buckets.entry(r.kind.clone()).or_insert(Bucket {
            success: 0,
            failure: 0,
            durations: Vec::new(),
        });
        match r.status.as_str() {
            "succeeded" => entry.success += 1,
            "dead" => entry.failure += 1,
            _ => continue,
        }
        if let (Some(start), Some(finish)) = (r.started_at, r.finished_at) {
            let d = finish.saturating_sub(start);
            if d >= 0 {
                entry.durations.push(d);
            }
        }
    }

    let mut wrote = 0usize;
    for (kind, mut b) in buckets {
        b.durations.sort_unstable();
        let p50 = pct(&b.durations, 0.50);
        let p95 = pct(&b.durations, 0.95);
        let targets = b.success + b.failure;
        queries::upsert_task_metrics_daily(
            &state.pool,
            day_key,
            &kind,
            b.success,
            b.failure,
            p50,
            p95,
            targets,
        )
        .await?;
        wrote += 1;
    }
    append_log(
        log,
        format!(
            "rolled up {wrote} kinds for {} (window {} → {})",
            yesterday_midnight, yesterday_midnight, today_midnight,
        ),
    );
    Ok(())
}

/// Index a sorted slice at the given percentile in [0.0, 1.0].
/// Returns None for an empty slice. Uses the simple nearest-rank
/// method which is exact for any percentile.
fn pct(sorted: &[i64], q: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
    sorted.get(idx).copied()
}

/// Sweep: enqueue one `fetch_external_ratings` job per item whose
/// ratings are missing or stale (>30 days). The per-item handler
/// dedups against the same staleness window, so re-enqueueing a
/// fresh item is a no-op.
async fn refresh_ratings_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let batch = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(100);
    use sqlx::Row;
    let stale_cutoff = chimpflix_common::now_ms()
        - crate::jobs::handlers::fetch_external_ratings::RATINGS_STALE_MS;
    let rows = sqlx::query(
        "SELECT id FROM items
         WHERE (ratings_updated_at IS NULL OR ratings_updated_at < ?)
           AND imdb_id IS NOT NULL
         ORDER BY id LIMIT ?",
    )
    .bind(stale_cutoff)
    .bind(batch)
    .fetch_all(&state.pool)
    .await?;
    let item_ids: Vec<i64> = rows
        .iter()
        .filter_map(|r| r.try_get::<i64, _>("id").ok())
        .collect();

    let queued = crate::jobs::handlers::fetch_external_ratings::enqueue_for_items(
        &state.pool,
        &item_ids,
    )
    .await?;
    append_log(
        log,
        format!(
            "ratings: enqueued {queued} per-item jobs (batch {} items)",
            item_ids.len()
        ),
    );
    Ok(())
}

/// Sweep: enqueue one `extract_embedded_subs` job per file whose
/// container subtitles haven't been extracted yet.
async fn extract_subs_sweep_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let batch = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(500);
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id FROM media_files
         WHERE embedded_subs_extracted_at IS NULL
           AND removed_at IS NULL
         ORDER BY id LIMIT ?",
    )
    .bind(batch)
    .fetch_all(&state.pool)
    .await?;
    let file_ids: Vec<i64> = rows
        .iter()
        .filter_map(|r| r.try_get::<i64, _>("id").ok())
        .collect();

    let queued = crate::jobs::handlers::extract_embedded_subs::enqueue_for_files(
        &state.pool,
        &file_ids,
    )
    .await?;
    append_log(
        log,
        format!(
            "embedded subs: enqueued {queued} per-file jobs (batch {} files)",
            file_ids.len()
        ),
    );
    Ok(())
}

/// Sweep: enqueue one `detect_extras_item` job per item that hasn't
/// been scanned yet or whose parent directory mtime has advanced.
/// The walk + insert work lives in the per-item handler.
async fn scan_extras_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let batch = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(50);

    // Pick items either never scanned or stale (>30 days). The
    // handler dedups on parent-dir mtime, so re-enqueueing a fresh
    // item is a no-op there too.
    use sqlx::Row;
    let stale_cutoff = chimpflix_common::now_ms() - 30 * 24 * 60 * 60 * 1000;
    let rows = sqlx::query(
        "SELECT id FROM items
         WHERE extras_scanned_at IS NULL OR extras_scanned_at < ?
         ORDER BY id LIMIT ?",
    )
    .bind(stale_cutoff)
    .bind(batch)
    .fetch_all(&state.pool)
    .await?;
    let item_ids: Vec<i64> = rows
        .iter()
        .filter_map(|r| r.try_get::<i64, _>("id").ok())
        .collect();

    let queued = crate::jobs::handlers::detect_extras_item::enqueue_for_items(
        &state.pool,
        &item_ids,
    )
    .await?;
    append_log(
        log,
        format!(
            "extras: enqueued {queued} per-item jobs (batch {} items)",
            item_ids.len()
        ),
    );
    Ok(())
}

/// Sweep: enqueue one `refresh_logos_item` job per item missing a
/// logo. The actual TMDB fetch + DB write lives in the per-item
/// handler ([`crate::jobs::handlers::refresh_logos_item`]) — this
/// function just feeds the queue.
///
/// Pre-job-queue this function used to do the TMDB fetches inline,
/// which meant a stuck network call blocked the entire sweep, retry
/// semantics were ad-hoc, and per-item failures muddied the task
/// outcome. Moving to per-item jobs gives each item its own retry
/// curve under the worker pool's per-kind concurrency cap.
async fn refresh_logos_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    if state.tmdb_snapshot().await.is_none() {
        append_log(log, "TMDB disabled — skipping logo refresh");
        return Ok(());
    }
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let batch = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(50);

    // Pick a bounded batch of items missing a logo. ORDER BY id keeps
    // iteration deterministic across runs so a backlog drains
    // predictably; the per-item handler dedups on item_id so a re-run
    // before workers finish is a no-op for in-flight items.
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id FROM items \
         WHERE logo_path IS NULL AND tmdb_id IS NOT NULL \
         ORDER BY id LIMIT ?",
    )
    .bind(batch)
    .fetch_all(&state.pool)
    .await?;
    let item_ids: Vec<i64> = rows
        .iter()
        .filter_map(|r| r.try_get::<i64, _>("id").ok())
        .collect();

    let queued = crate::jobs::handlers::refresh_logos_item::enqueue_for_items(
        &state.pool,
        &item_ids,
    )
    .await?;
    append_log(
        log,
        format!(
            "logos: enqueued {queued} per-item jobs (batch {} items)",
            item_ids.len()
        ),
    );
    Ok(())
}

/// Pull Trakt history + playback for every linked user. Per-user
/// failures log and the task itself still succeeds — one bad token
/// shouldn't poison the run.
async fn trakt_pull_task(
    state: &AppState,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    if state.trakt_snapshot().await.is_none() {
        append_log(log, "Trakt disabled — skipping pull");
        return Ok(());
    }
    let user_ids = queries::list_trakt_linked_user_ids(&state.pool).await?;
    let mut total_movies = 0usize;
    let mut total_episodes = 0usize;
    let mut total_playback = 0usize;
    for uid in &user_ids {
        match crate::trakt_sync::pull_user_history(state, *uid).await {
            Ok((m, e)) => {
                total_movies += m;
                total_episodes += e;
            }
            Err(e) => warn!(user_id = uid, error = %format!("{e:#}"), "trakt pull history failed"),
        }
        match crate::trakt_sync::pull_user_playback(state, *uid).await {
            Ok(n) => total_playback += n,
            Err(e) => warn!(user_id = uid, error = %format!("{e:#}"), "trakt pull playback failed"),
        }
    }
    append_log(
        log,
        format!(
            "trakt pull: {} users, {} movies, {} episodes marked watched, {} resume points applied",
            user_ids.len(),
            total_movies,
            total_episodes,
            total_playback,
        ),
    );
    Ok(())
}

/// Safety-net sweep: finds every item that lacks a subtitle row
/// for any configured language and enqueues one
/// `fetch_subtitles_item` job per item. The job queue worker does
/// the OpenSubtitles call.
///
/// Per-item dedup means re-running while jobs are in flight is a
/// no-op. The actual hit/miss/error counting happens inside the
/// handler; this task just reports how many jobs it queued.
async fn fetch_subtitles_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    if state.opensubtitles_snapshot().await.is_none() {
        append_log(log, "OpenSubtitles disabled — set credentials in /admin/server/credentials");
        return Ok(());
    };
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let library_id = params.get("library_id").and_then(|v| v.as_i64());
    let languages: Vec<String> = params
        .get("languages")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_else(|| vec!["en".to_string()]);

    // Item ids with at least one external metadata id — those are the
    // only ones the provider can look up. The handler does its own
    // existing-subtitle check per (target, language) so we don't need
    // to filter on that here.
    let item_ids: Vec<i64> = if let Some(lid) = library_id {
        sqlx::query_scalar(
            "SELECT id FROM items
             WHERE library_id = ?
               AND (tmdb_id IS NOT NULL OR imdb_id IS NOT NULL)",
        )
        .bind(lid)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT id FROM items
             WHERE tmdb_id IS NOT NULL OR imdb_id IS NOT NULL",
        )
        .fetch_all(&state.pool)
        .await?
    };

    let enqueued = crate::jobs::handlers::fetch_subtitles_item::enqueue_for_items(
        &state.pool,
        &item_ids,
        &languages,
    )
    .await?;
    append_log(
        log,
        format!(
            "enqueued {enqueued} fetch_subtitles_item jobs across {} items",
            item_ids.len()
        ),
    );
    Ok(())
}

// Inline `fetch_one_for_item` / `fetch_one_for_episode` helpers
// moved to `crate::subtitles_lookup` as part of the subtitle job
// migration. The handler in `crate::jobs::handlers::fetch_subtitles_item`
// is the active caller; the safety-net scheduled task above just
// enqueues per-item jobs.

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Local, TimeZone};

    fn local_ms(year: i32, month: u32, day: u32, hour: u32, min: u32) -> i64 {
        Local
            .with_ymd_and_hms(year, month, day, hour, min, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn frequency_intervals_are_sane() {
        assert_eq!(frequency_interval_ms("hourly"), Some(3_600_000));
        assert_eq!(frequency_interval_ms("daily"), Some(86_400_000));
        assert_eq!(frequency_interval_ms("weekly"), Some(604_800_000));
        assert_eq!(frequency_interval_ms("monthly"), Some(2_592_000_000));
        assert_eq!(frequency_interval_ms("custom"), None);
        assert_eq!(frequency_interval_ms("manual"), None);
        assert_eq!(frequency_interval_ms("on_change"), None);
    }

    #[test]
    fn snap_inside_window_returns_input() {
        let t = local_ms(2026, 5, 18, 3, 0);
        let out = snap_to_maintenance_window(t, "02:00", "09:00");
        assert_eq!(out, t);
    }

    #[test]
    fn snap_before_window_snaps_to_today_start() {
        let t = local_ms(2026, 5, 18, 1, 0);
        let want = local_ms(2026, 5, 18, 2, 0);
        let out = snap_to_maintenance_window(t, "02:00", "09:00");
        assert_eq!(out, want);
    }

    #[test]
    fn snap_after_window_snaps_to_tomorrow_start() {
        let t = local_ms(2026, 5, 18, 12, 0);
        let want = local_ms(2026, 5, 19, 2, 0);
        let out = snap_to_maintenance_window(t, "02:00", "09:00");
        assert_eq!(out, want);
    }

    #[test]
    fn snap_wraparound_window_inside_03_00() {
        // Window 22:00 → 06:00 wraps midnight; 03:00 sits inside the
        // window that opened the previous day.
        let t = local_ms(2026, 5, 18, 3, 0);
        let out = snap_to_maintenance_window(t, "22:00", "06:00");
        assert_eq!(out, t);
    }

    #[test]
    fn snap_wraparound_window_10_00_snaps_to_today_22() {
        let t = local_ms(2026, 5, 18, 10, 0);
        let want = local_ms(2026, 5, 18, 22, 0);
        let out = snap_to_maintenance_window(t, "22:00", "06:00");
        assert_eq!(out, want);
    }

    #[test]
    fn compute_next_run_manual_never_fires() {
        let now = local_ms(2026, 5, 18, 12, 0);
        let next =
            compute_next_run("manual", "0 0 * * * *", now, false, "02:00", "09:00").unwrap();
        assert_eq!(next, NEVER_RUN_AT_MS);
    }

    #[test]
    fn compute_next_run_on_change_never_fires() {
        let now = local_ms(2026, 5, 18, 12, 0);
        let next = compute_next_run("on_change", "", now, false, "02:00", "09:00").unwrap();
        assert_eq!(next, NEVER_RUN_AT_MS);
    }

    #[test]
    fn compute_next_run_hourly_no_window_is_plus_one_hour() {
        let now = local_ms(2026, 5, 18, 12, 0);
        let next =
            compute_next_run("hourly", "0 0 * * * *", now, false, "02:00", "09:00").unwrap();
        assert_eq!(next, now + 3_600_000);
    }

    #[test]
    fn compute_next_run_daily_with_window_snaps_forward() {
        // now+24h = May 19 12:00 — past that day's window (closed at
        // 09:00) — should snap to May 20 02:00.
        let now = local_ms(2026, 5, 18, 12, 0);
        let next =
            compute_next_run("daily", "0 0 3 * * *", now, true, "02:00", "09:00").unwrap();
        let want = local_ms(2026, 5, 20, 2, 0);
        assert_eq!(next, want);
    }

    #[test]
    fn compute_next_run_custom_uses_cron() {
        // 5-field cron normalized to 7-field: every minute. After
        // 12:00:00 the next firing is at most ~60s later.
        let now = local_ms(2026, 5, 18, 12, 0);
        let next = compute_next_run("custom", "* * * * *", now, false, "02:00", "09:00")
            .unwrap();
        assert!(next > now);
        assert!(next - now <= Duration::minutes(2).num_milliseconds());
    }

    #[test]
    fn snap_garbage_hhmm_uses_fallback() {
        // Invalid window strings shouldn't panic the scheduler — the
        // fallbacks recover the default 02:00 → 09:00 behavior.
        let t = local_ms(2026, 5, 18, 12, 0);
        let out = snap_to_maintenance_window(t, "garbage", "more garbage");
        let want = local_ms(2026, 5, 19, 2, 0);
        assert_eq!(out, want);
    }
}

