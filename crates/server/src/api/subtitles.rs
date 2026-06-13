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
use sqlx::Row;

use crate::api::access;
use crate::api::error::ApiError;
use crate::auth::{AuthUser, OwnerAuth};
use crate::jobs::handlers::fetch_subtitles_item;
use crate::state::AppState;
use crate::subtitles_lookup;

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
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                ApiError::NotFound
            } else {
                ApiError::Internal(e.into())
            }
        })?;

    let lower = row.file_path.to_ascii_lowercase();
    let is_ass = lower.ends_with(".ass") || lower.ends_with(".ssa");
    let body_bytes = if is_ass {
        // Served as text/x-ssa for a future JS renderer; the native
        // `<track>` element ignores it. Leave the raw ASS untouched —
        // running the WebVTT cue sanitizer on it would mangle the
        // format, and it isn't rendered by the browser anyway.
        bytes
    } else if lower.ends_with(".srt") {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        // Convert SubRip → WebVTT, then run the shared ASS sanitizer:
        // OpenSubtitles sometimes returns ASS content under a `.srt`
        // name, so strip any override blocks / drawing paths that would
        // otherwise leak on screen as a wall of numbers.
        chimpflix_transcoder::sanitize_ass_webvtt(&srt_to_vtt(&text)).into_bytes()
    } else {
        // `.vtt` (or unknown) — already WebVTT, but still sanitize in
        // case it was converted from ASS upstream and carries drawing
        // paths. A clean VTT passes through unchanged save for whitespace.
        let text = String::from_utf8_lossy(&bytes).into_owned();
        chimpflix_transcoder::sanitize_ass_webvtt(&text).into_bytes()
    };

    // ASS/SSA files are not WebVTT; give them their own MIME type so a JS
    // renderer can distinguish the format rather than getting a mislabelled
    // text/vtt payload.
    let content_type = if is_ass {
        "text/x-ssa; charset=utf-8"
    } else {
        "text/vtt; charset=utf-8"
    };

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000"),
        ],
        Body::from(body_bytes),
    )
        .into_response())
}

// ─── On-demand fetch triggers (owner-only) ──────────────────────────────────
//
// The download engine, persistence, dedup, and player rendering all
// already exist (scheduled `fetch_subtitles` sweep + `fetch_subtitles_item`
// job + the serving routes above). These two endpoints just give the
// operator a manual trigger from the title UI.
//
// They run independently of the `subtitle_fetch_enabled` gate — that
// gate governs the *background* sweep / on-add pipeline (see
// `tasks::gates`), not the job worker. A manual click is an explicit
// request, so the only precondition we enforce is that OpenSubtitles
// credentials are configured; `configured: false` lets the UI point the
// operator at the credentials page instead of silently doing nothing.

#[derive(Debug, Serialize)]
pub struct FetchSubtitlesResponse {
    /// Number of `fetch_subtitles_item` jobs enqueued (0 or 1 — deduped
    /// on item_id, so a re-trigger while one is in flight returns 0).
    /// The job itself fans out across every downloaded episode for a
    /// show, so a single queued job can yield many subtitle rows.
    pub queued: usize,
    /// False when OpenSubtitles credentials aren't set — nothing was
    /// queued and the UI should prompt the operator to configure them.
    pub configured: bool,
    /// The language code the fetch will target (owner default → server
    /// metadata language → "en").
    pub language: String,
}

#[derive(Debug, Serialize)]
pub struct EpisodeFetchSubtitlesResponse {
    /// 1 when a fresh subtitle was downloaded and stored, 0 on a miss
    /// (no provider match) or when a row already existed.
    pub added: usize,
    pub configured: bool,
    pub language: String,
}

/// Resolve which language to fetch: the owner's saved
/// `default_subtitle_lang`, falling back to the server's
/// `metadata_language`, then `"en"`. Empty/whitespace values are
/// skipped at each tier.
async fn resolve_subtitle_language(state: &AppState, user_id: i64) -> String {
    let pref = sqlx::query_scalar::<_, Option<String>>(
        "SELECT default_subtitle_lang FROM users WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten()
    .flatten()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());
    if let Some(lang) = pref {
        return lang;
    }
    let meta = state.settings.read().await.metadata_language.trim().to_string();
    if meta.is_empty() { "en".to_string() } else { meta }
}

