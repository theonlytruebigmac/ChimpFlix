//! `/api/admin/tasks/overview` — registry-driven view of every
//! task kind the binary knows about, grouped for the rebuilt admin
//! UI ([`docs/pipelines/tasks-ui.html`]).
//!
//! Distinct from [`super::tasks`] which surfaces the
//! `scheduled_tasks` table CRUD for the legacy advanced view.
//! This endpoint joins:
//!
//!   - the static [`crate::tasks::registry`] (one row per kind)
//!   - the live [`crate::state::AppState::task_metrics`]
//!     (in_flight per kind)
//!   - the live `jobs` table (`queued` count per kind)
//!   - the `scheduled_tasks` row if one exists for this kind's
//!     sweep counterpart (last_run_at, next_run_at, enabled,
//!     last_status).
//!
//! Shaped for the overview screen: groups → sections → kinds with
//! everything the card row needs in one round-trip.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chimpflix_common::now_ms;
use chimpflix_library::{NewAuditEntry, ScheduledTaskUpdate, ServerSettingsUpdate, queries};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::AdminAuth;
use crate::scheduler;
use crate::state::AppState;
use crate::tasks::kind::{KindMetadata, TaskMode};
use crate::tasks::registry;

/// Top-level response: groups → sections → kinds. Matches the
/// mockup's overview-screen layout 1:1.
#[derive(Debug, Serialize)]
pub struct OverviewResponse {
    pub groups: Vec<KindGroup>,
}

#[derive(Debug, Serialize)]
pub struct KindGroup {
    /// Machine-stable identifier — clients use this for lookups
    /// (e.g. the flow page filters to "media_ingest"). Renaming
    /// `name` for UI purposes won't break those callers.
    pub id: &'static str,
    /// Human-readable header. "Media ingest pipeline", "Watch
    /// state & housekeeping", "System tasks".
    pub name: &'static str,
    pub sections: Vec<KindSection>,
}

#[derive(Debug, Serialize)]
pub struct KindSection {
    /// Machine-stable identifier (e.g. "automatic", "gated").
    /// See `KindGroup::id`.
    pub id: &'static str,
    /// Human-readable label rendered in the subgroup divider.
    pub label: &'static str,
    pub kinds: Vec<KindCard>,
}

/// One row in the task list. Self-contained — the UI doesn't need
/// to refetch anything to render a card.
#[derive(Debug, Serialize)]
pub struct KindCard {
    /// Stable identifier — the job-kind name from the registry for
    /// kinds shipped in the binary, or the raw `scheduled_tasks.kind`
    /// for legacy rows (custom operator-defined cron jobs that
    /// pre-date the registry).
    pub name: String,
    pub display_name: String,
    pub mode: TaskMode,
    pub scope: &'static str,
    pub gate: GateInfo,
    pub schedule: Option<ScheduleInfo>,
    pub live: LiveInfo,
    /// Registry-shipped concurrency default for this kind. Used by
    /// the admin "Per-kind concurrency" editor to render the
    /// "default" hint next to the editable override field. Always
    /// surfaced (not flagged as a private setting) so the UI can
    /// dim/show "default" when no override exists.
    pub default_concurrency: u32,
}

