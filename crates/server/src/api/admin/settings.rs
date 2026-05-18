//! `GET /admin/settings` and `PATCH /admin/settings`.
//!
//! The settings cache on `AppState` is the source of truth for *readers*
//! (transcoder, CORS layer, etc.). DB persistence is the source of truth
//! across restarts. On PATCH we (1) write the diff to SQLite, (2) refresh
//! the cache from the canonical row, (3) audit the change.

use axum::Json;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::header::USER_AGENT;
use chimpflix_library::{NewAuditEntry, ServerSettings, ServerSettingsUpdate, queries};
use serde::Serialize;
use serde_json::json;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct SettingsResponse {
    pub settings: ServerSettings,
}

pub async fn get(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<SettingsResponse>, ApiError> {
    // Serve from the cache — it's already populated at startup and kept
    // fresh by `patch`. Cheap clone, no DB round-trip.
    let settings = state.settings.read().await.clone();
    Ok(Json(SettingsResponse { settings }))
}

pub async fn patch(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<ServerSettingsUpdate>,
) -> Result<Json<SettingsResponse>, ApiError> {
    validate(&input)?;

    let updated = queries::update_server_settings(&state.pool, Some(actor.id), input.clone())
        .await
        .map_err(ApiError::Internal)?;

    {
        let mut guard = state.settings.write().await;
        *guard = updated.clone();
    }

    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let payload = serde_json::to_string(&input).ok();
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "settings.update".into(),
            target_kind: Some("settings".into()),
            target_id: Some("1".into()),
            payload_json: payload,
            ip: None,
            user_agent,
        },
    )
    .await;

    Ok(Json(SettingsResponse { settings: updated }))
}

fn validate(patch: &ServerSettingsUpdate) -> Result<(), ApiError> {
    if let Some(ref s) = patch.secure_connections {
        if !matches!(s.as_str(), "required" | "preferred" | "disabled") {
            return Err(ApiError::validation(
                "secure_connections must be one of: required, preferred, disabled",
            ));
        }
    }
    if let Some(ref s) = patch.transcoder_hw_accel {
        if !matches!(
            s.as_str(),
            "auto" | "none" | "vaapi" | "nvenc" | "qsv" | "videotoolbox" | "amf"
        ) {
            return Err(ApiError::validation(
                "transcoder_hw_accel must be one of: auto, none, vaapi, nvenc, qsv, videotoolbox, amf",
            ));
        }
    }
    if let Some(ref s) = patch.transcoder_encoder_preset {
        if !matches!(s.as_str(), "speed" | "balanced" | "quality") {
            return Err(ApiError::validation(
                "transcoder_encoder_preset must be one of: speed, balanced, quality",
            ));
        }
    }
    if let Some(ref s) = patch.transcoder_hw_strictness {
        if !matches!(s.as_str(), "auto" | "prefer_hw" | "require_hw") {
            return Err(ApiError::validation(
                "transcoder_hw_strictness must be one of: auto, prefer_hw, require_hw",
            ));
        }
    }
    if let Some(n) = patch.transcoder_max_concurrent {
        if !(1..=64).contains(&n) {
            return Err(ApiError::validation(
                "transcoder_max_concurrent must be between 1 and 64",
            ));
        }
    }
    if let Some(Some(kbps)) = patch.transcoder_quality_ceiling_kbps {
        if !(100..=200_000).contains(&kbps) {
            return Err(ApiError::validation(
                "transcoder_quality_ceiling_kbps must be between 100 and 200000 when set",
            ));
        }
    }
    if let Some(ref raw) = patch.cors_origins {
        let parsed: serde_json::Result<Vec<String>> = serde_json::from_str(raw);
        match parsed {
            Ok(_) => {}
            Err(_) => {
                return Err(ApiError::validation(
                    "cors_origins must be a JSON array of strings",
                ));
            }
        }
    }
    if let Some(ref raw) = patch.extras_json {
        let parsed: serde_json::Result<serde_json::Value> = serde_json::from_str(raw);
        if let Ok(v) = parsed {
            if !v.is_object() {
                return Err(ApiError::validation("extras_json must be a JSON object"));
            }
        } else {
            return Err(ApiError::validation("extras_json must be valid JSON"));
        }
    }
    if let Some(Some(ref url)) = patch.public_url {
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err(ApiError::validation(
                "public_url must start with http:// or https://",
            ));
        }
    }
    if let Some(Some(ref s)) = patch.email_smtp_security {
        if !matches!(s.as_str(), "starttls" | "tls" | "none") {
            return Err(ApiError::validation(
                "email_smtp_security must be one of: starttls, tls, none",
            ));
        }
    }
    if let Some(Some(port)) = patch.email_smtp_port {
        if !(1..=65535).contains(&port) {
            return Err(ApiError::validation(
                "email_smtp_port must be between 1 and 65535",
            ));
        }
    }
    if let Some(Some(ref host)) = patch.email_smtp_host {
        if host.len() > 253 {
            return Err(ApiError::validation(
                "email_smtp_host must be at most 253 characters",
            ));
        }
        if host.contains(char::is_whitespace) {
            return Err(ApiError::validation(
                "email_smtp_host must not contain whitespace",
            ));
        }
    }
    if let Some(Some(ref user)) = patch.email_smtp_username {
        if user.len() > 256 {
            return Err(ApiError::validation(
                "email_smtp_username must be at most 256 characters",
            ));
        }
    }
    if let Some(Some(ref addr)) = patch.email_from_address {
        // Light-touch sanity check; lettre re-validates when building
        // the Mailbox. Reject obvious garbage early so the UI shows a
        // useful message instead of a generic SMTP connect failure.
        if !addr.contains('@') || addr.len() > 320 {
            return Err(ApiError::validation(
                "email_from_address must look like local@domain (max 320 chars)",
            ));
        }
    }
    if let Some(Some(ref name)) = patch.email_from_name {
        if name.len() > 128 {
            return Err(ApiError::validation(
                "email_from_name must be at most 128 characters",
            ));
        }
    }
    if let Some(ref s) = patch.totp_enforcement {
        if !matches!(s.as_str(), "disabled" | "optional" | "required") {
            return Err(ApiError::validation(
                "totp_enforcement must be one of: disabled, optional, required",
            ));
        }
    }
    // Silence the unused `json!` import when downstream features go away.
    let _ = json!({});
    Ok(())
}
