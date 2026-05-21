-- ChimpFlix v0.1 initial schema.
-- See docs/SCHEMA.md for the design rationale.
--
-- Conventions:
--   * All *_at columns are Unix epoch milliseconds (INTEGER).
--   * Booleans are INTEGER 0/1.
--   * Foreign keys cascade unless noted.

PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- Users, sessions, invites
-- ---------------------------------------------------------------------------

CREATE TABLE users (
    id              INTEGER PRIMARY KEY,
    username        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash   TEXT NOT NULL,
    role            TEXT NOT NULL,
    display_name    TEXT,
    avatar_path     TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE invites (
    id              INTEGER PRIMARY KEY,
    code            TEXT NOT NULL UNIQUE,
    created_by      INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at      INTEGER,
    consumed_by     INTEGER REFERENCES users(id) ON DELETE SET NULL,
    consumed_at     INTEGER,
    created_at      INTEGER NOT NULL
);

CREATE TABLE sessions (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nonce           BLOB NOT NULL,
    user_agent      TEXT,
    ip              TEXT,
    last_seen_at    INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);

-- ---------------------------------------------------------------------------
-- Libraries
-- ---------------------------------------------------------------------------

CREATE TABLE libraries (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL,
    scan_interval_s INTEGER NOT NULL DEFAULT 3600,
    last_scan_at    INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE library_paths (
    id              INTEGER PRIMARY KEY,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    path            TEXT NOT NULL,
    UNIQUE(library_id, path)
);

CREATE TABLE library_access (
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, library_id)
);

-- ---------------------------------------------------------------------------
-- Media items (movies, shows, episodes)
-- ---------------------------------------------------------------------------

CREATE TABLE items (
    id              INTEGER PRIMARY KEY,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    title           TEXT NOT NULL,
    sort_title      TEXT NOT NULL,
    original_title  TEXT,
    summary         TEXT,
    tagline         TEXT,
    year            INTEGER,
    rating_age      TEXT,
    rating_audience REAL,
    duration_ms     INTEGER,
    tmdb_id         INTEGER,
    imdb_id         TEXT,
    tvdb_id         INTEGER,
    added_at        INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    refreshed_at    INTEGER
);
CREATE INDEX idx_items_library_kind ON items(library_id, kind);
CREATE INDEX idx_items_added ON items(added_at DESC);
CREATE INDEX idx_items_tmdb ON items(tmdb_id) WHERE tmdb_id IS NOT NULL;

CREATE TABLE seasons (
    id              INTEGER PRIMARY KEY,
    show_id         INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    season_number   INTEGER NOT NULL,
    title           TEXT,
    summary         TEXT,
    tmdb_id         INTEGER,
    UNIQUE(show_id, season_number)
);

CREATE TABLE episodes (
    id              INTEGER PRIMARY KEY,
    season_id       INTEGER NOT NULL REFERENCES seasons(id) ON DELETE CASCADE,
    episode_number  INTEGER NOT NULL,
    title           TEXT NOT NULL,
    summary         TEXT,
    air_date        INTEGER,
    duration_ms     INTEGER,
    tmdb_id         INTEGER,
    added_at        INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE(season_id, episode_number)
);
CREATE INDEX idx_episodes_season ON episodes(season_id);

-- ---------------------------------------------------------------------------
-- Media files & streams
-- ---------------------------------------------------------------------------

CREATE TABLE media_files (
    id              INTEGER PRIMARY KEY,
    item_id         INTEGER REFERENCES items(id) ON DELETE CASCADE,
    episode_id      INTEGER REFERENCES episodes(id) ON DELETE CASCADE,
    path            TEXT NOT NULL UNIQUE,
    size_bytes      INTEGER NOT NULL,
    mtime_ms        INTEGER NOT NULL,
    container       TEXT,
    duration_ms     INTEGER,
    bit_rate        INTEGER,
    width           INTEGER,
    height          INTEGER,
    hdr_format      TEXT,
    scanned_at      INTEGER NOT NULL,
    CHECK ((item_id IS NULL) <> (episode_id IS NULL))
);
CREATE INDEX idx_media_files_item ON media_files(item_id);
CREATE INDEX idx_media_files_episode ON media_files(episode_id);

CREATE TABLE media_streams (
    id              INTEGER PRIMARY KEY,
    media_file_id   INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    stream_index    INTEGER NOT NULL,
    kind            TEXT NOT NULL,
    codec           TEXT,
    profile         TEXT,
    language        TEXT,
    title           TEXT,
    pix_fmt         TEXT,
    frame_rate      REAL,
    channels        INTEGER,
    channel_layout  TEXT,
    sample_rate     INTEGER,
    is_forced       INTEGER NOT NULL DEFAULT 0,
    is_default      INTEGER NOT NULL DEFAULT 0,
    is_external     INTEGER NOT NULL DEFAULT 0,
    external_path   TEXT,
    UNIQUE(media_file_id, stream_index, is_external)
);
CREATE INDEX idx_streams_file ON media_streams(media_file_id);

-- ---------------------------------------------------------------------------
-- Metadata side tables
-- ---------------------------------------------------------------------------

CREATE TABLE images (
    id              INTEGER PRIMARY KEY,
    item_id         INTEGER REFERENCES items(id) ON DELETE CASCADE,
    episode_id      INTEGER REFERENCES episodes(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    source          TEXT NOT NULL,
    source_url      TEXT,
    local_path      TEXT,
    width           INTEGER,
    height          INTEGER,
    is_primary      INTEGER NOT NULL DEFAULT 0,
    CHECK ((item_id IS NULL) <> (episode_id IS NULL))
);
CREATE INDEX idx_images_item ON images(item_id, kind);
CREATE INDEX idx_images_episode ON images(episode_id, kind);

CREATE TABLE genres (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE item_genres (
    item_id         INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    genre_id        INTEGER NOT NULL REFERENCES genres(id) ON DELETE CASCADE,
    PRIMARY KEY (item_id, genre_id)
);

CREATE TABLE people (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    tmdb_id         INTEGER UNIQUE,
    photo_url       TEXT
);

CREATE TABLE item_credits (
    id              INTEGER PRIMARY KEY,
    item_id         INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    person_id       INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    role            TEXT NOT NULL,
    character_name  TEXT,
    sort_order      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_credits_item ON item_credits(item_id, sort_order);

-- ---------------------------------------------------------------------------
-- Play state
-- ---------------------------------------------------------------------------

CREATE TABLE play_state (
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id         INTEGER REFERENCES items(id) ON DELETE CASCADE,
    episode_id      INTEGER REFERENCES episodes(id) ON DELETE CASCADE,
    position_ms     INTEGER NOT NULL DEFAULT 0,
    duration_ms     INTEGER,
    watched         INTEGER NOT NULL DEFAULT 0,
    view_count      INTEGER NOT NULL DEFAULT 0,
    last_played_at  INTEGER NOT NULL,
    CHECK ((item_id IS NULL) <> (episode_id IS NULL))
);
CREATE UNIQUE INDEX uq_play_state_item
    ON play_state(user_id, item_id)
    WHERE item_id IS NOT NULL;
CREATE UNIQUE INDEX uq_play_state_episode
    ON play_state(user_id, episode_id)
    WHERE episode_id IS NOT NULL;
CREATE INDEX idx_play_state_user_recent
    ON play_state(user_id, last_played_at DESC);

-- ---------------------------------------------------------------------------
-- Markers
-- ---------------------------------------------------------------------------

CREATE TABLE markers (
    id              INTEGER PRIMARY KEY,
    media_file_id   INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,
    start_ms        INTEGER NOT NULL,
    end_ms          INTEGER NOT NULL,
    label           TEXT,
    source          TEXT NOT NULL
);
CREATE INDEX idx_markers_file ON markers(media_file_id, start_ms);

-- ---------------------------------------------------------------------------
-- Scan jobs
-- ---------------------------------------------------------------------------

CREATE TABLE scan_jobs (
    id              INTEGER PRIMARY KEY,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    triggered_by    INTEGER REFERENCES users(id) ON DELETE SET NULL,
    status          TEXT NOT NULL,
    started_at      INTEGER,
    finished_at     INTEGER,
    files_seen      INTEGER NOT NULL DEFAULT 0,
    files_added     INTEGER NOT NULL DEFAULT 0,
    files_updated   INTEGER NOT NULL DEFAULT 0,
    files_removed   INTEGER NOT NULL DEFAULT 0,
    error_message   TEXT,
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_scan_jobs_library ON scan_jobs(library_id, created_at DESC);

-- ---------------------------------------------------------------------------
-- Full-text search (items + episodes)
-- ---------------------------------------------------------------------------

CREATE VIRTUAL TABLE items_fts USING fts5(
    title,
    original_title,
    summary,
    cast_names,
    content=''
);
