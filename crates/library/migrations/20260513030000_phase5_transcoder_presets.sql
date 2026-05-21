-- Phase 5: transcoder presets.
--
-- Presets describe target output profiles for HLS variants and the future
-- Optimized Versions feature (Phase 9). The set seeded here mirrors Plex's
-- defaults so the player picker has a familiar starting point. Owners can
-- add/remove/disable rows via the admin UI; the server treats the table
-- as the authoritative list.

CREATE TABLE transcoder_presets (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    name                    TEXT NOT NULL UNIQUE,
    max_video_bitrate_kbps  INTEGER NOT NULL,            -- 0 = no cap (passthrough)
    max_height              INTEGER NOT NULL,            -- 0 = no cap
    audio_codec             TEXT NOT NULL DEFAULT 'aac',
    audio_bitrate_kbps      INTEGER NOT NULL DEFAULT 192,
    enabled                 INTEGER NOT NULL DEFAULT 1,
    sort_order              INTEGER NOT NULL DEFAULT 0,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL
);

INSERT INTO transcoder_presets (name, max_video_bitrate_kbps, max_height, audio_codec, audio_bitrate_kbps, enabled, sort_order, created_at, updated_at) VALUES
    ('Original',     0,    0,    'aac', 192, 1, 0, CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),
    ('1080p 8Mbps',  8000, 1080, 'aac', 192, 1, 1, CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),
    ('720p 4Mbps',   4000, 720,  'aac', 192, 1, 2, CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000),
    ('480p 2Mbps',   2000, 480,  'aac', 128, 1, 3, CAST(strftime('%s','now') AS INTEGER) * 1000, CAST(strftime('%s','now') AS INTEGER) * 1000);
