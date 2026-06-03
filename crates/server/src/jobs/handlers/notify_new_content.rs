//! `notify_new_content` — fan out content-arrival notifications for a
//! library after a scan added new files. Payload:
//! `{ "library_id": i64, "scan_job_id": i64 }`.
//!
//! ## Why a job (and not inline on the scan)
//!
//! Resolving the audience for each new movie/episode (access-matrix joins,
//! per-show watcher lookups) and fanning out per-user bell rows + emails +
//! Discord POSTs is exactly the kind of network-touching, multi-query work
//! that must NOT run on the scanner's per-file hot loop. The scan instead
//! enqueues ONE of these jobs on `ScanEvent::Completed` (see
//! `crate::jobs::pipeline`) — a single cheap, non-blocking `enqueue_job`
//! with a tiny payload. All the heavy work happens here, on the durable
//! worker pool, where a failure retries and a slow SMTP server can't wedge
//! the scan.
//!
//! ## What counts as "new" (dedup)
//!
//! Newness is defined by the `notified_content` ledger, NOT by which rows
//! the scan inserted. The handler asks the DB for movies / episodes in the
//! library that have a live media_file and are absent from the ledger, then
//! records every one it announces. Consequences:
//!   * A re-scan that re-persists an already-announced title returns nothing
//!     → no re-notify.
//!   * A title that existed before this feature shipped is announced exactly
//!     once, on the next scan that completes.
//!   * The scan path stays free of any "is this row new?" bookkeeping.
//!
//! ## Anti-spam batching
//!
//! Episodes are GROUPED per show: a scan that adds 24 episodes of one show
//! produces ONE "24 new episodes of <Show>" notification per watcher, not
//! 24. Movies are announced individually up to [`MOVIE_DETAIL_THRESHOLD`];
//! above that the per-library audience gets a single
//! "N new movies in <Library>" summary instead of N rows. Either way each
//! announced title is recorded in the ledger so the NEXT scan won't repeat
//! it.

use anyhow::{Context, Result};
use chimpflix_library::queries::{self, UnannouncedEpisode, UnannouncedMovie};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};

use crate::notifier::{self, KIND_NEW_EPISODE, KIND_NEW_MOVIE};
use crate::state::AppState;

pub const KIND: &str = "notify_new_content";

/// Cap on how many movies / episodes a single scan-completion job will
/// resolve. A scan can add tens of thousands of files; we don't want to
/// build one notification per title past a sane point. The remaining
/// titles are still recorded in the ledger as announced (so they don't
/// pile up forever), but their announcement is rolled into the summary /
/// simply suppressed past the cap. Generous enough that a normal
/// incremental scan (a handful of new episodes) is always fully detailed.
const RESOLVE_CAP: i64 = 2_000;

/// At or above this many new movies for one library in one scan, send a
/// single "N new movies in <Library>" summary to the library audience
/// instead of one notification per movie. Below it, each movie gets its
/// own (richer) notification with title + year + deep-link.
const MOVIE_DETAIL_THRESHOLD: usize = 6;

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub library_id: i64,
    /// The `scan_jobs.id` that triggered this fan-out. Informational —
    /// used only for logging/correlation; newness is resolved from the
    /// ledger, not this id.
    #[serde(default)]
    pub scan_job_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload {
        library_id,
        scan_job_id,
    } = serde_json::from_value(payload).context("invalid payload")?;

    let library = queries::get_library(&state.pool, library_id)
        .await
        .context("load library for new-content notify")?;
    let Some(library) = library else {
        // Library was deleted between scan completion and this job running.
        // Nothing to announce; the ledger rows (if any) cascade-deleted.
        info!(library_id, "library gone; skipping new-content notify");
        return Ok(());
    };

    let server_name = state.settings.read().await.server_name.clone();

    let movies = queries::list_unannounced_movies(&state.pool, library_id, RESOLVE_CAP)
        .await
        .context("list unannounced movies")?;
    let episodes = queries::list_unannounced_episodes(&state.pool, library_id, RESOLVE_CAP)
        .await
        .context("list unannounced episodes")?;

    if movies.is_empty() && episodes.is_empty() {
        info!(
            library_id,
            scan_job_id, "new-content notify: nothing unannounced; done"
        );
        return Ok(());
    }

    // ── Movies → library audience ────────────────────────────────────────
    if !movies.is_empty() {
        notify_movies(&state, &library, &server_name, &movies).await;
    }

    // ── Episodes → per-show watcher audience, grouped per show ───────────
    if !episodes.is_empty() {
        notify_episodes(&state, library_id, &server_name, &episodes).await;
    }

    info!(
        library_id,
        scan_job_id,
        movies = movies.len(),
        episodes = episodes.len(),
        "new-content notifications fanned out"
    );
    Ok(())
}

