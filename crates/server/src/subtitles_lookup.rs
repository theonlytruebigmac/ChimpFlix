//! Shared OpenSubtitles fetch helpers used by both the scheduled
//! `fetch_subtitles` safety-net task and the per-item job handler.
//!
//! Each helper handles one (target, language) tuple: skips if we
//! already have a subtitle row, queries the provider, downloads
//! the first hit, persists to disk + `external_subtitles`.
//!
//! Returns `true` on a fresh insert, `false` when there was nothing
//! to do (already have it, or no provider match). Errors bubble up
//! for the caller to count toward error totals.

use anyhow::Result;
use chimpflix_library::{NewExternalSubtitle, queries};
use chimpflix_metadata::{OpenSubtitlesClient, SearchParams};

use crate::state::AppState;

pub async fn fetch_one_for_item(
    state: &AppState,
    client: &OpenSubtitlesClient,
    item_id: i64,
    tmdb_id: Option<i64>,
    imdb_id: Option<&str>,
    language: &str,
    base_dir: &std::path::Path,
) -> Result<bool> {
    let existing = sqlx::query(
        "SELECT 1 FROM external_subtitles WHERE item_id = ? AND language = ? LIMIT 1",
    )
    .bind(item_id)
    .bind(language)
    .fetch_optional(&state.pool)
    .await?;
    if existing.is_some() {
        return Ok(false);
    }

    let langs = [language.to_string()];
    let hits = client
        .search_for_movie(SearchParams {
            tmdb_id,
            imdb_id,
            languages: &langs,
        })
        .await?;
    let Some(hit) = hits.into_iter().next() else {
        return Ok(false);
    };
    let bytes = client.download(hit.file_id).await?;
    let item_dir = base_dir.join(format!("item-{item_id}"));
    tokio::fs::create_dir_all(&item_dir).await?;
    let path = item_dir.join(format!("{language}-{}.srt", hit.file_id));
    tokio::fs::write(&path, &bytes).await?;
    queries::insert_external_subtitle(
        &state.pool,
        NewExternalSubtitle {
            item_id: Some(item_id),
            episode_id: None,
            language: hit.language,
            source: "opensubtitles".into(),
            source_file_id: Some(hit.file_id.to_string()),
            file_path: path.to_string_lossy().into_owned(),
            forced: hit.forced,
            sdh: hit.hearing_impaired,
        },
    )
    .await?;
    Ok(true)
}

pub async fn fetch_one_for_episode(
    state: &AppState,
    client: &OpenSubtitlesClient,
    episode_id: i64,
    tmdb_id: Option<i64>,
    imdb_id: Option<&str>,
    season: i32,
    episode: i32,
    language: &str,
    base_dir: &std::path::Path,
) -> Result<bool> {
    let existing = sqlx::query(
        "SELECT 1 FROM external_subtitles WHERE episode_id = ? AND language = ? LIMIT 1",
    )
    .bind(episode_id)
    .bind(language)
    .fetch_optional(&state.pool)
    .await?;
    if existing.is_some() {
        return Ok(false);
    }

    let langs = [language.to_string()];
    let hits = client
        .search_for_episode(
            SearchParams {
                tmdb_id,
                imdb_id,
                languages: &langs,
            },
            season,
            episode,
        )
        .await?;
    let Some(hit) = hits.into_iter().next() else {
        return Ok(false);
    };
    let bytes = client.download(hit.file_id).await?;
    let ep_dir = base_dir.join(format!("episode-{episode_id}"));
    tokio::fs::create_dir_all(&ep_dir).await?;
    let path = ep_dir.join(format!("{language}-{}.srt", hit.file_id));
    tokio::fs::write(&path, &bytes).await?;
    queries::insert_external_subtitle(
        &state.pool,
        NewExternalSubtitle {
            item_id: None,
            episode_id: Some(episode_id),
            language: hit.language,
            source: "opensubtitles".into(),
            source_file_id: Some(hit.file_id.to_string()),
            file_path: path.to_string_lossy().into_owned(),
            forced: hit.forced,
            sdh: hit.hearing_impaired,
        },
    )
    .await?;
    Ok(true)
}
