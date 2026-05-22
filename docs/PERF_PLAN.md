<!-- markdownlint-disable MD024 -->
<!-- Phases A-D deliberately share parallel subsection headings
     (Problem / Shape / Expected impact / Verification / etc.) so each
     phase block reads the same way. MD024 (no duplicate headings) is
     disabled for the whole doc; siblings_only would be cleaner but
     requires a repo-level markdownlint config that doesn't exist yet. -->

# Library-import performance plan

Created 2026-05-21 as the canonical roadmap for making library import
fast, transparent, and efficient enough to be a credible Plex
replacement. Distilled from the architecture discussion that followed
the phase-71 drop of preview sprites and chapter thumbnails.

This document is the source of truth for the design decisions. The
phase blocks below are tracked work items; when one ships, append a
"shipped" note rather than rewriting it, so the reasoning trail stays
visible.

---

## Operator bar

What users actually expect from the system:

- **Instant**: media files appear in the library and are playable as
  soon as the scanner has written their row. ffprobe at scan time is
  the only synchronous step; everything else is background.
- **Background but bounded**: marker detection, loudness, metadata
  enrichment, etc., progress visibly over time. "Background" does not
  mean "a week." A normal 1,000-file library should be fully enriched
  in hours; a Sonarr season-drop should drain in tens of minutes.
- **Transparent**: the queue is observable. The operator can see what
  is being worked on, how much is left, and roughly when it will be
  done. The Plex "Generating markers…" black box is the anti-pattern.
- **Honest about failures**: when something fails, the operator can
  see *why* without digging into logs.

Throughput is a *means* to those ends, not an end in itself. We do
not need to beat ffmpeg on raw decode speed. We need to do the right
amount of work and clearly communicate that we are doing it.

---

## Design principles

These came out of the architecture discussion and constrain the
choices below:

1. **Don't decode what we don't have to.** The cheapest decode is the
   one that never runs. Probe the container for cheap signals (chapter
   labels, embedded markers) before reaching for audio analysis.
2. **Decode once, fan out.** When audio analysis is unavoidable, a
   single decode pass should feed every consumer that wants the audio
   (markers, loudness, future audio features). Sequential decode
   passes for the same file are the bottleneck.
3. **Tacet is the audio-analysis crate.** Not "tacet is our ffmpeg
   tool." Container probing (ffprobe), transcoding (live HLS sessions),
   video frame extraction — these stay in ChimpFlix's transcoder crate.
   Audio decode + analysis lives in tacet. Loudness moves *into* tacet
   to share the decode pipeline; it doesn't stay in a separate ffmpeg
   process.
4. **Loudness stays gated/opt-in.** Default off. The architecture has
   to honor the gate cleanly: when off, no loudness work runs and no
   decode capacity is wasted speculating about it.
5. **Observability is a first-class API concern, not a bolt-on.**
   Per-stage timing, progress events, and cancellation tokens go into
   tacet's API from the start. Retrofitting them later is ugly.
6. **Job-queue kinds stay separate.** `detect_markers_file` and
   `analyze_loudness` remain distinct job kinds (queue UI, gate
   semantics, retry policy, safety-net sweeps depend on this). They
   coordinate at the handler level so that when both are pending for
   the same file, one handler does both via tacet's combined entry
   point.
7. **Per-marker-kind granularity, not per-file all-or-nothing.** Intro
   and credits are evaluated independently. Having a labeled "Opening"
   chapter and no labeled credits should mean "use the intro, run
   tacet for credits only" — not "trust everything" or "trust nothing."

---

## Phase A — Chapter-first short-circuit

**Self-contained ChimpFlix change. No tacet API change. Ship first.**

> **✅ Shipped 2026-05-21.** See [memory: project_phase_a_shipped_2026_05_21](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_phase_a_shipped_2026_05_21.md). Chapter title matcher, per-marker-kind granularity, expanded source taxonomy (`embedded` / `tacet` / `blackframe` / `manual` / legacy `auto`), MarkerEditor source badge, and dead-code cleanup of the legacy `transcoder::markers` module. Six unit tests cover the matcher. Verified `cargo check --tests` + `tsc` + `eslint` clean.