/// Announce new movies to everyone with access to the library. Below the
/// detail threshold, one notification per movie; at/above it, a single
/// "N new movies in <Library>" summary. Every announced movie is recorded
/// in the ledger regardless of which path is taken.
async fn notify_movies(
    state: &AppState,
    library: &chimpflix_library::Library,
    server_name: &str,
    movies: &[UnannouncedMovie],
) {
    let audience = match queries::list_library_audience_user_ids(&state.pool, library.id).await {
        Ok(ids) => ids,
        Err(e) => {
            warn!(
                library_id = library.id,
                error = %format!("{e:#}"),
                "resolve library audience for new movies failed; skipping movie notifications"
            );
            return;
        }
    };

    if audience.is_empty() {
        // No one can see this library (e.g. a brand-new library with no
        // grants yet besides… well, owners are always included, so this is
        // rare). Still record the movies as announced so a later scan that
        // runs after access is granted doesn't dump the whole back catalog.
        record_movies(state, library.id, movies).await;
        return;
    }

    // Record to the ledger BEFORE dispatching notifications. `ON CONFLICT DO
    // NOTHING` makes this idempotent: if the process crashes between recording
    // and sending, the next job run sees the ledger rows and skips re-sending,
    // trading a possible miss for preventing duplicate delivery.
    record_movies(state, library.id, movies).await;

    if movies.len() >= MOVIE_DETAIL_THRESHOLD {
        // Summary path — one notification for the whole batch.
        let payload = notifier::NewMovieBatchPayload {
            library_id: library.id,
            library_name: &library.name,
            count: movies.len(),
        };
        let (subject, text, html) =
            notifier::render_new_movies_batch(server_name, &payload);
        notifier::notify_users(
            state,
            &audience,
            KIND_NEW_MOVIE,
            &payload,
            &subject,
            &text,
            &html,
        )
        .await;
    } else {
        // Detail path — one notification per movie.
        for m in movies {
            let payload = notifier::NewMoviePayload {
                item_id: m.item_id,
                title: &m.title,
                year: m.year,
            };
            let (subject, text, html) = notifier::render_new_movie(server_name, &payload);
            notifier::notify_users(
                state,
                &audience,
                KIND_NEW_MOVIE,
                &payload,
                &subject,
                &text,
                &html,
            )
            .await;
        }
    }
}

