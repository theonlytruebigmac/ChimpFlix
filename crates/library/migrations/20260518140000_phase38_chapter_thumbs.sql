-- Phase 38 — chapter thumbnails.
--
-- Plex's chapter-thumbs view extracts a frame near the start of each
-- chapter in a media file so the seek menu can show a poster strip
-- ("Cold Open" / "Act I" / "Credits").
--
-- We don't persist the chapter list itself — ffprobe surfaces it on
-- demand from the source file, which is authoritative and rare to
-- change without a rescan. The only persistent state needed is "have
-- we already processed this file?" so the scheduled task can skip
-- already-done work.
--
-- Thumbs land at `<data_dir>/chapter_thumbs/<media_file_id>/<chapter_index>.jpg`.
-- Files without any chapters still get `chapter_thumbs_generated_at`
-- stamped so the task doesn't re-probe them every run.

ALTER TABLE media_files ADD COLUMN chapter_thumbs_generated_at INTEGER;
ALTER TABLE media_files ADD COLUMN chapter_count INTEGER;
