-- Per-user "My List". Single list per user (no named lists yet).
-- Cascade-delete when either the user or the item is removed.

CREATE TABLE user_my_list (
    user_id    INTEGER NOT NULL REFERENCES users(id)  ON DELETE CASCADE,
    item_id    INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    added_at   INTEGER NOT NULL,
    PRIMARY KEY (user_id, item_id)
);

-- Lookups are user-scoped + sorted by recency, so a single composite
-- covering index serves both the rail and the membership check.
CREATE INDEX idx_user_my_list_user_added ON user_my_list (user_id, added_at DESC);
