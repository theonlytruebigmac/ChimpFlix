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
use chrono::{TimeZone, Utc};
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

/// Seed the default task set on first run. Idempotent — if any tasks
/// exist (created by a previous boot or by the user) this does nothing.
pub async fn seed_defaults(pool: &SqlitePool) -> Result<()> {
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scheduled_tasks")
        .fetch_one(pool)
        .await?;
    if existing > 0 {
        return Ok(());
    }
    let defaults: &[(&str, &str, &str, &str)] = &[
        // (kind, name, cron, params_json)
        (
            "prune_sessions",
            "Prune expired sessions",
            "0 0 * * * *", // hourly at :00
            "{}",
        ),
        (
            "backup_db",
            "Daily backup snapshot",
            "0 0 3 * * *", // 03:00 daily
            "{}",
        ),
    ];
    for (kind, name, cron, params) in defaults {
        let next = next_after(cron, now_ms())?;
        let _ = queries::create_scheduled_task(
            pool,
            NewScheduledTask {
                kind: (*kind).into(),
                name: (*name).into(),
                cron_expr: (*cron).into(),
                params_json: (*params).into(),
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

    let next = match next_after(&task.cron_expr, finished_at) {
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
            let tmdb = state.tmdb.clone();
            let tvmaze = state.tvmaze.clone();
            let job_id = job.id;
            let hub = state.hub.clone();
            tokio::spawn(async move {
                let emitter: chimpflix_library::ScanEmitter = Arc::new(move |evt| {
                    hub.publish(crate::events::Event::Scan(evt));
                });
                if let Err(e) =
                    scanner::run_scan(pool, ffmpeg, tmdb, tvmaze, library_id, job_id, emitter)
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
            let Some(tmdb) = state.tmdb.clone() else {
                append_log(log, "TMDB disabled — skipping refresh");
                return Ok(());
            };
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
            // Stub: full marker detection is invoked via the markers API
            // today. A scheduled variant would iterate library files and
            // queue them — Phase-9 territory; we log a no-op for now.
            append_log(log, "detect_markers not yet implemented in scheduler");
            Ok(())
        }
        "optimize_versions" => {
            // Process up to `batch_size` queued rows. Per-row failures are
            // captured in the optimized_versions table, not in the task
            // outcome — the task itself succeeds whenever it ran to
            // completion.
            let params: serde_json::Value =
                serde_json::from_str(&task.params_json).unwrap_or_default();
            let batch = params
                .get("batch_size")
                .and_then(|v| v.as_i64())
                .unwrap_or(2);
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
    args.push("veryfast".into());
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
/// human label, and which JSON params it accepts. Keep this in sync with
/// the `dispatch` match arms.
pub fn registry() -> Vec<TaskKindInfo> {
    vec![
        TaskKindInfo {
            kind: "prune_sessions",
            display_name: "Prune expired sessions",
            description: "Delete session rows past their expires_at.",
            params_schema: r#"{}"#,
        },
        TaskKindInfo {
            kind: "backup_db",
            display_name: "Backup database",
            description: "VACUUM INTO snapshot under data/backups/auto/.",
            params_schema: r#"{}"#,
        },
        TaskKindInfo {
            kind: "scan_library",
            display_name: "Scan library",
            description: "Walk a library's paths and refresh media_files.",
            params_schema: r#"{"library_id": "number (required)"}"#,
        },
        TaskKindInfo {
            kind: "refresh_metadata",
            display_name: "Refresh metadata",
            description: "Re-pull metadata for every item (optionally restricted to one library).",
            params_schema: r#"{"library_id": "number (optional)"}"#,
        },
        TaskKindInfo {
            kind: "detect_markers",
            display_name: "Detect markers (intros/credits)",
            description: "Reserved for Phase 9; currently a no-op.",
            params_schema: r#"{"library_id": "number (required)"}"#,
        },
        TaskKindInfo {
            kind: "optimize_versions",
            display_name: "Optimize versions",
            description: "Reserved for Phase 9; currently a no-op.",
            params_schema: r#"{}"#,
        },
    ]
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskKindInfo {
    pub kind: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub params_schema: &'static str,
}
