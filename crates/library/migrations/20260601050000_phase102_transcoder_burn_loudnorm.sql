-- Phase 102: two transcoder feature toggles surfaced on the
-- Settings → Transcoding page (matching the redesign mockup).
--
--   - `transcoder_burn_ass_subtitles` — master gate for burning
--     text subtitles (SRT / ASS / SSA / mov_text) into the video via
--     libavfilter's `subtitles=` filter. When false, a selected text
--     subtitle takes the existing WebVTT-sidecar path (player overlays
--     it client-side) instead of being baked into the pixels. Picture
--     subs (PGS / VobSub / DVB) are unaffected — they always overlay
--     because browsers can't render bitmaps as a separate track.
--     Default OFF preserves the sidecar-first behavior shipped today
--     (so the off-path transcode command is byte-identical).
--
--   - `transcoder_two_pass_loudnorm` — when true, EBU R 128 volume
--     leveling uses the precise per-file measurements stamped by the
--     `analyze_loudness` task (loudnorm `linear=true`, measure-then-
--     apply) instead of the single-pass streaming-window estimate.
--     Only consulted when normalization is actually engaged
--     (`audio_normalize_enabled` / the per-session toggle). Defaults ON
--     (1) to PRESERVE prior behavior: before this toggle, two-pass was
--     used automatically whenever measurements existed + normalization
--     was on. Installs without stored measurements fall back to
--     single-pass regardless; turning this OFF forces single-pass even
--     when measurements exist.

ALTER TABLE server_settings
    ADD COLUMN transcoder_burn_ass_subtitles INTEGER NOT NULL DEFAULT 0;

ALTER TABLE server_settings
    ADD COLUMN transcoder_two_pass_loudnorm INTEGER NOT NULL DEFAULT 1;