### Problem

`detect_markers_file` currently calls tacet for every file, regardless
of whether the container already has labeled chapter markers that
answer the question for free. Anime releases from Sonarr almost always
embed Matroska chapters named "Opening" / "Ending"; many Blu-ray rips
have semantically labeled chapters too. For those files, audio
fingerprinting is wasted CPU.

### Shape

Before any tacet work in `detect_markers_file`:

1. Call `probe_chapters` on the file (~50ms, already implemented in
   `crates/transcoder/src/probe.rs`).
2. Pattern-match chapter titles against intro/credits/outro
   conventions (case-insensitive regex). Suggested coverage:
   - Intro: `intro`, `opening`, `op`, `theme`, `opening theme`,
     `opening sequence`
   - Credits: `credits`, `end credits`, `outro`, `ending`, `ed`,
     `ending theme`, `closing`, `closing credits`
3. Decide per-marker-kind what's still needed:
   - Both labeled → write markers, stamp, return. No audio decode.
   - One labeled → write that one; pass a narrowed window hint to
     tacet for the other (Phase C dependency for window hints; until
     Phase C lands, the handler can still write the one labeled
     marker and run full tacet for the other).
   - Unlabeled chapters present → fall through to tacet; record the
     chapter boundaries so they can become window hints once Phase C
     lands.
   - No chapters → full tacet, current behavior.

### Schema / API changes

- New `MarkerSource` variant: `EmbeddedLabeled` (chapter title
  matched a pattern). Possibly also `EmbeddedPositional` for the
  future case where unnamed chapters at expected positions are
  trusted; defer adding until Phase C.
- The MarkerEditor UI shows a source badge so operators can see which
  markers came from chapter labels vs. tacet analysis. Useful when a
  mislabeled chapter produces wrong markers.

### Expected impact

For an anime-heavy library: ~70-90% of episodes short-circuit with
zero audio decode. For a live-action library with no labeled chapters:
no impact from Phase A; Phase C is what helps there.

### Verification

- Manual: point at a known-labeled anime file, confirm markers come
  out matching the chapter timestamps with `EmbeddedLabeled` source.
- Manual: point at a live-action file with no chapters, confirm
  fall-through to tacet still works.
- Regression: existing skip-intro UX should be unchanged for files
  that were previously detected by tacet.

---

## Phase B — Bootstrap redundant-decode fix

**Small tacet handler edit. No tacet API addition.**

> **✅ Shipped 2026-05-21.** See [memory: project_phase_b_shipped_2026_05_21](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_phase_b_shipped_2026_05_21.md). Bootstrap handler now calls `tacet::detect_season` and persists per-episode markers in one decode pass; the "re-enqueue per-episode detection" block is gone. New `list_episodes_in_season_for_detection` query returns file_id + path + episode_number so tacet's synthetic episode_ids map back cleanly. Verified `cargo test` (158 pass) + `clippy -D warnings` + `tsc` + `eslint`.

### Problem

`bootstrap_season_refs` currently calls `tacet::detection::bootstrap_season`,
which decodes + fingerprints every episode in a season and *returns
only the season references*, throwing away the per-episode
fingerprints. The handler then re-enqueues `detect_markers_file` for
every episode in the season, which decodes the same audio *again* to
match against the references that were just produced from it.

Net: every episode in a freshly-bootstrapped season is decoded twice.
For a 13-episode season that's ~2 hours of redundant CPU work per
bootstrap.

### Shape

Switch the bootstrap handler to use `tacet::detection::detect_season`,
which already exists in tacet and produces both the references *and*
the per-episode markers from a single decode pass. The handler then:

1. Persists the references (as today).
2. Persists the per-episode marker rows in the same transaction.
3. Stamps `markers_detected_at` on every episode.
4. Drops the "re-enqueue per-episode detection" block entirely — the
   work is done.

