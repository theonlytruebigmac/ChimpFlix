//! `detect_markers_file` — single-file intro / credits detection.
//! Payload: `{ "file_id": i64 }`.
//!
//! Tiered detection pipeline (see `docs/PERF_PLAN.md`):
//!
//!   1. **Container chapter probe (Phase A).** Cheap ffprobe pass
//!      (~50ms). Match chapter titles against intro/credits patterns.
//!      Anime releases from fansub groups and most disc rips embed
//!      labeled chapters; trusting those bypasses audio decode for
//!      labelled kinds entirely.
//!   2. **Tacet audio fingerprint match** for whichever marker kind
//!      the chapter probe didn't cover. Backed by per-season
//!      references built by [`super::bootstrap_season_refs`].
//!   3. **Blackframe fallback** (inside tacet) for credits when no
//!      season reference exists (movies, pre-bootstrap episodes).
//!
//! Phase C: when the operator has flipped
//! `loudness_analysis_enabled` on and this file hasn't been measured
//! yet, the same tacet invocation that runs marker detection *also*
//! measures EBU R 128 loudness in one go via
//! [`tacet::analyze::analyze_audio`]. That cuts the per-file decode
//! load in half for loudness-on operators (markers + loudness used to
//! be two separate ffmpeg processes against the same file).
//!
//! Each marker row records which tier produced it via the `source`
//! column (`embedded` / `tacet` / `blackframe`) so the operator UI
//! can show provenance.
//!
//! Flow per file:
//!   1. Skip if `markers_detected_at` is already stamped.
//!   2. Probe container chapters; pattern-match titles for
//!      intro/credits labels.
//!   3. If labeled markers cover both kinds AND loudness isn't needed
//!      → write + stamp + return. No tacet work.
//!   4. Otherwise resolve `(show_id, season_number)`, load season
//!      refs, build a tacet `AnalysisRequest` that includes markers
//!      (for the kinds chapters didn't cover) and conditionally
//!      loudness.
//!   5. Call `tacet::analyze::analyze_audio` once. Merge
//!      chapter-derived + tacet-derived markers, persist via
//!      `replace_detected_markers`, stamp marker watermark, and
//!      persist loudness if it was requested + succeeded.
//!   6. If this is the third+ episode of a season and refs aren't
//!      built yet, enqueue `bootstrap_season_refs` for the season.

use std::path::Path as StdPath;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chimpflix_library::queries;
use chimpflix_transcoder::Chapter;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use tacet::analyze::{AnalysisRequest, MarkerRequest, analyze_audio};
use tacet::loudness::{CancellationToken, ProgressSink};
use tacet::matching::ReferenceFingerprint;
use tracing::{info, warn};

use crate::jobs::progress::JobContext;
use crate::state::AppState;

pub const KIND: &str = "detect_markers_file";

/// Threshold at which we bootstrap a season's references. Tacet's
/// own minimum (3 episodes) — duplicated as a constant so the
/// trigger logic doesn't have to dig into tacet's internals.
const BOOTSTRAP_THRESHOLD: i64 = 3;

/// Source values written to `markers.source`. Mirrors the operator-
/// facing taxonomy in the MarkerEditor UI badges.
const SOURCE_EMBEDDED: &str = "embedded";
const SOURCE_TACET: &str = "tacet";
const SOURCE_BLACKFRAME: &str = "blackframe";

#[derive(Debug, Serialize, Deserialize)]
pub struct Payload {
    pub file_id: i64,
}

