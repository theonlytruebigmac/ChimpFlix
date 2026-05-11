# ChimpFlix Database Schema (v0.1 draft)

> Status: **design draft**. SQLite. WAL mode, `foreign_keys=ON`,
> `synchronous=NORMAL`. All integer IDs are `INTEGER PRIMARY KEY` (rowid
> aliases — fast and small).

## Conventions

- `created_at` / `updated_at` / `*_at` columns are Unix epoch milliseconds
  stored as `INTEGER`. Avoids timezone bugs and SQLite text-date pitfalls.
- Booleans are `INTEGER` (0/1).
- All foreign keys cascade on delete unless noted.
- Enum-ish columns store short ASCII strings (`'movie'`, `'show'`, `'episode'`)
  rather than ints — readable on inspection, costs little.
- IDs are exposed in URLs directly. Auth gates access; non-guessability is
  not a security boundary.

## Users & sessions

```sql
CREATE TABLE users (
    id              INTEGER PRIMARY KEY,
    username        TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash   TEXT NOT NULL,             -- argon2id
    role            TEXT NOT NULL,             -- 'owner' | 'user'
    display_name    TEXT,
    avatar_path     TEXT,                      -- relative to cache dir
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE invites (
    id              INTEGER PRIMARY KEY,
    code            TEXT NOT NULL UNIQUE,      -- random URL-safe token
    created_by      INTEGER NOT NULL REFERENCES users(id),
    expires_at      INTEGER,                   -- NULL = no expiry
    consumed_by     INTEGER REFERENCES users(id),
    consumed_at     INTEGER,
    created_at      INTEGER NOT NULL
);

-- Session is the long-lived refresh credential. The HTTP cookie is a signed
-- (HMAC) reference to a session id + a nonce; revocation works by deleting
-- the row.
CREATE TABLE sessions (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    nonce           BLOB NOT NULL,             -- 32 random bytes
    user_agent      TEXT,
    ip              TEXT,
    last_seen_at    INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_sessions_user ON sessions(user_id);
CREATE INDEX idx_sessions_expires ON sessions(expires_at);
```

## Libraries

```sql
CREATE TABLE libraries (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL,             -- 'movies' | 'shows'
    -- Scan tuning
    scan_interval_s INTEGER NOT NULL DEFAULT 3600,
    last_scan_at    INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

-- A library can have multiple root paths (a movie library spanning two
-- mounts, etc.). Mirrors how Plex/Jellyfin/Kodi work.
CREATE TABLE library_paths (
    id              INTEGER PRIMARY KEY,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    path            TEXT NOT NULL,
    UNIQUE(library_id, path)
);

-- Per-user library access. Default: owner has all libraries; users get
-- explicit grants.
CREATE TABLE library_access (
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, library_id)
);
```

## Media items (movies, shows, episodes)

```sql
-- "Item" is the unit of browseable content: a movie or a show.
-- Episodes are NOT items — they live in their own table because they have
-- a parent (show) and a sibling structure (season).
CREATE TABLE items (
    id              INTEGER PRIMARY KEY,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,             -- 'movie' | 'show'
    title           TEXT NOT NULL,
    sort_title      TEXT NOT NULL,             -- 'Matrix, The'
    original_title  TEXT,
    summary         TEXT,
    tagline         TEXT,
    year            INTEGER,
    rating_age      TEXT,                      -- 'PG-13', 'TV-MA', etc.
    rating_audience REAL,                      -- 0..10
    duration_ms     INTEGER,                   -- runtime, may be NULL for shows
    -- External IDs (sparse; many will be NULL)
    tmdb_id         INTEGER,
    imdb_id         TEXT,
    tvdb_id         INTEGER,
    -- Lifecycle
    added_at        INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    refreshed_at    INTEGER                    -- last successful metadata refresh
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
    air_date        INTEGER,                   -- epoch ms (date-only granularity)
    duration_ms     INTEGER,
    tmdb_id         INTEGER,
    added_at        INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    UNIQUE(season_id, episode_number)
);
CREATE INDEX idx_episodes_season ON episodes(season_id);
```

## Media files & streams

```sql
-- A media_file is one physical file on disk. A movie typically has one
-- file; an episode typically has one. Multi-version libraries (e.g. a 4K
-- and 1080p version of the same movie) have multiple rows referencing the
-- same item.
CREATE TABLE media_files (
    id              INTEGER PRIMARY KEY,
    -- Parent: exactly one of item_id (for movies) or episode_id is set
    item_id         INTEGER REFERENCES items(id) ON DELETE CASCADE,
    episode_id      INTEGER REFERENCES episodes(id) ON DELETE CASCADE,
    path            TEXT NOT NULL UNIQUE,
    size_bytes      INTEGER NOT NULL,
    mtime_ms        INTEGER NOT NULL,
    container       TEXT,                      -- 'mp4', 'mkv', etc.
    duration_ms     INTEGER,
    bit_rate        INTEGER,
    width           INTEGER,
    height          INTEGER,
    hdr_format      TEXT,                      -- 'sdr', 'hdr10', 'hdr10+', 'dovi'
    scanned_at      INTEGER NOT NULL,
    CHECK ((item_id IS NULL) <> (episode_id IS NULL))  -- exactly one parent
);
CREATE INDEX idx_media_files_item ON media_files(item_id);
CREATE INDEX idx_media_files_episode ON media_files(episode_id);

CREATE TABLE media_streams (
    id              INTEGER PRIMARY KEY,
    media_file_id   INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    stream_index    INTEGER NOT NULL,          -- ffprobe stream index
    kind            TEXT NOT NULL,             -- 'video' | 'audio' | 'subtitle'
    codec           TEXT,
    profile         TEXT,
    language        TEXT,                      -- BCP-47 / ISO 639
    title           TEXT,
    -- Video
    pix_fmt         TEXT,
    frame_rate      REAL,
    -- Audio
    channels        INTEGER,
    channel_layout  TEXT,
    sample_rate     INTEGER,
    -- Subtitle
    is_forced       INTEGER NOT NULL DEFAULT 0,
    is_default      INTEGER NOT NULL DEFAULT 0,
    is_external     INTEGER NOT NULL DEFAULT 0, -- sidecar .srt/.ass
    external_path   TEXT,
    UNIQUE(media_file_id, stream_index, is_external)
);
CREATE INDEX idx_streams_file ON media_streams(media_file_id);
```

