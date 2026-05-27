-- Phase 91: polling-watcher backend for network-mounted libraries.
--
-- The default notify backend on Linux is inotify, which does not see
-- filesystem events on NFS/SMB/CIFS mounts — events fired on the remote
-- server never reach the local kernel's watcher. Operators running their
-- media drive on NFS (common Docker-on-NAS pattern) were silently never
-- getting auto-scans, and had to fall back to manual scans on every add.
--
-- Toggling this on swaps the watcher to notify::PollWatcher, which
-- recursively stats watched paths every `file_watcher_poll_interval_secs`
-- and synthesizes Create/Remove/Modify events from the diff. Higher CPU
-- + I/O cost than inotify, but it works against remote mounts and bind
-- mounts that don't propagate inotify into the container namespace.
--
-- Read once at startup like `scan_automatically`; toggling requires a
-- server restart to re-arm with the new backend.

ALTER TABLE server_settings
    ADD COLUMN file_watcher_use_polling INTEGER NOT NULL DEFAULT 0;
ALTER TABLE server_settings
    ADD COLUMN file_watcher_poll_interval_secs INTEGER NOT NULL DEFAULT 30;
