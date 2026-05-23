-- Phase 88 — per-user snapshot of the Trakt watchlist as we last saw
-- it. Used by the watchlist reconcile to compute *two-way* diffs:
--   - In Trakt now but not in state → add locally to My List
--   - In state but not in Trakt now → user removed on Trakt → remove
--     locally (this is the new behaviour; the original pull was
--     additive-only by design but now that we have a snapshot to diff
--     against, we can safely propagate removes without nuking items
--     that were never on Trakt to begin with).
--
-- Storage: one row per (user, kind, tmdb_id). Kind is 'movie' or
-- 'show'; seasons/episodes aren't stored in My List so they don't
-- need representation here.
CREATE TABLE user_trakt_watchlist_state (
    user_id  INTEGER NOT NULL,
    kind     TEXT    NOT NULL CHECK(kind IN ('movie', 'show')),
    tmdb_id  INTEGER NOT NULL,
    seen_at  INTEGER NOT NULL,
    PRIMARY KEY (user_id, kind, tmdb_id),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX idx_user_trakt_watchlist_state_user
    ON user_trakt_watchlist_state(user_id);
