-- Phase 34: master toggle for the filesystem watcher.
--
-- The notify-backed file_watcher gives second-latency library updates
-- when media files appear/disappear on disk. Most operators want it on
-- (matches Plex's "Scan my library automatically"); a few — running
-- against network filesystems where notify is unreliable, or wanting
-- strict "scans only when I say so" semantics — want it off.
--
-- Gated at startup: main.rs reads the setting and decides whether to
-- spawn the watcher. Changes require a server restart; surfaced in the
-- admin UI as "Restart pending" when the operator toggles it.

ALTER TABLE server_settings
    ADD COLUMN scan_automatically INTEGER NOT NULL DEFAULT 1;
