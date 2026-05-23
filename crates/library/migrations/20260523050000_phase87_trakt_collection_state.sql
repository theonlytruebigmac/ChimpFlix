-- Phase 87 — per-user snapshot of what *we* told Trakt the user
-- collected. The nightly `trakt_collection_push` task diffs this
-- against the current local catalogue, then pushes (adds) and removes
-- (deletes) only the delta. This is the contract that lets us call
-- `/sync/collection/remove` safely without nuking items the user has
-- collected via another media server or by hand on the Trakt site —
-- we only ever remove rows that *we* previously inserted.
--
-- Storage: one row per (user, movie tmdb_id) or (user, show tmdb_id,
-- season, episode_number). Movie rows use season=0/episode_num=0 as
-- placeholders so the composite PK still works.
CREATE TABLE user_trakt_collection_state (
    user_id     INTEGER NOT NULL,
    kind        TEXT    NOT NULL CHECK(kind IN ('movie', 'episode')),
    tmdb_id     INTEGER NOT NULL,
    season      INTEGER NOT NULL DEFAULT 0,
    episode_num INTEGER NOT NULL DEFAULT 0,
    pushed_at   INTEGER NOT NULL,
    PRIMARY KEY (user_id, kind, tmdb_id, season, episode_num),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX idx_user_trakt_collection_state_user
    ON user_trakt_collection_state(user_id);
