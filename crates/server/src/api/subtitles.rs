//! `/items/{id}/external-subtitles`, `/episodes/{id}/external-subtitles`,
//! and the file-serving endpoint at `/external-subtitles/{id}/file`.
//!
//! Listing is JSON metadata for the unified player picker; the file
//! endpoint streams the bytes (with a far-future cache header — the
//! file_id in the URL is stable).

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chimpflix_library::{ExternalSubtitle, queries};
use serde::Serialize;

use crate::api::access;
use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ExternalSubtitlesResponse {
    pub subtitles: Vec<ExternalSubtitle>,
}

pub async fn list_for_item(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ExternalSubtitlesResponse>, ApiError> {
    access::ensure_item_accessible(&state, &user, id).await?;
    let subtitles = queries::list_external_subtitles_for_item(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(ExternalSubtitlesResponse { subtitles }))
}

pub async fn list_for_episode(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<ExternalSubtitlesResponse>, ApiError> {
    access::ensure_episode_accessible(&state, &user, id).await?;
    let subtitles = queries::list_external_subtitles_for_episode(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(ExternalSubtitlesResponse { subtitles }))
}

/// Serves every external subtitle as WebVTT — the only sidecar format
/// HTML5 `<track>` actually renders. SRT (the OpenSubtitles default) is
/// converted on the fly; .vtt and .ass pass through (.ass gracefully
/// degrades — the browser ignores it but at least the bytes are there
/// for a JS renderer to consume later).
pub async fn serve_file(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    access::ensure_external_subtitle_accessible(&state, &user, id).await?;
    let row = queries::get_external_subtitle(&state.pool, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound)?;
    let bytes = tokio::fs::read(&row.file_path)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let lower = row.file_path.to_ascii_lowercase();
    let body_bytes = if lower.ends_with(".srt") {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        srt_to_vtt(&text).into_bytes()
    } else {
        bytes
    };

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/vtt; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000"),
        ],
        Body::from(body_bytes),
    )
        .into_response())
}

/// SubRip → WebVTT: prepend the "WEBVTT" header, then rewrite every
/// timestamp line `00:01:23,456 --> 00:01:24,789` to use a period
/// instead of a comma in the milliseconds field. The cue numbering and
/// the cue text are spec-compatible across the two formats, so we leave
/// them untouched.
fn srt_to_vtt(srt: &str) -> String {
    let mut out = String::with_capacity(srt.len() + 16);
    out.push_str("WEBVTT\n\n");
    for line in srt.lines() {
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::srt_to_vtt;

    #[test]
    fn timestamp_commas_become_periods() {
        let srt = "1\n00:00:01,000 --> 00:00:02,500\nHello\n";
        let vtt = srt_to_vtt(srt);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:02.500"));
        assert!(vtt.contains("Hello"));
    }

    #[test]
    fn commas_in_cue_text_are_preserved() {
        let srt = "1\n00:00:01,000 --> 00:00:02,500\nHello, world.\n";
        let vtt = srt_to_vtt(srt);
        assert!(vtt.contains("Hello, world."));
    }
}