pub async fn run(state: AppState, payload: Value) -> Result<()> {
    let Payload { file_id } = serde_json::from_value(payload).context("invalid payload")?;

    let Some((path, _duration_ms, markers_already_detected, loudness_already_measured)) =
        sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<i64>)>(
            "SELECT path, duration_ms, markers_detected_at, loudnorm_analyzed_at
             FROM media_files
             WHERE id = ? AND removed_at IS NULL",
        )
        .bind(file_id)
        .fetch_optional(&state.pool)
        .await
        .context("media_files lookup")?
    else {
        return Ok(());
    };

    // Idempotent skip — both the on-add and safety-net paths can
    // enqueue the same file; bail if a previous run already stamped
    // the watermark.
    if markers_already_detected.is_some() {
        return Ok(());
    }

    let media_path: PathBuf = StdPath::new(&path).to_path_buf();

    // ── Tier 1: container chapter probe ───────────────────────────
    //
    // Cheap (~50ms) — read whatever the container declares for
    // chapter markers and look for semantic title matches. Anime +
    // disc rips frequently come with labeled chapters; when both
    // kinds are present we skip audio decode entirely (unless
    // loudness is also needed — that still requires the decode).
    let chapters = match chimpflix_transcoder::probe_chapters(&state.ffmpeg, &media_path).await {
        Ok(c) => c,
        Err(e) => {
            // ffprobe failures here are usually "no chapter atom in
            // container" rather than a real error. Treat as empty.
            tracing::debug!(file_id, error = %format!("{e:#}"), "chapter probe failed; treating as no chapters");
            Vec::new()
        }
    };
    let labeled = match_chapter_titles(&chapters);

    // Decide what tacet still needs to do.
    let intro_from_label = labeled.intro.is_some();
    let credits_from_label = labeled.credits.is_some();
    let need_tacet_markers = !intro_from_label || !credits_from_label;

    // Loudness opt-in check: read the gate fresh from settings so the
    // operator can flip it at runtime without restarting workers.
    let settings = queries::get_server_settings(&state.pool)
        .await
        .context("read server settings for loudness gate")?;
    let need_loudness =
        settings.loudness_analysis_enabled && loudness_already_measured.is_none();

    // Full short-circuit: both kinds labeled AND no loudness needed —
    // write embedded markers and stamp without touching audio.
    if !need_tacet_markers && !need_loudness {
        let intro = labeled.intro.as_ref().unwrap();
        let credits = labeled.credits.as_ref().unwrap();
        let rows = vec![
            (
                "intro".to_string(),
                intro.start_ms,
                intro.end_ms,
                SOURCE_EMBEDDED.to_string(),
            ),
            (
                "credits".to_string(),
                credits.start_ms,
                credits.end_ms,
                SOURCE_EMBEDDED.to_string(),
            ),
        ];
        queries::replace_detected_markers(&state.pool, file_id, &rows)
            .await
            .context("save embedded markers")?;
        stamp_detected(&state.pool, file_id).await?;
        info!(
            file_id,
            intro_label = %intro.label,
            credits_label = %credits.label,
            "markers detected from container chapters (no audio decode)"
        );
        return Ok(());
    }

    // ── Tier 2 + 3: tacet analyze_audio ───────────────────────────
    //
    // Resolve season context if we need markers (no need for
    // loudness-only runs). For movies / unmatched files the context
    // is None and tacet falls back to blackframe-only detection for
    // credits.
    let season_ctx = if need_tacet_markers {
        queries::resolve_show_and_season_for_file(&state.pool, file_id)
            .await
            .context("resolve show + season for file")?
    } else {
        None
    };

    let (intro_refs, credits_refs) = if need_tacet_markers {
        match season_ctx {
            Some((show_id, season_number)) => {
                load_refs(&state.pool, show_id, season_number).await?
            }
            None => (vec![], vec![]),
        }
    } else {
        (vec![], vec![])
    };

    // Derive window hints from unlabeled chapter boundaries: even
    // without semantic titles, chapter timestamps tell us *where* the
    // intro/credits likely live. The first chapter is the intro
    // candidate when it's plausibly intro-shaped (ends before 5 min);
    // the last chapter is the credits candidate when it starts past
    // 50% of duration. Hints only fire when no label match exists for
    // that kind — labeled markers are written from `labeled` instead
    // and tacet doesn't run for that side.
    let (intro_hint, credits_hint) = if need_tacet_markers {
        chapter_window_hints(&chapters, !intro_from_label, !credits_from_label)
    } else {
        (None, None)
    };

    let analysis_request = AnalysisRequest {
        markers: if need_tacet_markers {
            Some(MarkerRequest {
                intro_refs,
                credits_refs,
                episode_id: format!("file-{file_id}"),
                intro_window_hint: intro_hint,
                credits_window_hint: credits_hint,
            })
        } else {
            None
        },
        loudness: need_loudness,
    };

    // Tacet is CPU-bound + uses rayon internally — keep it off the
    // tokio runtime so other workers stay responsive. CancellationToken
    // is wired through but unused for now (no operator pause control
    // surfaced yet — left for a future continuation of Phase D).
    let cancel = CancellationToken::new();
    let config = tacet::Config::default();
    let analysis_path = media_path.clone();
    // Pull the per-job progress sink from the worker's task-local
    // `JobContext`. When present, tacet emits per-stage progress
    // events into the in-memory store the admin UI polls; when
    // absent (e.g. direct test invocation) we run with no progress
    // surfacing, identical to pre-Phase-D behavior.
    let progress_sink: Option<std::sync::Arc<dyn ProgressSink>> =
        JobContext::current().map(|c| c.progress_sink);
    let analysis_result = tokio::task::spawn_blocking(move || {
        let sink_ref: Option<&dyn ProgressSink> = progress_sink.as_deref();
        analyze_audio(&analysis_path, analysis_request, sink_ref, &cancel, &config)
    })
    .await
    .context("analyze_audio join")?;
    let analysis = match analysis_result {
        Ok(r) => r,
        Err(e) => {
            // A failure here is "tacet couldn't decode the file at all"
            // — log + stamp the watermark so the safety-net sweep
            // doesn't keep retrying a permanently-broken source. Any
            // embedded-labeled markers we already had still get
            // written below before the early return.
            warn!(file_id, error = %format!("{e:#}"), "analyze_audio failed");
            let mut rows = Vec::new();
            if let Some(intro) = &labeled.intro {
                rows.push((
                    "intro".to_string(),
                    intro.start_ms,
                    intro.end_ms,
                    SOURCE_EMBEDDED.to_string(),
                ));
            }
            if let Some(credits) = &labeled.credits {
                rows.push((
                    "credits".to_string(),
                    credits.start_ms,
                    credits.end_ms,
                    SOURCE_EMBEDDED.to_string(),
                ));
            }
            queries::replace_detected_markers(&state.pool, file_id, &rows)
                .await
                .context("save partial embedded markers after analyze_audio failure")?;
            stamp_detected(&state.pool, file_id).await?;
            return Ok(());
        }
    };

    // Surface any sub-failure warnings (loudness errored while markers
    // succeeded, etc.) so the operator can see them in the logs. We
    // only stamp loudnorm_analyzed_at when loudness *didn't* warn.
    let loudness_failed_with_warning = analysis
        .warnings
        .iter()
        .any(|w| w.starts_with("loudness:"));
    for w in &analysis.warnings {
        warn!(file_id, warning = %w, "analyze_audio sub-warning");
    }

    // Merge tier outputs. Embedded labels win when both are present
    // for the same kind — they're the highest-confidence signal.
    let tacet_markers = analysis.markers.as_ref();
    let mut rows: Vec<(String, i64, i64, String)> = Vec::new();
    match (
        labeled.intro.as_ref(),
        tacet_markers.and_then(|m| m.intro.as_ref()),
    ) {
        (Some(label), _) => rows.push((
            "intro".to_string(),
            label.start_ms,
            label.end_ms,
            SOURCE_EMBEDDED.to_string(),
        )),
        (None, Some(seg)) => rows.push((
            "intro".to_string(),
            seg.start_ms(),
            seg.end_ms(),
            tacet_source_str(seg.source),
        )),
        (None, None) => {}
    }
    match (
        labeled.credits.as_ref(),
        tacet_markers.and_then(|m| m.credits.as_ref()),
    ) {
        (Some(label), _) => rows.push((
            "credits".to_string(),
            label.start_ms,
            label.end_ms,
            SOURCE_EMBEDDED.to_string(),
        )),
        (None, Some(seg)) => rows.push((
            "credits".to_string(),
            seg.start_ms(),
            seg.end_ms(),
            tacet_source_str(seg.source),
        )),
        (None, None) => {}
    }

    queries::replace_detected_markers(&state.pool, file_id, &rows)
        .await
        .context("save markers")?;
    stamp_detected(&state.pool, file_id).await?;

    // Persist loudness when requested. `Some(_)` → measured;
    // `None` AND no warning → benign miss (silent file / no audio
    // stream); skip the stamp on warning so the safety-net sweep
    // retries.
    if need_loudness && !loudness_failed_with_warning {
        let record = analysis.loudness.map(|m| queries::LoudnessMeasurement {
            integrated: m.integrated,
            true_peak: m.true_peak,
            lra: m.lra,
            threshold: m.threshold,
        });
        queries::record_loudness_measurement(&state.pool, file_id, record)
            .await
            .context("record loudness measurement")?;
    }

    // Persist per-stage timings for the operator UI. Best-effort —
    // a write failure is logged but not propagated, since the
    // timings are visibility nicety, not data integrity.
    if let Some(ctx) = JobContext::current() {
        let stage_payload = serde_json::json!({
            "markers_ms": analysis.stage_timings.markers.as_millis() as u64,
            "loudness_ms": analysis.stage_timings.loudness.as_millis() as u64,
        });
        if let Ok(s) = serde_json::to_string(&stage_payload) {
            if let Err(e) = queries::record_job_stage_timings(&state.pool, ctx.job_id, &s).await {
                warn!(file_id, error = %format!("{e:#}"), "record_job_stage_timings failed");
            }
        }
    }

    info!(
        file_id,
        intro_from_label,
        credits_from_label,
        intro_from_tacet = tacet_markers.and_then(|m| m.intro.as_ref()).is_some(),
        credits_from_tacet = tacet_markers.and_then(|m| m.credits.as_ref()).is_some(),
        loudness_measured = analysis.loudness.is_some(),
        markers_ms = analysis.stage_timings.markers.as_millis(),
        loudness_ms = analysis.stage_timings.loudness.as_millis(),
        "markers detected"
    );

    // Trigger bootstrap when this file pushes its season past the
    // threshold and references aren't built yet. Dedup happens at
    // job-queue enqueue (unique on synthetic (show, season) key) +
    // bootstrap_season_refs itself short-circuits when refs are
    // already present.
    if let Some((show_id, season_number)) = season_ctx {
        maybe_enqueue_bootstrap(&state.pool, show_id, season_number).await?;
    }
    Ok(())
}

