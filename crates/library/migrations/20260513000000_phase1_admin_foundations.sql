-- Phase 1 admin foundations: typed server settings + admin audit log.
--
-- server_settings is a singleton row (id = 1) holding all globally-configured
-- knobs the admin surface will manipulate. Typed columns for the values we
-- already know we need; extras_json is escape-hatch storage so phases 2-10
-- can add fields without each writing their own migration.
--
-- audit_log records administrative mutations only. Play/scrobble events stay
-- in their own existing tables (see play_state, scan_jobs, etc.).

CREATE TABLE server_settings (
    id                              INTEGER PRIMARY KEY CHECK (id = 1),
    server_name                     TEXT    NOT NULL DEFAULT 'ChimpFlix',
    public_url                      TEXT,
    cors_origins                    TEXT    NOT NULL DEFAULT '[]',  -- JSON array
    secure_connections              TEXT    NOT NULL DEFAULT 'preferred',  -- required|preferred|disabled
    telemetry_opt_in                INTEGER NOT NULL DEFAULT 0,
    transcoder_max_concurrent       INTEGER NOT NULL DEFAULT 2,
    transcoder_hw_accel             TEXT    NOT NULL DEFAULT 'none',  -- none|vaapi|nvenc|qsv|videotoolbox
    transcoder_quality_ceiling_kbps INTEGER,                          -- NULL = unlimited
    extras_json                     TEXT    NOT NULL DEFAULT '{}',
    updated_at                      INTEGER NOT NULL,
    updated_by                      INTEGER REFERENCES users(id) ON DELETE SET NULL
);

INSERT INTO server_settings (id, updated_at)
VALUES (1, CAST(strftime('%s','now') AS INTEGER) * 1000);

CREATE TABLE audit_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_user_id   INTEGER REFERENCES users(id) ON DELETE SET NULL,
    action          TEXT    NOT NULL,             -- e.g. 'settings.update', 'library.create'
    target_kind     TEXT,                         -- e.g. 'settings', 'library', 'user'
    target_id       TEXT,                         -- string-form so we can record non-int targets
    payload_json    TEXT,                         -- diff / extra context
    ip              TEXT,
    user_agent      TEXT,
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_audit_log_created_at ON audit_log(created_at DESC);
CREATE INDEX idx_audit_log_actor      ON audit_log(actor_user_id);
CREATE INDEX idx_audit_log_action     ON audit_log(action, created_at DESC);
