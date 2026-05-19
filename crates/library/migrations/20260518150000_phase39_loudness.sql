-- Phase 39 — per-file loudness analysis (EBU R 128 / ffmpeg loudnorm).
--
-- Stores the four measurements ffmpeg's loudnorm filter produces in
-- print mode: integrated loudness (LUFS), true peak (dBTP), loudness
-- range (LU), and noise floor / threshold. With these stamped per
-- file, the transcoder can drive a *second-pass* loudnorm with
-- precise targets (vs. the generic one-pass `I=-16:LRA=11:TP=-1.5`)
-- so volume stays consistent across the library, matching Plex's
-- "loudness analysis" feature.
--
-- Also adds `audio_normalize_enabled` to server_settings so the
-- operator can default normalization on for every session (still
-- overridable per-session via the player's audio settings).

ALTER TABLE media_files ADD COLUMN loudnorm_integrated REAL;
ALTER TABLE media_files ADD COLUMN loudnorm_true_peak REAL;
ALTER TABLE media_files ADD COLUMN loudnorm_lra REAL;
ALTER TABLE media_files ADD COLUMN loudnorm_threshold REAL;
ALTER TABLE media_files ADD COLUMN loudnorm_analyzed_at INTEGER;

ALTER TABLE server_settings ADD COLUMN audio_normalize_enabled INTEGER NOT NULL DEFAULT 0;