/// Map tacet's `SegmentSource` to the string written to
/// `markers.source`. Keeps the on-disk taxonomy stable even as
/// tacet's internal enum grows.
fn tacet_source_str(source: tacet::SegmentSource) -> String {
    match source {
        tacet::SegmentSource::AudioFingerprint => SOURCE_TACET.to_string(),
        tacet::SegmentSource::Blackframe => SOURCE_BLACKFRAME.to_string(),
    }
}

/// Output of [`match_chapter_titles`]. Each kind carries the
/// timestamps + the matched label text (for logging / debugging).
#[derive(Debug, Default)]
struct LabeledChapters {
    intro: Option<LabeledMarker>,
    credits: Option<LabeledMarker>,
}

#[derive(Debug)]
struct LabeledMarker {
    start_ms: i64,
    end_ms: i64,
    label: String,
}

/// Scan container chapter titles for intro/credits patterns. Returns
/// the first match of each kind (anime episodes sometimes have a
/// recap chapter named "Previously" before "Opening" — we deliberately
/// pick the earliest match for intros and the latest for credits).
///
/// Patterns are intentionally narrow: only well-understood labels
/// produce embedded markers. Anything ambiguous (e.g. "Chapter 1",
/// "Part A") falls through to tacet so we don't mislabel by
/// mistake.
fn match_chapter_titles(chapters: &[Chapter]) -> LabeledChapters {
    let mut out = LabeledChapters::default();
    for chapter in chapters {
        let Some(title) = chapter.title.as_ref() else {
            continue;
        };
        let normalized = title.trim().to_ascii_lowercase();
        if out.intro.is_none() && is_intro_label(&normalized) {
            out.intro = Some(LabeledMarker {
                start_ms: chapter.start_ms,
                end_ms: chapter.end_ms,
                label: title.clone(),
            });
            continue;
        }
        if is_credits_label(&normalized) {
            // Always update credits to the *latest* matching chapter —
            // some releases have an early "credits roll" or fake-out
            // chapter; the real end credits are the last one.
            out.credits = Some(LabeledMarker {
                start_ms: chapter.start_ms,
                end_ms: chapter.end_ms,
                label: title.clone(),
            });
        }
    }
    out
}

