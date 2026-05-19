-- Phase 33: per-library "allow operator to delete media files" toggle.
--
-- ChimpFlix has always had a *soft*-delete path (scanner marks
-- missing files; the `purge_removed_files` task hard-deletes after
-- a grace window). What's been missing is an operator-initiated
-- "delete this file from disk right now" surface — the Plex
-- equivalent of the item context menu's "Delete from library".
--
-- We gate this per-library so a casual operator can't blow away
-- their movie collection by clicking the wrong button. Default
-- false; the library admin page surfaces a checkbox to opt in.
--
-- Permanence note: a delete via this flow does NOT use the 7-day
-- grace window. It collects paths + preview sprites, drops the
-- media_files row (cascading media_streams / markers /
-- optimized_versions), sweeps orphaned episodes/seasons/items,
-- and unlinks the file from disk. There is no undo.

ALTER TABLE libraries
    ADD COLUMN allow_media_deletion INTEGER NOT NULL DEFAULT 0;
