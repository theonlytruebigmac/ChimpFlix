//! Discovery-triggered processing pipeline.
//!
//! When the scanner (scheduled, manual, or file-watcher path)
//! emits `ScanEvent::FileAdded`, we enqueue the per-file jobs that
//! make the new file fully playable: marker detection, preview
//! sprite, loudness analysis, chapter thumbs. They all run through
//! the durable queue so a server crash mid-processing resumes on
//! next startup.
//!
//! Each kind is enqueued via `enqueue_job_unique` keyed on file_id
//! so a repeated FileAdded (e.g. a file that was deleted and
//! re-added quickly) doesn't pile up duplicate jobs.

use chimpflix_library::ScanEvent;
use sqlx::SqlitePool;
use tracing::warn;

use crate::jobs::handlers;

/// Wrap an inner emitter with FileAdded handling. The inner emitter
/// is still called for every event (so the hub still gets them);
/// when a FileAdded comes through, we additionally spawn a tokio
/// task that enqueues the full per-file pipeline.
///
/// Spawning rather than awaiting keeps the scanner's per-file loop
/// from blocking on DB writes — enqueue is fast but the scanner
/// already does ffprobe + TMDB per file and we don't want to add
/// another sync point.
pub fn wrap_emitter_for_pipeline(
    pool: SqlitePool,
    inner: chimpflix_library::ScanEmitter,
) -> chimpflix_library::ScanEmitter {
    use std::sync::Arc;
    Arc::new(move |evt: ScanEvent| {
        // Forward to the original consumer (hub publishes to WS).
        inner(evt.clone());
        if let ScanEvent::FileAdded { media_file_id, .. } = evt {
            let pool = pool.clone();
            tokio::spawn(async move {
                if let Err(e) = enqueue_pipeline(&pool, media_file_id).await {
                    warn!(
                        media_file_id,
                        error = %format!("{e:#}"),
                        "discovery pipeline enqueue failed",
                    );
                }
            });
        }
    })
}

/// Enqueue every per-file pipeline job for `file_id`. Each is
/// deduped on file_id so a repeated call is a no-op. Bundled into
/// a single transaction so a bulk rsync that fires N FileAdded
/// events does N fsyncs, not 4×N.
///
/// Order within the tx doesn't matter — the kinds are independent
/// — but we still insert them in priority order so the queue
/// `priority DESC, id ASC` claim picks marker detection (most
/// user-visible) before loudness analysis (least).
pub async fn enqueue_pipeline(pool: &SqlitePool, file_id: i64) -> anyhow::Result<()> {
    use chimpflix_library::queries::{JobInput, enqueue_job_unique_tx};
    let payload = serde_json::json!({ "file_id": file_id });
    let mut tx = pool.begin().await?;
    for kind in [
        handlers::detect_markers_file::KIND,
        handlers::generate_preview_sprite::KIND,
        handlers::build_chapter_thumbs::KIND,
        handlers::analyze_loudness::KIND,
    ] {
        enqueue_job_unique_tx(
            &mut tx,
            JobInput::new(kind, payload.clone()),
            "file_id",
            file_id,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Counts returned by [`enqueue_full_sweep`] — surfaced in the admin
/// "Process all pending" response so the operator sees how much was
/// queued without having to refresh the queue list afterward.
#[derive(Debug, Default, serde::Serialize)]
pub struct SweepCounts {
    pub markers: usize,
    pub previews: usize,
    pub chapter_thumbs: usize,
    pub loudness: usize,
}


/// Retroactively run the discovery pipeline against the existing
/// library: query for every file lacking each artifact, enqueue
/// the corresponding kind. Dedup means a re-trigger while jobs are
/// in flight is a no-op per file.
///
/// Use this to backfill items that existed before the pipeline
/// migration shipped — or after restoring from a backup that
/// didn't carry the artifact tables. The scheduled safety-net
/// tasks would eventually catch the same files, but this drains
/// the entire backlog in one click rather than across many ticks.
pub async fn enqueue_full_sweep(pool: &SqlitePool) -> anyhow::Result<SweepCounts> {
    use chimpflix_library::queries;
    // Generous caps — the queue is fine with thousands of rows and
    // each kind dedups on file_id so re-running this is harmless.
    const CAP: i64 = 100_000;
    let mut counts = SweepCounts::default();

    // Markers: needs per-library iteration because the existing
    // query is scoped that way (the LEFT JOIN access-controls by
    // library_id). For the sweep we just want every library.
    for lib in queries::list_libraries(pool, None).await? {
        let rows = queries::list_media_files_needing_markers(pool, lib.id, CAP).await?;
        let ids: Vec<i64> = rows.into_iter().map(|(id, _, _)| id).collect();
        counts.markers +=
            handlers::detect_markers_file::enqueue_for_files(pool, &ids).await?;
    }
    // Previews / loudness / chapter thumbs all support library_id=None
    // for "every library" so a single query each suffices.
    let previews = queries::list_media_files_needing_previews(pool, None, CAP).await?;
    let preview_ids: Vec<i64> = previews.iter().map(|c| c.id).collect();
    counts.previews =
        handlers::generate_preview_sprite::enqueue_for_files(pool, &preview_ids).await?;

    let thumbs = queries::list_media_files_needing_chapter_thumbs(pool, None, CAP).await?;
    let thumb_ids: Vec<i64> = thumbs.iter().map(|c| c.id).collect();
    counts.chapter_thumbs =
        handlers::build_chapter_thumbs::enqueue_for_files(pool, &thumb_ids).await?;

    let loud = queries::list_media_files_needing_loudness(pool, None, CAP).await?;
    let loud_ids: Vec<i64> = loud.iter().map(|c| c.id).collect();
    counts.loudness =
        handlers::analyze_loudness::enqueue_for_files(pool, &loud_ids).await?;

    Ok(counts)
}
