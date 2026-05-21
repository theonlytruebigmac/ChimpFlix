-- Per-user playback defaults. ISO 639-2 codes (e.g. "eng", "spa") matching
-- what we store in media_streams.language. NULL means "let the server pick
-- the first track" (audio) or "no subtitles" (subtitle).

ALTER TABLE users ADD COLUMN default_audio_lang TEXT;
ALTER TABLE users ADD COLUMN default_subtitle_lang TEXT;