#[derive(Debug, Serialize)]
pub struct GateInfo {
    /// True when the kind is currently allowed to dispatch.
    /// Automatic kinds are always `true`; gated kinds reflect the
    /// `*_enabled` setting.
    pub enabled: bool,
    /// True when the kind has no admin-flippable switch (Automatic
    /// mode). UI shows a locked toggle in that case.
    pub locked: bool,
    /// Setting key the toggle PATCHes when not locked. `None` for
    /// Automatic kinds.
    pub setting_key: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct ScheduleInfo {
    /// Mirrors `scheduled_tasks.frequency`. "weekly", "daily",
    /// "on_change", "custom".
    pub frequency: String,
    /// `scheduled_tasks.enabled` — whether the sweep cron is
    /// armed. Independent of `GateInfo::enabled` (a gated kind
    /// can have a disabled sweep but still run on-add, or vice
    /// versa).
    pub enabled: bool,
    /// Whether `next_run_at` is snapped forward to the next
    /// maintenance window opening. Needed by the detail page's
    /// editable Schedule card so the toggle reflects current state
    /// instead of starting in a guessed default.
    pub requires_maintenance_window: bool,
    /// `scheduled_tasks.next_run_at`. Epoch ms.
    pub next_at: i64,
    /// `scheduled_tasks.last_run_at`. Epoch ms.
    pub last_at: Option<i64>,
    /// "ok", "warn", "bad" — derived from `last_status` + recent
    /// run history. "ok" when last run succeeded; "warn" when last
    /// run had partial failures; "bad" when last run failed
    /// outright.
    pub last_status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct LiveInfo {
    pub in_flight: u32,
    pub queued: i64,
    /// Epoch ms of the most recent successful job completion. None
    /// if no successful run has been observed since process start
    /// (live counters reset on restart; the rollup table holds
    /// historical data).
    pub last_success_at_ms: Option<i64>,
}

pub async fn overview(
    State(state): State<AppState>,
    _admin: AdminAuth,
) -> Result<Json<OverviewResponse>, ApiError> {
    // One round-trip: pull per-kind queued counts in a single SQL
    // grouped count, then layer in registry + live metrics +
    // scheduled_tasks rows.
    let queued_per_kind = sqlx::query(
        "SELECT kind, COUNT(*) AS n
         FROM jobs
         WHERE status = 'queued'
         GROUP BY kind",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let queued: std::collections::HashMap<String, i64> = queued_per_kind
        .iter()
        .filter_map(|r| {
            let k: String = r.try_get("kind").ok()?;
            let n: i64 = r.try_get("n").ok()?;
            Some((k, n))
        })
        .collect();

    // scheduled_tasks rows keyed by kind. The sweep-side name lives
    // here (e.g. `detect_markers`, not `detect_markers_file`), so
    // we'll match by either side.
    let scheduled_rows = queries::list_scheduled_tasks(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    let schedules: std::collections::HashMap<String, chimpflix_library::ScheduledTask> =
        scheduled_rows
            .into_iter()
            .map(|r| (r.kind.clone(), r))
            .collect();

    // Pre-snapshot in-flight counts so we read the lock once.
    let in_flight_snap = state.task_metrics.in_flight_snapshot();

    // Determine gate state per kind via the existing evaluator.
    // Read settings once; the evaluator only checks the cached
    // struct so this is essentially free.
    let mut groups: Vec<KindGroup> = Vec::new();
    let mut media_auto: Vec<KindCard> = Vec::new();
    let mut media_gated: Vec<KindCard> = Vec::new();
    let mut housekeeping: Vec<KindCard> = Vec::new();
    let mut system: Vec<KindCard> = Vec::new();

    for k in registry::all_kinds() {
        let card = build_card(&state, k, &queued, &schedules, &in_flight_snap).await;
        // Hardcoded grouping: simpler than threading a group field
        // through the registry. If we add many more kinds the
        // mapping moves into KindMetadata.
        match (k.mode, k.job_kind) {
            (TaskMode::Periodic, _) => system.push(card),
            (_, "bootstrap_season_refs") => media_auto.push(card),
            (TaskMode::Automatic, _) => media_auto.push(card),
            (TaskMode::Gated, _) => media_gated.push(card),
        }
    }

    // Housekeeping group: scheduled_tasks rows that aren't in the
    // registry (trakt_pull, prune_sessions, cleanup_*). Surface
    // them as a minimal card so the operator still sees them in
    // the rebuilt UI without needing a per-kind registry entry.
    //
    // Iterate via a sorted view rather than `&schedules` directly:
    // HashMap iteration order is randomized per process, which made
    // every overview poll re-shuffle these cards in the UI. Sort by
    // kind name (machine-stable, doesn't depend on display strings).
    let mut legacy_keys: Vec<&String> = schedules.keys().collect();
    legacy_keys.sort();
    for kind in legacy_keys {
        if registry::find_kind(kind).is_some() {
            continue; // already rendered above
        }
        let row = &schedules[kind];
        let card = card_from_legacy_scheduled(row, &queued, &in_flight_snap);
        if is_system_kind(kind) {
            system.push(card);
        } else {
            housekeeping.push(card);
        }
    }

    groups.push(KindGroup {
        id: "media_ingest",
        name: "Media ingest pipeline",
        sections: vec![
            KindSection {
                id: "automatic",
                label: "Automatic",
                kinds: media_auto,
            },
            KindSection {
                id: "gated",
                label: "Gated",
                kinds: media_gated,
            },
        ],
    });
    groups.push(KindGroup {
        id: "watch_state",
        name: "Watch state & housekeeping",
        sections: vec![KindSection {
            id: "all",
            label: "All",
            kinds: housekeeping,
        }],
    });
    groups.push(KindGroup {
        id: "system",
        name: "System tasks",
        sections: vec![KindSection {
            id: "all",
            label: "All",
            kinds: system,
        }],
    });

    Ok(Json(OverviewResponse { groups }))
}

async fn build_card(
    state: &AppState,
    k: &'static KindMetadata,
    queued: &std::collections::HashMap<String, i64>,
    schedules: &std::collections::HashMap<String, chimpflix_library::ScheduledTask>,
    in_flight: &std::collections::HashMap<String, u32>,
) -> KindCard {
    let gate_state = crate::tasks::gates::is_kind_allowed(state, k.job_kind).await;
    let gate = GateInfo {
        enabled: gate_state.is_allowed(),
        locked: matches!(k.mode, TaskMode::Automatic | TaskMode::Periodic),
        setting_key: k.gate_setting_key,
    };

    // Schedule row: prefer sweep_kind, then fall back to job_kind
    // (some periodic kinds reuse the job_kind name as their cron).
    let schedule_row = k
        .sweep_kind
        .and_then(|n| schedules.get(n))
        .or_else(|| schedules.get(k.job_kind));
    let schedule = schedule_row.map(|r| ScheduleInfo {
        frequency: r.frequency.clone(),
        enabled: r.enabled,
        requires_maintenance_window: r.requires_maintenance_window,
        next_at: r.next_run_at,
        last_at: r.last_run_at,
        last_status: status_label(r.last_status.as_deref()),
    });

    let last_success_at_ms = state
        .task_metrics
        .recent(k.job_kind)
        .into_iter()
        .find(|r| r.success)
        .map(|r| r.finished_at_ms);

    KindCard {
        name: k.job_kind.to_string(),
        display_name: k.display_name.to_string(),
        mode: k.mode,
        scope: scope_label(k.scope),
        gate,
        schedule,
        live: LiveInfo {
            in_flight: *in_flight.get(k.job_kind).unwrap_or(&0),
            queued: *queued.get(k.job_kind).unwrap_or(&0),
            last_success_at_ms,
        },
        default_concurrency: k.concurrency,
    }
}

fn card_from_legacy_scheduled(
    row: &chimpflix_library::ScheduledTask,
    queued: &std::collections::HashMap<String, i64>,
    in_flight: &std::collections::HashMap<String, u32>,
) -> KindCard {
    KindCard {
        name: row.kind.clone(),
        display_name: row.name.clone(),
        mode: TaskMode::Periodic,
        scope: "global",
        gate: GateInfo {
            enabled: row.enabled,
            locked: false,
            setting_key: None,
        },
        schedule: Some(ScheduleInfo {
            frequency: row.frequency.clone(),
            enabled: row.enabled,
            requires_maintenance_window: row.requires_maintenance_window,
            next_at: row.next_run_at,
            last_at: row.last_run_at,
            last_status: status_label(row.last_status.as_deref()),
        }),
        live: LiveInfo {
            in_flight: *in_flight.get(row.kind.as_str()).unwrap_or(&0),
            queued: *queued.get(row.kind.as_str()).unwrap_or(&0),
            last_success_at_ms: None, // legacy kinds don't push into LiveMetrics
        },
        // Legacy custom-cron rows don't carry a registry concurrency.
        // Surface 1 so the UI's editor (if it ever renders them) shows
        // a safe placeholder; in practice these rows are filtered out
        // of the per-kind cap UI which only iterates registry kinds.
        default_concurrency: 1,
    }
}

fn scope_label(s: crate::tasks::kind::TaskScope) -> &'static str {
    use crate::tasks::kind::TaskScope::*;
    match s {
        PerFile => "per_file",
        PerItem => "per_item",
        Global => "global",
    }
}

fn status_label(s: Option<&str>) -> &'static str {
    match s {
        Some("succeeded") | Some("ok") => "ok",
        Some("failed") | Some("dead") | Some("error") => "bad",
        Some("warn") | Some("partial") => "warn",
        _ => "ok",
    }
}

fn is_system_kind(kind: &str) -> bool {
    matches!(
        kind,
        "backup_db"
            | "verify_backups"
            | "optimize_versions"
            | "cleanup_jobs"
            | "cleanup_audit_log"
            | "rollup_task_metrics"
    )
}

/// Hero-strip data for the overview screen — running / queued /
/// succeeded-24h / failed-24h / next-maintenance-window. Backed by
/// the live metrics (no DB hit) for the first two, and SQL counts
/// for the 24h success/failure tallies.
#[derive(Debug, Serialize)]
pub struct SummaryResponse {
    /// Total jobs currently in `status = 'running'`, summed across
    /// every kind.
    pub running: u32,
    /// Total jobs currently in `status = 'queued'`.
    pub queued: i64,
    /// Count of jobs that reached `status = 'succeeded'` with
    /// `finished_at >= now - 24h`.
    pub succeeded_24h: i64,
    /// Count of jobs in dead/failed terminal state with
    /// `finished_at >= now - 24h`.
    pub failed_24h: i64,
    /// Next maintenance window opening (epoch ms). Computed from
    /// `server_settings.maintenance_window_start` + current time.
    /// Null if the window is currently open.
    pub next_maintenance_window_ms: Option<i64>,
}

pub async fn summary(
    State(state): State<AppState>,
    _admin: AdminAuth,
) -> Result<Json<SummaryResponse>, ApiError> {
    let now = chimpflix_common::now_ms();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let cutoff = now - day_ms;

    // Two counts in one round-trip via a conditional aggregate.
    let row = sqlx::query(
        "SELECT
            COUNT(CASE WHEN status = 'queued' THEN 1 END) AS queued,
            COUNT(CASE WHEN status = 'succeeded' AND finished_at >= ? THEN 1 END) AS succ_24h,
            COUNT(CASE WHEN status = 'dead' AND finished_at >= ? THEN 1 END) AS failed_24h
         FROM jobs",
    )
    .bind(cutoff)
    .bind(cutoff)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;

    let queued: i64 = row.try_get("queued").unwrap_or(0);
    let succeeded_24h: i64 = row.try_get("succ_24h").unwrap_or(0);
    let failed_24h: i64 = row.try_get("failed_24h").unwrap_or(0);

    let running: u32 = state.task_metrics.in_flight_snapshot().values().sum();

    let next_maintenance_window_ms = next_window_open_ms(&state, now).await.unwrap_or(None);

    Ok(Json(SummaryResponse {
        running,
        queued,
        succeeded_24h,
        failed_24h,
        next_maintenance_window_ms,
    }))
}

/// Compute when the operator's configured maintenance window next
/// opens. Returns None when the window is currently open. Uses the
/// HH:MM format from `server_settings`; wrapping windows (22:00 →
/// 06:00) are handled by treating the next open as "tomorrow's
/// start" if we're past today's start time.
///
/// HH:MM is interpreted in **server-local time** — the same timezone
/// the scheduler's `snap_to_maintenance_window` uses. Doing the
/// math via `chrono::Local` (rather than naive epoch-ms division)
/// keeps the hero-strip countdown consistent with when the
/// scheduler actually fires window-eligible tasks. Without this,
/// a server in EDT showed "opens in 45m" at 21:15 local because
/// `(now / day_ms) * day_ms` rounded to midnight UTC, treating the
/// stored "02:00" as 02:00 UTC instead of 02:00 EDT.
async fn next_window_open_ms(state: &AppState, now: i64) -> Option<Option<i64>> {
    use chrono::{Local, TimeZone};

    let s = state.settings.read().await;
    let start_t = parse_hhmm_naive(&s.maintenance_window_start)?;
    let end_t = parse_hhmm_naive(&s.maintenance_window_end)?;
    drop(s);

    let now_local = Local.timestamp_millis_opt(now).single()?;
    let today = now_local.date_naive();
    let start_today = Local
        .from_local_datetime(&today.and_time(start_t))
        .single()?;

    let wraps = end_t <= start_t;
    let end_today_or_tomorrow = if wraps {
        let tomorrow = today.succ_opt()?;
        Local
            .from_local_datetime(&tomorrow.and_time(end_t))
            .single()?
    } else {
        Local.from_local_datetime(&today.and_time(end_t)).single()?
    };

    // Already inside today's window?
    if now_local >= start_today && now_local < end_today_or_tomorrow {
        return Some(None);
    }
    // For wrapping windows, also check yesterday's start → today's
    // end (e.g. window 22:00 → 06:00 and now is 03:00).
    if wraps {
        if let (Some(yesterday), end_today) = (
            today.pred_opt(),
            Local.from_local_datetime(&today.and_time(end_t)).single(),
        ) {
            if let (Some(start_y), Some(end_y)) = (
                Local
                    .from_local_datetime(&yesterday.and_time(start_t))
                    .single(),
                end_today,
            ) {
                if now_local >= start_y && now_local < end_y {
                    return Some(None);
                }
            }
        }
    }

    // Snap forward to the next opening — today's start if still
    // ahead, else tomorrow's.
    let next_open = if now_local < start_today {
        start_today
    } else {
        let tomorrow = today.succ_opt()?;
        Local
            .from_local_datetime(&tomorrow.and_time(start_t))
            .single()?
    };
    Some(Some(next_open.timestamp_millis()))
}

/// Parse "HH:MM" → `chrono::NaiveTime`. None on malformed input —
/// caller treats that as "window not configured".
fn parse_hhmm_naive(s: &str) -> Option<chrono::NaiveTime> {
    let (h_str, m_str) = s.split_once(':')?;
    let h: u32 = h_str.parse().ok()?;
    let m: u32 = m_str.parse().ok()?;
    chrono::NaiveTime::from_hms_opt(h, m, 0)
}

/// Parse "HH:MM" → offset ms from midnight. Retained for the unit
/// tests below which exercise edge cases independently of the
/// timezone-aware computation in `next_window_open_ms`.
#[cfg(test)]
fn parse_hhmm(s: &str) -> Option<i64> {
    let (h_str, m_str) = s.split_once(':')?;
    let h: i64 = h_str.parse().ok()?;
    let m: i64 = m_str.parse().ok()?;
    if !(0..24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some((h * 60 + m) * 60 * 1000)
}

/// Detail-screen payload for one kind. Joins everything the
/// drill-in page needs: live counters, recent ring-buffer runs,
/// 30-day history rollup, and the scheduled_tasks row.
#[derive(Debug, Serialize)]
pub struct KindDetailResponse {
    pub name: String,
    pub display_name: String,
    pub mode: TaskMode,
    pub scope: &'static str,
    pub gate: GateInfo,
    pub schedule: Option<ScheduleInfo>,
    pub live: LiveInfo,
    pub p95_duration_ms: Option<i64>,
    pub recent_runs: Vec<RecentRun>,
    /// One entry per day for the last 30 days. Days with no runs
    /// are omitted; the chart renderer interpolates / draws gaps.
    pub history: Vec<DailyMetrics>,
}

#[derive(Debug, Serialize)]
pub struct DailyMetrics {
    pub day_ms: i64,
    pub success_count: i64,
    pub failure_count: i64,
    pub p50_duration_ms: Option<i64>,
    pub p95_duration_ms: Option<i64>,
    pub targets_processed: i64,
}

pub async fn kind_detail(
    State(state): State<AppState>,
    _admin: AdminAuth,
    Path(name): Path<String>,
) -> Result<Json<KindDetailResponse>, ApiError> {
    // Two-track lookup: registry kinds (modern, gate-aware) and
    // legacy scheduled_tasks rows (kinds dispatched directly by
    // the scheduler — backup_db, prune_sessions, cleanup_*, …).
    // The overview includes both in the list, so the detail page
    // must handle both too — otherwise rows render but 404 on
    // click, which is what bug-report screenshots show.
    let schedules = queries::list_scheduled_tasks(&state.pool)
        .await
        .map_err(ApiError::Internal)?;

    let meta = registry::find_kind(&name);
    if meta.is_none() {
        // Fall back to a legacy scheduled_tasks row lookup. If the
        // name doesn't match any row either, the kind genuinely
        // doesn't exist → 404.
        let row = schedules
            .iter()
            .find(|r| r.kind == name)
            .ok_or(ApiError::NotFound)?;
        return Ok(Json(legacy_kind_detail(&state, row).await?));
    }
    let meta = meta.unwrap();

    // Gate + schedule + live: same logic as the overview card builder.
    let gate_state = crate::tasks::gates::is_kind_allowed(&state, meta.job_kind).await;
    let gate = GateInfo {
        enabled: gate_state.is_allowed(),
        locked: matches!(meta.mode, TaskMode::Automatic | TaskMode::Periodic),
        setting_key: meta.gate_setting_key,
    };
    let schedule_row = meta
        .sweep_kind
        .and_then(|n| schedules.iter().find(|r| r.kind == n))
        .or_else(|| schedules.iter().find(|r| r.kind == meta.job_kind));
    let schedule = schedule_row.map(|r| ScheduleInfo {
        frequency: r.frequency.clone(),
        enabled: r.enabled,
        requires_maintenance_window: r.requires_maintenance_window,
        next_at: r.next_run_at,
        last_at: r.last_run_at,
        last_status: status_label(r.last_status.as_deref()),
    });

    // Queue depth: one COUNT query scoped to this kind.
    let queued: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE kind = ? AND status = 'queued'")
            .bind(meta.job_kind)
            .fetch_one(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

    let in_flight = state.task_metrics.in_flight(meta.job_kind);
    let recent_records = state.task_metrics.recent(meta.job_kind);
    let last_success_at_ms = recent_records
        .iter()
        .find(|r| r.success)
        .map(|r| r.finished_at_ms);

    // p95 over the successful runs in the ring buffer.
    let mut durations: Vec<i64> = recent_records
        .iter()
        .filter(|r| r.success)
        .map(|r| r.duration_ms)
        .collect();
    durations.sort_unstable();
    let p95 = if durations.len() >= 5 {
        pct95(&durations)
    } else {
        None
    };

    let recent_runs: Vec<RecentRun> = recent_records
        .into_iter()
        .map(|r| RecentRun {
            kind: meta.job_kind.to_string(),
            finished_at_ms: r.finished_at_ms,
            duration_ms: r.duration_ms,
            success: r.success,
            error_class: r.error_class,
        })
        .collect();

    // 30-day rollup history.
    let now = chimpflix_common::now_ms();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let cutoff = now - 30 * day_ms;
    let hist_rows = sqlx::query(
        "SELECT day, success_count, failure_count, p50_duration_ms, p95_duration_ms, targets_processed
         FROM task_kind_metrics_daily
         WHERE kind = ? AND day >= ?
         ORDER BY day ASC",
    )
    .bind(meta.job_kind)
    .bind(cutoff)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let history: Vec<DailyMetrics> = hist_rows
        .iter()
        .map(|r| DailyMetrics {
            day_ms: r.try_get::<i64, _>("day").unwrap_or(0),
            success_count: r.try_get::<i64, _>("success_count").unwrap_or(0),
            failure_count: r.try_get::<i64, _>("failure_count").unwrap_or(0),
            p50_duration_ms: r
                .try_get::<Option<i64>, _>("p50_duration_ms")
                .ok()
                .flatten(),
            p95_duration_ms: r
                .try_get::<Option<i64>, _>("p95_duration_ms")
                .ok()
                .flatten(),
            targets_processed: r.try_get::<i64, _>("targets_processed").unwrap_or(0),
        })
        .collect();

    Ok(Json(KindDetailResponse {
        name: meta.job_kind.to_string(),
        display_name: meta.display_name.to_string(),
        mode: meta.mode,
        scope: scope_label(meta.scope),
        gate,
        schedule,
        live: LiveInfo {
            in_flight,
            queued,
            last_success_at_ms,
        },
        p95_duration_ms: p95,
        recent_runs,
        history,
    }))
}

/// Build a detail payload for a `scheduled_tasks` row that has no
/// registry entry (the legacy housekeeping kinds: `backup_db`,
/// `prune_sessions`, `cleanup_jobs`, `trakt_pull`, …). These rows
/// were dispatched directly by the scheduler before the registry
/// existed, so they don't have:
///   - a gate setting (their on/off is just `scheduled_tasks.enabled`)
///   - a ring-buffer presence in `LiveMetrics` (their handler never
///     pushes there), so `recent_runs` and `p95_duration_ms` are
///     always empty/None.
///
/// They DO still appear in `task_kind_metrics_daily` (the rollup
/// task writes one row per kind regardless of where it came from),
/// so the 30-day history chart works the same way.
async fn legacy_kind_detail(
    state: &AppState,
    row: &chimpflix_library::ScheduledTask,
) -> Result<KindDetailResponse, ApiError> {
    let queued: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE kind = ? AND status = 'queued'")
            .bind(&row.kind)
            .fetch_one(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;

    let now = chimpflix_common::now_ms();
    let day_ms: i64 = 24 * 60 * 60 * 1000;
    let cutoff = now - 30 * day_ms;
    let hist_rows = sqlx::query(
        "SELECT day, success_count, failure_count, p50_duration_ms, p95_duration_ms, targets_processed
         FROM task_kind_metrics_daily
         WHERE kind = ? AND day >= ?
         ORDER BY day ASC",
    )
    .bind(&row.kind)
    .bind(cutoff)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let history: Vec<DailyMetrics> = hist_rows
        .iter()
        .map(|r| DailyMetrics {
            day_ms: r.try_get::<i64, _>("day").unwrap_or(0),
            success_count: r.try_get::<i64, _>("success_count").unwrap_or(0),
            failure_count: r.try_get::<i64, _>("failure_count").unwrap_or(0),
            p50_duration_ms: r
                .try_get::<Option<i64>, _>("p50_duration_ms")
                .ok()
                .flatten(),
            p95_duration_ms: r
                .try_get::<Option<i64>, _>("p95_duration_ms")
                .ok()
                .flatten(),
            targets_processed: r.try_get::<i64, _>("targets_processed").unwrap_or(0),
        })
        .collect();

    Ok(KindDetailResponse {
        name: row.kind.clone(),
        display_name: row.name.clone(),
        mode: TaskMode::Periodic,
        scope: "global",
        gate: GateInfo {
            enabled: row.enabled,
            // Legacy rows' "gate" is just the scheduled-task enabled
            // flag — the schedule PATCH editor controls it, no
            // separate locked toggle.
            locked: true,
            setting_key: None,
        },
        schedule: Some(ScheduleInfo {
            frequency: row.frequency.clone(),
            enabled: row.enabled,
            requires_maintenance_window: row.requires_maintenance_window,
            next_at: row.next_run_at,
            last_at: row.last_run_at,
            last_status: status_label(row.last_status.as_deref()),
        }),
        live: LiveInfo {
            in_flight: state.task_metrics.in_flight(&row.kind),
            queued,
            last_success_at_ms: None,
        },
        p95_duration_ms: None,
        recent_runs: Vec::new(),
        history,
    })
}

/// Per-kind health summary for the activity screen. One row per
/// known kind (registry + scheduled_tasks union), with live
/// counters from `LiveMetrics` and live queue depth from `jobs`.
#[derive(Debug, Serialize)]
pub struct ActivityResponse {
    pub per_kind: Vec<KindHealth>,
    /// Currently-running jobs with their resolved target title.
    /// Capped at 50 — the header activity popover only displays a
    /// handful, but the cap is generous so the admin tasks page can
    /// reuse the same payload for a "live now" panel without a
    /// second fetch. Sorted oldest-first so the longest-running job
    /// reads at the top.
    pub running_jobs: Vec<RunningJob>,
    /// Newest-first across the whole pool — last 50 completed
    /// runs, regardless of kind.
    pub recent_runs: Vec<RecentRun>,
    /// Currently-dead jobs (`status = 'dead'`), capped at 50,
    /// newest-first. Drives the failure panel.
    pub failed: Vec<FailedJob>,
}

/// One currently-running job with enough context for the header
/// activity popover to render "Detecting markers: WIND BREAKER S01".
/// The `title` is resolved server-side by following the payload's
/// `item_id` or `file_id` through media_files → episodes → seasons
/// → items so the client doesn't need per-kind payload knowledge.
#[derive(Debug, Serialize)]
pub struct RunningJob {
    pub id: i64,
    pub kind: String,
    pub display_name: String,
    /// Resolved item / episode title. `None` when the payload
    /// targets something the JOIN couldn't resolve (rare — usually
    /// scan jobs that operate on a library, not a specific item).
    pub title: Option<String>,
    /// "S02E04"-style suffix when the target is an episode. Empty
    /// for movie / non-episodic targets. Kept separate from `title`
    /// so the client can render it as muted secondary text.
    pub episode_code: Option<String>,
    /// Epoch-ms when the job started running. `None` for queued
    /// rows — but this list only includes `status = 'running'` so
    /// in practice always set.
    pub started_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct KindHealth {
    pub kind: String,
    pub display_name: String,
    pub queue_depth: i64,
    pub in_flight: u32,
    /// Successful runs observed since the process started divided
    /// by the elapsed minutes, expressed as jobs/minute (rounded
    /// to one decimal). Reset on restart.
    pub jobs_per_minute: f32,
    /// p95 of `duration_ms` over the in-memory ring buffer (last
    /// 100 runs). None until 5+ runs have completed.
    pub p95_duration_ms: Option<i64>,
    /// Count of error_class != null entries in the recent ring.
    pub recent_errors: u32,
    /// Concurrency default shipped by the registry. Surfaced so the
    /// admin "Per-kind concurrency" editor on the activity page can
    /// label the editable override with its baseline.
    pub default_concurrency: u32,
    /// Rough wall-clock ETA to drain the queue, in seconds. Computed
    /// as `queue_depth × (p95_duration_ms / 1000) / effective_concurrency`
    /// where `effective_concurrency` reads the operator override if
    /// present and falls back to `default_concurrency`. `None` when:
    ///
    /// - queue is empty (nothing to drain), or
    /// - fewer than 5 successful runs are in the ring buffer (no
    ///   p95 yet — too noisy to estimate honestly).
    ///
    /// Refreshed on each activity poll. This is a deliberately coarse
    /// signal — p95 from the last 100 runs over the requested-but-
    /// unknown future queue mix isn't predictive to the minute, but
    /// "~30 min remaining" beats "0.6 jobs/min" for an operator
    /// trying to decide whether to wait or come back later.
    pub eta_seconds_remaining: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RecentRun {
    pub kind: String,
    pub finished_at_ms: i64,
    pub duration_ms: i64,
    pub success: bool,
    pub error_class: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct FailedJob {
    pub id: i64,
    pub kind: String,
    pub last_error: Option<String>,
    pub error_class: Option<String>,
    pub finished_at_ms: Option<i64>,
}

pub async fn activity(
    State(state): State<AppState>,
    _admin: AdminAuth,
) -> Result<Json<ActivityResponse>, ApiError> {
    let in_flight_snap = state.task_metrics.in_flight_snapshot();

    // Per-kind queue depth.
    let queued_rows =
        sqlx::query("SELECT kind, COUNT(*) AS n FROM jobs WHERE status = 'queued' GROUP BY kind")
            .fetch_all(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
    let queued: std::collections::HashMap<String, i64> = queued_rows
        .iter()
        .filter_map(|r| {
            let k: String = r.try_get("kind").ok()?;
            let n: i64 = r.try_get("n").ok()?;
            Some((k, n))
        })
        .collect();

    let started_at_ms = state.started_at_ms;
    let now = chimpflix_common::now_ms();
    let elapsed_minutes = ((now - started_at_ms).max(60_000) as f32) / 60_000.0;

    // Union of kinds we know about: registry + any kind that has
    // jobs in the queue or has ever recorded a run.
    let mut kinds: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for k in registry::all_kinds() {
        kinds.insert(k.job_kind.to_string());
    }
    for k in queued.keys() {
        kinds.insert(k.clone());
    }
    for k in in_flight_snap.keys() {
        kinds.insert(k.clone());
    }

    // Concurrency override map for the ETA computation — reading
    // settings once outside the loop. JSON is `{ "kind": cap }`;
    // anything malformed silently falls back to registry defaults
    // (already validated at PATCH time).
    let settings = queries::get_server_settings(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    let concurrency_overrides: std::collections::HashMap<String, u32> =
        serde_json::from_str(&settings.job_kind_concurrency).unwrap_or_default();

    let mut per_kind: Vec<KindHealth> = Vec::new();
    let mut all_recent: Vec<RecentRun> = Vec::new();
    for kind in &kinds {
        let recent = state.task_metrics.recent(kind);
        let success_count = recent.iter().filter(|r| r.success).count() as u32;
        let error_count = recent.iter().filter(|r| !r.success).count() as u32;
        let mut durations: Vec<i64> = recent
            .iter()
            .filter(|r| r.success)
            .map(|r| r.duration_ms)
            .collect();
        durations.sort_unstable();
        let p95 = if durations.len() >= 5 {
            pct95(&durations)
        } else {
            None
        };
        let meta = registry::find_kind(kind);
        let display_name = meta
            .map(|k| k.display_name.to_string())
            .unwrap_or_else(|| kind.clone());
        let default_concurrency = meta.map(|k| k.concurrency).unwrap_or(1);
        let effective_concurrency = concurrency_overrides
            .get(kind)
            .copied()
            .unwrap_or(default_concurrency)
            .max(1);
        let queue_depth = *queued.get(kind).unwrap_or(&0);
        let eta_seconds_remaining = match (queue_depth, p95) {
            (n, Some(p95_ms)) if n > 0 => {
                // n × (p95_ms / 1000) / effective_concurrency, in
                // integer seconds. Use i128 inside the multiply so a
                // big-queue × long-p95 product can't overflow i64.
                let secs = (n as i128 * p95_ms as i128)
                    / (effective_concurrency as i128 * 1000);
                i64::try_from(secs).ok()
            }
            _ => None,
        };
        per_kind.push(KindHealth {
            kind: kind.clone(),
            display_name,
            queue_depth,
            in_flight: *in_flight_snap.get(kind).unwrap_or(&0),
            jobs_per_minute: (success_count as f32 / elapsed_minutes * 10.0).round() / 10.0,
            p95_duration_ms: p95,
            recent_errors: error_count,
            default_concurrency,
            eta_seconds_remaining,
        });
        for r in recent {
            all_recent.push(RecentRun {
                kind: kind.clone(),
                finished_at_ms: r.finished_at_ms,
                duration_ms: r.duration_ms,
                success: r.success,
                error_class: r.error_class,
            });
        }
    }
    // Newest first, capped at 200 — the activity feed page-size
    // selector lets the operator pick 10/25/50/100 client-side, so
    // we send a chunk big enough that "100 per page" is meaningful
    // without paying for an unbounded scan of the in-memory ring
    // buffer per poll. Higher than 200 starts looking like a
    // separate "history" surface that belongs against the rollup
    // table, not the live ring.
    all_recent.sort_unstable_by(|a, b| b.finished_at_ms.cmp(&a.finished_at_ms));
    all_recent.truncate(200);

    // Currently-running jobs with their target item title resolved.
    // SQLite's `json_extract` reads `item_id` or `file_id` out of the
    // opaque payload; the CASE collapses both shapes into one title
    // column so the client gets a flat row per job regardless of
    // which handler emitted it. Capped at 50 — bigger than the header
    // popover needs but cheap enough to send so admin pages can reuse
    // the payload without a second fetch.
    let running_rows = sqlx::query(
        "SELECT
            j.id,
            j.kind,
            j.started_at,
            CASE
                WHEN json_extract(j.payload, '$.item_id') IS NOT NULL THEN (
                    SELECT i.title FROM items i
                    WHERE i.id = json_extract(j.payload, '$.item_id')
                )
                WHEN json_extract(j.payload, '$.file_id') IS NOT NULL THEN (
                    SELECT COALESCE(show.title, mfi.title)
                    FROM media_files mf
                    LEFT JOIN items mfi ON mfi.id = mf.item_id
                    LEFT JOIN episodes ep ON ep.id = mf.episode_id
                    LEFT JOIN seasons s ON s.id = ep.season_id
                    LEFT JOIN items show ON show.id = s.show_id
                    WHERE mf.id = json_extract(j.payload, '$.file_id')
                )
            END AS title,
            CASE
                WHEN json_extract(j.payload, '$.file_id') IS NOT NULL THEN (
                    SELECT
                        CASE WHEN s.season_number IS NOT NULL
                            THEN printf('S%02dE%02d', s.season_number, ep.episode_number)
                            ELSE NULL
                        END
                    FROM media_files mf
                    LEFT JOIN episodes ep ON ep.id = mf.episode_id
                    LEFT JOIN seasons s ON s.id = ep.season_id
                    WHERE mf.id = json_extract(j.payload, '$.file_id')
                )
            END AS episode_code
         FROM jobs j
         WHERE j.status = 'running'
         ORDER BY j.started_at ASC NULLS LAST, j.id ASC
         LIMIT 50",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let running_jobs: Vec<RunningJob> = running_rows
        .iter()
        .map(|r| {
            let kind: String = r.try_get("kind").unwrap_or_default();
            let display_name = registry::find_kind(&kind)
                .map(|k| k.display_name.to_string())
                .unwrap_or_else(|| kind.clone());
            RunningJob {
                id: r.try_get("id").unwrap_or(0),
                display_name,
                kind,
                title: r.try_get::<Option<String>, _>("title").ok().flatten(),
                episode_code: r
                    .try_get::<Option<String>, _>("episode_code")
                    .ok()
                    .flatten(),
                started_at_ms: r.try_get::<Option<i64>, _>("started_at").ok().flatten(),
            }
        })
        .collect();

    // Last 50 dead jobs, newest first.
    let failed_rows = sqlx::query(
        "SELECT id, kind, last_error, error_class, finished_at
         FROM jobs
         WHERE status = 'dead'
         ORDER BY finished_at DESC
         LIMIT 50",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?;
    let failed: Vec<FailedJob> = failed_rows
        .iter()
        .map(|r| FailedJob {
            id: r.try_get("id").unwrap_or(0),
            kind: r.try_get("kind").unwrap_or_default(),
            last_error: r.try_get::<Option<String>, _>("last_error").ok().flatten(),
            error_class: r.try_get::<Option<String>, _>("error_class").ok().flatten(),
            finished_at_ms: r.try_get::<Option<i64>, _>("finished_at").ok().flatten(),
        })
        .collect();

    Ok(Json(ActivityResponse {
        per_kind,
        running_jobs,
        recent_runs: all_recent,
        failed,
    }))
}

fn pct95(sorted: &[i64]) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((sorted.len() as f64 - 1.0) * 0.95).round() as usize;
    sorted.get(idx).copied()
}

/// PATCH body for `/admin/tasks/kind/{name}` — edits the
/// `scheduled_tasks` row that backs this kind. Mirrors the legacy
/// row-id PATCH (`ScheduledTaskUpdate`) but keyed by stable kind
/// name so the UI doesn't need to thread row IDs.
///
/// Fields are independently optional — passing `{frequency: "daily"}`
/// leaves enabled, window, params untouched.
#[derive(Debug, Default, Deserialize)]
pub struct KindScheduleUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frequency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_maintenance_window: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GateUpdate {
    pub enabled: bool,
}

/// PATCH the gate setting for one kind. Returns 204 on success,
/// 400 when the kind is Automatic/Periodic (no admin-flippable
/// switch), 404 when the kind isn't in the registry.
///
/// Mirrors the cache-reload + audit-log flow from
/// [`crate::api::admin::settings::patch`] so a flipped gate takes
/// effect on the next call to [`crate::tasks::is_kind_allowed`].
pub async fn update_gate(
    State(state): State<AppState>,
    admin: AdminAuth,
    Path(kind): Path<String>,
    Json(update): Json<GateUpdate>,
) -> Result<StatusCode, ApiError> {
    let Some(meta) = registry::find_kind(&kind) else {
        return Err(ApiError::NotFound);
    };
    if matches!(meta.mode, TaskMode::Automatic | TaskMode::Periodic) {
        return Err(ApiError::validation(format!(
            "kind `{}` is {:?} — no admin-flippable gate",
            meta.job_kind, meta.mode
        )));
    }
    let Some(key) = meta.gate_setting_key else {
        return Err(ApiError::validation(format!(
            "kind `{}` is Gated but declares no gate_setting_key (registry bug)",
            meta.job_kind
        )));
    };

    let patch = build_gate_patch(key, update.enabled).ok_or_else(|| {
        ApiError::validation(format!(
            "gate setting key `{key}` not wired into PATCH builder"
        ))
    })?;

    let updated = queries::update_server_settings(&state.pool, Some(admin.0.id), patch.clone())
        .await
        .map_err(ApiError::Internal)?;
    {
        let mut guard = state.settings.write().await;
        *guard = updated;
    }

    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(admin.0.id),
            action: "tasks.gate.update".into(),
            target_kind: Some("task_kind".into()),
            target_id: Some(meta.job_kind.into()),
            payload_json: serde_json::to_string(&serde_json::json!({
                "setting_key": key,
                "enabled": update.enabled,
            }))
            .ok(),
            ip: None,
            user_agent: None,
        },
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

/// Translate a registry `gate_setting_key` + bool value into a
/// `ServerSettingsUpdate` patch that sets the corresponding
/// optional field. Closed enumeration of known keys — adding a
/// new gate forces a compile error here, which keeps the patch
/// builder and the registry in sync.
fn build_gate_patch(key: &str, value: bool) -> Option<ServerSettingsUpdate> {
    let mut patch = ServerSettingsUpdate::default();
    match key {
        "loudness_analysis_enabled" => patch.loudness_analysis_enabled = Some(value),
        "subtitle_fetch_enabled" => patch.subtitle_fetch_enabled = Some(value),
        "embedded_subs_extract_enabled" => patch.embedded_subs_extract_enabled = Some(value),
        "external_ratings_enabled" => patch.external_ratings_enabled = Some(value),
        _ => return None,
    }
    Some(patch)
}

/// Locate the single `scheduled_tasks` row that backs a kind name in
/// the new admin UI. Mirrors the lookup the overview/detail builders
/// use: registry kinds resolve via their `sweep_kind` (preferred) or
/// `job_kind`; non-registry (legacy) names are looked up verbatim.
///
/// Returns 404 when no row exists. Per-library kinds (e.g.
/// `scan_library` with `library_id` params) are intentionally NOT
/// surfaced in this UI — those rows are managed from the library
/// admin page; if multiple matches exist we pick the first row that
/// has empty/global params, falling back to the first match.
async fn find_kind_schedule_row(
    state: &AppState,
    name: &str,
) -> Result<chimpflix_library::ScheduledTask, ApiError> {
    let mut rows = queries::list_scheduled_tasks(&state.pool)
        .await
        .map_err(ApiError::Internal)?;

    let candidate_kinds: Vec<String> = match registry::find_kind(name) {
        Some(meta) => {
            let mut v = Vec::new();
            if let Some(s) = meta.sweep_kind {
                v.push(s.to_string());
            }
            v.push(meta.job_kind.to_string());
            v
        }
        None => vec![name.to_string()],
    };

    // Prefer the global row (empty params) for kinds that may have
    // per-library siblings. Falls back to the first match.
    fn is_global(row: &chimpflix_library::ScheduledTask) -> bool {
        match serde_json::from_str::<serde_json::Value>(&row.params_json) {
            Ok(serde_json::Value::Object(o)) => o.get("library_id").is_none(),
            _ => true,
        }
    }

    for candidate in &candidate_kinds {
        if let Some(idx) = rows
            .iter()
            .position(|r| &r.kind == candidate && is_global(r))
        {
            return Ok(rows.swap_remove(idx));
        }
    }
    for candidate in &candidate_kinds {
        if let Some(idx) = rows.iter().position(|r| &r.kind == candidate) {
            return Ok(rows.swap_remove(idx));
        }
    }
    Err(ApiError::NotFound)
}

/// Frequency strings accepted by the scheduler. Kept here next to
/// the handler so the new UI's PATCH validates without re-routing
/// through the legacy per-row endpoint.
fn validate_frequency_value(frequency: &str) -> Result<(), ApiError> {
    const VALID: &[&str] = &[
        "manual",
        "hourly",
        "every_3_hours",
        "every_6_hours",
        "every_12_hours",
        "daily",
        "every_3_days",
        "weekly",
        "monthly",
        "on_change",
        "custom",
    ];
    if !VALID.contains(&frequency) {
        return Err(ApiError::validation(format!(
            "unknown frequency `{frequency}` — valid: {}",
            VALID.join(", ")
        )));
    }
    Ok(())
}

/// PATCH `/admin/tasks/kind/{name}` — edit the schedule row backing
/// a kind. Used by the rebuilt detail page's editable Schedule card.
/// Recomputes `next_run_at` whenever a schedule-affecting field
/// (frequency or maintenance-window) changes.
pub async fn update_kind_schedule(
    State(state): State<AppState>,
    admin: AdminAuth,
    Path(name): Path<String>,
    Json(update): Json<KindScheduleUpdate>,
) -> Result<Json<KindDetailResponse>, ApiError> {
    let existing = find_kind_schedule_row(&state, &name).await?;
    if let Some(ref freq) = update.frequency {
        validate_frequency_value(freq)?;
    }
    if let Some(ref params) = update.params_json {
        let parsed: serde_json::Value = serde_json::from_str(params)
            .map_err(|e| ApiError::validation(format!("params_json must be JSON: {e}")))?;
        if !parsed.is_object() {
            return Err(ApiError::validation("params_json must be a JSON object"));
        }
    }

    // Recompute next_run_at only when a schedule-affecting field
    // moved. enabled/params alone keep the existing tick.
    let schedule_changed =
        update.frequency.is_some() || update.requires_maintenance_window.is_some();
    let recomputed_next = if schedule_changed {
        let freq = update.frequency.as_deref().unwrap_or(&existing.frequency);
        let requires = update
            .requires_maintenance_window
            .unwrap_or(existing.requires_maintenance_window);
        Some(
            scheduler::compute_next_run_with_settings(
                &state,
                freq,
                &existing.cron_expr,
                now_ms(),
                requires,
            )
            .await
            .map_err(|e| ApiError::validation(format!("{e:#}")))?,
        )
    } else {
        None
    };

    let patch = ScheduledTaskUpdate {
        name: None,
        cron_expr: None,
        frequency: update.frequency.clone(),
        requires_maintenance_window: update.requires_maintenance_window,
        params_json: update.params_json.clone(),
        enabled: update.enabled,
    };
    queries::update_scheduled_task(&state.pool, existing.id, patch, recomputed_next)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;

    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(admin.0.id),
            action: "tasks.kind.schedule.update".into(),
            target_kind: Some("task_kind".into()),
            target_id: Some(name.clone()),
            payload_json: serde_json::to_string(&serde_json::json!({
                "frequency": update.frequency,
                "enabled": update.enabled,
                "requires_maintenance_window": update.requires_maintenance_window,
                "params_json": update.params_json,
            }))
            .ok(),
            ip: None,
            user_agent: None,
        },
    )
    .await;

    // Return the fresh detail payload so the client doesn't need a
    // follow-up GET to render the updated schedule.
    kind_detail(State(state), admin, Path(name))
        .await
        .map(|Json(d)| Json(d))
}

/// POST `/admin/tasks/kind/{name}/run` — dispatch the kind once via
/// the scheduler. Same path the legacy `/admin/tasks/{id}/run` takes
/// but keyed by kind name so the rebuilt UI doesn't need row IDs.
pub async fn run_kind_now(
    State(state): State<AppState>,
    admin: AdminAuth,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    let row = find_kind_schedule_row(&state, &name).await?;
    scheduler::run_now(state.clone(), row.id)
        .await
        .map_err(|e| ApiError::validation(format!("{e:#}")))?;
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(admin.0.id),
            action: "tasks.kind.run_now".into(),
            target_kind: Some("task_kind".into()),
            target_id: Some(name),
            payload_json: None,
            ip: None,
            user_agent: None,
        },
    )
    .await;
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_patch_known_keys() {
        let p = build_gate_patch("loudness_analysis_enabled", false).unwrap();
        assert_eq!(p.loudness_analysis_enabled, Some(false));
        let p = build_gate_patch("subtitle_fetch_enabled", true).unwrap();
        assert_eq!(p.subtitle_fetch_enabled, Some(true));
        let p = build_gate_patch("embedded_subs_extract_enabled", false).unwrap();
        assert_eq!(p.embedded_subs_extract_enabled, Some(false));
        let p = build_gate_patch("external_ratings_enabled", true).unwrap();
        assert_eq!(p.external_ratings_enabled, Some(true));
    }

    #[test]
    fn gate_patch_rejects_unknown_key() {
        assert!(build_gate_patch("not_a_real_gate", true).is_none());
    }

    #[test]
    fn parses_valid_hhmm() {
        assert_eq!(parse_hhmm("02:00"), Some(2 * 60 * 60 * 1000));
        assert_eq!(parse_hhmm("23:59"), Some((23 * 60 + 59) * 60 * 1000));
        assert_eq!(parse_hhmm("00:00"), Some(0));
    }

    #[test]
    fn rejects_invalid_hhmm() {
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
        assert_eq!(parse_hhmm("noon"), None);
        assert_eq!(parse_hhmm(""), None);
    }
}