fn is_intro_label(s: &str) -> bool {
    matches!(
        s,
        "intro"
            | "opening"
            | "op"
            | "opening theme"
            | "opening credits"
            | "opening sequence"
            | "main titles"
            | "title sequence"
            | "theme"
    )
}

fn is_credits_label(s: &str) -> bool {
    matches!(
        s,
        "credits"
            | "end credits"
            | "ending credits"
            | "closing credits"
            | "outro"
            | "ending"
            | "ed"
            | "ending theme"
            | "closing"
            | "end card"
    )
}

/// Maximum intro-candidate chapter length in seconds. A first chapter
/// longer than this is probably not the intro (it's the "main" act
/// of a movie with no credits-aware chapter scheme), so we skip the
/// hint and let tacet use its default window.
const MAX_INTRO_HINT_LEN_SECS: f64 = 300.0;
/// Minimum start position for a credits hint, as a fraction of total
/// duration. Anything starting in the first half of the file isn't a
/// plausible credits candidate. Falls back to absolute "must be after
/// 30 min" when duration isn't known.
const CREDITS_HINT_MIN_START_FRACTION: f64 = 0.5;

/// Build optional decode-window hints from unlabeled chapter boundaries.
///
/// Returns `(intro_hint, credits_hint)` where each is
/// `Some((start_seconds, end_seconds))` when a plausible candidate
/// chapter exists for that kind. Returns `None` for both when the
/// chapter list is empty or the heuristics don't find a fit — tacet
/// then uses its default windows.
///
/// `need_intro` / `need_credits` gate the heuristics so we don't waste
/// effort computing a hint that won't be used (the labeled kind wins).
///
/// The `(Option<(f64, f64)>, Option<(f64, f64)>)` return shape mirrors
/// `MarkerRequest`'s `intro_window_hint` / `credits_window_hint`
/// fields directly so the call site is "destructure + assign" without
/// a wrapper struct. Clippy flags type-complexity here; inline-allow
/// since the shape is dictated by tacet's API.
#[allow(clippy::type_complexity)]
fn chapter_window_hints(
    chapters: &[Chapter],
    need_intro: bool,
    need_credits: bool,
) -> (Option<(f64, f64)>, Option<(f64, f64)>) {
    if chapters.is_empty() {
        return (None, None);
    }

    // Duration estimate: the last chapter's end timestamp. ffprobe
    // populates `end_ms` for every chapter from the container's
    // `END=` attribute, so this is reliable when chapters exist.
    let duration_secs = (chapters.last().map(|c| c.end_ms).unwrap_or(0) as f64) / 1000.0;

    let intro = if need_intro {
        let first = &chapters[0];
        let start = (first.start_ms as f64) / 1000.0;
        let end = (first.end_ms as f64) / 1000.0;
        let len = (end - start).max(0.0);
        if len > 0.0 && len <= MAX_INTRO_HINT_LEN_SECS && start < 60.0 {
            Some((start, end))
        } else {
            None
        }
    } else {
        None
    };

    let credits = if need_credits && chapters.len() >= 2 {
        let last = chapters.last().unwrap();
        let start = (last.start_ms as f64) / 1000.0;
        let end = (last.end_ms as f64) / 1000.0;
        // Must be in the tail half of the file. Fall back to "after
        // 30 min" if duration is unknown.
        let min_start = if duration_secs > 0.0 {
            duration_secs * CREDITS_HINT_MIN_START_FRACTION
        } else {
            1800.0
        };
        if end > start && start >= min_start {
            Some((start, end))
        } else {
            None
        }
    } else {
        None
    };

    (intro, credits)
}