## Metadata side tables

```sql
CREATE TABLE images (
    id              INTEGER PRIMARY KEY,
    item_id         INTEGER REFERENCES items(id) ON DELETE CASCADE,
    episode_id      INTEGER REFERENCES episodes(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,             -- 'poster' | 'backdrop' | 'thumb' | 'logo'
    source          TEXT NOT NULL,             -- 'tmdb' | 'local' | 'embedded'
    source_url      TEXT,
    local_path      TEXT,                      -- relative to cache dir, NULL if not yet fetched
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
    person_id       INTEGER NOT NULL REFERENCES people(id),
    role            TEXT NOT NULL,             -- 'director' | 'writer' | 'actor' | ...
    character_name  TEXT,                      -- for actors
    sort_order      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_credits_item ON item_credits(item_id, sort_order);
```

## Play state

```sql
-- One row per (user, parent). Parent is item (movie) or episode.
CREATE TABLE play_state (
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id         INTEGER REFERENCES items(id) ON DELETE CASCADE,
    episode_id      INTEGER REFERENCES episodes(id) ON DELETE CASCADE,
    position_ms     INTEGER NOT NULL DEFAULT 0,
    duration_ms     INTEGER,                   -- snapshot at last update
    watched         INTEGER NOT NULL DEFAULT 0,
    view_count      INTEGER NOT NULL DEFAULT 0,
    last_played_at  INTEGER NOT NULL,
    CHECK ((item_id IS NULL) <> (episode_id IS NULL)),
    PRIMARY KEY (user_id, item_id, episode_id)
);
CREATE INDEX idx_play_state_user_recent ON play_state(user_id, last_played_at DESC);
```

> Note: `PRIMARY KEY (user_id, item_id, episode_id)` works in SQLite even
> with NULLable columns because SQLite treats NULL as distinct from other
> NULLs in indexes by default — but for the CHECK constraint to guarantee
> exclusivity, we still rely on (one_is_null XOR other_is_null). To enforce
> uniqueness correctly we add two unique partial indexes:

```sql
CREATE UNIQUE INDEX uq_play_state_item
    ON play_state(user_id, item_id)
    WHERE item_id IS NOT NULL;
CREATE UNIQUE INDEX uq_play_state_episode
    ON play_state(user_id, episode_id)
    WHERE episode_id IS NOT NULL;
```

## Markers (intro / credits / chapters)

```sql
CREATE TABLE markers (
    id              INTEGER PRIMARY KEY,
    media_file_id   INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,             -- 'intro' | 'credits' | 'chapter'
    start_ms        INTEGER NOT NULL,
    end_ms          INTEGER NOT NULL,
    label           TEXT,                      -- chapter name
    source          TEXT NOT NULL              -- 'embedded' | 'detected' | 'manual'
);
CREATE INDEX idx_markers_file ON markers(media_file_id, start_ms);
```

## Scan jobs

```sql
CREATE TABLE scan_jobs (
    id              INTEGER PRIMARY KEY,
    library_id      INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    triggered_by    INTEGER REFERENCES users(id),  -- NULL = scheduler
    status          TEXT NOT NULL,             -- 'queued' | 'running' | 'completed' | 'failed' | 'canceled'
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
```

## Search (FTS5)

```sql
-- Mirror table populated by triggers from items and episodes.
CREATE VIRTUAL TABLE items_fts USING fts5(
    title,
    original_title,
    summary,
    cast_names,                                 -- concatenated for searchability
    content=''                                  -- contentless, we manage rows
);
-- Triggers (omitted here for brevity) keep items_fts in sync on
-- INSERT/UPDATE/DELETE to items and to item_credits.
```

## Future tables (sketched, NOT in v0.1)

These are listed here only to make sure the v0.1 schema doesn't paint us
into a corner. We do NOT create them yet.

- `studios`, `item_studios` — production company associations.
- `collections`, `item_collections` — Plex-style collections.
- `playlists`, `playlist_items`.
- `sharing` — remote-friend access tokens.
- `music_artists`, `music_albums`, `music_tracks` — music library (v0.2+).
- `live_channels`, `live_recordings` — Live TV/DVR (v1+).

## Migration philosophy

- Migrations are forward-only, timestamped `YYYYMMDDHHMMSS_*.sql`, in
  `crates/library/migrations/`.
- No `DROP TABLE` or destructive change in the same migration that adds
  the replacement. Two-step: add new, dual-write, cutover, then drop in a
  later release.
- Schema is **not** stable until v0.1.0 tag. Anything before that is fair
  game to rewrite — no migrations preserved across pre-release versions
  until then.