`detect_markers_file` stays as the incremental path for episodes that
arrive after their season has already been bootstrapped. Its purpose
is unchanged.

### Schema / API changes

None. Tacet already exposes `detect_season`; the handler just needs
to map tacet's synthetic episode ids back to file_ids and write the
markers.

### Expected impact

Eliminates the ~13× per-episode redundant decode for any season that
does need tacet (i.e., didn't short-circuit via Phase A's chapter
labels). For the user's current 2,875-row bootstrap queue, this is
the difference between "weeks" and "overnight."

### Verification

- Manual: trigger a bootstrap for a known season, confirm markers
  appear for every episode in one job rather than waiting for N
  follow-up detect_markers_file jobs.
- Regression: incremental detect_markers_file for an episode added
  after bootstrap completes should still work normally.

---

## Phase C — Tacet redesign as multi-consumer audio analysis crate

**The big lift. Spans tacet and ChimpFlix. Design as Tier 4 from the
start — do not ship a stepping-stone "tacet shells out to ffmpeg for
loudness" version that gets thrown away.**

> **✅ Shipped 2026-05-21 in full.** See [memory: project_phase_c_shipped_2026_05_21](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_phase_c_shipped_2026_05_21.md) for the initial drop and [memory: project_perf_plan_deferrals_shipped_2026_05_21](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_perf_plan_deferrals_shipped_2026_05_21.md) for the deferrals knock-out. All three deferred Phase C items now landed:
>
> - **Tacet owns all audio-domain analysis** — new `tacet::loudness` module (pure-Rust `ebur128`, ffmpeg fallback) + new `tacet::analyze::analyze_audio` unified entry point. Both ChimpFlix handlers call it. Legacy `transcoder::loudness` deleted.
> - **True single-decode fan-out** — `fused_decode` runs one symphonia pass that feeds the ebur128 analyser + intro window mono buffer + credits window mono buffer. `analyze_audio` routes to it when both markers and loudness are requested; falls back to sequential paths when only one is requested or when symphonia rejects the codec. ~2× CPU reduction for loudness-on operators.
> - **Window hints from unlabeled chapters** — `MarkerRequest` carries `intro_window_hint` / `credits_window_hint`; tacet's new `detect_single_episode_with_hints` honors them; ChimpFlix's `chapter_window_hints` heuristic derives them from first/last chapter boundaries when chapter titles aren't matchable.
> - **Cancellation + progress** — wired through analyze_audio AND emitted by the fused-decode path. The handler-side sink is plumbed via tokio task-local (see Phase D).

### Problem

Today the per-file audio path is at minimum two full decode passes
when loudness is on: one for tacet's fingerprint analysis, one for
ffmpeg's `ebur128` loudness measurement. Future audio features
(silence detection, ad-break candidates, dialog-level analysis) would
each add another decode pass. The architecture doesn't scale.

### Shape

Tacet's identity broadens from "intro/credits fingerprinting" to
"single-pass audio analysis for media files." Same focus on audio,
expanded scope.

New tacet entry point:

```rust
pub fn analyze_audio(
    path: &Path,
    request: AnalysisRequest,
    progress: Option<ProgressSink>,
    cancel: CancellationToken,
) -> Result<AnalysisResult>;

pub struct AnalysisRequest {
    pub markers: Option<MarkerRequest>,    // refs + optional window hints
    pub loudness: bool,
    pub blackframe_scan: Option<TimeRange>, // for credits fallback
}

pub struct MarkerRequest {
    pub intro_refs: Vec<ReferenceFingerprint>,
    pub credits_refs: Vec<ReferenceFingerprint>,
    pub intro_window_hint: Option<TimeRange>,   // narrows decode/FFT
    pub credits_window_hint: Option<TimeRange>,
}

pub struct AnalysisResult {
    pub markers: Option<SegmentMarkers>,
    pub loudness: Option<LoudnessMeasurement>,
    pub stage_timings: StageTimings, // for observability
}
```