/// `POST /items/{id}/fetch-subtitles` — enqueue a background
/// OpenSubtitles fetch for a movie (one row) or show (fans out to every
/// downloaded episode). Returns immediately with a queued count, like
/// `detect-markers`; fetched tracks appear in the player automatically.
pub async fn fetch_for_item(
    State(state): State<AppState>,
    owner: OwnerAuth,
    Path(item_id): Path<i64>,
) -> Result<(StatusCode, Json<FetchSubtitlesResponse>), ApiError> {
    // 404 cleanly when the item doesn't exist instead of enqueueing a
    // job that the handler would later no-op on a missing row.
    let exists = sqlx::query_scalar::<_, i64>("SELECT id FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }

    let language = resolve_subtitle_language(&state, owner.0.id).await;

    if state.opensubtitles_snapshot().await.is_none() {
        return Ok((
            StatusCode::OK,
            Json(FetchSubtitlesResponse {
                queued: 0,
                configured: false,
                language,
            }),
        ));
    }

    let queued =
        fetch_subtitles_item::enqueue_for_items(&state.pool, &[item_id], &[language.clone()])
            .await
            .map_err(ApiError::Internal)?;

    Ok((
        StatusCode::ACCEPTED,
        Json(FetchSubtitlesResponse {
            queued,
            configured: true,
            language,
        }),
    ))
}

/// `POST /episodes/{id}/fetch-subtitles` — fetch a single episode's
/// subtitle inline so the row can show an immediate "added / none
/// found" result. One provider call (cheap enough for the request
/// path, matching `items::refresh`), idempotent against
/// `external_subtitles`.
pub async fn fetch_for_episode(
    State(state): State<AppState>,
    owner: OwnerAuth,
    Path(episode_id): Path<i64>,
) -> Result<Json<EpisodeFetchSubtitlesResponse>, ApiError> {
    let row = sqlx::query(
        "SELECT s.season_number AS season, e.episode_number AS episode,
                i.tmdb_id AS tmdb_id, i.imdb_id AS imdb_id
         FROM episodes e
         JOIN seasons s ON s.id = e.season_id
         JOIN items i ON i.id = s.show_id
         WHERE e.id = ?",
    )
    .bind(episode_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::Internal(e.into()))?
    .ok_or(ApiError::NotFound)?;

    let season = row.try_get::<i64, _>("season").unwrap_or(0) as i32;
    let episode = row.try_get::<i64, _>("episode").unwrap_or(0) as i32;
    let tmdb_id: Option<i64> = row.try_get("tmdb_id").ok().flatten();
    let imdb_id: Option<String> = row.try_get("imdb_id").ok().flatten();

    let language = resolve_subtitle_language(&state, owner.0.id).await;

    let Some(client) = state.opensubtitles_snapshot().await else {
        return Ok(Json(EpisodeFetchSubtitlesResponse {
            added: 0,
            configured: false,
            language,
        }));
    };

    // No external ids on the parent show → nothing to query the
    // provider with. Report a clean miss rather than erroring.
    if tmdb_id.is_none() && imdb_id.is_none() {
        return Ok(Json(EpisodeFetchSubtitlesResponse {
            added: 0,
            configured: true,
            language,
        }));
    }

    let dir = state.data_dir.join("subtitles");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;

    let added = subtitles_lookup::fetch_one_for_episode(
        &state,
        &client,
        episode_id,
        tmdb_id,
        imdb_id.as_deref(),
        season,
        episode,
        &language,
        &dir,
    )
    .await
    .map_err(ApiError::Internal)?;

    Ok(Json(EpisodeFetchSubtitlesResponse {
        added: usize::from(added),
        configured: true,
        language,
    }))
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
        // Only treat a line as a timestamp line if it starts with a digit.
        // This prevents corrupting cue-text that happens to contain '-->'.
        let is_timestamp = line.trim_start().as_bytes().first().is_some_and(|b| b.is_ascii_digit())
            && line.contains("-->");
        if is_timestamp {
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

    #[test]
    fn cue_text_containing_arrow_is_not_corrupted() {
        // A cue-text line that happens to contain '-->' must not have its
        // commas replaced, since it is not a timestamp line.
        let srt = "1\n00:00:01,000 --> 00:00:02,500\nGo to Edit --> Preferences, then save.\n";
        let vtt = srt_to_vtt(srt);
        assert!(vtt.contains("Go to Edit --> Preferences, then save."));
        // The actual timestamp line should still be converted.
        assert!(vtt.contains("00:00:01.000 --> 00:00:02.500"));
    }
}
