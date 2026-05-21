//! Streaming endpoints:
//!
//! * `GET /api/v1/stream/{file_id}/direct` — HTTP Range direct play.
//! * `POST /api/v1/stream/sessions` — decide direct-play vs transcode.
//! * `DELETE /api/v1/stream/sessions/{id}` — close transcode session.
//! * `GET /api/v1/stream/sessions/{id}/master.m3u8` — synthesized master.
//! * `GET /api/v1/stream/sessions/{id}/{variant}/{name}` — variant
//!   manifest and segments.

use std::net::IpAddr;
use std::path::Path as StdPath;
use std::time::{Duration, Instant};

use axum::Extension;
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
use tracing::{info, warn};

use crate::api::error::ApiError;
use crate::auth::AuthUser;
use crate::client_ip::EffectiveClientIp;
use crate::net;
use crate::state::AppState;

/// Classify the request as "remote" (not on the operator-defined LAN)
/// for the per-user remote-streams cap. Empty LAN list = always remote
/// (cap-then-reject is the conservative path).
///
/// `ip` MUST be the effective client IP — the trusted-proxy-resolved
/// value from [`crate::client_ip::EffectiveClientIp`]. Reading raw
/// `X-Forwarded-For` here would let a remote attacker spoof `LAN` and
/// bypass the per-user cap.
fn is_remote_request(ip: IpAddr, lan_raw: &str) -> bool {
    let trimmed = lan_raw.trim();
    if trimmed.is_empty() {
        return true;
    }
    let nets = net::parse_cidr_list(trimmed);
    !net::ip_in_list(ip, &nets)
}

// Stream-side library access enforcement lives in [`crate::api::access`].
// Re-export under the local name so existing callsites don't need to
// chase a different module path.
use crate::api::access::ensure_file_accessible;

// ---------------------------------------------------------------------------
// Direct play (HTTP Range)
// ---------------------------------------------------------------------------