/// Group new episodes per show, then send ONE notification per show to the
/// users who watch that show. A 24-episode batch for one show = one
/// "24 new episodes of <Show>" per watcher. Single-episode shows get the
/// richer per-episode message (with SxxExx + title).
async fn notify_episodes(
    state: &AppState,
    library_id: i64,
    server_name: &str,
    episodes: &[UnannouncedEpisode],
) {
    // `list_unannounced_episodes` returns rows ordered by
    // (show_id, season, episode), so equal show_ids are contiguous — group
    // by walking runs. Preserve order for a deterministic "first episode"
    // representative in the single-episode case.
    let mut idx = 0usize;
    while idx < episodes.len() {
        let show_id = episodes[idx].show_id;
        let start = idx;
        while idx < episodes.len() && episodes[idx].show_id == show_id {
            idx += 1;
        }
        let group = &episodes[start..idx];

        // Audience = users who watch this show (play history), intersected
        // with library visibility implicitly: a watcher necessarily had
        // access when they watched. We resolve watchers directly.
        let watchers = match queries::list_show_watcher_user_ids(&state.pool, show_id).await {
            Ok(ids) => ids,
            Err(e) => {
                warn!(
                    show_id,
                    error = %format!("{e:#}"),
                    "resolve show watchers failed; skipping this show's episode notifications"
                );
                // Still record so we don't retry-storm announcing them; a
                // watcher who appears later gets caught by the home-rail
                // refresh, not a backfilled blast.
                record_episodes(state, library_id, group).await;
                continue;
            }
        };

        if watchers.is_empty() {
            // No one watches this show (yet) — nothing to send, but record
            // as announced so we never blast the back catalog at the first
            // user who later starts watching.
            record_episodes(state, library_id, group).await;
            continue;
        }

        // Record to the ledger BEFORE dispatching notifications (same
        // rationale as notify_movies: prefer a miss on crash over duplicate
        // delivery; `ON CONFLICT DO NOTHING` makes retries safe).
        record_episodes(state, library_id, group).await;

        let show_title = group[0].show_title.as_str();
        if group.len() == 1 {
            let ep = &group[0];
            let payload = notifier::NewEpisodePayload {
                show_id,
                show_title,
                season_number: ep.season_number,
                episode_number: ep.episode_number,
                episode_title: ep.episode_title.as_deref(),
            };
            let (subject, text, html) = notifier::render_new_episode(server_name, &payload);
            notifier::notify_users(
                state,
                &watchers,
                KIND_NEW_EPISODE,
                &payload,
                &subject,
                &text,
                &html,
            )
            .await;
        } else {
            let payload = notifier::NewEpisodeBatchPayload {
                show_id,
                show_title,
                count: group.len(),
            };
            let (subject, text, html) =
                notifier::render_new_episodes_batch(server_name, &payload);
            notifier::notify_users(
                state,
                &watchers,
                KIND_NEW_EPISODE,
                &payload,
                &subject,
                &text,
                &html,
            )
            .await;
        }
    }
}

/// Stamp every movie in the slice into the ledger. Best-effort per row so
/// one failed write doesn't drop the rest; the `ON CONFLICT DO NOTHING`
/// upsert makes a partial-then-retry safe.
async fn record_movies(state: &AppState, library_id: i64, movies: &[UnannouncedMovie]) {
    for m in movies {
        if let Err(e) =
            queries::record_notified_content(&state.pool, KIND_NEW_MOVIE, m.item_id, library_id)
                .await
        {
            warn!(
                item_id = m.item_id,
                error = %format!("{e:#}"),
                "record_notified_content (movie) failed"
            );
        }
    }
}

async fn record_episodes(state: &AppState, library_id: i64, episodes: &[UnannouncedEpisode]) {
    for ep in episodes {
        if let Err(e) = queries::record_notified_content(
            &state.pool,
            KIND_NEW_EPISODE,
            ep.episode_id,
            library_id,
        )
        .await
        {
            warn!(
                episode_id = ep.episode_id,
                error = %format!("{e:#}"),
                "record_notified_content (episode) failed"
            );
        }
    }
}

/// Enqueue exactly one fan-out job for a completed scan. Deduped on
/// `library_id` so two scan completions of the same library that overlap
/// (manual + scheduled racing) collapse to one queued job — the handler
/// resolves the full unannounced set from the ledger either way, so a
/// single run after both scans is correct.
///
/// Cheap + non-blocking: a single `enqueue_job_unique` (one short
/// `BEGIN IMMEDIATE`), no network, no fan-out. Safe to call from the scan
/// event emitter.
pub async fn enqueue_for_scan(
    pool: &sqlx::SqlitePool,
    library_id: i64,
    scan_job_id: i64,
) -> Result<bool> {
    let payload = serde_json::json!({
        "library_id": library_id,
        "scan_job_id": scan_job_id,
    });
    let res = queries::enqueue_job_unique(
        pool,
        // Low priority — content announcements should never preempt the
        // per-file discovery pipeline (markers/loudness) that makes the
        // content actually playable.
        queries::JobInput::new(KIND, payload).with_priority(-1),
        "library_id",
        library_id,
    )
    .await?;
    Ok(res.is_some())
}
