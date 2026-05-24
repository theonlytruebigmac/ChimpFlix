-- Phase 90: backup retention cap. Without this, the daily backup task
-- writes to <data_dir>/backups/auto/ with no upper bound; over weeks
-- the directory fills the partition and the SQLite WAL stops
-- checkpointing. The post-backup prune step (server-side) honours
-- this value, deleting the oldest snapshots once the on-disk count
-- exceeds it. Default 14 ≈ two weeks at one snapshot per day, which
-- matches what most home Plex deployments retain manually.
--
-- See docs/PUBLIC_RELEASE_HARDENING.md BLOCK #4.

ALTER TABLE server_settings
    ADD COLUMN backup_retention_count INTEGER NOT NULL DEFAULT 14;
