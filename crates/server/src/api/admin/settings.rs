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
            "none" | "vaapi" | "nvenc" | "qsv" | "videotoolbox"
        ) {
            return Err(ApiError::validation(
                "transcoder_hw_accel must be one of: none, vaapi, nvenc, qsv, videotoolbox",
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
    // Silence the unused `json!` import when downstream features go away.
    let _ = json!({});
    Ok(())
}
