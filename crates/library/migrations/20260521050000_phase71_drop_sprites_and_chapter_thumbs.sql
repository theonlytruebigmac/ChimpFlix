-- Phase 71 — Drop preview sprites + chapter thumbnails.
--
-- 2026-05-21 priority refocus: scrub-bar preview sprites and per-chapter
-- thumbnails are gone. They were the heaviest per-file jobs in the queue
-- (5–9 min per sprite × thousands of files) for features the user
-- doesn't need. Removing both is a direct win for library-import speed.
--
-- This migration is the keystone for the code removal — once it lands,
-- queries referencing the dropped columns won't compile. The matching
-- handlers, transcoder modules, API routes, and frontend UI are all
-- removed in the same commit.
--
-- Note: `probe_chapters` stays — `detect_markers_file` still uses it
-- as a credits-detection fallback for movies (no chapter THUMBS, but
-- chapter probing of the container is essentially free).

-- 1. Clear any queued jobs for the dropped kinds so workers don't trip
--    over them on next tick. ~3,500 sprite jobs were pending at the
--    time of this change. (Table name is `jobs`; the phase-57
--    migration file is named `phase57_jobs_queue.sql` but the
--    actual SQL creates `jobs`.)
DELETE FROM jobs
    WHERE kind IN ('generate_preview_sprite', 'build_chapter_thumbs');

-- 2. Drop the scheduled-task rows for the safety-net sweeps.
DELETE FROM scheduled_tasks
    WHERE kind IN ('generate_previews', 'generate_chapter_thumbs');

-- 3. Drop the partial index on the chapter-thumbs idempotency column
--    before the column itself goes.
DROP INDEX IF EXISTS idx_media_files_chapter_thumbs_pending;

-- 4. Drop sprite columns from media_files (phase 12b).
ALTER TABLE media_files DROP COLUMN preview_sprite_path;
ALTER TABLE media_files DROP COLUMN preview_interval_ms;
ALTER TABLE media_files DROP COLUMN preview_tile_width;
ALTER TABLE media_files DROP COLUMN preview_tile_height;
ALTER TABLE media_files DROP COLUMN preview_tile_cols;
ALTER TABLE media_files DROP COLUMN preview_tile_count;

-- 5. Drop chapter-thumb columns from media_files (phase 38).
ALTER TABLE media_files DROP COLUMN chapter_thumbs_generated_at;
ALTER TABLE media_files DROP COLUMN chapter_count;

-- 6. Drop the chapter_thumbs_enabled gate from server_settings (phase 59).
ALTER TABLE server_settings DROP COLUMN chapter_thumbs_enabled;
