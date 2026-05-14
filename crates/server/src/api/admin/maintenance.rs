//! /admin/logs, /admin/alerts, /admin/privacy — Phase 10.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, queries};
use serde::{Deserialize, Serialize};

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::log_buffer::LogLine;
use crate::state::AppState;

// ─── Logs ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LogsParams {
    pub level: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct LogsResponse {
    pub lines: Vec<LogLine>,
}

pub async fn logs(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(params): Query<LogsParams>,
) -> Result<Json<LogsResponse>, ApiError> {
    let limit = params.limit.unwrap_or(200).min(2_000);
    let lines = state.log_buffer.snapshot(params.level.as_deref(), limit);
    Ok(Json(LogsResponse { lines }))
}

// ─── Alerts ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AlertsParams {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AlertsResponse {
    pub log_alerts: Vec<LogLine>,
    pub audit: Vec<chimpflix_library::AuditLogEntry>,
}

pub async fn alerts(
    State(state): State<AppState>,
    _owner: OwnerAuth,
    Query(params): Query<AlertsParams>,
) -> Result<Json<AlertsResponse>, ApiError> {
    let limit = params.limit.unwrap_or(50);
    // Alerts surface = recent WARN/ERROR log lines + the audit feed.
    let log_alerts = state.log_buffer.snapshot(Some("WARN"), limit as usize);
    let audit = queries::list_audit(&state.pool, None, limit)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(AlertsResponse { log_alerts, audit }))
}

// ─── Privacy ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PrivacyResponse {
    pub telemetry_opt_in: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PrivacyUpdate {
    pub telemetry_opt_in: bool,
}

pub async fn get_privacy(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<PrivacyResponse>, ApiError> {
    let s = state.settings.read().await.clone();
    Ok(Json(PrivacyResponse {
        telemetry_opt_in: s.telemetry_opt_in,
    }))
}

pub async fn patch_privacy(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<PrivacyUpdate>,
) -> Result<Json<PrivacyResponse>, ApiError> {
    let updated = queries::update_server_settings(
        &state.pool,
        Some(actor.id),
        chimpflix_library::ServerSettingsUpdate {
            telemetry_opt_in: Some(input.telemetry_opt_in),
            ..Default::default()
        },
    )
    .await
    .map_err(ApiError::Internal)?;
    {
        let mut g = state.settings.write().await;
        *g = updated.clone();
    }

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "privacy.update".into(),
            target_kind: Some("settings".into()),
            target_id: Some("1".into()),
            payload_json: serde_json::to_string(&input).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok(Json(PrivacyResponse {
        telemetry_opt_in: updated.telemetry_opt_in,
    }))
}
