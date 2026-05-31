-- Phase 94: Plex-style periodic library scan + empty-trash-after-scan.
--
-- The filesystem watcher (`scan_automatically`) fires scans on inotify
-- events, but inotify is unreliable: it sees nothing on NFS/SMB mounts,
-- can silently exceed the kernel's max_user_watches limit, and misses
-- events that don't propagate into a container's mount namespace. A
-- periodic safety-net scan — exactly Plex's "Scan my library
-- periodically" — guarantees the library converges even when the watcher
-- misses changes.
--
-- `periodic_scan_enabled`   : master toggle. Default ON (Plex parity).
-- `periodic_scan_frequency` : one of the scheduler frequency tokens
--                             (every_15_minutes | every_30_minutes |
--                             hourly | every_2_hours | every_6_hours |
--                             every_12_hours | daily). Default hourly,
--                             matching Plex's default. Drives the
--                             `periodic_library_scan` scheduled task.
-- `empty_trash_after_scan`  : when ON, a completed scan immediately
--                             hard-deletes that library's soft-removed
--                             files instead of waiting for the 7-day
--                             `purge_removed_files` grace window. Plex's
--                             "Empty trash automatically after every
--                             scan". Default OFF — a temporary unmount
--                             shouldn't nuke play state / markers.

ALTER TABLE server_settings
    ADD COLUMN periodic_scan_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE server_settings
    ADD COLUMN periodic_scan_frequency TEXT NOT NULL DEFAULT 'hourly';
ALTER TABLE server_settings
    ADD COLUMN empty_trash_after_scan INTEGER NOT NULL DEFAULT 0;
