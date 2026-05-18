-- Phase 19: orphan-file cleanup state.
--
-- Adds a soft-delete marker on `media_files` so the periodic library
-- verify pass can flag rows whose underlying disk file has gone away
-- without immediately destroying scan metadata, watch history, or
-- subtitle attachments. A separate hard-purge pass runs after a grace
-- period (default 7 days) and removes the row entirely — cascading
-- via the existing FK ON DELETE CASCADE chains to media_streams,
-- markers, preview_sprites, etc.
--
-- The grace window covers the common "drive temporarily unmounted"
-- and "moving files between mountpoints" cases without nuking the
-- entire library when verify runs against an empty mount.

ALTER TABLE media_files
    ADD COLUMN removed_at INTEGER;
    -- Epoch ms when verify first observed the file as missing.
    -- NULL = present, non-NULL = soft-deleted at that timestamp.

-- Index to speed up the "find expired soft-deleted rows" sweep.
CREATE INDEX idx_media_files_removed_at
    ON media_files(removed_at)
    WHERE removed_at IS NOT NULL;
