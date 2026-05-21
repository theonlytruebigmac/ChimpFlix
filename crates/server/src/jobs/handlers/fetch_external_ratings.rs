//! `fetch_external_ratings` — pull IMDb / Rotten Tomatoes / Metacritic /
//! MPAA scores for one item via OMDb. Payload: `{ "item_id": i64 }`.
//!
//! Free OMDb tier is 1,000 requests / day. The handler keys idempotency
//! on `items.ratings_updated_at`: anything refreshed within 30 days is
//! skipped. The sweep that feeds this kind respects the same window so
//! a re-trigger while jobs are queued is also a no-op.

use anyhow::{Context, Result};
use chimpflix_library::queries;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::Row;
use tracing::{info, warn};

use crate::state::AppState;

pub const KIND: &str = "fetch_external_ratings";

/// Refresh ratings older than this. Matches the sweep's selection.
pub const RATINGS_STALE_MS: i64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub item_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { item_id } = serde_json::from_value(payload).context("invalid payload")?;

    let Some(client) = state.omdb_snapshot().await else {
        // No API key configured — succeed silently. The operator
        // hasn't set up OMDb yet; failing here would just rack up
        // dead jobs while they sort that out.
        return Ok(());
    };

    let row = sqlx::query(
        "SELECT imdb_id, ratings_updated_at
         FROM items WHERE id = ?",
    )
    .bind(item_id)
    .fetch_optional(&state.pool)
    .await
    .context("items lookup")?;
    let Some(row) = row else {
        return Ok(()); // item deleted between enqueue + run
    };
    let imdb_id: Option<String> = row.try_get("imdb_id").ok().flatten();
    let updated_at: Option<i64> = row
        .try_get::<Option<i64>, _>("ratings_updated_at")
        .ok()
        .flatten();

    // Idempotency: skip if already fresh. Re-check at execute time
    // because a parallel run could have updated it between enqueue
    // and pickup.
    let now = chimpflix_common::now_ms();
    if let Some(u) = updated_at {
        if now - u < RATINGS_STALE_MS {
            return Ok(());
        }
    }
    let Some(imdb_id) = imdb_id else {
        // Without an IMDb id, OMDb can't be queried. Stamp the
        // watermark anyway so the sweep doesn't keep re-enqueueing.
        sqlx::query("UPDATE items SET ratings_updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(item_id)
            .execute(&state.pool)
            .await
            .context("items ratings_updated_at update (no imdb_id)")?;
        return Ok(());
    };

    match client.fetch_ratings(&imdb_id).await {
        Ok(Some(ratings)) => {
            let json = serde_json::to_string(&ratings).context("serialize OmdbRatings")?;
            sqlx::query(
                "UPDATE items
                 SET ratings_json = ?, ratings_updated_at = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&json)
            .bind(now)
            .bind(now)
            .bind(item_id)
            .execute(&state.pool)
            .await
            .context("items ratings upsert")?;
            info!(item_id, imdb_id = %imdb_id, "ratings refreshed");
        }
        Ok(None) => {
            // OMDb has no row for this IMDb id. Stamp anyway so the
            // sweep doesn't pick it up again next week.
            sqlx::query("UPDATE items SET ratings_updated_at = ? WHERE id = ?")
                .bind(now)
                .bind(item_id)
                .execute(&state.pool)
                .await
                .context("items ratings_updated_at update (omdb miss)")?;
        }
        Err(e) => {
            // Surface as a retryable error so the worker pool's
            // backoff curve handles transient OMDb failures (429,
            // 5xx, network blips). Permanent 4xx (e.g. invalid key)
            // gets the same retry treatment until Phase 5 lands the
            // error-class machinery.
            warn!(
                item_id,
                imdb_id = %imdb_id,
                error = %format!("{e:#}"),
                "omdb fetch failed"
            );
            anyhow::bail!("omdb fetch failed for item {item_id}: {e:#}");
        }
    }
    Ok(())
}

/// Enqueue one `fetch_external_ratings` job per item. Deduped on
/// item_id so a re-trigger while jobs are in flight is safe.
pub async fn enqueue_for_items(pool: &sqlx::SqlitePool, item_ids: &[i64]) -> Result<usize> {
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
