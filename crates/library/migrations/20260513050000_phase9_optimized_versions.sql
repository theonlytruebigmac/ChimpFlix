-- Phase 9: pre-transcoded media (Optimized Versions).
--
-- One row per (source_file_id, preset_id). status transitions:
--   queued -> running -> success | failed
-- The optimizer task (Phase 4 scheduler kind 'optimize_versions') runs the
-- queued ones; the player decision logic consults the success rows when
-- choosing direct-play vs transcode.

CREATE TABLE optimized_versions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    source_file_id      INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    preset_id           INTEGER NOT NULL REFERENCES transcoder_presets(id) ON DELETE CASCADE,
    output_path         TEXT NOT NULL,
    output_size_bytes   INTEGER,
    duration_ms         INTEGER,
    status              TEXT NOT NULL,                  -- 'queued' | 'running' | 'success' | 'failed'
    error               TEXT,
    created_at          INTEGER NOT NULL,
    completed_at        INTEGER,
    UNIQUE(source_file_id, preset_id)
);
CREATE INDEX idx_optimized_status ON optimized_versions(status);
CREATE INDEX idx_optimized_source ON optimized_versions(source_file_id);
