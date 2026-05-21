-- Phase 27 — Named access groups.
--
-- A user's effective library set is now the UNION of:
--   * library_access  (existing per-user direct grants — unchanged)
--   * access_group_libraries through user_access_groups  (new)
--
-- The two paths are independent: an admin can grant via group AND via
-- direct row without conflict; the SQL just OR's them. Removing
-- someone from a group removes only their group-derived access;
-- direct grants survive (and vice-versa).
--
-- `invite_groups` mirrors the Phase-22 `invite_libraries` table so
-- admins can pre-bind group membership at invite time. On consume,
-- the join rows fan out into `user_access_groups`.

CREATE TABLE access_groups (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    name        TEXT    NOT NULL UNIQUE COLLATE NOCASE,
    description TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE access_group_libraries (
    group_id   INTEGER NOT NULL REFERENCES access_groups(id) ON DELETE CASCADE,
    library_id INTEGER NOT NULL REFERENCES libraries(id)     ON DELETE CASCADE,
    PRIMARY KEY (group_id, library_id)
);
CREATE INDEX idx_access_group_libraries_library
    ON access_group_libraries(library_id);

CREATE TABLE user_access_groups (
    user_id  INTEGER NOT NULL REFERENCES users(id)          ON DELETE CASCADE,
    group_id INTEGER NOT NULL REFERENCES access_groups(id)  ON DELETE CASCADE,
    PRIMARY KEY (user_id, group_id)
);
CREATE INDEX idx_user_access_groups_group ON user_access_groups(group_id);

CREATE TABLE invite_groups (
    invite_id INTEGER NOT NULL REFERENCES invites(id)        ON DELETE CASCADE,
    group_id  INTEGER NOT NULL REFERENCES access_groups(id)  ON DELETE CASCADE,
    PRIMARY KEY (invite_id, group_id)
);
CREATE INDEX idx_invite_groups_group ON invite_groups(group_id);
