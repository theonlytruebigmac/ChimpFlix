-- Phase 15: Trakt.tv two-way sync.
--
-- One row per user; tokens come from the device-code OAuth flow (no
-- redirect URI needed). `expires_at` lets the runtime refresh proactively
-- on the next request instead of waiting for a 401. `last_synced_at`
-- is the cursor the hourly pull task uses to ask Trakt only for items
-- watched since the last successful pull.

CREATE TABLE user_trakt_tokens (
    user_id          INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    access_token     TEXT NOT NULL,
    refresh_token    TEXT NOT NULL,
    scope            TEXT,
    expires_at       INTEGER NOT NULL,
    linked_at        INTEGER NOT NULL,
    last_synced_at   INTEGER
);

-- Per-user 1–10 ratings. Two-way sync with Trakt's /sync/ratings; the
-- UI exposes a small picker on the detail modal. Movies use item_id;
-- episodes use episode_id. Exactly one of the two is set per row.

CREATE TABLE user_ratings (
    id          INTEGER PRIMARY KEY,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    item_id     INTEGER REFERENCES items(id) ON DELETE CASCADE,
    episode_id  INTEGER REFERENCES episodes(id) ON DELETE CASCADE,
    rating      INTEGER NOT NULL,                -- 1..10
    rated_at    INTEGER NOT NULL,
    CHECK (rating BETWEEN 1 AND 10),
    CHECK ((item_id IS NULL) <> (episode_id IS NULL))
);
CREATE UNIQUE INDEX idx_user_rating_item
    ON user_ratings(user_id, item_id) WHERE item_id IS NOT NULL;
CREATE UNIQUE INDEX idx_user_rating_episode
    ON user_ratings(user_id, episode_id) WHERE episode_id IS NOT NULL;
