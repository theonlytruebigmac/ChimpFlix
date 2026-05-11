//! Streaming endpoints:
//!
//! * `GET /api/v1/stream/{file_id}/direct` — HTTP Range direct play.
//! * `POST /api/v1/stream/sessions` — decide direct-play vs transcode.
//! * `DELETE /api/v1/stream/sessions/{id}` — close transcode session.
//! * `GET /api/v1/stream/sessions/{id}/master.m3u8` — synthesized master.
//! * `GET /api/v1/stream/sessions/{id}/{variant}/{name}` — variant
//!   manifest and segments.

use std::path::Path as StdPath;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio_util::io::ReaderStream;
use tracing::warn;

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Direct play (HTTP Range)
// ---------------------------------------------------------------------------

pub async fn direct(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(file_id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let locator = queries::get_media_file_locator(&state.pool, file_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let path = StdPath::new(&locator.path);
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))?;
    let total_size = meta.len();
    if total_size == 0 {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, content_type_for(path))
            .header(header::CONTENT_LENGTH, "0")
            .header(header::ACCEPT_RANGES, "bytes")
            .body(Body::empty())
            .map_err(|e| ApiError::Internal(anyhow::Error::from(e)));
    }

    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_range);

    let (status, start, end) = match range {
        Some((start, end_opt)) => {
            let end = end_opt.unwrap_or(total_size - 1).min(total_size - 1);
            if start >= total_size || start > end {
                return Ok(range_not_satisfiable(total_size));
            }
            (StatusCode::PARTIAL_CONTENT, start, end)
        }
        None => (StatusCode::OK, 0, total_size - 1),
    };

    let length = end - start + 1;
    let mut file = File::open(path)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))?;
    if start > 0 {
        file.seek(SeekFrom::Start(start))
            .await
            .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))?;
    }
    let stream = ReaderStream::new(file.take(length));
    let body = Body::from_stream(stream);

    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type_for(path))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, length.to_string());

    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{total_size}"),
        );
    }

    builder
        .body(body)
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))
}

fn range_not_satisfiable(total_size: u64) -> Response {
    warn!(total_size, "range not satisfiable");
    (
        StatusCode::RANGE_NOT_SATISFIABLE,
        [(header::CONTENT_RANGE, format!("bytes */{total_size}"))],
    )
        .into_response()
}

fn parse_range(s: &str) -> Option<(u64, Option<u64>)> {
    let rest = s.trim().strip_prefix("bytes=")?;
    let (start_s, end_s) = rest.split_once('-')?;
    let start_s = start_s.trim();
    let end_s = end_s.trim();
    if start_s.is_empty() {
        return None;
    }
    let start: u64 = start_s.parse().ok()?;
    let end: Option<u64> = if end_s.is_empty() {
        None
    } else {
        end_s.parse().ok()
    };
    Some((start, end))
}

fn content_type_for(path: &StdPath) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mkv" => "video/x-matroska",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "ts" | "m2ts" => "video/mp2t",
        "mpg" | "mpeg" => "video/mpeg",
        "wmv" => "video/x-ms-wmv",
        "flv" => "video/x-flv",
        "ogv" => "video/ogg",
        _ => "application/octet-stream",
    }
}

// ---------------------------------------------------------------------------
// Transcode sessions
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub media_file_id: i64,
    #[serde(default)]
    pub start_position_ms: i64,
    pub client: ClientCapabilities,
}

#[derive(Debug, Deserialize, Default)]
pub struct ClientCapabilities {
    #[serde(default)]
    pub supported_video_codecs: Vec<String>,
    #[serde(default)]
    pub supported_audio_codecs: Vec<String>,
    #[serde(default)]
    pub supported_containers: Vec<String>,
    #[serde(default)]
    pub max_bandwidth_bps: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub session: SessionInfo,
}

#[derive(Debug, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direct_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hls_master_url: Option<String>,
    pub media_file_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
}