pub async fn direct(
    State(state): State<AppState>,
    user: AuthUser,
    Path(file_id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    ensure_file_accessible(&state, &user, file_id).await?;
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
        // WebVTT subtitle sidecar served from the HLS subtitle
        // group. Wrong content-type = HLS.js refuses to parse the
        // body, sub track silently no-ops.
        "vtt" => "text/vtt",
        "m4s" => "video/iso.segment",
        "m3u8" => "application/vnd.apple.mpegurl",
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
    /// 0-indexed among the file's audio streams. None means "let ffmpeg
    /// pick" (default behavior — usually first audio).
    #[serde(default)]
    pub audio_index: Option<u32>,
    /// 0-indexed among the file's subtitle streams. When set, subs are
    /// burned in and the response will always be a transcode session.
    #[serde(default)]
    pub subtitle_index: Option<u32>,
    /// Pre-built ASS `force_style` argument (e.g.
    /// `Fontsize=28,PrimaryColour=&H00FFFFFF&,BorderStyle=3,Alignment=2`).
    /// Appended to ffmpeg's `subtitles=` filter so the burned-in track
    /// honors the user's font/color/background prefs. Ignored for
    /// picture subtitles (PGS/DVD) which can't be restyled. The
    /// transcoder validates this against an allowlist before splicing
    /// it into the filter string.
    #[serde(default)]
    pub subtitle_style: Option<String>,
    /// Explicit quality tier. When set, the session always transcodes
    /// (direct play can't reshape bitrate), and ffmpeg is configured
    /// with the requested scale + video bitrate. Omit to use the
    /// transcoder's defaults.
    #[serde(default)]
    pub quality_target: Option<QualityTarget>,
    /// EBU R128 audio loudness normalization. `true` forces the
    /// audio path through ffmpeg's `loudnorm` filter (which in turn
    /// requires audio re-encode — we suppress the audio-copy fast
    /// path automatically when this is set). Omitted / `false` keeps
    /// audio as-is.
    #[serde(default)]
    pub audio_normalize: bool,
    /// User-controlled subtitle sync offset in milliseconds. Positive
    /// values delay the subtitle (cues appear later than the video);
    /// negative values advance it. Applied to the WebVTT sidecar's
    /// cue timestamps so the player sees a track that's already
    /// in sync — no client-side adjustment required. Default 0
    /// (no offset).
    ///
    /// The player exposes this as a "+0.5 s / -0.5 s" stepper in
    /// the subtitle settings panel; saved per (user, media_file)
    /// pair so the same correction sticks across replays of the
    /// same source.
    #[serde(default)]
    pub subtitle_offset_ms: i64,
}

#[derive(Debug, Deserialize)]
pub struct QualityTarget {
    pub height: u32,
    pub bitrate_bps: u64,
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
    /// Source-time (ms) the session was started at. For freshly-
    /// created `/sessions` responses this echoes the request's
    /// `start_position_ms`; for prewarmed sessions returned via
    /// `/sessions/prewarm` the resolver picked it from the user's
    /// saved play_state. The client uses this to decide whether a
    /// cached prewarm matches the position it's about to play from
    /// — if the user has scrubbed since the prewarm, the cache miss
    /// is taken and a fresh session is created.
    pub start_position_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,
    /// Transcode sessions only: the height + bitrate the encoder
    /// settled on. Used by the player to display "Auto · 1080p"
    /// once Auto resolves to a concrete tier so users know what
    /// they're getting. Direct-play sessions leave these `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_video_bitrate_bps: Option<u64>,
    /// Native height of the source file. The player uses this to
    /// grey out quality tiers above the source (a 720p file shouldn't
    /// offer 1080p — the scale filter clamps to source anyway, so a
    /// 1080p pick would just burn CPU for the same pixels).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_height: Option<u32>,
    /// Human label for the encoder ffmpeg is running for this
    /// session ("NVIDIA NVENC" / "software (libx264)"). Lets the
    /// player display the encoder in the picker so users (and
    /// support reports) can tell if HW accel is actually engaged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoder: Option<String>,
    /// `copy` / `reencode` per stream — surfaced so the player can
    /// flag the cheap remux path in the same UI spot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_treatment: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_treatment: Option<&'static str>,
}

pub async fn create_session(
    State(state): State<AppState>,
    user: AuthUser,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), ApiError> {
    create_session_impl(&state, &user, &headers, ip, req).await
}

/// Fire-and-forget recorder for the playback `start` event. Failures
/// are logged but never surfaced — stats are observability and must
/// not gate playback. Spawned so the user's POST returns immediately;
/// the insert happens in the background.
///
/// `ip` is the trusted-proxy-resolved effective client IP.
#[allow(clippy::too_many_arguments)]
fn record_start_event(
    state: &AppState,
    user_id: i64,
    media_file_id: i64,
    decision: &'static str,
    container: Option<String>,
    duration_ms: Option<i64>,
    headers: &HeaderMap,
    ip: Option<IpAddr>,
    session_token: Option<String>,
) {
    let pool = state.pool.clone();
    let ip = ip.map(|i| i.to_string());
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    tokio::spawn(async move {
        // Look up which item/episode owns this file so the aggregator
        // can roll episodes up to shows. Both columns can be NULL when
        // the file row has been soft-deleted between session-start and
        // event-record — recording an orphan event is still useful for
        // "who watched at this time" but stops short of a parent join.
        let (item_id, episode_id) =
            chimpflix_library::queries::media_file_owner(&pool, media_file_id)
                .await
                .unwrap_or((None, None));
        let ev = chimpflix_library::queries::PlaybackEventInput {
            item_id,
            episode_id,
            media_file_id: Some(media_file_id),
            duration_ms,
            decision: Some(decision),
            container: container.as_deref(),
            ip: ip.as_deref(),
            user_agent: user_agent.as_deref(),
            session_token: session_token.as_deref(),
            ..chimpflix_library::queries::PlaybackEventInput::new(user_id, "start")
        };
        if let Err(e) = chimpflix_library::queries::record_playback_event(&pool, ev).await {
            tracing::warn!(error = %format!("{e:#}"), "record playback start event");
        }
    });
}

async fn create_session_impl(
    state: &AppState,
    user: &AuthUser,
    headers: &HeaderMap,
    ip: IpAddr,
    mut req: CreateSessionRequest,
) -> Result<(StatusCode, Json<CreateSessionResponse>), ApiError> {
    // Server-wide default for audio normalization. If the operator
    // flipped `audio_normalize_enabled` on, every session gets the
    // loudnorm filter — the player's per-session toggle can still
    // *opt in*, but cannot opt out of an admin-mandated default.
    // (Matching Plex's "Volume leveling" server setting.)
    if state.settings.read().await.audio_normalize_enabled {
        req.audio_normalize = true;
    }
    ensure_file_accessible(state, user, req.media_file_id).await?;
    let locator = queries::get_media_file_locator(&state.pool, req.media_file_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    // Fast-fail on a missing source file. The DB might still have a
    // row for media that's gone from disk (deleted by hand, moved to
    // another drive, drive unmounted) — without this check the
    // player would POST happily, spawn ffmpeg, and the encoder would
    // sit waiting on a path that never opens. We immediately mark
    // the row removed so subsequent requests 404 from the get-go,
    // and the next purge sweep cascades it cleanly. The verify task
    // catches these too, but inline detection at session-start is
    // cheaper than tens of seconds of spinner before the user gives
    // up.
    if tokio::fs::metadata(&locator.path).await.is_err() {
        let _ = queries::mark_media_files_removed(&state.pool, &[req.media_file_id]).await;
        warn!(
            media_file_id = req.media_file_id,
            path = %locator.path,
            "stream request for media_file whose path no longer exists; marked removed"
        );
        return Err(ApiError::NotFound);
    }

    // We need stream codecs to decide between direct/transcode. Pull them
    // from the existing detail helper plumbing.
    let files = sqlx::query(
        "SELECT container, duration_ms, bit_rate, hdr_format, height \
         FROM media_files WHERE id = ?",
    )
    .bind(req.media_file_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .ok_or(ApiError::NotFound)?;

    use sqlx::Row;
    let container: Option<String> = files.try_get("container").ok();
    let duration_ms: Option<i64> = files.try_get("duration_ms").ok();
    let bit_rate: Option<i64> = files.try_get("bit_rate").ok();
    let hdr_format: Option<String> = files
        .try_get::<Option<String>, _>("hdr_format")
        .ok()
        .flatten();
    let source_height: Option<i64> = files.try_get("height").ok();

    let streams =
        sqlx::query("SELECT kind, codec, pix_fmt FROM media_streams WHERE media_file_id = ?")
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
    // Pixel format of the video stream — used to detect 10-bit
    // sources that the browser-side codec advertisement doesn't
    // capture. A `yuv420p10le` source advertised as plain "hevc"
    // would copy through to the browser, which then can't decode
    // Main10 even though it accepts Main. Force re-encode in that
    // case so the user sees H.264 8-bit, always playable.
    let source_video_pix_fmt: Option<String> = streams
        .iter()
        .find(|r| r.try_get::<String, _>("kind").ok().as_deref() == Some("video"))
        .and_then(|r| r.try_get::<Option<String>, _>("pix_fmt").ok().flatten());

    // Subtitle selection always forces transcode — we burn it in.
    //
    // Audio selection only forces transcode if it actually changes what
    // would be played: picking audio index 0 (the file's first audio
    // stream) is a no-op compared to default playback, and the common
    // "preferred-language match was already the default" case shouldn't
    // pay the encode cost. Non-zero indices need transcode because
    // direct play can't remap tracks.
    //
    // A non-default quality_target also forces transcode — direct play
    // serves the source bytes as-is, so you can't ask for 480p without
    // running the encoder.
    let audio_forces_transcode = matches!(req.audio_index, Some(n) if n != 0);
    let needs_transcode = req.subtitle_index.is_some()
        || audio_forces_transcode
        || req.quality_target.is_some()
        // loudnorm has to run inside an ffmpeg filter graph, which is
        // a transcode session by definition. Without this, the request
        // would direct-play and the filter would silently never apply.
        || req.audio_normalize;
    let mode = if needs_transcode {
        PlayMode::Transcode
    } else {
        decide_mode(&stream_pairs, container.as_deref(), bit_rate, &req.client)
    };

    // Once we know we need a transcode session, decide whether the
    // VIDEO stream can be stream-copied (no re-encode). This is a big
    // CPU win for the common "user picked an alt audio track" case on
    // a source that's already h264 — we just remux the original video
    // into TS instead of going through libx264 / NVENC. Re-encoding is
    // still required when any of the following apply:
    //   * Burned-in subtitle (filter rewrites every frame)
    //   * Picked quality tier (scaling = re-encode)
    //   * HDR source (tonemap = re-encode)
    //   * Source video codec not in client's supported_video_codecs
    let source_video_codec: Option<String> = stream_pairs
        .iter()
        .find(|(kind, _)| kind == "video")
        .and_then(|(_, codec)| codec.clone());
    let source_audio_codec_outer: Option<String> = stream_pairs
        .iter()
        .find(|(kind, _)| kind == "audio")
        .and_then(|(_, codec)| codec.clone());
    // Container is picked once up front based on the source codecs +
    // client caps. fMP4 wins when at least one source stream is
    // browser-decodable but TS-incompatible (Opus, FLAC, AV1, VP9);
    // TS wins otherwise. The treatment functions both consult this
    // so the carriable check matches what ffmpeg is actually going
    // to mux into.
    let container_format = pick_container(
        source_video_codec.as_deref(),
        source_audio_codec_outer.as_deref(),
        &req.client,
        req.subtitle_index,
    );
    let mut video_treatment = pick_video_treatment(
        &req,
        source_video_codec.as_deref(),
        hdr_format.as_deref(),
        container_format,
        source_video_pix_fmt.as_deref(),
    );

    // GOP-aware copy fallback. Stream-copy into HLS only works cleanly
    // when the source's keyframes land at least once per HLS segment
    // (default 6 s) — otherwise the muxer either emits oversized
    // segments or stalls until the next IDR, surfacing to the user as
    // multi-second startup hangs and stuttery scrubbing. A quick
    // ffprobe over the first ~12 s of packets tells us which side of
    // the line a file sits on; sparse-GOP sources are silently
    // promoted to re-encode where libx264/NVENC inserts its own
    // keyframes at segment boundaries.
    //
    // We only spend the ~250 ms probe cost when we'd otherwise copy and
    // the session is going through the transcoder. Direct-play
    // sessions don't care about HLS at all, and a session already
    // pinned to Reencode for another reason (subs/HDR/quality) gains
    // nothing from probing.
    if matches!(mode, PlayMode::Transcode) && video_treatment == VideoTreatment::Copy {
        match chimpflix_transcoder::probe_gop(&state.ffmpeg, StdPath::new(&locator.path), 12.0)
            .await
        {
            Ok(gop) => {
                let seg = chimpflix_transcoder::HLS_SEGMENT_DURATION_S as f64;
                if gop.copy_unsafe(seg) {
                    info!(
                        media_file_id = req.media_file_id,
                        median_gop_s = ?gop.median_keyframe_interval_s,
                        keyframes = gop.keyframes_observed,
                        "downgrading copy to reencode: source keyframes too sparse for HLS segments"
                    );
                    video_treatment = VideoTreatment::Reencode;
                }
            }
            Err(e) => {
                // Probe failure (rare; usually a container ffprobe can't
                // enumerate packets for). Stay on Copy — the symptom of
                // a GOP-too-large source is gradual startup pain, not
                // total breakage, and false-promoting every "unknown"
                // file to Reencode would defeat the copy-mode CPU win.
                warn!(
                    media_file_id = req.media_file_id,
                    error = %e,
                    "gop probe failed; staying on copy"
                );
            }
        }
    }

    match mode {
        PlayMode::Direct => {
            record_start_event(
                state,
                user.id,
                req.media_file_id,
                "direct",
                container.clone(),
                duration_ms,
                headers,
                Some(ip),
                None,
            );
            Ok((
                StatusCode::CREATED,
                Json(CreateSessionResponse {
                    session: SessionInfo {
                        id: "direct".into(),
                        mode: "direct",
                        direct_url: Some(format!("/api/v1/stream/{}/direct", req.media_file_id,)),
                        hls_master_url: None,
                        media_file_id: req.media_file_id,
                        start_position_ms: req.start_position_ms,
                        duration_ms,
                        resolved_height: None,
                        resolved_video_bitrate_bps: None,
                        source_height: source_height.map(|h| h as u32),
                        encoder: None,
                        video_treatment: None,
                        audio_treatment: None,
                    },
                }),
            ))
        }
        PlayMode::Transcode => {
            // Enforce the operator's concurrent-transcode cap. The setting
            // is hot-reloaded, so we re-read it each time rather than
            // capturing it at startup. Race window between len() and
            // start() is acceptable — worst case we let in one extra.
            let max_concurrent = state.settings.read().await.transcoder_max_concurrent;
            let current = state.transcoder.list_sessions().len() as i64;
            if current >= max_concurrent {
                return Err(ApiError::TooManyRequests(format!(
                    "transcoder is at capacity ({current}/{max_concurrent} concurrent sessions)"
                )));
            }

            // Per-user remote-stream cap. When the operator set
            // `max_remote_streams_per_user > 0`, requests originating
            // outside `lan_networks` are rate-limited per-user. LAN
            // requests bypass — the point of the cap is to keep one
            // remote user from monopolising the encoder while still
            // letting trusted local clients play freely. Both
            // settings are hot-reloaded.
            let (remote_cap, lan_raw) = {
                let s = state.settings.read().await;
                (s.max_remote_streams_per_user, s.lan_networks.clone())
            };
            if remote_cap > 0 && is_remote_request(ip, &lan_raw) {
                let mine = state
                    .transcoder
                    .list_sessions()
                    .iter()
                    .filter(|s| s.user_id == user.id)
                    .count() as i64;
                if mine >= remote_cap {
                    return Err(ApiError::TooManyRequests(format!(
                        "you have {mine}/{remote_cap} remote streams in flight; \
                         close one before starting another",
                    )));
                }
            }

            // Resolve the codec of the chosen subtitle stream so the
            // transcoder can pick the right filter path (text vs picture
            // burn-in). `subtitle_index` is 0-indexed among subtitle
            // streams.
            //
            // We always run a fresh ffprobe instead of trusting the
            // DB — the DB row was written by whatever scanner ran when
            // the file was first imported, and scanner versions /
            // ffmpeg versions / codec_name aliases drift over time
            // (the same PGS stream is `hdmv_pgs_subtitle` to one
            // version of ffprobe and `pgssub` to another). Trusting a
            // stale name routes PGS to the text-only `subtitles=`
            // filter, which silently fails to produce any overlay.
            // The probe is ~50-100 ms per burn-in session start, only
            // runs when the user explicitly picked a subtitle, and
            // gives ground truth from the file as it stands today.
            // DB value is kept as a fallback if probe itself fails.
            //
            // Language + title come from the same DB row and are
            // forwarded only so the WebVTT sidecar can label its
            // `#EXT-X-MEDIA:NAME=` and `LANGUAGE=` attributes; the
            // burn-in path doesn't use them.
            let (subtitle_codec, subtitle_language, subtitle_title): (
                Option<String>,
                Option<String>,
                Option<String>,
            ) = if let Some(si) = req.subtitle_index {
                let row = sqlx::query(
                    "SELECT codec, language, title FROM media_streams
                     WHERE media_file_id = ? AND kind = 'subtitle'
                     ORDER BY stream_index ASC
                     LIMIT 1 OFFSET ?",
                )
                .bind(req.media_file_id)
                .bind(si as i64)
                .fetch_optional(&state.pool)
                .await
                .map_err(|e| ApiError::Internal(e.into()))?;
                let from_db: Option<String> = row
                    .as_ref()
                    .and_then(|r| r.try_get::<Option<String>, _>("codec").ok())
                    .flatten();
                let lang: Option<String> = row
                    .as_ref()
                    .and_then(|r| r.try_get::<Option<String>, _>("language").ok())
                    .flatten();
                let title: Option<String> = row
                    .as_ref()
                    .and_then(|r| r.try_get::<Option<String>, _>("title").ok())
                    .flatten();
                let probed = match chimpflix_transcoder::probe_subtitle_codec(
                    &state.ffmpeg,
                    StdPath::new(&locator.path),
                    si,
                )
                .await
                {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(
                            error = %e,
                            media_file_id = req.media_file_id,
                            subtitle_index = si,
                            "subtitle codec probe failed; falling back to DB value",
                        );
                        None
                    }
                };
                let resolved = probed.or(from_db.clone());
                info!(
                    media_file_id = req.media_file_id,
                    subtitle_index = si,
                    db = ?from_db,
                    resolved = ?resolved,
                    language = ?lang,
                    title = ?title,
                    "subtitle codec resolved"
                );
                (resolved, lang, title)
            } else {
                (None, None, None)
            };

            // Same lookup for the audio stream we'll actually serve,
            // plus channel count so we can size the AAC re-encode
            // bitrate. Default to index 0 when the caller didn't
            // pick one so the codec-copy heuristic still applies
            // for the "transcode is forced by quality/subs but
            // audio is fine as-is" case.
            let audio_index_for_lookup = req.audio_index.unwrap_or(0);
            let audio_row = sqlx::query(
                "SELECT codec, channels FROM media_streams
                 WHERE media_file_id = ? AND kind = 'audio'
                 ORDER BY stream_index ASC
                 LIMIT 1 OFFSET ?",
            )
            .bind(req.media_file_id)
            .bind(audio_index_for_lookup as i64)
            .fetch_optional(&state.pool)
            .await
            .map_err(|e| ApiError::Internal(e.into()))?;
            let audio_codec: Option<String> = audio_row
                .as_ref()
                .and_then(|r| r.try_get::<Option<String>, _>("codec").ok())
                .flatten();
            let audio_channels: Option<i32> = audio_row
                .as_ref()
                .and_then(|r| r.try_get::<Option<i32>, _>("channels").ok())
                .flatten();
            let audio_treatment =
                pick_audio_treatment(&req, audio_codec.as_deref(), container_format);
            let audio_bitrate_bps = pick_audio_bitrate(audio_channels);

            // Re-read the hw_accel choice on every session create so a
            // settings change takes effect on the next playback without
            // a restart. Auto / unknown / unavailable all fall back to
            // libx264 inside HwAccel::resolve.
            let settings_snapshot = state.settings.read().await.clone();
            let hwaccel = chimpflix_transcoder::HwAccel::resolve(
                &settings_snapshot.transcoder_hw_accel,
                &state.transcoder_caps,
            );
            // Per-encoder-type cap: software encodes (libx264 / libx265)
            // peg N CPU cores each. The overall `transcoder_max_concurrent`
            // already ran; this second gate kicks in only when the
            // session would land on the software path. Stops a wave of
            // fallback-to-software encodes from starving a parallel
            // GPU session.
            if matches!(hwaccel, chimpflix_transcoder::HwAccel::None) {
                let max_cpu_concurrent = settings_snapshot.transcoder_max_cpu_concurrent;
                let current_cpu = state
                    .transcoder
                    .list_sessions()
                    .iter()
                    .filter(|s| s.encoder.starts_with("software"))
                    .count() as i64;
                if current_cpu >= max_cpu_concurrent {
                    return Err(ApiError::TooManyRequests(format!(
                        "software (CPU) transcode capacity reached \
                         ({current_cpu}/{max_cpu_concurrent}) — wait for a slot or enable a hardware encoder"
                    )));
                }
            }
            // Operator's speed-vs-quality dial. Default 'balanced'
            // reproduces pre-Phase-18 behavior; 'speed' shaves CPU on
            // overloaded boxes, 'quality' burns more cycles for less
            // visible compression on detail-heavy sources.
            let encoder_preset = chimpflix_transcoder::EncoderPreset::resolve(
                &settings_snapshot.transcoder_encoder_preset,
            );

            // Hardware-strictness gate. In `require_hw` mode we refuse
            // sessions that can't run end-to-end on hardware — the
            // operator picked that mode because they want guaranteed
            // GPU usage, not a CPU bailout for a niche source codec.
            // The breakdown surfaces the exact reason so the player
            // overlay can tell the user what failed (e.g. "your GPU
            // doesn't decode AV1; transcode would require CPU
            // decode, which this server is configured to reject").
            //
            // `prefer_hw` mode logs the same breakdown at warn level
            // but lets the session continue — useful for the
            // operator to spot misconfigurations without breaking
            // playback for users.
            let strictness = settings_snapshot.transcoder_hw_strictness.as_str();
            if matches!(strictness, "require_hw" | "prefer_hw") {
                let breakdown = assess_hw_coverage(
                    hwaccel,
                    &state.transcoder_caps,
                    source_video_codec.as_deref(),
                    video_treatment,
                    req.subtitle_index.is_some(),
                    hdr_format.as_deref(),
                );
                if !breakdown.fully_hw {
                    if strictness == "require_hw" {
                        return Err(ApiError::Conflict(format!(
                            "playback requires software fallback ({}); server is set to require_hw",
                            breakdown.reasons.join(", ")
                        )));
                    } else {
                        warn!(
                            media_file_id = req.media_file_id,
                            reasons = ?breakdown.reasons,
                            "prefer_hw: session falling back to software for at least one stage"
                        );
                    }
                }
            }

            // Resolve the final (height, bitrate) the encoder will
            // target. Precedence: explicit quality_target → smart
            // default driven by source resolution → transcoder's
            // built-in 720p baseline. The operator's quality ceiling
            // (kbps) is applied last so it always wins.
            let resolved_quality = req
                .quality_target
                .as_ref()
                .map(|q| (q.height, q.bitrate_bps))
                .or_else(|| auto_quality_for_source(source_height.unwrap_or(0)));
            let resolved_quality = resolved_quality.map(|(h, bps)| {
                let bps = match settings_snapshot.transcoder_quality_ceiling_kbps {
                    Some(ceiling_kbps) if ceiling_kbps > 0 => bps.min((ceiling_kbps as u64) * 1000),
                    _ => bps,
                };
                (h, bps)
            });

            // ABR: pick a sensible fallback tier below the resolved
            // primary so HLS.js can step down on bandwidth pressure
            // without our `bandwidth-aware downgrade` round-tripping
            // a full session restart. Gating:
            //   * Only when the primary tier is high enough that a
            //     fallback actually buys something (≥720p; 480p sources
            //     have nothing useful to downgrade to).
            //   * Skipped for subtitle-BURN sessions only (the encoder
            //     branch's filter graph doesn't compose with `split`
            //     yet — the transcoder also enforces this defensively).
            //     Sidecar/text subtitles don't touch the video graph
            //     so they compose fine with the ABR split — important
            //     for mobile clients where cellular bandwidth makes
            //     the fallback variant the difference between smooth
            //     playback and a wedged buffer.
            //   * Skipped for explicit quality_target requests (user
            //     pinned a tier; respect that without auto-secondary).
            let subtitle_would_burn = req.subtitle_index.is_some()
                && !subtitle_codec
                    .as_deref()
                    .map(chimpflix_transcoder::is_text_subtitle_codec)
                    .unwrap_or(false);
            let fallback_variant = resolved_quality.and_then(|(h, _)| {
                if subtitle_would_burn || req.quality_target.is_some() {
                    return None;
                }
                fallback_for_primary(h)
            });

            // Try to adopt an existing compatible session before
            // spinning up a fresh ffmpeg. This catches the "user
            // refreshed the watch page" case — the play-state read
            // brings them back to (roughly) where they were, all the
            // session params match, and the existing session's HLS
            // buffer has segments at that position already. Saves an
            // ffmpeg startup + the 15 s manifest-wait the player
            // would otherwise sit through.
            //
            // Defaults for the lookup match the transcoder's own
            // defaults so a not-yet-resolved quality_target hits the
            // same key the session was registered with.
            let (lookup_height, lookup_bitrate) = resolved_quality.unwrap_or((720, 2_500_000));
            if let Some(existing) = state.transcoder.find_compatible(
                user.id,
                req.media_file_id,
                req.start_position_ms,
                req.audio_index,
                req.subtitle_index,
                lookup_height,
                lookup_bitrate,
                req.audio_normalize,
                fallback_variant,
            ) {
                // Resume the encoder if the previous client left it
                // paused — otherwise the adopted session would sit
                // waiting for SIGCONT and the new client would never
                // see new segments.
                let _ = existing.resume();
                existing.touch();
                let master_url = format!("/api/v1/stream/sessions/{}/master.m3u8", existing.id);
                let video_treatment_str = match existing.video_treatment {
                    chimpflix_transcoder::VideoTreatment::Copy => "copy",
                    chimpflix_transcoder::VideoTreatment::Reencode => "reencode",
                };
                let audio_treatment_str = match existing.audio_treatment {
                    chimpflix_transcoder::AudioTreatment::Copy => "copy",
                    chimpflix_transcoder::AudioTreatment::Reencode => "reencode",
                };
                return Ok((
                    StatusCode::OK,
                    Json(CreateSessionResponse {
                        session: SessionInfo {
                            id: existing.id.clone(),
                            mode: "transcode",
                            direct_url: None,
                            hls_master_url: Some(master_url),
                            media_file_id: existing.media_file_id,
                            start_position_ms: existing.start_position_ms,
                            duration_ms: existing.duration_ms,
                            resolved_height: Some(existing.target_height),
                            resolved_video_bitrate_bps: Some(existing.target_video_bitrate_bps),
                            source_height: existing.source_height,
                            encoder: Some(existing.hwaccel.label().to_string()),
                            video_treatment: Some(video_treatment_str),
                            audio_treatment: Some(audio_treatment_str),
                        },
                    }),
                ));
            }

            let session = state
                .transcoder
                .start(
                    req.media_file_id,
                    StdPath::new(&locator.path),
                    req.start_position_ms,
                    duration_ms,
                    user.id,
                    req.audio_index,
                    req.subtitle_index,
                    subtitle_codec.as_deref(),
                    subtitle_language.as_deref(),
                    subtitle_title.as_deref(),
                    req.subtitle_offset_ms,
                    hdr_format.as_deref(),
                    req.subtitle_style.as_deref(),
                    resolved_quality,
                    hwaccel,
                    encoder_preset,
                    match video_treatment {
                        VideoTreatment::Copy => chimpflix_transcoder::VideoTreatment::Copy,
                        VideoTreatment::Reencode => chimpflix_transcoder::VideoTreatment::Reencode,
                    },
                    match audio_treatment {
                        AudioTreatment::Copy => chimpflix_transcoder::AudioTreatment::Copy,
                        AudioTreatment::Reencode => chimpflix_transcoder::AudioTreatment::Reencode,
                    },
                    audio_bitrate_bps,
                    req.audio_normalize,
                    source_height.map(|h| h as u32),
                    fallback_variant,
                    container_format,
                    source_video_codec.as_deref(),
                    audio_codec.as_deref(),
                    source_video_pix_fmt.as_deref(),
                    chimpflix_transcoder::TonemapConfig {
                        enabled: settings_snapshot.transcoder_hdr_tonemap_enabled,
                        algorithm: settings_snapshot.transcoder_hdr_tonemap_algo.clone(),
                    },
                    pick_target_codec(
                        &settings_snapshot.transcoder_hevc_encoding_mode,
                        client_supports_codec(&req.client, "hevc"),
                        hwaccel,
                        &state.transcoder.capabilities(),
                    ),
                    settings_snapshot.transcoder_gpu_device.as_str(),
                    // Two-pass loudnorm when the analyze_loudness
                    // task has stored measurements; falls back to
                    // single-pass when absent. Always looked up
                    // (even when normalize is off) because the call
                    // is cheap (1 row) and centralising the decision
                    // here keeps Session::start dumb.
                    if req.audio_normalize {
                        queries::get_loudness_measurement(&state.pool, req.media_file_id)
                            .await
                            .ok()
                            .flatten()
                            .map(|m| chimpflix_transcoder::LoudnessTarget {
                                measured_i: m.integrated,
                                measured_tp: m.true_peak,
                                measured_lra: m.lra,
                                measured_thresh: m.threshold,
                            })
                    } else {
                        None
                    },
                )
                .await
                .map_err(ApiError::Internal)?;
            record_start_event(
                state,
                user.id,
                req.media_file_id,
                "transcode",
                container.clone(),
                duration_ms,
                headers,
                Some(ip),
                Some(session.id.clone()),
            );
            let master_url = format!("/api/v1/stream/sessions/{}/master.m3u8", session.id);
            // Mirror the transcoder's outward-facing per-session
            // labels back to the response so the player can display
            // them. `resolved_quality` is what the transcoder
            // actually used (after clamps + ceiling), not what the
            // client requested.
            let (resolved_height, resolved_video_bitrate_bps) = match resolved_quality {
                Some((h, bps)) => (Some(h), Some(bps)),
                None => (None, None),
            };
            Ok((
                StatusCode::CREATED,
                Json(CreateSessionResponse {
                    session: SessionInfo {
                        id: session.id.clone(),
                        mode: "transcode",
                        direct_url: None,
                        hls_master_url: Some(master_url),
                        media_file_id: req.media_file_id,
                        start_position_ms: req.start_position_ms,
                        duration_ms,
                        resolved_height,
                        resolved_video_bitrate_bps,
                        source_height: source_height.map(|h| h as u32),
                        encoder: Some(hwaccel.label().to_string()),
                        video_treatment: Some(match video_treatment {
                            VideoTreatment::Copy => "copy",
                            VideoTreatment::Reencode => "reencode",
                        }),
                        audio_treatment: Some(match audio_treatment {
                            AudioTreatment::Copy => "copy",
                            AudioTreatment::Reencode => "reencode",
                        }),
                    },
                }),
            ))
        }
    }
}

/// Confirm the caller owns this session (or is admin/owner). Returns
/// 404 (NOT 403) on mismatch so we don't leak whether the session id
/// exists. Critical: without this, any authenticated user could
/// terminate, pause, resume, or stream HLS segments from any other
/// user's session by guessing the session id — the audit's #1 IDOR.
fn ensure_session_accessible(
    state: &AppState,
    user: &AuthUser,
    session_id: &str,
) -> Result<std::sync::Arc<chimpflix_transcoder::Session>, ApiError> {
    let session = state.transcoder.get(session_id).ok_or(ApiError::NotFound)?;
    if session.user_id != user.id && !user.role.is_admin_or_owner() {
        return Err(ApiError::NotFound);
    }
    Ok(session)
}

pub async fn delete_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    // Verify ownership before destroying. Use the get-then-check
    // pattern rather than delete_returning so we can refuse to
    // operate on someone else's session.
    if let Some(session) = state.transcoder.get(&id) {
        if session.user_id != user.id && !user.role.is_admin_or_owner() {
            // 404 instead of 403: don't reveal whether the id exists.
            return Err(ApiError::NotFound);
        }
    } else {
        // Idempotent on already-gone sessions.
        return Ok(StatusCode::NO_CONTENT);
    }

    // `delete_returning` snapshots the session before destroying it
    // so we can emit a stop event with the final bandwidth count.
    // Reaper-driven closes go through the same `emit_session_stop_event`
    // helper via the hook registered in main.rs, so both paths agree.
    if let Some(snap) = state.transcoder.delete_returning(&id).await {
        let pool = state.pool.clone();
        tokio::spawn(async move {
            crate::emit_session_stop_event(&pool, &snap).await;
        });
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Player calls this on the HTML5 `pause` event so the ffmpeg child
/// stops burning CPU/GPU while the user isn't watching. The
/// transcoder sends SIGSTOP; the next [`resume_session`] sends
/// SIGCONT. Returns 404 for missing OR non-owned sessions — the same
/// shape masks the existence check.
pub async fn pause_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let session = ensure_session_accessible(&state, &user, &id)?;
    let _ = session.pause();
    Ok(StatusCode::NO_CONTENT)
}

/// Pair of [`pause_session`]. Player calls this on `play` (and on
/// initial mount, defensively — a hand-off from a prewarmed session
/// shouldn't leave the encoder stopped if the player UI didn't
/// observe an explicit pause first).
pub async fn resume_session(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let session = ensure_session_accessible(&state, &user, &id)?;
    let _ = session.resume();
    Ok(StatusCode::NO_CONTENT)
}

pub async fn master_playlist(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let session = ensure_session_accessible(&state, &user, &id)?;
    session.touch();
    let body = session.master_playlist();
    session.add_bytes_served(body.len() as u64);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/vnd.apple.mpegurl")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from(body))
        .map_err(|e| ApiError::Internal(anyhow::Error::from(e)))
}

pub async fn variant_file(
    State(state): State<AppState>,
    user: AuthUser,
    Path((id, variant, name)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let session = ensure_session_accessible(&state, &user, &id)?;
    session.touch();

    // Whitelist allowed filenames. fMP4 sessions add `.m4s` segments
    // and a single `init.mp4` init segment; TS sessions stay on
    // `.ts`. The `sub/` variant (WebVTT sidecar) additionally
    // serves a single `sub.vtt` next to its `index.m3u8` subtitle
    // media playlist.
    let is_manifest = name == "index.m3u8";
    let is_ts_segment = name.starts_with("seg-") && name.ends_with(".ts");
    let is_m4s_segment = name.starts_with("seg-") && name.ends_with(".m4s");
    let is_init = name == "init.mp4";
    let is_vtt = name == "sub.vtt";
    if !is_manifest && !is_ts_segment && !is_m4s_segment && !is_init && !is_vtt {
        return Err(ApiError::NotFound);
    }
    if !session.is_known_variant(&variant) {
        return Err(ApiError::NotFound);
    }

    // For the WebVTT sidecar variant, wait for the background
    // extraction task to finish before serving any file. start()
    // returns immediately and kicks the extraction off so the
    // initial /sessions HTTP request doesn't time out on slow
    // sources (a 30 GB Bluray remux takes minutes to scan for
    // every subtitle packet). The player picks subs up the
    // moment this request returns — even a several-minute wait
    // here doesn't block the video, which is already streaming
    // from /v1/*.
    if variant == "sub" {
        if let Some(sidecar) = session.webvtt_sidecar.as_ref() {
            // The previous 5-minute timeout outlasted every browser's
            // own network timeout — Safari and Android Chrome bail
            // around 30-60s of inactivity, and the user sees a
            // network-error toast while the server is happily still
            // extracting. Pin the wait to 60s: it still covers the
            // common case (a few-GB MKV gets its subs scanned in
            // <30s) but matches mobile network reality. Pathologically
            // slow sources (full Bluray remux end-to-end scan) will
            // serve a 404 here; the player drops captions and the
            // background task continues populating the cache for the
            // next request to pick up.
            let timeout = Duration::from_secs(60);
            let mut rx = (*sidecar.progress).clone();
            let wait = async {
                loop {
                    let status = rx.borrow_and_update().clone();
                    match status {
                        chimpflix_transcoder::SubExtractionStatus::Ready => return Ok(()),
                        chimpflix_transcoder::SubExtractionStatus::Failed(e) => {
                            return Err(e);
                        }
                        chimpflix_transcoder::SubExtractionStatus::Pending => {}
                    }
                    if rx.changed().await.is_err() {
                        return Err("watch channel closed".to_string());
                    }
                }
            };
            match tokio::time::timeout(timeout, wait).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    warn!(
                        session_id = %id,
                        reason = %e,
                        "subtitle extraction failed; serving 404 for sidecar"
                    );
                    return Err(ApiError::NotFound);
                }
                Err(_) => {
                    warn!(
                        session_id = %id,
                        "subtitle extraction still running after 60s timeout; serving 404"
                    );
                    return Err(ApiError::NotFound);
                }
            }
        }
    }

    let path = session.output_dir.join(&variant).join(&name);

    // For the variant manifest, give ffmpeg a short window to write it
    // out on the first request after session start — and wait until it
    // has non-zero content (the file appears empty while the writer is
    // still streaming heredoc / initial bytes). Segments use atomic
    // rename via `temp_file`, so they don't need the same grace; the
    // player retries 404s naturally.
    //
    // 30s (was 15s): heavily-loaded boxes and ffmpeg HEVC startup
    // (filter-graph compilation + GPU init) can take >10s, and the
    // previous 15s budget was racing with the idle reaper's 15s
    // interval — first-manifest fetches on slow mobile networks
    // would lose that race and 404 even though the encoder was
    // bootstrapping fine. The client's HLS.js
    // `manifestLoadingTimeOut` was bumped to 35s in lockstep so it
    // doesn't give up before this returns.
    if is_manifest {
        let deadline = Instant::now() + Duration::from_secs(30);
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
    // Bandwidth metering: every segment + playlist GET adds to the
    // per-session counter. Flushed to playback_events at session close
    // — segment-grained DB writes would dominate the actual segment
    // serving cost, which is the whole point of HLS.
    session.add_bytes_served(bytes.len() as u64);
    let content_type = if is_manifest {
        "application/vnd.apple.mpegurl"
    } else if is_init {
        // Init segment for fMP4 — both ffmpeg and HLS.js expect MP4.
        "video/mp4"
    } else if is_m4s_segment {
        // fMP4 media segment.
        "video/iso.segment"
    } else {
        // TS segment.
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

/// What to do with the VIDEO stream inside a transcode session. `Copy`
/// just remuxes the source bytes into the HLS container — no encoder,
/// near-instant session start, ~90% lower CPU than re-encoding. Only
/// safe when the source codec is already client-compatible and nothing
/// in the request asks us to modify frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoTreatment {
    Copy,
    Reencode,
}

/// Same pattern as [`VideoTreatment`] but for the audio stream. `Copy`
/// remuxes source audio packets straight into the HLS container,
/// saving the AAC re-encode. Picked when the source codec is already
/// in the client's supported list (typically AAC); falls back to
/// `Reencode` to AAC for anything else (AC3, FLAC, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTreatment {
    Copy,
    Reencode,
}

/// Target AAC bitrate (bps) for re-encoded audio. We always downmix
/// to stereo (`-ac 2`), so the bitrate is sized for stereo content
/// regardless of source layout — but we bump it up a notch when the
/// source has more channels so the downmix retains more detail from
/// the surround mix.
pub fn pick_audio_bitrate(source_channels: Option<i32>) -> u64 {
    match source_channels {
        Some(1) => 96_000,            // mono → stereo (duplicate); 96k plenty.
        Some(n) if n >= 6 => 256_000, // 5.1/7.1 downmix; preserve nuance.
        _ => 192_000,                 // typical stereo, AAC LC sweet spot.
    }
}

/// Whether the chosen audio stream's codec is already client-playable.
/// We only ever serve TS segments today, so we additionally require
/// the codec to be friendly to that container (AAC is the main case;
/// AC3 also rides in TS but most browsers can't decode it natively).
///
/// `audio_normalize` requests a `loudnorm` filter, which is a filter
/// (not a stream-copy operation), so we force `Reencode` whenever
/// the caller has asked for it.
pub fn pick_audio_treatment(
    req: &CreateSessionRequest,
    chosen_audio_codec: Option<&str>,
    container: chimpflix_transcoder::ContainerFormat,
) -> AudioTreatment {
    if req.audio_normalize {
        return AudioTreatment::Reencode;
    }
    // Subtitle burn-in forces audio re-encode so the transcoder can
    // add `asetpts=PTS-STARTPTS` to the audio filter chain. That
    // filter pairs with the video-side `setpts=PTS-STARTPTS` to keep
    // A/V sync when we're using `-copyts` to make `subtitles=` /
    // overlay see the original PTS (required for subtitle alignment
    // after a non-zero `-ss` seek). Copy-mode audio can't carry an
    // asetpts filter, so it would end up shifted by the seek amount
    // relative to the reset video — audible echo / drift.
    if req.subtitle_index.is_some() {
        return AudioTreatment::Reencode;
    }
    let Some(codec) = chosen_audio_codec else {
        return AudioTreatment::Reencode;
    };
    let codec_norm = normalize_audio_codec(codec);
    // Container-aware guard. TS-only carries AAC/MP3/AC-3/E-AC-3;
    // fMP4 also carries Opus + FLAC. The check matches the chosen
    // container so a session that's already on fMP4 (because video
    // needed it) gets to keep its audio copy fast-path for Opus/
    // FLAC too. Without this, a WebRip with HEVC video + Opus
    // audio would HEVC-copy but Opus-re-encode, wasting cycles.
    if !audio_carriable(&codec_norm, container) {
        return AudioTreatment::Reencode;
    }
    let compatible = req
        .client
        .supported_audio_codecs
        .iter()
        .any(|c| normalize_audio_codec(c) == codec_norm);
    if compatible {
        AudioTreatment::Copy
    } else {
        AudioTreatment::Reencode
    }
}

/// Audio analog of [`video_carriable`]. fMP4 nominally carries
/// Opus + FLAC too, and browsers can decode them when muxed
/// correctly — but ffmpeg 5.1's mp4 muxer (the version Debian
/// bookworm ships) emits "track 1: codec frame size is not set"
/// warnings when copying Opus / FLAC packets, and the resulting
/// fMP4 segments have incorrect duration metadata that breaks
/// HLS playback. Keeping those two off the carriable list
/// forces them to re-encode to AAC, which the mp4 muxer handles
/// cleanly. AC-3 / E-AC-3 are unaffected by the same bug.
fn audio_carriable(
    normalized_codec: &str,
    container: chimpflix_transcoder::ContainerFormat,
) -> bool {
    use chimpflix_transcoder::ContainerFormat;
    match container {
        ContainerFormat::Ts => matches!(normalized_codec, "aac" | "mp3" | "ac3" | "eac3"),
        ContainerFormat::Fmp4 => matches!(normalized_codec, "aac" | "mp3" | "ac3" | "eac3"),
    }
}

/// Pick the HLS container format for the session. Default is TS
/// (cheaper segments, fewer moving parts, the historical default).
/// Bumps to fMP4 when at least one source stream is browser-
/// decodable but not TS-carriable — that's the scenario where fMP4
/// unlocks the copy fast-path that TS would force into a re-encode.
///
/// Subtitle burn forces TS for now (the burn pipeline lands on
/// h264 + aac anyway and TS handles those cleanly).
/// Per-stage assessment of whether the planned ffmpeg pipeline can
/// run end-to-end on hardware. Used by the `hw_strictness` gate to
/// decide whether to refuse / warn / silently proceed when a stage
/// would fall back to software.
///
/// Three stages get checked:
///   1. **Decode** — does the selected encoder have a paired
///      hwaccel decoder AND does the runtime probe confirm it can
///      decode this source codec?
///   2. **Filter** — does any planned filter require CPU frames?
///      (libass subtitle burn; zscale tonemap; CPU `scale`.) The
///      GPU-native pipeline lights up only when no CPU-only filter
///      is in play.
///   3. **Encode** — `None`/`Reencode` software always-counts-as-
///      software; everything else is HW.
pub struct HwCoverage {
    pub fully_hw: bool,
    pub reasons: Vec<String>,
}

pub fn assess_hw_coverage(
    hwaccel: chimpflix_transcoder::HwAccel,
    caps: &chimpflix_transcoder::TranscoderCapabilities,
    source_video_codec: Option<&str>,
    video_treatment: VideoTreatment,
    has_subtitle_burn: bool,
    hdr_format: Option<&str>,
) -> HwCoverage {
    use chimpflix_transcoder::HwAccel;
    let mut reasons = Vec::new();

    // Stage 1: decode. Copy sessions don't decode (just demux), so
    // they pass the decode check unconditionally.
    if matches!(video_treatment, VideoTreatment::Reencode) {
        let decoder_ok = match hwaccel.paired_decoder() {
            Some(name) => source_video_codec
                .is_some_and(|c| caps.decoders.supports(name, &normalize_video_codec(c))),
            None => false,
        };
        if !decoder_ok {
            reasons.push(match source_video_codec {
                Some(c) => format!("decode: {c} not in {} HW decoder list", hwaccel.label()),
                None => "decode: source codec unknown".to_string(),
            });
        }
    }

    // Stage 2: filters that need CPU frames.
    if has_subtitle_burn {
        reasons.push("filter: subtitle burn-in runs on CPU (libass)".to_string());
    }
    if matches!(hdr_format, Some("hdr10" | "hlg" | "dovi")) {
        reasons.push("filter: HDR→SDR tonemap runs on CPU (zscale)".to_string());
    }

    // Stage 3: encode. Software encoder (libx264) is, well, software.
    if matches!(hwaccel, HwAccel::None) {
        reasons.push("encode: software libx264".to_string());
    }

    HwCoverage {
        fully_hw: reasons.is_empty(),
        reasons,
    }
}

pub fn pick_container(
    source_video_codec: Option<&str>,
    source_audio_codec: Option<&str>,
    client: &ClientCapabilities,
    subtitle_index: Option<u32>,
) -> chimpflix_transcoder::ContainerFormat {
    use chimpflix_transcoder::ContainerFormat;
    if subtitle_index.is_some() {
        return ContainerFormat::Ts;
    }
    let vc = source_video_codec.map(normalize_video_codec);
    let ac = source_audio_codec.map(normalize_audio_codec);
    let browser_supports_video = vc.as_deref().is_some_and(|c| {
        client
            .supported_video_codecs
            .iter()
            .any(|x| normalize_video_codec(x) == c)
    });
    let browser_supports_audio = ac.as_deref().is_some_and(|c| {
        client
            .supported_audio_codecs
            .iter()
            .any(|x| normalize_audio_codec(x) == c)
    });
    let video_needs_fmp4 = vc.as_deref().is_some_and(|c| {
        !video_carriable(c, ContainerFormat::Ts) && video_carriable(c, ContainerFormat::Fmp4)
    }) && browser_supports_video;
    let audio_needs_fmp4 = ac.as_deref().is_some_and(|c| {
        !audio_carriable(c, ContainerFormat::Ts) && audio_carriable(c, ContainerFormat::Fmp4)
    }) && browser_supports_audio;
    if video_needs_fmp4 || audio_needs_fmp4 {
        ContainerFormat::Fmp4
    } else {
        ContainerFormat::Ts
    }
}

/// Pick (height, bitrate) when the caller didn't specify a quality
/// tier. Goal: don't gratuitously downscale (4K source shouldn't
/// land at 720p just because that was the transcoder's old default),
/// but cap at 1080p so the encoder isn't asked to do real-time 4K
/// work on a CPU that probably can't keep up. Returns `None` when
/// source height is unknown — caller falls back to transcoder
/// defaults (720p).
pub fn auto_quality_for_source(source_height: i64) -> Option<(u32, u64)> {
    let h = source_height as u32;
    if h == 0 {
        return None;
    }
    // Bitrate targets calibrated for h264 streaming — comfortable
    // floor for each tier; the encoder caps via maxrate/bufsize so
    // bursts stay bounded.
    if h >= 1080 {
        Some((1080, 5_000_000))
    } else if h >= 720 {
        Some((720, 2_500_000))
    } else if h >= 480 {
        Some((480, 1_200_000))
    } else {
        Some((h.max(240), 800_000))
    }
}

/// One-tier-down companion to [`auto_quality_for_source`], used by
/// the ABR path to pick a fallback variant that's meaningfully
/// smaller than the primary. Returns `None` when there's nothing
/// useful below (a 480p primary doesn't benefit from a 240p
/// fallback — the bitrate is already low enough that even bad
/// links keep up).
pub fn fallback_for_primary(primary_height: u32) -> Option<(u32, u64)> {
    if primary_height >= 1080 {
        Some((720, 2_500_000))
    } else if primary_height >= 720 {
        Some((480, 1_200_000))
    } else {
        None
    }
}

/// Decide between stream-copying and re-encoding the video stream for
/// a session that has already been determined to need transcoding.
/// Returns `Reencode` whenever any of these are true:
///   * Burned-in subtitle requested (filter rewrites every frame)
///   * Explicit quality tier (scaling is a re-encode)
///   * HDR source (tonemap is a re-encode)
///   * Source codec absent or not in the client's supported list
pub fn pick_video_treatment(
    req: &CreateSessionRequest,
    source_video_codec: Option<&str>,
    hdr_format: Option<&str>,
    container: chimpflix_transcoder::ContainerFormat,
    source_pix_fmt: Option<&str>,
) -> VideoTreatment {
    if req.subtitle_index.is_some() {
        return VideoTreatment::Reencode;
    }
    if req.quality_target.is_some() {
        return VideoTreatment::Reencode;
    }
    if matches!(hdr_format, Some("hdr10" | "hlg" | "dovi")) {
        return VideoTreatment::Reencode;
    }
    let Some(src) = source_video_codec else {
        return VideoTreatment::Reencode;
    };
    let src_norm = normalize_video_codec(src);
    // Container-aware guard: the copy fast path muxes source video
    // packets into the chosen HLS container (TS or fMP4). Codecs
    // outside the container's carriable set would either fail to mux
    // or play back broken. Anything not carriable forces re-encode
    // to h264 (the universal target).
    if !video_carriable(&src_norm, container) {
        return VideoTreatment::Reencode;
    }
    // 10-bit pixel format guard. Browsers that claim HEVC support
    // typically only decode Main profile (8-bit) — Main10 fails
    // silently in MSE even though `isTypeSupported('hev1.1.6.L93.B0')`
    // (an 8-bit-Main probe) returns true. Same trap for AV1 / VP9
    // 10-bit. Without this check, copying a 10-bit source through
    // to a Main-only decoder hangs the player. The browser-cap
    // advertisement is per-codec, not per-profile, so the only
    // reliable signal is the source's pix_fmt.
    if source_pix_fmt.is_some_and(is_10bit_pix_fmt) {
        return VideoTreatment::Reencode;
    }
    let compatible = req
        .client
        .supported_video_codecs
        .iter()
        .any(|c| normalize_video_codec(c) == src_norm);
    if compatible {
        VideoTreatment::Copy
    } else {
        VideoTreatment::Reencode
    }
}

/// True when an ffprobe pix_fmt string indicates the source stores
/// more than 8 bits per channel. Common examples: `yuv420p10le` (HEVC
/// Main10, AV1 10-bit), `yuv420p12le` (HEVC Main12), `yuv444p10le`
/// (rare high-quality masters). The substring check catches all of
/// them; planar-vs-packed format details don't matter for the
/// decision — what matters is that the browser's 8-bit-only HEVC /
/// VP9 / AV1 decoders will reject the bitstream.
fn is_10bit_pix_fmt(pix_fmt: &str) -> bool {
    let lower = pix_fmt.to_ascii_lowercase();
    lower.contains("10le")
        || lower.contains("10be")
        || lower.contains("12le")
        || lower.contains("12be")
        || lower == "p010le"
        || lower == "p010be"
}

/// True when a video codec can be muxed into the given HLS
/// container *and* reliably decoded by browsers via that
/// container.
///
/// TS is intentionally narrow — only H.264. The TS spec
/// allows HEVC, but HLS.js's TS-to-fMP4 transmuxer (the layer
/// between TS segments and MSE) has unreliable HEVC support, and
/// Safari's native HEVC support is happy with fMP4 too. Keeping
/// HEVC out of the TS list forces it onto the fMP4 path which
/// works correctly everywhere.
///
/// fMP4 is the modern HLS container — Apple's own deployments
/// have used it for HEVC since 2017. It carries everything browsers
/// can decode (h264 / hevc / av1 / vp9), plus the wider audio set
/// (opus / flac) in [`audio_carriable`].
fn video_carriable(
    normalized_codec: &str,
    container: chimpflix_transcoder::ContainerFormat,
) -> bool {
    use chimpflix_transcoder::ContainerFormat;
    match container {
        ContainerFormat::Ts => matches!(normalized_codec, "h264"),
        ContainerFormat::Fmp4 => {
            matches!(normalized_codec, "h264" | "hevc" | "av1" | "vp9")
        }
    }
}

/// Map the codec-name variants ffprobe and browsers emit to a single
/// canonical lowercase form so equality comparisons across the
/// boundary actually work. Examples:
///
///   * ffprobe writes `hevc`; some older tooling and the codec
///     string browsers report (`hev1.x.x.x.x`) parse to `h265`.
///   * `e-ac-3`, `eac3`, `ec-3`, `ec3` all refer to Dolby Digital
///     Plus (currently only used in the audio path but kept
///     symmetric).
///
/// Anything not in the table passes through lowercased — unknown
/// codecs fail the `_carriable` check above, which is the safe
/// fallback.
fn normalize_video_codec(codec: &str) -> String {
    match codec.trim().to_ascii_lowercase().as_str() {
        "h265" | "x265" => "hevc".to_string(),
        "x264" => "h264".to_string(),
        "vp09" => "vp9".to_string(),
        "av01" => "av1".to_string(),
        // mpeg-4 part 2 (DivX/Xvid era). Different from h264 (mpeg-4
        // part 10). Canonical name varies; collapse to the ffprobe
        // form so the carriable check sees one symbol.
        "msmpeg4v3" | "msmpeg4" | "divx" | "xvid" | "div3" => "mpeg4".to_string(),
        other => other.to_string(),
    }
}

/// Audio analog of [`normalize_video_codec`]. Handles the dolby /
/// dts / pcm spelling soup.
fn normalize_audio_codec(codec: &str) -> String {
    match codec.trim().to_ascii_lowercase().as_str() {
        "e-ac-3" | "ec-3" | "ec3" => "eac3".to_string(),
        "ac-3" => "ac3".to_string(),
        "dca" | "a_dts" => "dts".to_string(),
        "mpga" => "mp3".to_string(),
        other => other.to_string(),
    }
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

    let v_norm = normalize_video_codec(&v);
    let a_norm = normalize_audio_codec(&a);
    let video_ok = client
        .supported_video_codecs
        .iter()
        .any(|c| normalize_video_codec(c) == v_norm);
    let audio_ok = client
        .supported_audio_codecs
        .iter()
        .any(|c| normalize_audio_codec(c) == a_norm);
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

// ---------------------------------------------------------------------------
// Hover-time session pre-warm
// ---------------------------------------------------------------------------

/// Body for [`prewarm_session`]. The client sends a `ratingKey` (the
/// same opaque slug the Title Modal uses for its Play link) and its
/// browser capability sweep; we resolve the slug to a concrete
/// media_file_id + resume position and spin up an HLS session before
/// the user has clicked anything. The resulting session sits idle in
/// the transcoder until the player adopts it (within `PREWARM_TTL_MS`
/// client-side) or the idle reaper sweeps it.
#[derive(Debug, Deserialize)]
pub struct PrewarmRequest {
    pub rating_key: String,
    #[serde(default)]
    pub client: ClientCapabilities,
    #[serde(default)]
    pub audio_normalize: bool,
}

/// Resolve a Title-Modal `ratingKey` to the same (media_file_id,
/// start_position_ms) pair the `/watch/[ratingKey]` page would land
/// on. Mirrors the resolution rules in
/// `web/src/app/watch/[ratingKey]/page.tsx`:
///
///   * `e<id>` — episode rating key. Resolves to that episode's first
///     media file and the user's saved position on it.
///   * `<id>` (movie item) — resolves to the movie's first media file
///     and the user's saved position on the item.
///   * `<id>` (show item) — resolves to the first episode of the first
///     season (sorted by season number, then episode number) and the
///     user's saved position on that episode.
///
/// Returns `Ok(None)` when the slug doesn't parse, the target row
/// doesn't exist, or the user can't see the library it belongs to —
/// the caller surfaces that as 404 so a bad slug doesn't leak the
/// difference between "no such item" and "permission denied".
async fn resolve_rating_key(
    state: &AppState,
    user: &AuthUser,
    rating_key: &str,
) -> Result<Option<(i64, i64)>, ApiError> {
    let accessible = queries::user_library_filter(&state.pool, user.id, user.role)
        .await
        .map_err(ApiError::Internal)?;
    let accessible_ref = accessible.as_deref();

    if let Some(rest) = rating_key.strip_prefix('e') {
        let Ok(ep_id) = rest.parse::<i64>() else {
            return Ok(None);
        };
        let Some(detail) = queries::get_episode_detail(&state.pool, ep_id, user.id, accessible_ref)
            .await
            .map_err(ApiError::Internal)?
        else {
            return Ok(None);
        };
        let Some(file) = detail.files.first() else {
            return Ok(None);
        };
        let pos = detail
            .play_state
            .as_ref()
            .map(|p| p.position_ms)
            .unwrap_or(0);
        return Ok(Some((file.id, pos)));
    }

    let Ok(id) = rating_key.parse::<i64>() else {
        return Ok(None);
    };
    let Some(detail) = queries::get_item_detail(&state.pool, id, user.id, accessible_ref)
        .await
        .map_err(ApiError::Internal)?
    else {
        return Ok(None);
    };

    // Movie: the item carries its own files. Shows leave `files`
    // empty and surface episodes via `seasons` instead.
    if let Some(file) = detail.files.first() {
        let pos = detail
            .play_state
            .as_ref()
            .map(|p| p.position_ms)
            .unwrap_or(0);
        return Ok(Some((file.id, pos)));
    }

    // Show: pick first episode of first season. Matches the TS
    // resolver's `resolveShowFirstEpisode` behavior — keeping the two
    // in lockstep means a prewarmed session always targets the same
    // file the watch page would.
    let Some(first_season) = detail.seasons.first() else {
        return Ok(None);
    };
    let Some(season) =
        queries::get_season_detail(&state.pool, first_season.id, user.id, accessible_ref)
            .await
            .map_err(ApiError::Internal)?
    else {
        return Ok(None);
    };
    let Some(first_listed) = season.episodes.first() else {
        return Ok(None);
    };
    let Some(ep_detail) = queries::get_episode_detail(
        &state.pool,
        first_listed.episode.id,
        user.id,
        accessible_ref,
    )
    .await
    .map_err(ApiError::Internal)?
    else {
        return Ok(None);
    };
    let Some(file) = ep_detail.files.first() else {
        return Ok(None);
    };
    let pos = ep_detail
        .play_state
        .as_ref()
        .map(|p| p.position_ms)
        .unwrap_or(0);
    Ok(Some((file.id, pos)))
}

/// Pre-warm a play session before the user has actually clicked Play.
/// Functionally identical to a default `POST /sessions` — same shape
/// of response, same backing ffmpeg invocation — but routed through
/// the watch-page slug resolver so the modal's hover handler can call
/// this with the ratingKey it already has rather than having to
/// duplicate resolution logic in the browser.
pub async fn prewarm_session(
    State(state): State<AppState>,
    user: AuthUser,
    Extension(EffectiveClientIp(ip)): Extension<EffectiveClientIp>,
    headers: HeaderMap,
    Json(req): Json<PrewarmRequest>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), ApiError> {
    let (media_file_id, start_position_ms) = resolve_rating_key(&state, &user, &req.rating_key)
        .await?
        .ok_or(ApiError::NotFound)?;

    let inner = CreateSessionRequest {
        media_file_id,
        start_position_ms,
        client: req.client,
        audio_index: None,
        subtitle_index: None,
        subtitle_style: None,
        quality_target: None,
        audio_normalize: req.audio_normalize,
        subtitle_offset_ms: 0,
    };
    create_session_impl(&state, &user, &headers, ip, inner).await
}

/// Whether the client's declared capabilities include the given
/// codec short-name. Case-insensitive; matches both common spellings
/// ("hevc" / "h265").
fn client_supports_codec(caps: &ClientCapabilities, codec: &str) -> bool {
    let want = codec.to_ascii_lowercase();
    let aliases: &[&str] = match want.as_str() {
        "hevc" | "h265" => &["hevc", "h265"],
        "h264" | "avc" => &["h264", "avc"],
        _ => {
            return caps
                .supported_video_codecs
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&want));
        }
    };
    caps.supported_video_codecs
        .iter()
        .any(|c| aliases.iter().any(|a| c.eq_ignore_ascii_case(a)))
}

/// Pick the output codec for this session based on the operator's
/// mode setting, the client's declared HEVC support, and whether the
/// selected hwaccel actually has an HEVC encoder available. Falls
/// back to H264 whenever any of those gates fail — never breaks
/// playback to chase HEVC.
fn pick_target_codec(
    mode: &str,
    client_supports_hevc: bool,
    hwaccel: chimpflix_transcoder::HwAccel,
    caps: &chimpflix_transcoder::TranscoderCapabilities,
) -> chimpflix_transcoder::VideoCodec {
    use chimpflix_transcoder::VideoCodec;
    let want_hevc = match mode.to_ascii_lowercase().as_str() {
        "always" => true,
        "when_client_supports" => client_supports_hevc,
        _ => false, // 'off' or anything unknown
    };
    if !want_hevc {
        return VideoCodec::H264;
    }
    if !hwaccel.is_available_for(VideoCodec::Hevc, caps) {
        return VideoCodec::H264;
    }
    VideoCodec::Hevc
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

    fn req(audio_index: Option<u32>, subtitle_index: Option<u32>) -> CreateSessionRequest {
        CreateSessionRequest {
            media_file_id: 1,
            start_position_ms: 0,
            client: caps_mp4_h264_aac(),
            audio_index,
            subtitle_index,
            subtitle_style: None,
            quality_target: None,
            audio_normalize: false,
            subtitle_offset_ms: 0,
        }
    }

    #[test]
    fn treatment_copy_when_only_audio_swap_on_compatible_source() {
        // Picked alt audio track on an h264 source — should copy video.
        let r = req(Some(1), None);
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("h264"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Copy,
        );
    }

    #[test]
    fn treatment_reencode_when_subtitle_burn_requested() {
        let r = req(None, Some(0));
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("h264"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn treatment_reencode_when_quality_target_set() {
        let mut r = req(Some(1), None);
        r.quality_target = Some(QualityTarget {
            height: 720,
            bitrate_bps: 2_500_000,
        });
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("h264"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn treatment_reencode_when_source_is_hdr() {
        let r = req(Some(1), None);
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("h264"),
                Some("hdr10"),
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("h264"),
                Some("hlg"),
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("h264"),
                Some("dovi"),
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn treatment_reencode_when_source_codec_not_in_client_list() {
        let r = req(Some(1), None);
        // VP9 source — client list has only h264/hevc.
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("vp9"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn treatment_reencode_when_source_codec_missing_from_db() {
        let r = req(Some(1), None);
        assert_eq!(
            pick_video_treatment(
                &r,
                None,
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn treatment_copy_case_insensitive_codec_match() {
        let r = req(Some(1), None);
        // ffprobe sometimes reports H264 uppercase; the client list
        // typically uses lowercase. Match should be insensitive.
        // HEVC is asserted under fMP4 (the correct container for HEVC
        // copy — see `pick_container_bumps_to_fmp4_for_hevc_*`).
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("H264"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Copy,
        );
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("HEVC"),
                None,
                chimpflix_transcoder::ContainerFormat::Fmp4,
                None
            ),
            VideoTreatment::Copy,
        );
    }

    #[test]
    fn audio_treatment_copy_when_source_codec_compatible() {
        let r = req(Some(1), None);
        assert_eq!(
            pick_audio_treatment(&r, Some("aac"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Copy,
        );
        assert_eq!(
            pick_audio_treatment(&r, Some("AAC"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Copy,
        );
        // ac3 is in the client caps too.
        assert_eq!(
            pick_audio_treatment(&r, Some("ac3"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Copy,
        );
    }

    #[test]
    fn audio_treatment_reencode_when_source_codec_incompatible() {
        let r = req(Some(1), None);
        assert_eq!(
            pick_audio_treatment(&r, Some("flac"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Reencode,
        );
        assert_eq!(
            pick_audio_treatment(&r, Some("opus"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Reencode,
        );
    }

    #[test]
    fn audio_treatment_reencode_when_codec_not_ts_carriable_even_if_client_supports_it() {
        // Browser claims Opus support (it can decode Opus in WebM/fMP4) —
        // but MPEG-TS HLS can't carry Opus, so the copy fast-path would
        // produce broken output. The TS guard rail forces re-encode
        // regardless of the client's `supported_audio_codecs` advert.
        let mut r = req(Some(1), None);
        r.client.supported_audio_codecs = vec!["aac".into(), "opus".into(), "flac".into()];
        assert_eq!(
            pick_audio_treatment(&r, Some("opus"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Reencode,
        );
        assert_eq!(
            pick_audio_treatment(&r, Some("flac"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Reencode,
        );
    }

    #[test]
    fn video_treatment_reencode_for_av1_even_if_browser_supports_it() {
        // Same trap as the Opus case but for video. Chrome/Firefox
        // report AV1 support (in WebM/fMP4 containers) but MPEG-TS
        // HLS doesn't carry AV1 — copy would crash ffmpeg or produce
        // un-decodable segments and hang the player.
        let mut r = req(Some(1), None);
        r.client.supported_video_codecs = vec!["h264".into(), "av1".into()];
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("av1"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn video_treatment_reencode_for_vp9_even_if_browser_supports_it() {
        let mut r = req(Some(1), None);
        r.client.supported_video_codecs = vec!["h264".into(), "vp9".into()];
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("vp9"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn video_treatment_reencode_for_legacy_mpeg4() {
        // mpeg-4 part 2 (DivX/Xvid) rides MPEG-TS but no current
        // browser decodes it — the playback would just fail.
        let mut r = req(Some(1), None);
        r.client.supported_video_codecs = vec!["h264".into(), "mpeg4".into()];
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("mpeg4"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn video_treatment_reencode_for_hevc_in_ts_container() {
        // HEVC is intentionally NOT in the TS-carriable set —
        // HLS.js's TS-to-fMP4 transmuxer has unreliable HEVC
        // support, so even if the browser claims HEVC the copy
        // path through TS hangs in practice. The session planner
        // bumps container to fMP4 instead (see
        // `pick_container_bumps_to_fmp4_for_hevc_...`). This test
        // guards the TS-only assertion.
        let mut r = req(Some(1), None);
        r.client.supported_video_codecs = vec!["h264".into(), "hevc".into()];
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("hevc"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                None
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn video_treatment_copy_for_hevc_when_container_is_fmp4() {
        // The complement of the TS test: once container is bumped
        // to fMP4 (because the browser advertised HEVC), the copy
        // fast-path is valid — Safari and HEVC-capable HLS.js
        // builds both decode HEVC out of fMP4 cleanly.
        let mut r = req(Some(1), None);
        r.client.supported_video_codecs = vec!["h264".into(), "hevc".into()];
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("hevc"),
                None,
                chimpflix_transcoder::ContainerFormat::Fmp4,
                None,
            ),
            VideoTreatment::Copy,
        );
    }

    #[test]
    fn pick_container_bumps_to_fmp4_for_hevc_when_browser_supports_it() {
        // The Marvels case: HEVC source + browser claims HEVC
        // support. Container auto-bumps to fMP4 so the copy
        // fast-path is on a transport HLS.js + MSE can actually
        // play.
        let mut client = caps_mp4_h264_aac();
        client.supported_video_codecs = vec!["h264".into(), "hevc".into()];
        assert_eq!(
            pick_container(Some("hevc"), Some("aac"), &client, None),
            chimpflix_transcoder::ContainerFormat::Fmp4,
        );
    }

    #[test]
    fn codec_normalization_handles_hevc_and_h265_as_equivalent() {
        // ffprobe writes "hevc" today but some older tools and
        // certain remuxers emit "h265". The client's caps list
        // pushes both forms; normalization keeps them comparable.
        assert_eq!(normalize_video_codec("hevc"), "hevc");
        assert_eq!(normalize_video_codec("h265"), "hevc");
        assert_eq!(normalize_video_codec("H.265"), "h.265"); // not normalized
        assert_eq!(normalize_video_codec("x265"), "hevc");
        assert_eq!(normalize_video_codec("DivX"), "mpeg4");
        assert_eq!(normalize_video_codec("xvid"), "mpeg4");
    }

    #[test]
    fn audio_codec_normalization_handles_dolby_variants() {
        assert_eq!(normalize_audio_codec("e-ac-3"), "eac3");
        assert_eq!(normalize_audio_codec("EAC3"), "eac3");
        assert_eq!(normalize_audio_codec("ec-3"), "eac3");
        assert_eq!(normalize_audio_codec("ac-3"), "ac3");
        assert_eq!(normalize_audio_codec("dca"), "dts");
        assert_eq!(normalize_audio_codec("mpga"), "mp3");
        assert_eq!(normalize_audio_codec("aac"), "aac");
    }

    #[test]
    fn audio_treatment_copy_handles_eac3_spelling_variants() {
        // Source row from sqlite has "eac3"; client cap list pushes
        // "e-ac-3" because that's the ffprobe-original spelling some
        // tools surface. Both should normalize to the same symbol.
        let mut r = req(Some(1), None);
        r.client.supported_audio_codecs = vec!["aac".into(), "e-ac-3".into()];
        assert_eq!(
            pick_audio_treatment(&r, Some("eac3"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Copy,
        );
    }

    #[test]
    fn pick_container_chooses_fmp4_for_av1_when_browser_supports_it() {
        let mut client = caps_mp4_h264_aac();
        client.supported_video_codecs = vec!["h264".into(), "av1".into()];
        assert_eq!(
            pick_container(Some("av1"), Some("aac"), &client, None),
            chimpflix_transcoder::ContainerFormat::Fmp4,
        );
    }

    #[test]
    fn pick_container_stays_on_ts_for_opus_audio_h264_video() {
        // ffmpeg 5.1's mp4 muxer can't reliably copy Opus packets
        // (the "track 1: codec frame size is not set" bug), so we
        // dropped Opus from the fMP4-carriable list. Result: an
        // Opus-audio + h264-video source stays on TS with audio
        // reencoded to AAC, instead of bumping to fMP4 to copy Opus.
        let mut client = caps_mp4_h264_aac();
        client.supported_audio_codecs = vec!["aac".into(), "opus".into()];
        assert_eq!(
            pick_container(Some("h264"), Some("opus"), &client, None),
            chimpflix_transcoder::ContainerFormat::Ts,
        );
    }

    #[test]
    fn pick_container_stays_on_ts_when_codecs_fit() {
        let client = caps_mp4_h264_aac();
        assert_eq!(
            pick_container(Some("h264"), Some("aac"), &client, None),
            chimpflix_transcoder::ContainerFormat::Ts,
        );
    }

    #[test]
    fn pick_container_stays_on_ts_for_av1_when_browser_doesnt_decode_it() {
        // Browser doesn't list AV1 → fMP4 wouldn't help (player
        // would fail to decode). Stay on TS so we re-encode to h264.
        let client = caps_mp4_h264_aac();
        assert_eq!(
            pick_container(Some("av1"), Some("aac"), &client, None),
            chimpflix_transcoder::ContainerFormat::Ts,
        );
    }

    #[test]
    fn video_treatment_copy_for_av1_when_container_is_fmp4() {
        let mut r = req(Some(1), None);
        r.client.supported_video_codecs = vec!["h264".into(), "av1".into()];
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("av1"),
                None,
                chimpflix_transcoder::ContainerFormat::Fmp4,
                None,
            ),
            VideoTreatment::Copy,
        );
    }

    #[test]
    fn audio_treatment_reencode_for_opus_even_when_container_is_fmp4() {
        // Same reason as pick_container_stays_on_ts_for_opus_audio:
        // ffmpeg 5.1's mp4 muxer breaks on Opus copy, so we force
        // re-encode regardless of container. Test stays in case
        // someone unconditionally puts a session on fMP4 (e.g. an
        // HEVC-copy + Opus combo on Safari).
        let mut r = req(Some(1), None);
        r.client.supported_audio_codecs = vec!["aac".into(), "opus".into()];
        assert_eq!(
            pick_audio_treatment(
                &r,
                Some("opus"),
                chimpflix_transcoder::ContainerFormat::Fmp4,
            ),
            AudioTreatment::Reencode,
        );
    }

    #[test]
    fn audio_treatment_reencode_when_burning_subtitles() {
        // Subtitle burn forces -copyts on the input side for sub
        // alignment, and -copyts requires asetpts on the audio side
        // (only works on reencoded streams) to keep A/V in sync.
        let mut r = req(Some(1), Some(0));
        r.client.supported_audio_codecs = vec!["aac".into()];
        assert_eq!(
            pick_audio_treatment(&r, Some("aac"), chimpflix_transcoder::ContainerFormat::Ts,),
            AudioTreatment::Reencode,
        );
    }

    #[test]
    fn video_treatment_reencode_for_10bit_hevc_even_when_browser_claims_hevc() {
        // The Marvels Remux trap: source is HEVC Main10
        // (yuv420p10le), browser advertises HEVC support (Main only).
        // Without the pix_fmt guard we'd pick Copy, mux Main10 into
        // TS, and the browser's 8-bit decoder would silently choke.
        let mut r = req(Some(1), None);
        r.client.supported_video_codecs = vec!["h264".into(), "hevc".into()];
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("hevc"),
                None,
                chimpflix_transcoder::ContainerFormat::Ts,
                Some("yuv420p10le"),
            ),
            VideoTreatment::Reencode,
        );
    }

    #[test]
    fn video_treatment_copy_for_8bit_hevc_in_fmp4() {
        // Counter to the 10-bit guard: an 8-bit HEVC source should
        // still take the copy fast-path — but only via fMP4, since
        // HEVC is no longer TS-carriable.
        let mut r = req(Some(1), None);
        r.client.supported_video_codecs = vec!["h264".into(), "hevc".into()];
        assert_eq!(
            pick_video_treatment(
                &r,
                Some("hevc"),
                None,
                chimpflix_transcoder::ContainerFormat::Fmp4,
                Some("yuv420p"),
            ),
            VideoTreatment::Copy,
        );
    }

    #[test]
    fn is_10bit_pix_fmt_catches_common_variants() {
        assert!(is_10bit_pix_fmt("yuv420p10le"));
        assert!(is_10bit_pix_fmt("YUV420P10LE"));
        assert!(is_10bit_pix_fmt("yuv422p10le"));
        assert!(is_10bit_pix_fmt("yuv444p12le"));
        assert!(is_10bit_pix_fmt("p010le"));
        assert!(!is_10bit_pix_fmt("yuv420p"));
        assert!(!is_10bit_pix_fmt("yuv422p"));
        assert!(!is_10bit_pix_fmt("nv12"));
    }

    #[test]
    fn audio_treatment_reencode_when_codec_missing_from_db() {
        let r = req(Some(1), None);
        assert_eq!(
            pick_audio_treatment(&r, None, chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Reencode,
        );
    }

    #[test]
    fn audio_treatment_reencode_when_loudnorm_requested_even_on_aac() {
        let mut r = req(Some(1), None);
        r.audio_normalize = true;
        // Source is AAC (would normally copy) but loudnorm requires
        // a filter graph, which means re-encode.
        assert_eq!(
            pick_audio_treatment(&r, Some("aac"), chimpflix_transcoder::ContainerFormat::Ts),
            AudioTreatment::Reencode,
        );
    }

    #[test]
    fn audio_bitrate_scales_with_source_channels() {
        // Mono caps at 96k — duplicating a single channel to stereo
        // doesn't need a high bitrate, and saving the bits matters on
        // long talk-heavy content.
        assert_eq!(pick_audio_bitrate(Some(1)), 96_000);
        // 5.1 / 7.1 sources bump to 256k to preserve more detail
        // after the downmix.
        assert_eq!(pick_audio_bitrate(Some(6)), 256_000);
        assert_eq!(pick_audio_bitrate(Some(8)), 256_000);
        // Everything else (typical stereo, unknown) stays at the AAC
        // LC sweet spot.
        assert_eq!(pick_audio_bitrate(Some(2)), 192_000);
        assert_eq!(pick_audio_bitrate(Some(3)), 192_000);
        assert_eq!(pick_audio_bitrate(None), 192_000);
    }

    #[test]
    fn auto_quality_picks_tier_at_or_below_source() {
        // 4K source caps at 1080p — real-time 4K encode is too much
        // CPU on most hosts, and HLS clients in browsers rarely
        // benefit visually beyond 1080p anyway.
        assert_eq!(auto_quality_for_source(2160), Some((1080, 5_000_000)));
        assert_eq!(auto_quality_for_source(1080), Some((1080, 5_000_000)));
        // Below 1080 we stay at the source's natural tier — no
        // gratuitous upscale, no gratuitous downscale.
        assert_eq!(auto_quality_for_source(720), Some((720, 2_500_000)));
        assert_eq!(auto_quality_for_source(480), Some((480, 1_200_000)));
        assert_eq!(auto_quality_for_source(360), Some((360, 800_000)));
        // Floor of 240 — anything weirdly small still gets a real
        // bitrate floor so the encoder doesn't produce postage stamps.
        assert_eq!(auto_quality_for_source(144), Some((240, 800_000)));
        // No height info → None, transcoder falls back to its 720p
        // baseline.
        assert_eq!(auto_quality_for_source(0), None);
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
