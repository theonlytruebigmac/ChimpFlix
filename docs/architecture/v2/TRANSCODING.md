# V2 Transcoding — ffmpeg Pipeline Port

> Status: **RFC skeleton.** Carries forward from V1 with minimal
> changes. Stable subsystem.

## Scope

How V2 transcodes media files for adaptive playback, including direct-
play, HLS multi-rendition, hardware acceleration, HDR tonemapping,
subtitle handling, and intro/credits/loudness analysis.

## Carry forward from V1

The transcoder is one of V1's most-iterated subsystems and currently
works well. V2 ports it largely unchanged. What carries forward:

- ffmpeg + ffprobe subprocess model.
- Hardware acceleration: NVENC primary, CUDA decode, VAAPI/QSV/
  V4L2 capability-probed with graceful demotion when smoke tests
  fail.
- HEVC end-to-end (V1 Phase 43).
- HDR tonemap configuration.
- ABR with operator-configurable rendition ladder + bitrate ceiling.
- Per-user resolution + bitrate caps; operator ceiling on top.
- Subtitle burn-in for non-WebVTT-compatible formats.
- WebVTT pre-warming with bounded concurrency.
- Two-pass loudnorm (operator-tunable precision).
- Chapter probe.
- Tacet integration for intro / credits / loudness via single-decode
  fan-out (V1 Phase A through deferrals).
- Live transcode session priority (always wins over background
  precompute).
- Operator-tunable scanner ffmpeg nice level.

## What changes

- **Job-based precompute.** Background transcoder work (warmup,
  thumbnail generation if any, loudness analysis) routes through the
  V2 jobs subsystem with the `transcoder_bg` concurrency key.
- **Repository writes.** Any DB writes from transcoder go through
  the repository layer per `STORAGE.md`.

## Open questions

- **Cache layout.** V1 stores per-session HLS segments under
  `/data/cache/sessions/<id>/`. V2 keeps this; verify the operator
  can configure cache root.
- **Warm cache vs. cold cache hit ratio.** Worth measuring in Phase 3
  whether V2's job-priority model affects perceived play_start
  latency.
- **Software fallback.** V1 falls back to software encode when no HW
  encoder probes clean. V2 keeps this; document the perf expectation.

## Cut list

- **Sprite generation for scrub previews.** V1 Phase 71 removed.
  V2 does not restore.
- **Chapter thumbnail generation.** Same — removed in Phase 71, not
  restored.
- **Generic plugin transcoder.** ffmpeg is the only backend.