Inside tacet, one symphonia decode pass produces PCM samples that
are fanned to whichever consumers were requested. When loudness is
off, the ebur128 path doesn't run and no samples are duplicated for
it. When window hints are present, the decoder seeks to the hinted
ranges instead of full-file decoding.

### Loudness implementation in Rust

ffmpeg's `ebur128` filter is the current source of truth for loudness
measurements; we need to match it closely. Approach:

1. Pick a Rust EBU R 128 crate. Top candidates: `ebur128` (libebur128
   Rust bindings, most accurate) or `bs1770` (pure Rust, possibly
   less battle-tested).
2. Implement the loudness consumer in tacet against PCM samples from
   symphonia.
3. Validate output against ffmpeg's `ebur128` filter on a corpus of
   reference files. Tolerance: ~0.1 LU on integrated loudness, ~0.5
   LU on LRA, ~0.5 dBTP on true peak. If we can't hit those, the
   crate isn't suitable.
4. Only after validation passes, switch `analyze_loudness` handler to
   call tacet instead of shelling out to ffmpeg.

### Handler coordination

- `detect_markers_file` reads the loudness gate. When the gate is on
  *and* the file has no `loudnorm_analyzed_at` stamp, it requests
  `{ markers: Some(...), loudness: true }` from tacet and writes both
  markers and loudness in one transaction.
- `analyze_loudness` stays as a distinct job kind for the safety-net
  sweep (files that pre-date the markers pipeline, files where the
  gate was flipped on after import).
- The two kinds keep separate concurrency caps, separate gate
  semantics, separate retry policies. Coordination is at the handler
  layer, not the queue layer.

### Observability through tacet's API

The `ProgressSink` callback emits milestone events the handler can
forward into the activity feed:

```rust
ProgressEvent::DecodeStarted { duration_ms }
ProgressEvent::DecodeProgress { sample_position_ms }
ProgressEvent::FingerprintStarted
ProgressEvent::FingerprintProgress { window_kind, progress: f32 }
ProgressEvent::MatchStarted
ProgressEvent::Complete
```

`StageTimings` in the result is for the activity-feed display:

```text
detect_markers_file done in 4m 12s
  decode: 3m 02s
  fingerprint: 1m 04s
  match: 6s
```

That level of detail differentiates from Plex's opaque "Generating
markers…" by an order of magnitude.

### Cancellation

`CancellationToken` is polled at sample-chunk boundaries inside the
decode loop. When the operator hits "pause queue" the in-flight
worker yields within ~1 second instead of finishing its 8-minute file.

### Memory pressure

Multi-consumer fan-out buffering PCM samples needs bounded channels.
Use `tokio::sync::broadcast` with a small buffer (e.g., 256 chunks of
1024 samples) so consumers exert backpressure on the decoder if any
one of them stalls. Test with 4-hour movies before declaring done.

### Schema / API changes

- New tacet public API (above).
- ChimpFlix's `analyze_loudness` handler is rewritten to call tacet.
- ChimpFlix's `detect_markers_file` handler is rewritten to call the
  combined entry point.
- DB schema unchanged (markers + loudness already have their own
  tables/columns).

### Expected impact

For loudness-on operators: ~2× reduction in per-file CPU during
discovery. For loudness-off operators: no immediate throughput change,
but sets up future audio analyses cleanly. For everyone: per-stage
timing and progress visibility in the activity feed; cancellation
that actually works.

### Verification

- Loudness measurement parity: bulk-compare tacet vs. ffmpeg
  loudnorm on a corpus of ~50 reference files. Confirm within
  tolerance.
- Cancellation: trigger a long detect_markers_file job, hit pause,
  confirm the worker exits within 1-2 seconds.
- Memory: run a backfill against a library containing 4-hour movies,
  confirm no OOM, confirm RSS stays bounded.
