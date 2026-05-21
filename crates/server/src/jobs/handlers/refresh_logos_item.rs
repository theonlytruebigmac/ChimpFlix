//! `refresh_logos_item` — fetch the title-treatment logo from TMDB
//! for one `items` row. Payload: `{ "item_id": i64 }`.
//!
//! Extracted from the monolithic `refresh_logos` scheduled task. The
//! sweep now enqueues one of these per item missing a logo; the job
//! queue's worker pool then runs them with per-kind concurrency caps
//! and the standard backoff curve. Splitting the sweep into per-item
//! jobs means a TMDB outage no longer poisons the entire run — only
//! the in-flight items fail and retry.

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "refresh_logos_item";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub item_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { item_id } =
        serde_json::from_value(payload).context("invalid payload")?;

    let Some(tmdb) = state.tmdb_snapshot().await else {
        // No TMDB client — succeed quietly. Operator removed the
        // credential; we don't want jobs piling up dead while they
        // sort that out.
        return Ok(());
    };

    // Idempotent skip — the sweep enqueues items where logo_path is
    // NULL, but state could have changed between enqueue and pickup
    // (e.g. an admin manually set the URL). Re-check before doing
    // the network round-trip.
    let row = sqlx::query(
        "SELECT kind, tmdb_id, logo_path
         FROM items
         WHERE id = ?",
    )
    .bind(item_id)
    .fetch_optional(&state.pool)
    .await
    .context("items lookup")?;
    let Some(row) = row else {
        // Item deleted between enqueue and run.
        return Ok(());
    };
    let kind: String = row.try_get("kind").unwrap_or_default();
    let tmdb_id: Option<i64> = row.try_get("tmdb_id").ok().flatten();
    let existing_logo: Option<String> = row.try_get("logo_path").ok().flatten();
    if existing_logo.is_some() {
        // Already has a logo — nothing to do.
        return Ok(());
    }
    let Some(tmdb_id) = tmdb_id else {
        // Items without a TMDB id can't be looked up. Succeed; the
        // sweep query won't pick them up again (it filters on
        // `tmdb_id IS NOT NULL`).
        return Ok(());
    };

    let result = match kind.as_str() {
        "movie" => tmdb.fetch_movie_logo(tmdb_id).await,
        "show" => tmdb.fetch_show_logo(tmdb_id).await,
        other => {
            // Unknown kinds (extras, special items) don't have a
            // TMDB logo endpoint — succeed quietly so the job
            // doesn't churn through retries on a permanent miss.
            warn!(
                item_id,
                kind = other,
                "refresh_logos_item: unsupported kind"
            );
            return Ok(());
        }
    };

    match result {
        Ok(Some(path)) => {
            let url = chimpflix_metadata::tmdb_image_url(&path, "w500");
            let now = chimpflix_common::now_ms();
            sqlx::query("UPDATE items SET logo_path = ?, updated_at = ? WHERE id = ?")
                .bind(&url)
                .bind(now)
                .bind(item_id)
                .execute(&state.pool)
                .await
                .context("items logo_path update")?;
            info!(item_id, "logo updated");
        }
        Ok(None) => {
            // TMDB has the item but no logo asset — record nothing.
            // The sweep query will keep picking this up forever
            // unless we mark "tried, none available". For now, let
            // it churn at sweep cadence (weekly) which is cheap; if
            // it becomes a problem add an `items.logo_attempted_at`
            // column.
        }
        Err(e) => {
            // Surface to the worker as a retryable error.
            anyhow::bail!("tmdb logo fetch failed for item {item_id}: {e:#}");
        }
    }
    Ok(())
}

/// Enqueue one `refresh_logos_item` job per item id. Deduped on
/// item_id so re-triggering while jobs are in flight is safe.
pub async fn enqueue_for_items(
    pool: &sqlx::SqlitePool,
    item_ids: &[i64],
) -> Result<usize> {
    let mut queued = 0usize;
    for &item_id in item_ids {
        let payload = serde_json::json!({ "item_id": item_id });
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