pub async fn create_session(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), ApiError> {
    let locator = queries::get_media_file_locator(&state.pool, req.media_file_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // We need stream codecs to decide between direct/transcode. Pull them
    // from the existing detail helper plumbing.
    let files = sqlx::query("SELECT id, container, duration_ms FROM media_files WHERE id = ?")
        .bind(req.media_file_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?
        .ok_or(ApiError::NotFound)?;

    use sqlx::Row;
    let container: Option<String> = files.try_get("container").ok();
    let duration_ms: Option<i64> = files.try_get("duration_ms").ok();
    let bit_rate_row = sqlx::query("SELECT bit_rate FROM media_files WHERE id = ?")
        .bind(req.media_file_id)
        .fetch_one(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    let bit_rate: Option<i64> = bit_rate_row.try_get("bit_rate").ok();

    let streams = sqlx::query("SELECT kind, codec FROM media_streams WHERE media_file_id = ?")
        .bind(req.media_file_id)
        .fetch_all(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let stream_pairs: Vec<(String, Option<String>)> = streams
        .iter()
        .map(|r| {
            (
                r.try_get::<String, _>("kind").unwrap_or_default(),
                r.try_get::<Option<String>, _>("codec").ok().flatten(),
            )
        })
        .collect();

    let mode = decide_mode(&stream_pairs, container.as_deref(), bit_rate, &req.client);

    match mode {
        PlayMode::Direct => Ok((
            StatusCode::CREATED,
            Json(CreateSessionResponse {
                session: SessionInfo {
                    id: "direct".into(),
                    mode: "direct",
                    direct_url: Some(format!("/api/v1/stream/{}/direct", req.media_file_id)),
                    hls_master_url: None,
                    media_file_id: req.media_file_id,
                    duration_ms,
                },
            }),
        )),
        PlayMode::Transcode => {
            let session = state
                .transcoder
                .start(
                    req.media_file_id,
                    StdPath::new(&locator.path),
                    req.start_position_ms,
                    duration_ms,
                    user.id,
                )
                .await
                .map_err(ApiError::Internal)?;
            let master_url = format!("/api/v1/stream/sessions/{}/master.m3u8", session.id);
            Ok((
                StatusCode::CREATED,
                Json(CreateSessionResponse {
                    session: SessionInfo {
                        id: session.id.clone(),
                        mode: "transcode",
                        direct_url: None,
                        hls_master_url: Some(master_url),
                        media_file_id: req.media_file_id,
                        duration_ms,
                    },
                }),
            ))
        }
    }
}

pub async fn delete_session(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let removed = state.transcoder.delete(&id).await;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        // Idempotent on already-gone sessions.
        Ok(StatusCode::NO_CONTENT)
    }
}

pub async fn master_playlist(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let session = state.transcoder.get(&id).ok_or(ApiError::NotFound)?;
    session.touch();
    let body = session.master_playlist();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(body))
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))
}

pub async fn variant_file(
    State(state): State<AppState>,
    _user: AuthUser,
    Path((id, variant, name)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let session = state.transcoder.get(&id).ok_or(ApiError::NotFound)?;
    session.touch();

    // Whitelist allowed filenames.
    let is_manifest = name == "index.m3u8";
    let is_segment = name.starts_with("seg-") && name.ends_with(".ts");
    if !is_manifest && !is_segment {
        return Err(ApiError::NotFound);
    }
    if variant != chimpflix_transcoder::Session::variant_name() {
        return Err(ApiError::NotFound);
    }

    let path = session.output_dir.join(&variant).join(&name);

    // For the variant manifest, give ffmpeg a short window to write it
    // out on the first request after session start — and wait until it
    // has non-zero content (the file appears empty while the writer is
    // still streaming heredoc / initial bytes). Segments use atomic
    // rename via `temp_file`, so they don't need the same grace; the
    // player retries 404s naturally.
    if is_manifest {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(meta) = tokio::fs::metadata(&path).await {
                if meta.len() > 0 {
                    break;
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    if !path.exists() {
        return Err(ApiError::NotFound);
    }

    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))?;
    let content_type = if is_manifest {
        "application/vnd.apple.mpegurl"
    } else {
        "video/mp2t"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))
}

// ---------------------------------------------------------------------------
// Decision logic
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    Direct,
    Transcode,
}

