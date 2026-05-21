//! `/admin/preroll` + `/preroll/blob` — operator-uploaded pre-roll video.

use axum::Json;
use axum::body::Body;
use axum::extract::{Multipart, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::http::header::USER_AGENT;
use axum::response::Response;
use chimpflix_library::{NewAuditEntry, ServerSettingsUpdate, queries};
use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::api::admin::audit_log;
use crate::api::error::ApiError;
use crate::auth::{AuthUser, OwnerAuth};
use crate::state::AppState;

const MAX_PREROLL_BYTES: usize = 200 * 1024 * 1024; // 200 MiB
const PREROLL_DIR: &str = "preroll";

#[derive(Debug, Serialize)]
pub struct PrerollStatus {
    pub enabled: bool,
    pub configured: bool,
    /// Server-relative URL for the player to fetch the file. None when
    /// no pre-roll is configured.
    pub url: Option<String>,
    /// Bytes on disk (best-effort stat); None when not configured.
    pub size_bytes: Option<u64>,
    /// Output level 0..=100; the player applies this as `video.volume`
    /// when it mounts the gate, so a single fetch covers both
    /// playability and volume.
    pub volume: i64,
}

pub async fn get_status(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<PrerollStatus>, ApiError> {
    let settings = state.settings.read().await.clone();
    let configured = settings.preroll_path.is_some();
    let size_bytes = if let Some(rel) = &settings.preroll_path {
        let abs = state.data_dir.join(PREROLL_DIR).join(rel);
        tokio::fs::metadata(&abs).await.ok().map(|m| m.len())
    } else {
        None
    };
    Ok(Json(PrerollStatus {
        enabled: settings.preroll_enabled,
        configured,
        url: settings
            .preroll_path
            .as_ref()
            .map(|_| format!("/api/v1/preroll/blob?v={}", chimpflix_common::now_ms())),
        size_bytes,
        volume: settings.preroll_volume,
    }))
}

pub async fn upload(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<PrerollStatus>, ApiError> {
    let mut bytes: Option<Vec<u8>> = None;
    let mut ext: Option<&'static str> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::validation(format!("multipart error: {e}")))?
    {
        if field.name() == Some("file") {
            let ct = field.content_type().map(str::to_owned);
            ext = match ct.as_deref() {
                Some("video/mp4") => Some("mp4"),
                Some("video/webm") => Some("webm"),
                Some("video/x-matroska" | "video/matroska") => Some("mkv"),
                Some(other) => {
                    return Err(ApiError::validation(format!(
                        "unsupported content-type `{other}` (use video/mp4, video/webm, video/x-matroska)"
                    )));
                }
                None => return Err(ApiError::validation("missing content-type")),
            };
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::validation(format!("read field: {e}")))?;
            if data.len() > MAX_PREROLL_BYTES {
                return Err(ApiError::validation(format!(
                    "pre-roll must be ≤ {MAX_PREROLL_BYTES} bytes"
                )));
            }
            bytes = Some(data.to_vec());
            break;
        }
    }
    let bytes = bytes.ok_or_else(|| ApiError::validation("missing `file` field"))?;
    let ext = ext.expect("ext set alongside bytes above");

    let dir = state.data_dir.join(PREROLL_DIR);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    // Wipe sibling extensions before writing so a switch from .mp4 to
    // .webm doesn't leave a stale file the next get_status fingers as
    // configured.
    for prev_ext in ["mp4", "webm", "mkv"] {
        let prev = dir.join(format!("preroll.{prev_ext}"));
        if prev.exists() {
            let _ = tokio::fs::remove_file(&prev).await;
        }
    }
    let filename = format!("preroll.{ext}");
    let path = dir.join(&filename);
    let mut f = tokio::fs::File::create(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    f.write_all(&bytes)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    f.flush().await.ok();

    let updated = queries::update_server_settings(
        &state.pool,
        Some(actor.id),
        ServerSettingsUpdate {
            preroll_path: Some(Some(filename.clone())),
            ..Default::default()
        },
    )
    .await
    .map_err(ApiError::Internal)?;
    *state.settings.write().await = updated.clone();

    audit_with(&state, actor.id, &headers, "preroll.upload", &filename).await;

    Ok(Json(PrerollStatus {
        enabled: updated.preroll_enabled,
        configured: true,
        url: Some(format!("/api/v1/preroll/blob?v={}", chimpflix_common::now_ms())),
        size_bytes: Some(bytes.len() as u64),
        volume: updated.preroll_volume,
    }))
}

pub async fn clear(
    State(state): State<AppState>,
    OwnerAuth(actor): OwnerAuth,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let dir = state.data_dir.join(PREROLL_DIR);
    for ext in ["mp4", "webm", "mkv"] {
        let path = dir.join(format!("preroll.{ext}"));
        if path.exists() {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
    let updated = queries::update_server_settings(
        &state.pool,
        Some(actor.id),
        ServerSettingsUpdate {
            preroll_path: Some(None),
            preroll_enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .map_err(ApiError::Internal)?;
    *state.settings.write().await = updated;
    audit_with(&state, actor.id, &headers, "preroll.clear", "").await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn serve_blob(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Response, ApiError> {
    let settings = state.settings.read().await.clone();
    let Some(rel) = settings.preroll_path else {
        return Err(ApiError::NotFound);
    };
    if !settings.preroll_enabled {
        return Err(ApiError::NotFound);
    }
    let path = state.data_dir.join(PREROLL_DIR).join(&rel);
    let content_type = match path.extension().and_then(|e| e.to_str()) {
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        _ => "application/octet-stream",
    };
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, HeaderValue::from_static(content_type))
        .header(header::CACHE_CONTROL, "public, max-age=3600")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal(e.into()))
}

async fn audit_with(
    state: &AppState,
    actor_id: i64,
    headers: &HeaderMap,
    action: &str,
    payload: &str,
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
            target_kind: Some("preroll".into()),
            target_id: None,
            payload_json: serde_json::to_string(payload).ok(),
            ip: None,
            user_agent,
        },
    )
    .await;
}