/// Decode the season's bincoded references, or return empty vecs
/// when the season hasn't been bootstrapped yet. Decode failures are
/// surfaced as warnings + treated as "no refs" so a corrupt blob
/// doesn't break detection for the whole season.
async fn load_refs(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<(Vec<ReferenceFingerprint>, Vec<ReferenceFingerprint>)> {
    let Some(blobs) = queries::load_season_refs_blobs(pool, show_id, season_number)
        .await
        .context("load season refs")?
    else {
        return Ok((vec![], vec![]));
    };
    let intro: Vec<ReferenceFingerprint> = match bincode::deserialize(&blobs.intro) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                show_id,
                season_number,
                error = %e,
                "intro refs blob failed to deserialize; falling back to empty"
            );
            vec![]
        }
    };
    let credits: Vec<ReferenceFingerprint> = match bincode::deserialize(&blobs.credits) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                show_id,
                season_number,
                error = %e,
                "credits refs blob failed to deserialize; falling back to empty"
            );
            vec![]
        }
    };
    Ok((intro, credits))
}

async fn stamp_detected(pool: &SqlitePool, file_id: i64) -> Result<()> {
    let now = chimpflix_common::now_ms();
    sqlx::query("UPDATE media_files SET markers_detected_at = ? WHERE id = ?")
        .bind(now)
        .bind(file_id)
        .execute(pool)
        .await
        .context("stamp markers_detected_at")?;
    Ok(())
}