pub fn decide_mode(
    streams: &[(String, Option<String>)],
    container: Option<&str>,
    file_bit_rate: Option<i64>,
    client: &ClientCapabilities,
) -> PlayMode {
    let Some(container) = container else {
        return PlayMode::Transcode;
    };

    let video_codec = streams
        .iter()
        .find(|(kind, _)| kind == "video")
        .and_then(|(_, c)| c.clone());
    let audio_codec = streams
        .iter()
        .find(|(kind, _)| kind == "audio")
        .and_then(|(_, c)| c.clone());

    let Some(v) = video_codec else {
        return PlayMode::Transcode;
    };
    let Some(a) = audio_codec else {
        return PlayMode::Transcode;
    };

    let video_ok = client
        .supported_video_codecs
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&v));
    let audio_ok = client
        .supported_audio_codecs
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&a));
    let container_ok = client
        .supported_containers
        .iter()
        .any(|c| c.eq_ignore_ascii_case(container));
    let bandwidth_ok = match (client.max_bandwidth_bps, file_bit_rate) {
        (Some(max), Some(file)) => file <= max,
        _ => true,
    };

    if video_ok && audio_ok && container_ok && bandwidth_ok {
        PlayMode::Direct
    } else {
        PlayMode::Transcode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps_mp4_h264_aac() -> ClientCapabilities {
        ClientCapabilities {
            supported_video_codecs: vec!["h264".into(), "hevc".into()],
            supported_audio_codecs: vec!["aac".into(), "ac3".into()],
            supported_containers: vec!["mp4".into(), "mkv".into()],
            max_bandwidth_bps: Some(25_000_000),
        }
    }

    #[test]
    fn direct_when_all_match() {
        let streams = vec![
            ("video".into(), Some("h264".into())),
            ("audio".into(), Some("aac".into())),
        ];
        assert_eq!(
            decide_mode(&streams, Some("mp4"), Some(8_000_000), &caps_mp4_h264_aac()),
            PlayMode::Direct,
        );
    }

    #[test]
    fn transcode_on_codec_mismatch() {
        let streams = vec![
            ("video".into(), Some("vp9".into())),
            ("audio".into(), Some("aac".into())),
        ];
        assert_eq!(
            decide_mode(&streams, Some("mp4"), Some(8_000_000), &caps_mp4_h264_aac()),
            PlayMode::Transcode,
        );
    }

    #[test]
    fn transcode_on_container_mismatch() {
        let streams = vec![
            ("video".into(), Some("h264".into())),
            ("audio".into(), Some("aac".into())),
        ];
        assert_eq!(
            decide_mode(
                &streams,
                Some("matroska"),
                Some(8_000_000),
                &caps_mp4_h264_aac()
            ),
            PlayMode::Transcode,
        );
    }

    #[test]
    fn transcode_when_bandwidth_too_high() {
        let streams = vec![
            ("video".into(), Some("h264".into())),
            ("audio".into(), Some("aac".into())),
        ];
        assert_eq!(
            decide_mode(
                &streams,
                Some("mp4"),
                Some(50_000_000),
                &caps_mp4_h264_aac()
            ),
            PlayMode::Transcode,
        );
    }

    #[test]
    fn transcode_when_no_audio_or_video_stream() {
        let streams_no_audio = vec![("video".into(), Some("h264".into()))];
        assert_eq!(
            decide_mode(
                &streams_no_audio,
                Some("mp4"),
                Some(8_000_000),
                &caps_mp4_h264_aac()
            ),
            PlayMode::Transcode,
        );
    }

    #[test]
    fn parse_full_range() {
        assert_eq!(parse_range("bytes=0-99"), Some((0, Some(99))));
    }

    #[test]
    fn parse_open_range() {
        assert_eq!(parse_range("bytes=500-"), Some((500, None)));
    }

    #[test]
    fn parse_suffix_unsupported() {
        assert_eq!(parse_range("bytes=-200"), None);
    }

    #[test]
    fn parse_garbage_returns_none() {
        assert_eq!(parse_range("seconds=1-2"), None);
        assert_eq!(parse_range("bytes=abc-def"), None);
    }
}
