-- Phase 92 — local mirror of each user's Trakt watch history.
--
-- Before this, `pull_user_history` matched each Trakt history entry against
-- the CURRENT library and DISCARDED anything unmatched, then advanced the
-- `last_synced_at` cursor. So a library added later never got its watched
-- status: the history was already pulled-and-dropped and the cursor had
-- moved past it (and a resync would re-hammer the API). Meanwhile
-- `/sync/playback` has no cursor and kept stamping resume positions every
-- sync — hence "position shows up, watched doesn't."
--
-- This table stores EVERY pulled history event (keyed by Trakt's own
-- per-event id, so re-pulls dedupe). Items are then reconciled against it on
-- demand — after a Trakt pull, and on scan completion when new items appear
-- — with NO additional API calls. For movies the ids are the movie's; for
-- episodes they are the SHOW's ids (episodes match by show id + season +
-- episode number). Storing tvdb/imdb alongside tmdb also fixes the old
-- tmdb-only matcher, which silently dropped anime watch history.
CREATE TABLE user_trakt_history (
    user_id        INTEGER NOT NULL,
    trakt_event_id INTEGER NOT NULL,
    media_type     TEXT    NOT NULL,   -- 'movie' | 'episode'
    tmdb_id        INTEGER,
    tvdb_id        INTEGER,
    imdb_id        TEXT,
    season         INTEGER,            -- episodes only
    episode        INTEGER,            -- episodes only
    watched_at     INTEGER NOT NULL,   -- epoch ms
    PRIMARY KEY (user_id, trakt_event_id)
);

-- Reconcile lookups: movies by (user, tmdb), episodes by (user, tmdb, season, episode).
CREATE INDEX idx_trakt_history_movie
    ON user_trakt_history (user_id, media_type, tmdb_id);
CREATE INDEX idx_trakt_history_episode
    ON user_trakt_history (user_id, media_type, tmdb_id, season, episode);
