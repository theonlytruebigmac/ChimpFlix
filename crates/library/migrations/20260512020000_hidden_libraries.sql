-- Per-user list of library IDs that should be excluded from browse views.
-- ON DELETE CASCADE on both sides so the table self-cleans when a user or
-- a library is removed.

CREATE TABLE user_hidden_libraries (
    user_id     INTEGER NOT NULL REFERENCES users(id)     ON DELETE CASCADE,
    library_id  INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, library_id)
);