- Regression: existing markers + loudness measurements should remain
  consistent (or differ only within tolerance) after switching
  handlers.

### Non-goals for Phase C

- Hardware-accelerated decode (NVENC/VAAPI for analysis). Real
  differentiator long-term, but a Phase E or later concern. Design
  tacet's public API so it could plug in later without breaking
  callers, but don't implement.
- Generic "open file once, fan to video + audio + container consumers"
  pipeline. Tacet handles audio; the transcoder crate handles video;
  no plan to unify those yet.

---

## Phase D — Observability polish

**ChimpFlix UI work. Depends on Phase C for the event sources.**

> **✅ Shipped 2026-05-21 (mostly complete).** See [memory: project_phase_d_shipped_2026_05_21](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_phase_d_shipped_2026_05_21.md) for the initial drop and [memory: project_perf_plan_deferrals_shipped_2026_05_21](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_perf_plan_deferrals_shipped_2026_05_21.md) for the deferrals knock-out. All but one item landed:
>
> - **Wall-clock ETA per kind** — server computes `queue × p95 ÷ effective_concurrency` (honors per-kind concurrency overrides). UI renders `~30 min` / `~2h 15m` / `~3d 4h` with explanatory tooltip.
> - **Per-job live progress** — workers install a `JobContext` task-local with a `ProgressSink` that writes into an in-memory `JobProgressStore`. Handlers pull the sink via `JobContext::current()` and pass it to `tacet::analyze::analyze_audio`. UI renders a pulsing "Loudness · decoding · 42%" line for in-flight jobs. Handler signatures unchanged — opt-in via task-local.
> - **Per-stage timing in the failure feed** — new `jobs.stage_timings_json` column (phase-72 migration); handlers serialize `StageTimings` after `analyze_audio` returns. UI renders "decode 3m 02s · fingerprint 1m 04s · loudness 67s" inline.
>
> **Still deferred (genuinely future work):**
>
> - **Structured failure-reason surfacing** — `error_class` already exists and is shown as a UI pill; richer taxonomy is a separate design discussion, not implementation work.

---

## Bootstrap efficiency follow-up (2026-05-21)

After the deferrals knock-out, the user asked why `bootstrap_season_refs` was decoding every episode when the per-file `detect_markers_file` would touch them again anyway. Investigation showed the design was already correct post-Phase-B (one decode per file, not two — bootstrap writes markers directly), but two real optimizations remained:

> **✅ Shipped 2026-05-21.** See [memory: project_bootstrap_efficiency_2026_05_21](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_bootstrap_efficiency_2026_05_21.md).
>
> - **Skip-when-redundant** — new `queries::count_episodes_needing_markers_in_season` query gates `maybe_enqueue_bootstrap` and `bootstrap_season_refs::run`. Bootstrap doesn't fire when every episode in the season already has markers (Phase A chapter labels, manual edits). Zero decode work for anime libraries where labels cover everything.
> - **Adaptive scan window** — new `tacet::detection::detect_season_adaptive` tries a narrow 5-min intro / 4-min credits window first; falls back to the configured wide window (default 18 / 8) only when the narrow attempt fails to find cross-episode consensus. ~3-4× decode reduction for typical TV (intro in first 3-5 min); slightly worse for outliers like Silo S01E05 (intro at 14:28) where narrow fails and we pay both attempts.
> - **Per-episode timing diagnostics** — tacet's `analyze_episode` logs `intro_ms`, `credits_ms`, `total_ms` per episode; bootstrap handler logs `detect_elapsed_ms` + `per_episode_avg_ms`. Operator can grep the activity log to see exactly where the wall-clock goes.
>
> **Honest about limits:** for content like Silo (no labeled chapters + outlier episode with intro at 14:28), adaptive scanning does NOT help — the conservative 18-min default exists *because* of Silo. The bootstrap takes 14-15 min because tacet has to decode ~26 min of audio per episode × 10 episodes on the user's hardware. Diagnostics will reveal whether that time is genuinely I/O-bound or whether one episode is anomalously slow.

