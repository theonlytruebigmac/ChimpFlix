//! Transcoder admin surface: capability report + preset CRUD.
//!
//! Capabilities are read from the static `AppState.transcoder_caps` populated
//! at startup. Presets are persisted in SQLite and managed through this
//! module's CRUD endpoints; the player picker reads them via the same
//! endpoint to offer matching qualities.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::http::header::USER_AGENT;
use chimpflix_library::{
    NewAuditEntry, NewTranscoderPreset, TranscoderPreset, TranscoderPresetUpdate, queries,
};
use chimpflix_transcoder::TranscoderCapabilities;
use serde::Serialize;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::OwnerAuth;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct CapabilitiesResponse {
    pub capabilities: TranscoderCapabilities,
    /// Absolute path to where the transcoder writes HLS segments, init
    /// segments, sidecar VTTs, and per-file preview sprites. Surfaced
    /// so the admin UI can show "Transcoder temp directory" the way
    /// Plex does without giving the operator a knob to break it
    /// (changing the path needs a server restart so all in-flight
    /// sessions don't lose their working dir).
    pub cache_root: String,
}

#[derive(Debug, Serialize)]
pub struct PresetsListResponse {
    pub presets: Vec<TranscoderPreset>,
}

#[derive(Debug, Serialize)]
pub struct PresetResponse {
    pub preset: TranscoderPreset,
}

pub async fn capabilities(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<CapabilitiesResponse>, ApiError> {
    Ok(Json(CapabilitiesResponse {
        capabilities: (*state.transcoder_caps).clone(),
        cache_root: state.transcoder.cache_root().display().to_string(),
    }))
}

pub async fn list_presets(
    State(state): State<AppState>,
    _owner: OwnerAuth,
) -> Result<Json<PresetsListResponse>, ApiError> {
    let presets = queries::list_transcoder_presets(&state.pool)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(PresetsListResponse { presets }))
}

pub async fn create_preset(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    Json(input): Json<NewTranscoderPreset>,
) -> Result<(StatusCode, Json<PresetResponse>), ApiError> {
    validate(&input)?;
    let preset = queries::create_transcoder_preset(&state.pool, input.clone())
        .await
        .map_err(|e| {
            let msg = format!("{e:#}");
            if msg.contains("UNIQUE constraint") {
                ApiError::Conflict("preset name already exists".into())
            } else {
                ApiError::Internal(e)
            }
        })?;
    audit(&state, actor.id, &headers, "preset.create", preset.id, &input).await;
    Ok((StatusCode::CREATED, Json(PresetResponse { preset })))
}

pub async fn update_preset(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<TranscoderPresetUpdate>,
) -> Result<Json<PresetResponse>, ApiError> {
    if let Some(b) = input.max_video_bitrate_kbps {
        if !(0..=200_000).contains(&b) {
            return Err(ApiError::validation(
                "max_video_bitrate_kbps must be 0..=200000",
            ));
        }
    }
    if let Some(h) = input.max_height {
        if !(0..=4320).contains(&h) {
            return Err(ApiError::validation("max_height must be 0..=4320"));
        }
    }
    let preset = queries::update_transcoder_preset(&state.pool, id, input.clone())
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    audit(&state, actor.id, &headers, "preset.update", id, &input).await;
    Ok(Json(PresetResponse { preset }))
}

pub async fn delete_preset(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let removed = queries::delete_transcoder_preset(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    if !removed {
        return Err(ApiError::NotFound);
    }
    audit(&state, actor.id, &headers, "preset.delete", id, &()).await;
    Ok(StatusCode::NO_CONTENT)
}

fn validate(input: &NewTranscoderPreset) -> Result<(), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::validation("name is required"));
    }
    if !(0..=200_000).contains(&input.max_video_bitrate_kbps) {
        return Err(ApiError::validation(
            "max_video_bitrate_kbps must be 0..=200000",
        ));
    }
    if !(0..=4320).contains(&input.max_height) {
        return Err(ApiError::validation("max_height must be 0..=4320"));
    }
    if !(0..=512).contains(&input.audio_bitrate_kbps) {
        return Err(ApiError::validation("audio_bitrate_kbps must be 0..=512"));
    }
    Ok(())
}

async fn audit<T: serde::Serialize>(
    state: &AppState,
    actor_id: i64,
    headers: &HeaderMap,
    action: &str,
    target_id: i64,
    payload: &T,
) {
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        state,
        NewAuditEntry {
            actor_user_id: Some(actor_id),
            action: action.to_string(),
            target_kind: Some("transcoder_preset".into()),
            target_id: Some(target_id.to_string()),
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}