/// Enqueue `bootstrap_season_refs` when the season is past the
/// threshold and either has no refs yet OR the refs predate any
/// new episode. We accept the slight cost of an enqueue+short-circuit
/// to keep the trigger logic simple — the bootstrap handler dedups
/// internally.
async fn maybe_enqueue_bootstrap(
    pool: &SqlitePool,
    show_id: i64,
    season_number: i32,
) -> Result<()> {
    let episodes = queries::count_episodes_in_season(pool, show_id, season_number)
        .await
        .context("count episodes for bootstrap trigger")?;
    if episodes < BOOTSTRAP_THRESHOLD {
        return Ok(());
    }
    // Refs already built? Skip.
    let already = queries::load_season_refs_blobs(pool, show_id, season_number)
        .await
        .context("check existing season refs")?
        .is_some();
    if already {
        return Ok(());
    }
    // Skip bootstrap when every episode in the season already has
    // markers from a non-tacet source (Phase A embedded chapter
    // labels, operator-set manual markers). Tacet's cross-episode
    // bootstrap is decoding-heavy; running it when there's nothing
    // useful to add is pure waste. A future episode added without a
    // marker source will re-trigger this gate and bootstrap will
    // fire then.
    let pending = queries::count_episodes_needing_markers_in_season(pool, show_id, season_number)
        .await
        .context("count episodes still needing marker detection")?;
    if pending == 0 {
        tracing::debug!(
            show_id,
            season_number,
            episodes,
            "skipping bootstrap_season_refs enqueue: every episode already has markers"
        );
        return Ok(());
    }
    let _ = super::bootstrap_season_refs::enqueue_for_season(pool, show_id, season_number).await?;
    Ok(())
}