### Shape

Once Phase C is emitting progress events and stage timings:

- **Wall-clock ETA in the per-kind table.** Queue depth × p95-per-kind
  → a "~18 minutes remaining" estimate. Update on every poll tick.
- **Per-job progress percentage.** When tacet emits
  `DecodeProgress { sample_position_ms }`, the activity feed shows
  "Decoding · 42%" instead of just "running." Same for fingerprint
  and match stages.
- **Per-stage timing in the failure feed.** When a job fails, the
  partial `StageTimings` tell the operator where it died. "Failed
  during fingerprint at 1m 12s" beats "Failed at 4m 30s" for
  diagnosis.
- **Visible failure reasons.** Surface the structured error class
  (timeout, decode error, no audio stream, etc.) directly in the
  activity feed, not just the job-detail drawer.

### Schema / API changes

- Job-progress table or in-memory ring buffer for live progress
  events. SSE or polling for the UI to consume them.
- Job row gets a `progress_json` column or similar for the latest
  stage + percentage. Reset on each retry.

### Expected impact

This is the "feels good" layer. No throughput change, but the
difference between Plex's opaque status bar and ChimpFlix's
"decoding 42%, ~3 min remaining" is the differentiator that operators
will actually notice.

---

## Out of scope / explicitly decided against

- **Re-adding preview sprites.** Dropped in phase 71. Decision is
  final unless explicitly revisited. The scrub-bar hover preview
  feature is gone and not coming back.
- **Re-adding chapter thumbnails.** Same. The chapter menu UI is gone;
  `probe_chapters` is retained only as an internal helper for
  marker-detection fallback and for Phase A's chapter-label probe.
- **Tacet absorbing video work** (sprite gen, chapter thumb
  extraction, anything frame-based). Even if the features came back,
  they belong in the transcoder crate. Tacet is audio.
- **Moving the transcoder/HLS pipeline into tacet.** Different
  lifetime, different concerns. Live playback sessions vs. analysis
  jobs are unrelated even though both use ffmpeg.

---

## Ordering and dependencies

```text
Phase A (chapter-first)  ──┐
                           ├── independent, ship in any order
Phase B (bootstrap fix)  ──┘

Phase C (tacet redesign) ── depends on nothing structurally, but A+B
                            should ship first since they're cheap and
                            inform whether C's full scope is needed
                            for the user's actual workload.

Phase D (observability)  ── depends on Phase C emitting progress events.
                            Some pieces (ETA from queue depth) could
                            ship earlier independently.
```

Phases A and B can be days each. Phase C is multiple sessions and
spans both repos (tacet + ChimpFlix). Phase D is incremental as
Phase C's API events become available.

---

## Glossary / references

- **tacet**: vendored at [`crates/tacet-core`](../crates/tacet-core/),
  also published. We own it and can change its API freely.
- **bootstrap_season_refs**: [crates/server/src/jobs/handlers/bootstrap_season_refs.rs](../crates/server/src/jobs/handlers/bootstrap_season_refs.rs)
- **detect_markers_file**: [crates/server/src/jobs/handlers/detect_markers_file.rs](../crates/server/src/jobs/handlers/detect_markers_file.rs)
- **analyze_loudness**: [crates/server/src/jobs/handlers/analyze_loudness.rs](../crates/server/src/jobs/handlers/analyze_loudness.rs)
- **probe_chapters**: [crates/transcoder/src/probe.rs](../crates/transcoder/src/probe.rs)
- **MarkerSource enum**: [crates/library/src/models.rs](../crates/library/src/models.rs)
- **Phase 71 (dropped sprites + chapter thumbs)**: see
  [memory: project_drop_sprites_chapter_thumbs_2026_05_21](../../home/dev/.claude/projects/-mnt-data-GitHub-ChimpFlix/memory/project_drop_sprites_chapter_thumbs_2026_05_21.md)
