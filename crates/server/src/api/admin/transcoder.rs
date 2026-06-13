//! Transcoder admin surface: capability report + preset CRUD.
//!
//! Capabilities are read from `AppState.transcoder_caps` (a refreshable
//! `SharedCapabilities` holder) populated at startup and re-runnable at
//! runtime via the re-probe endpoint. Presets are persisted in SQLite
//! and managed through this module's CRUD endpoints; the player picker
//! reads them via the same endpoint to offer matching qualities.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::USER_AGENT;
use axum::http::{HeaderMap, StatusCode};
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
    /// segments, and sidecar VTTs. Surfaced
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
        capabilities: (*state.transcoder_caps.load()).clone(),
        cache_root: state.transcoder.cache_root().display().to_string(),
    }))
}

/// Re-run hardware capability detection without a server restart.
///
/// The boot-time probe runs `ffmpeg -hwaccels`/`-encoders` + per-encoder
/// and per-decoder one-frame smoke tests, then caches the result. After
/// a driver upgrade or a GPU hot-add/-remove that cache is stale until a
/// restart. This endpoint re-runs the *same* `detect_capabilities` used
/// at boot and atomically swaps the fresh result into the shared
/// `SharedCapabilities` holder, so subsequent GETs *and* live encoder
/// selection (the manager shares the same handle) immediately see it.
///
/// Owner-only and audited. The probe shells out to ffmpeg up to a dozen
/// times (bounded by the per-test SMOKE_TIMEOUT), so it can take a few
/// seconds on a cold box; the swap itself is instantaneous.
pub async fn reprobe_capabilities(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
) -> Result<Json<CapabilitiesResponse>, ApiError> {
    // Same detection routine used at boot, against the configured
    // ffmpeg binary. `detect_capabilities` invokes ffmpeg directly
    // (not the background-nice wrapper), so any scanner nice level on
    // `state.ffmpeg` is irrelevant here — identical detection to boot.
    let fresh = chimpflix_transcoder::detect_capabilities(&state.ffmpeg).await;
    // Publish atomically. Both `AppState.transcoder_caps` and the
    // transcode manager hold this same handle, so both observe the swap.
    state.transcoder_caps.store(fresh.clone());
    let user_agent = headers
        .get(USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    audit_log(
        &state,
        NewAuditEntry {
            actor_user_id: Some(actor.id),
            action: "transcoder.capabilities.reprobe".to_string(),
            target_kind: Some("transcoder".into()),
            target_id: None,
            payload_json: None,
            ip: None,
            user_agent,
        },
    )
    .await;
    Ok(Json(CapabilitiesResponse {
        capabilities: fresh,
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
    audit(
        &state,
        actor.id,
        &headers,
        "preset.create",
        preset.id,
        &input,
    )
    .await;
    Ok((StatusCode::CREATED, Json(PresetResponse { preset })))
}

pub async fn update_preset(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Json(input): Json<TranscoderPresetUpdate>,
) -> Result<Json<PresetResponse>, ApiError> {
    if let Some(n) = input.name.as_deref() {
        if n.trim().is_empty() {
            return Err(ApiError::validation("name is required"));
        }
    }
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
    if let Some(b) = input.audio_bitrate_kbps {
        if !(0..=512).contains(&b) {
            return Err(ApiError::validation("audio_bitrate_kbps must be 0..=512"));
        }
    }
    if let Some(codec) = input.audio_codec.as_deref() {
        validate_audio_codec(codec)?;
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

/// Allowlisted `-c:a` values passed verbatim to ffmpeg. Any value outside
/// this set causes ffmpeg to exit non-zero, silently failing every encode
/// job queued against that preset.
fn validate_audio_codec(codec: &str) -> Result<(), ApiError> {
    if !matches!(
        codec,
        "aac" | "opus" | "mp3" | "ac3" | "eac3" | "flac" | "copy"
    ) {
        return Err(ApiError::validation(
            "audio_codec must be one of: aac, opus, mp3, ac3, eac3, flac, copy",
        ));
    }
    Ok(())
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
    validate_audio_codec(&input.audio_codec)?;
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
