-- Phase 50 — playback events log (Tautulli-style stats).
--
-- One row per "interesting" moment in a stream: start, complete,
-- (future: pause/resume/stop/error). Drives the admin Stats page —
-- recent activity feed, top users, top items, transcode mix, etc.
--
-- Append-only (no UPDATEs), capped by retention policy elsewhere if
-- it gets large — for now the indexes are tuned for "recent N" and
-- "by-user/item over a window" queries that dominate the dashboard.
--
-- Schema choices:
--   * `item_id` xor `episode_id` (mirror of `play_state` shape — a
--     row is either a movie or a TV episode, never both). Both
--     ON DELETE SET NULL so deleting a library item retains the
--     event for historical aggregates.
--   * `media_file_id` nullable for the same reason — files come and
--     go, the user activity record stays.
--   * `decision` is one of 'direct' | 'transcode' | NULL. Start
--     events always populate it; complete/progress events leave it
--     NULL (the start decision is the authoritative one for that
--     session — re-deciding mid-stream isn't a thing in our pipeline).
--   * `bytes_sent` is a placeholder for the bandwidth-metering piece
--     the transcoder doesn't track yet. Adding the metric later is
--     a one-column UPDATE path — schema is ready.

CREATE TABLE playback_events (
    id              INTEGER PRIMARY KEY,
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id         INTEGER REFERENCES items(id) ON DELETE SET NULL,
    episode_id      INTEGER REFERENCES episodes(id) ON DELETE SET NULL,
    media_file_id   INTEGER REFERENCES media_files(id) ON DELETE SET NULL,
    event_type      TEXT NOT NULL
        CHECK (event_type IN ('start', 'progress', 'pause', 'resume', 'complete', 'stop')),
    occurred_at     INTEGER NOT NULL,
    position_ms     INTEGER,
    duration_ms     INTEGER,
    decision        TEXT
        CHECK (decision IS NULL OR decision IN ('direct', 'transcode')),
    video_codec     TEXT,
    audio_codec     TEXT,
    container       TEXT,
    bytes_sent      INTEGER,
    ip              TEXT,
    user_agent      TEXT,
    session_token   TEXT
);

-- "Recent activity" feed (admin Stats hero query).
CREATE INDEX idx_playback_events_time ON playback_events(occurred_at DESC);

-- "User detail" / top-users-by-watch-time aggregation.
CREATE INDEX idx_playback_events_user_time
    ON playback_events(user_id, occurred_at DESC);

-- "Item detail" / top-items aggregation. Partial index — events
-- without an item_id (episode rows) skip this and use the episode
-- index below.
CREATE INDEX idx_playback_events_item_time
    ON playback_events(item_id, occurred_at DESC)
    WHERE item_id IS NOT NULL;

CREATE INDEX idx_playback_events_episode_time
    ON playback_events(episode_id, occurred_at DESC)
    WHERE episode_id IS NOT NULL;

-- Group all events of one stream by their transcoder session id —
-- used by the "now playing" detail panel to show "started 2 min ago,
-- paused 30s in" rather than just the latest event.
CREATE INDEX idx_playback_events_session
    ON playback_events(session_token)
    WHERE session_token IS NOT NULL;
