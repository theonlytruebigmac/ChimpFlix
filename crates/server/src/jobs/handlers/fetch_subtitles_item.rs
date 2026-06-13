//! `fetch_subtitles_item` — OpenSubtitles lookup for one item across
//! a list of languages. Payload: `{ item_id, languages }`.
//!
//! For movies: one provider call per language (movie has one
//! external_subtitles row per language).
//!
//! For shows: walks every episode and does one provider call per
//! (episode, language) pair. Idempotent — each call checks the
//! `external_subtitles` table first and short-circuits if a row
//! already exists for that target+language.
//!
//! The scheduled `fetch_subtitles` task is now a sweep that enqueues
//! one of these jobs per item lacking subtitles. The job queue's
//! worker pool processes them with retry semantics for free.

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use tracing::{info, warn};

use crate::state::AppState;
use crate::subtitles_lookup;

pub const KIND: &str = "fetch_subtitles_item";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub item_id: i64,
    /// Empty / missing falls back to `["en"]`. Each language is
    /// queried independently; a miss on one language doesn't fail
    /// the others.
    #[serde(default)]
    pub languages: Vec<String>,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload {
        item_id,
        mut languages,
    } = serde_json::from_value(payload).context("invalid payload")?;
    if languages.is_empty() {
        languages.push("en".into());
    }

    let Some(client) = state.opensubtitles_snapshot().await else {
        // No credentials — succeed without doing anything. The
        // operator either hasn't set up OpenSubtitles yet or has
        // disabled it; we don't want failing jobs piling up dead.
        return Ok(());
    };

    // Resolve item metadata.
    let row = sqlx::query("SELECT kind, tmdb_id, imdb_id FROM items WHERE id = ?")
        .bind(item_id)
        .fetch_optional(&state.pool)
        .await
        .context("items lookup")?;
    let Some(row) = row else {
        return Ok(()); // item deleted between enqueue and run
    };
    let kind: String = row.try_get("kind").unwrap_or_default();
    let tmdb_id: Option<i64> = row.try_get("tmdb_id").ok().flatten();
    let imdb_id: Option<String> = row.try_get("imdb_id").ok().flatten();
    if tmdb_id.is_none() && imdb_id.is_none() {
        // No ids → no way to query the provider.
        return Ok(());
    }

    let dir = state.data_dir.join("subtitles");
    tokio::fs::create_dir_all(&dir).await?;

    let mut hits = 0usize;
    let mut misses = 0usize;
    let mut errors = 0usize;

    if kind == "movie" {
        for lang in &languages {
            match subtitles_lookup::fetch_one_for_item(
                &state,
                &client,
                item_id,
                tmdb_id,
                imdb_id.as_deref(),
                lang,
                &dir,
            )
            .await
            {
                Ok(true) => hits += 1,
                Ok(false) => misses += 1,
                Err(e) => {
                    errors += 1;
                    warn!(item_id, lang, error = %format!("{e:#}"), "fetch_subtitles failed for movie");
                }
            }
        }
    } else {
        // show — walk episodes. Only DOWNLOADED episodes: a placeholder
        // row (no media_files, materialized to complete a season for the
        // finale flag / calendar) has no file for subtitles to attach to,
        // so fetching for it would burn OpenSubtitles API calls / rate
        // limit for nothing.
        let eps = sqlx::query(
            "SELECT e.id AS id, s.season_number AS season, e.episode_number AS episode
             FROM episodes e
             JOIN seasons s ON s.id = e.season_id
             WHERE s.show_id = ?
               AND EXISTS (SELECT 1 FROM media_files mf
                           WHERE mf.episode_id = e.id AND mf.removed_at IS NULL)",
        )
        .bind(item_id)
        .fetch_all(&state.pool)
        .await
        .context("list episodes for subtitle fetch")?;
        for ep in &eps {
            let episode_id: i64 = ep.try_get("id").unwrap_or(0);
            let season: i32 = ep.try_get("season").unwrap_or(0);
            let episode: i32 = ep.try_get("episode").unwrap_or(0);
            for lang in &languages {
                match subtitles_lookup::fetch_one_for_episode(
                    &state,
                    &client,
                    episode_id,
                    tmdb_id,
                    imdb_id.as_deref(),
                    season,
                    episode,
                    lang,
                    &dir,
                )
                .await
                {
                    Ok(true) => hits += 1,
                    Ok(false) => misses += 1,
                    Err(e) => {
                        errors += 1;
                        warn!(
                            episode_id,
                            lang,
                            error = %format!("{e:#}"),
                            "fetch_subtitles failed for episode"
                        );
                    }
                }
            }
        }
    }

    info!(
        item_id,
        hits, misses, errors, "fetch_subtitles_item complete"
    );
    Ok(())
}

/// Enqueue one `fetch_subtitles_item` job per item id. Deduped on
/// item_id so re-triggering while jobs are in flight is safe.
pub async fn enqueue_for_items(
    pool: &sqlx::SqlitePool,
    item_ids: &[i64],
    languages: &[String],
) -> Result<usize> {
    let mut queued = 0usize;
    for &item_id in item_ids {
        let payload = serde_json::json!({
            "item_id": item_id,
            "languages": languages,
        });
        let res = queries::enqueue_job_unique(
            pool,
            queries::JobInput::new(KIND, payload),
            "item_id",
            item_id,
        )
        .await?;
        if res.is_some() {
            queued += 1;
        }
    }
    Ok(queued)
}
