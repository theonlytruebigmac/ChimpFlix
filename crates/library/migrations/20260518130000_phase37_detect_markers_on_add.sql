-- Phase 37 — opt-in "detect markers when media is added" trigger.
--
-- When ON, the file_watcher queues `chimpflix_transcoder::detect_markers`
-- for every new media file after each completed scan. Off by default
-- because blackdetect is expensive (~30s+ per 45-min episode); operators
-- with small libraries or strong hardware can flip it on for Plex-style
-- behavior where intros/credits are detected within minutes of a file
-- landing on disk.

ALTER TABLE server_settings ADD COLUMN detect_markers_on_add INTEGER NOT NULL DEFAULT 0;