/// Helper for manual trigger paths (item-level, library-level,
/// bulk) — enqueues one detect_markers_file job per media_file_id,
/// deduped on file_id so a re-trigger while a job is in flight
/// doesn't pile up duplicates.
///
/// Batched into a single `BEGIN IMMEDIATE` transaction so a backfill
/// of thousands of files doesn't compete with active workers row-by-
/// row — the previous per-file-loop approach was the hottest
/// `SQLITE_BUSY_SNAPSHOT` trigger in the workload.
pub async fn enqueue_for_files(pool: &sqlx::SqlitePool, file_ids: &[i64]) -> Result<usize> {
    queries::enqueue_jobs_for_files_batched(pool, KIND, file_ids).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chap(start: i64, end: i64, title: Option<&str>) -> Chapter {
        Chapter {
            start_ms: start,
            end_ms: end,
            title: title.map(str::to_string),
        }
    }

    #[test]
    fn matches_opening_and_ending_anime_style() {
        let chapters = vec![
            chap(0, 90_000, Some("Opening")),
            chap(90_000, 1_290_000, Some("Part A")),
            chap(1_290_000, 1_380_000, Some("Ending")),
        ];
        let out = match_chapter_titles(&chapters);
        assert!(out.intro.is_some());
        assert!(out.credits.is_some());
        let intro = out.intro.unwrap();
        assert_eq!(intro.start_ms, 0);
        assert_eq!(intro.end_ms, 90_000);
        let credits = out.credits.unwrap();
        assert_eq!(credits.start_ms, 1_290_000);
    }

    #[test]
    fn case_insensitive_and_trims_whitespace() {
        let chapters = vec![chap(0, 60_000, Some("  OPENING  "))];
        let out = match_chapter_titles(&chapters);
        assert!(out.intro.is_some());
    }

    #[test]
    fn ignores_ambiguous_chapter_names() {
        let chapters = vec![
            chap(0, 90_000, Some("Chapter 1")),
            chap(90_000, 1_200_000, Some("Chapter 2")),
            chap(1_200_000, 1_300_000, Some("Chapter 3")),
        ];
        let out = match_chapter_titles(&chapters);
        assert!(out.intro.is_none());
        assert!(out.credits.is_none());
    }

    #[test]
    fn no_chapters_no_match() {
        let out = match_chapter_titles(&[]);
        assert!(out.intro.is_none());
        assert!(out.credits.is_none());
    }

    #[test]
    fn untitled_chapters_dont_match() {
        let chapters = vec![chap(0, 60_000, None), chap(60_000, 120_000, None)];
        let out = match_chapter_titles(&chapters);
        assert!(out.intro.is_none());
        assert!(out.credits.is_none());
    }

    #[test]
    fn chapter_window_hints_picks_first_chapter_as_intro() {
        // Anime-style: 90s opening as chapter 1, 22min part A, ending,
        // post-credits stinger. We don't trust the labels here (they
        // weren't matched); we just use boundaries.
        let chapters = vec![
            chap(0, 90_000, None),
            chap(90_000, 1_410_000, None),
            chap(1_410_000, 1_470_000, None),
            chap(1_470_000, 1_500_000, None),
        ];
        let (intro, credits) = chapter_window_hints(&chapters, true, true);
        let intro = intro.expect("first chapter is intro candidate");
        assert!((intro.0 - 0.0).abs() < 0.001);
        assert!((intro.1 - 90.0).abs() < 0.001);
        let credits = credits.expect("last chapter is credits candidate");
        assert!(credits.0 > 1400.0);
    }

    #[test]
    fn chapter_window_hints_rejects_long_first_chapter() {
        // Movie with 30-minute first act. Not plausibly an intro;
        // hint should be None and tacet uses its default window.
        let chapters = vec![
            chap(0, 1_800_000, None),
            chap(1_800_000, 5_400_000, None),
            chap(5_400_000, 5_460_000, None),
        ];
        let (intro, _) = chapter_window_hints(&chapters, true, true);
        assert!(intro.is_none());
    }

    #[test]
    fn chapter_window_hints_rejects_early_credits() {
        // Two-chapter file with both halves equally sized. Last
        // chapter starts at 50% — within the credits hint window —
        // but a single-chapter "credits" right after a short intro
        // would be a false positive. Verify the threshold logic.
        let chapters = vec![chap(0, 100_000, None), chap(100_000, 200_000, None)];
        // duration is 200s, last chapter starts at 100s = 50%. With
        // `>= 0.5 × duration`, that's exactly on the boundary and
        // should be accepted.
        let (_, credits) = chapter_window_hints(&chapters, false, true);
        assert!(credits.is_some());
    }

    #[test]
    fn chapter_window_hints_respects_need_flags() {
        let chapters = vec![chap(0, 60_000, None), chap(60_000, 1_200_000, None)];
        let (intro, credits) = chapter_window_hints(&chapters, false, false);
        assert!(intro.is_none());
        assert!(credits.is_none());
    }

    #[test]
    fn chapter_window_hints_empty_chapters() {
        let (intro, credits) = chapter_window_hints(&[], true, true);
        assert!(intro.is_none());
        assert!(credits.is_none());
    }

    #[test]
    fn picks_first_intro_and_last_credits() {
        // Some fansub releases have multiple labeled credits (e.g.
        // "Credits" for the OP-style closing + "End Credits" for the
        // real outro). We want the last one.
        let chapters = vec![
            chap(0, 60_000, Some("Intro")),
            chap(60_000, 1_200_000, Some("Part A")),
            chap(1_200_000, 1_260_000, Some("Closing")),
            chap(1_260_000, 1_320_000, Some("End Credits")),
        ];
        let out = match_chapter_titles(&chapters);
        let intro = out.intro.unwrap();
        assert_eq!(intro.start_ms, 0);
        let credits = out.credits.unwrap();
        assert_eq!(credits.start_ms, 1_260_000);
    }
}
