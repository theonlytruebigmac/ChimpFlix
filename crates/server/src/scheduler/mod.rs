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
use chimpflix_library::{NewExternalSubtitle, NewScheduledTask, ScheduledTask};
use chimpflix_metadata::{OpenSubtitlesClient, SearchParams};
use chrono::{Local, NaiveTime, TimeZone, Utc};
use cron::Schedule;
use sqlx::Row;
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
/// out-of-band of the schedule. Concurrency control matches the normal
/// path: the task row is marked `running` before dispatch.
pub async fn run_now(state: AppState, task_id: i64) -> Result<()> {
    let Some(task) = queries::get_scheduled_task(&state.pool, task_id).await? else {
        bail!("task {task_id} not found");
    };
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
    match task.kind.as_str() {
        "prune_sessions" => {
            let removed = queries::cleanup_expired_sessions(&state.pool).await?;
            append_log(log, format!("pruned {removed} expired sessions"));
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
            let token_removed =
                queries::cleanup_expired_password_reset_tokens(&state.pool).await?;
            append_log(
                log,
                format!(
                    "trimmed {removed} audit rows older than {retention_days}d; \
                     also dropped {token_removed} expired password-reset tokens"
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
            tokio::spawn(async move {
                let emitter: chimpflix_library::ScanEmitter = Arc::new(move |evt| {
                    hub.publish(crate::events::Event::Scan(evt));
                });
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
            // Walk every media file in the library that doesn't yet
            // have auto-detected markers, run chapter+blackdetect on
            // each, and persist results. Idempotent across runs: the
            // query filters out files that already have an `auto`
            // marker, so re-running only processes newcomers (typical
            // case after a new episode lands).
            //
            // Per-file failures (corrupt file, ffprobe couldn't seek)
            // log + count toward `err` but don't fail the task —
            // they'd otherwise poison the whole batch and block all
            // remaining files. Operators re-trigger from the item
            // modal if they want to force a retry on a specific file.
            //
            // Batch caps the per-tick work so the scheduler doesn't
            // freeze for an hour on a fresh library; subsequent ticks
            // pick up the remainder.
            // params.library_id is optional: scoped to one library
            // when present, all libraries when omitted. The Plex-style
            // simple-view toggle creates this task with `{}` params
            // (meaning "all"); operators who want a per-library
            // schedule create a separate task row via Advanced.
            let params: serde_json::Value =
                serde_json::from_str(&task.params_json).unwrap_or_default();
            let scoped_library_id = params.get("library_id").and_then(|v| v.as_i64());
            let batch = params
                .get("batch_size")
                .and_then(|v| v.as_i64())
                .unwrap_or(32);

            let library_ids: Vec<i64> = match scoped_library_id {
                Some(lid) => vec![lid],
                None => queries::list_libraries(&state.pool, None)
                    .await?
                    .into_iter()
                    .map(|l| l.id)
                    .collect(),
            };

            // Walk every library (or the one requested), draining up
            // to `batch` files per library per tick. Hard per-tick
            // ceiling = batch * library_count; subsequent ticks pick
            // up remaining files. Per-file failures log + count toward
            // `err` but don't fail the task — they'd otherwise poison
            // the whole batch and block remaining files. Operators
            // re-trigger from the item modal if they want to force a
            // retry on a specific file.
            let mut ok = 0usize;
            let mut empty = 0usize;
            let mut err = 0usize;
            let mut total_pending = 0usize;
            for lib_id in &library_ids {
                let pending = queries::list_media_files_needing_markers(
                    &state.pool,
                    *lib_id,
                    batch,
                )
                .await?;
                total_pending += pending.len();
                for (file_id, path, duration_ms) in &pending {
                    match chimpflix_transcoder::detect_markers(
                        &state.ffmpeg,
                        std::path::Path::new(path),
                        *duration_ms,
                    )
                    .await
                    {
                        Ok(detected) => {
                            if detected.is_empty() {
                                empty += 1;
                            }
                            let tuples: Vec<(String, i64, i64)> = detected
                                .into_iter()
                                .map(|m| (m.kind.as_str().to_string(), m.start_ms, m.end_ms))
                                .collect();
                            if let Err(e) =
                                queries::replace_auto_markers(&state.pool, *file_id, &tuples)
                                    .await
                            {
                                err += 1;
                                warn!(
                                    file_id,
                                    error = %format!("{e:#}"),
                                    "detect_markers: persist failed"
                                );
                            } else {
                                ok += 1;
                            }
                        }
                        Err(e) => {
                            err += 1;
                            warn!(
                                file_id,
                                path = %path,
                                error = %format!("{e:#}"),
                                "detect_markers: ffmpeg/probe failed"
                            );
                        }
                    }
                }
            }
            if total_pending == 0 {
                append_log(log, "all files already have auto markers");
            } else {
                append_log(
                    log,
                    format!(
                        "processed {total_pending} files across {} libraries: \
                         {ok} markered, {empty} no-markers, {err} failed",
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
/// Iterate up to `batch_size` media files without a preview sprite and
/// generate one for each. Best-effort: per-file failures log and move on
/// so a single corrupt file doesn't poison the batch.
async fn generate_previews_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let library_id = params.get("library_id").and_then(|v| v.as_i64());
    let batch = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(4)
        .max(1);
    let interval_s = params
        .get("interval_s")
        .and_then(|v| v.as_i64())
        .map(|v| v as u32)
        .unwrap_or(chimpflix_transcoder::DEFAULT_INTERVAL_S);

    let candidates =
        queries::list_media_files_needing_previews(&state.pool, library_id, batch).await?;
    if candidates.is_empty() {
        append_log(log, "no media files need previews");
        return Ok(());
    }

    let dir = state.data_dir.join("previews");
    tokio::fs::create_dir_all(&dir).await?;

    let mut ok = 0usize;
    let mut err = 0usize;
    for cand in &candidates {
        let duration = cand.duration_ms.unwrap_or(0);
        let output = dir.join(format!("{}.jpg", cand.id));
        let result = chimpflix_transcoder::generate_sprite(
            &state.ffmpeg,
            std::path::Path::new(&cand.path),
            &output,
            duration,
            interval_s,
            chimpflix_transcoder::DEFAULT_TILE_WIDTH,
        )
        .await;
        match result {
            Ok(info) => {
                if let Err(e) = queries::record_preview_sprite(
                    &state.pool,
                    queries::PreviewSpriteRecord {
                        media_file_id: cand.id,
                        path: info.path.to_string_lossy().into_owned(),
                        interval_ms: info.interval_ms,
                        tile_width: i64::from(info.tile_width),
                        tile_height: i64::from(info.tile_height),
                        tile_cols: i64::from(info.tile_cols),
                        tile_count: i64::from(info.tile_count),
                    },
                )
                .await
                {
                    err += 1;
                    warn!(file_id = cand.id, error = %format!("{e:#}"), "record preview failed");
                } else {
                    ok += 1;
                }
            }
            Err(e) => {
                err += 1;
                warn!(file_id = cand.id, error = %format!("{e:#}"), "preview generation failed");
            }
        }
    }
    append_log(log, format!("generated {ok} sprites, {err} failed"));
    Ok(())
}

async fn generate_chapter_thumbs_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let library_id = params.get("library_id").and_then(|v| v.as_i64());
    let batch = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(8)
        .max(1);

    let candidates = queries::list_media_files_needing_chapter_thumbs(
        &state.pool,
        library_id,
        batch,
    )
    .await?;
    if candidates.is_empty() {
        append_log(log, "no media files need chapter thumbs");
        return Ok(());
    }

    let root = state.data_dir.join("chapter_thumbs");
    let mut total_chapters = 0usize;
    let mut files_with_chapters = 0usize;
    let mut files_without_chapters = 0usize;
    let mut err = 0usize;

    for cand in &candidates {
        let path = std::path::Path::new(&cand.path);
        // Probe chapters for this file. ffprobe failures are treated
        // as "no chapters" rather than a hard error — many containers
        // don't expose chapter metadata, and the task should keep
        // making progress through the batch.
        let chapters = match chimpflix_transcoder::probe_chapters(&state.ffmpeg, path).await {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!(file_id = cand.id, error = %format!("{e:#}"), "chapter probe failed");
                Vec::new()
            }
        };
        if chapters.is_empty() {
            // Still stamp the file as processed so we don't keep
            // re-probing it. chapter_count = 0 is the "checked, none"
            // marker; a future re-scan can clear it explicitly.
            if let Err(e) = queries::record_chapter_thumbs_generated(&state.pool, cand.id, 0).await {
                warn!(file_id = cand.id, error = %format!("{e:#}"), "stamp no-chapters failed");
                err += 1;
            } else {
                files_without_chapters += 1;
            }
            continue;
        }
        let mut produced = 0usize;
        for (idx, ch) in chapters.iter().enumerate() {
            let output = chimpflix_transcoder::chapter_thumbs::thumb_path(
                &root,
                cand.id,
                idx as u32,
            );
            match chimpflix_transcoder::chapter_thumbs::extract_chapter_thumb(
                &state.ffmpeg,
                path,
                &output,
                ch,
                chimpflix_transcoder::chapter_thumbs::DEFAULT_WIDTH,
            )
            .await
            {
                Ok(()) => produced += 1,
                Err(e) => {
                    warn!(file_id = cand.id, chapter = idx, error = %format!("{e:#}"), "chapter thumb extraction failed");
                }
            }
        }
        if let Err(e) = queries::record_chapter_thumbs_generated(
            &state.pool,
            cand.id,
            chapters.len() as i64,
        )
        .await
        {
            warn!(file_id = cand.id, error = %format!("{e:#}"), "stamp chapter-thumbs failed");
            err += 1;
        }
        total_chapters += produced;
        files_with_chapters += 1;
    }
    append_log(
        log,
        format!(
            "processed {} files: {files_with_chapters} with chapters ({total_chapters} thumbs), \
             {files_without_chapters} without, {err} errored",
            candidates.len()
        ),
    );
    Ok(())
}

async fn analyze_loudness_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let library_id = params.get("library_id").and_then(|v| v.as_i64());
    let batch = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(4)
        .max(1);

    let candidates = queries::list_media_files_needing_loudness(
        &state.pool,
        library_id,
        batch,
    )
    .await?;
    if candidates.is_empty() {
        append_log(log, "no media files need loudness analysis");
        return Ok(());
    }

    let mut measured = 0usize;
    let mut silent = 0usize;
    let mut err = 0usize;
    for cand in &candidates {
        let path = std::path::Path::new(&cand.path);
        let result = chimpflix_transcoder::loudness::measure(&state.ffmpeg, path).await;
        match result {
            Ok(Some(m)) => {
                if let Err(e) = queries::record_loudness_measurement(
                    &state.pool,
                    cand.id,
                    Some(queries::LoudnessMeasurement {
                        integrated: m.integrated,
                        true_peak: m.true_peak,
                        lra: m.lra,
                        threshold: m.threshold,
                    }),
                )
                .await
                {
                    err += 1;
                    warn!(file_id = cand.id, error = %format!("{e:#}"), "record loudness failed");
                } else {
                    measured += 1;
                }
            }
            Ok(None) => {
                // No audio / silent — still stamp so we don't re-probe.
                if let Err(e) = queries::record_loudness_measurement(
                    &state.pool,
                    cand.id,
                    None,
                )
                .await
                {
                    err += 1;
                    warn!(file_id = cand.id, error = %format!("{e:#}"), "record null loudness failed");
                } else {
                    silent += 1;
                }
            }
            Err(e) => {
                err += 1;
                warn!(file_id = cand.id, error = %format!("{e:#}"), "loudness analysis failed");
            }
        }
    }
    append_log(
        log,
        format!(
            "analyzed {}: {measured} measured, {silent} silent / no-audio, {err} errored",
            candidates.len()
        ),
    );
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
async fn refresh_logos_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let Some(tmdb) = state.tmdb_snapshot().await else {
        append_log(log, "TMDB disabled — skipping logo refresh");
        return Ok(());
    };
    let params: serde_json::Value =
        serde_json::from_str(&task.params_json).unwrap_or_default();
    let batch = params
        .get("batch_size")
        .and_then(|v| v.as_i64())
        .unwrap_or(50);

    // Pull (item_id, tmdb_id, kind) tuples for items missing a logo.
    // ORDER BY id keeps the iteration deterministic across runs so a
    // backlog drains predictably.
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT id, tmdb_id, kind FROM items \
         WHERE logo_path IS NULL AND tmdb_id IS NOT NULL \
         ORDER BY id LIMIT ?",
    )
    .bind(batch)
    .fetch_all(&state.pool)
    .await?;

    let mut ok = 0usize;
    let mut empty = 0usize;
    let mut failed = 0usize;
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let tmdb_id: i64 = row.try_get("tmdb_id")?;
        let kind: String = row.try_get("kind")?;
        let result = match kind.as_str() {
            "movie" => tmdb.fetch_movie_logo(tmdb_id).await,
            "show" => tmdb.fetch_show_logo(tmdb_id).await,
            other => {
                warn!(item_id = id, kind = other, "unknown kind in refresh_logos");
                continue;
            }
        };
        match result {
            Ok(Some(path)) => {
                let url = chimpflix_metadata::tmdb_image_url(&path, "w500");
                let now = chimpflix_common::now_ms();
                if let Err(e) = sqlx::query(
                    "UPDATE items SET logo_path = ?, updated_at = ? WHERE id = ?",
                )
                .bind(&url)
                .bind(now)
                .bind(id)
                .execute(&state.pool)
                .await
                {
                    failed += 1;
                    warn!(item_id = id, error = %format!("{e:#}"), "logo upsert failed");
                } else {
                    ok += 1;
                }
            }
            Ok(None) => empty += 1,
            Err(e) => {
                failed += 1;
                warn!(item_id = id, tmdb_id, error = %format!("{e:#}"), "tmdb logo fetch failed");
            }
        }
    }
    append_log(
        log,
        format!("logos: {ok} added, {empty} unavailable, {failed} failed"),
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

async fn fetch_subtitles_task(
    state: &AppState,
    task: &ScheduledTask,
    log: &Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let Some(client) = state.opensubtitles_snapshot().await else {
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

    let dir = state.data_dir.join("subtitles");
    tokio::fs::create_dir_all(&dir).await?;

    let item_rows = if let Some(lid) = library_id {
        sqlx::query("SELECT id, kind, tmdb_id, imdb_id FROM items WHERE library_id = ?")
            .bind(lid)
            .fetch_all(&state.pool)
            .await?
    } else {
        sqlx::query("SELECT id, kind, tmdb_id, imdb_id FROM items")
            .fetch_all(&state.pool)
            .await?
    };

    let mut hits = 0usize;
    let mut misses = 0usize;
    let mut errors = 0usize;

    for row in &item_rows {
        let item_id: i64 = row.try_get("id").unwrap_or(0);
        let kind: String = row.try_get("kind").unwrap_or_default();
        let tmdb_id: Option<i64> = row.try_get("tmdb_id").ok().flatten();
        let imdb_id: Option<String> = row.try_get("imdb_id").ok().flatten();
        if tmdb_id.is_none() && imdb_id.is_none() {
            continue;
        }

        if kind == "movie" {
            for lang in &languages {
                match fetch_one_for_item(state, &client, item_id, tmdb_id, imdb_id.as_deref(), lang, &dir).await {
                    Ok(true) => hits += 1,
                    Ok(false) => misses += 1,
                    Err(e) => {
                        errors += 1;
                        warn!(item_id, lang, error = %format!("{e:#}"), "fetch_subtitles failed for movie");
                    }
                }
            }
        } else {
            // shows: walk episodes
            let eps = sqlx::query(
                "SELECT e.id AS id, s.season_number AS season, e.episode_number AS episode
                 FROM episodes e
                 JOIN seasons s ON s.id = e.season_id
                 WHERE s.show_id = ?",
            )
            .bind(item_id)
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
            for ep in &eps {
                let episode_id: i64 = ep.try_get("id").unwrap_or(0);
                let season: i32 = ep.try_get("season").unwrap_or(0);
                let episode: i32 = ep.try_get("episode").unwrap_or(0);
                for lang in &languages {
                    match fetch_one_for_episode(
                        state,
                        &client,
                        episode_id,
                        tmdb_id,
                        imdb_id.as_deref(),
                        season,
                        episode,
                        lang,
                        &dir,
                    )
                    .await
                    {
                        Ok(true) => hits += 1,
                        Ok(false) => misses += 1,
                        Err(e) => {
                            errors += 1;
                            warn!(
                                episode_id,
                                lang,
                                error = %format!("{e:#}"),
                                "fetch_subtitles failed for episode"
                            );
                        }
                    }
                }
            }
        }
    }

    append_log(
        log,
        format!(
            "fetched {hits} subtitles, {misses} not found, {errors} errored across {} items",
            item_rows.len()
        ),
    );
    Ok(())
}

async fn fetch_one_for_item(
    state: &AppState,
    client: &OpenSubtitlesClient,
    item_id: i64,
    tmdb_id: Option<i64>,
    imdb_id: Option<&str>,
    language: &str,
    base_dir: &std::path::Path,
) -> Result<bool> {
    // Skip if we already have any external subtitle for this item+language.
    let existing = sqlx::query(
        "SELECT 1 FROM external_subtitles WHERE item_id = ? AND language = ? LIMIT 1",
    )
    .bind(item_id)
    .bind(language)
    .fetch_optional(&state.pool)
    .await?;
    if existing.is_some() {
        return Ok(false);
    }

    let langs = [language.to_string()];
    let hits = client
        .search_for_movie(SearchParams {
            tmdb_id,
            imdb_id,
            languages: &langs,
        })
        .await?;
    let Some(hit) = hits.into_iter().next() else {
        return Ok(false);
    };
    let bytes = client.download(hit.file_id).await?;
    let item_dir = base_dir.join(format!("item-{item_id}"));
    tokio::fs::create_dir_all(&item_dir).await?;
    let path = item_dir.join(format!("{language}-{}.srt", hit.file_id));
    tokio::fs::write(&path, &bytes).await?;
    queries::insert_external_subtitle(
        &state.pool,
        NewExternalSubtitle {
            item_id: Some(item_id),
            episode_id: None,
            language: hit.language,
            source: "opensubtitles".into(),
            source_file_id: Some(hit.file_id.to_string()),
            file_path: path.to_string_lossy().into_owned(),
            forced: hit.forced,
            sdh: hit.hearing_impaired,
        },
    )
    .await?;
    Ok(true)
}

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

#[allow(clippy::too_many_arguments)]
async fn fetch_one_for_episode(
    state: &AppState,
    client: &OpenSubtitlesClient,
    episode_id: i64,
    tmdb_id: Option<i64>,
    imdb_id: Option<&str>,
    season: i32,
    episode: i32,
    language: &str,
    base_dir: &std::path::Path,
) -> Result<bool> {
    let existing = sqlx::query(
        "SELECT 1 FROM external_subtitles WHERE episode_id = ? AND language = ? LIMIT 1",
    )
    .bind(episode_id)
    .bind(language)
    .fetch_optional(&state.pool)
    .await?;
    if existing.is_some() {
        return Ok(false);
    }

    let langs = [language.to_string()];
    let hits = client
        .search_for_episode(
            SearchParams {
                tmdb_id,
                imdb_id,
                languages: &langs,
            },
            season,
            episode,
        )
        .await?;
    let Some(hit) = hits.into_iter().next() else {
        return Ok(false);
    };
    let bytes = client.download(hit.file_id).await?;
    let ep_dir = base_dir.join(format!("episode-{episode_id}"));
    tokio::fs::create_dir_all(&ep_dir).await?;
    let path = ep_dir.join(format!("{language}-{}.srt", hit.file_id));
    tokio::fs::write(&path, &bytes).await?;
    queries::insert_external_subtitle(
        &state.pool,
        NewExternalSubtitle {
            item_id: None,
            episode_id: Some(episode_id),
            language: hit.language,
            source: "opensubtitles".into(),
            source_file_id: Some(hit.file_id.to_string()),
            file_path: path.to_string_lossy().into_owned(),
            forced: hit.forced,
            sdh: hit.hearing_impaired,
        },
    )
    .await?;
    Ok(true)
}
