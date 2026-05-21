-- Phase 49 — restore ON DELETE SET NULL on invites.consumed_by.
--
-- The phase-22 rebuild (`20260517040000_phase22_invite_hardening`) re-
-- created the invites table to swap `code` for `code_hash`, but dropped
-- the `ON DELETE SET NULL` from the `consumed_by` column declaration
-- the original init.sql had. With NO ACTION (the default) in place,
-- deleting a user who'd ever accepted an invite failed with a 787
-- FOREIGN KEY constraint error from the user-management UI — operators
-- couldn't remove users at all once they'd registered.
--
-- Same rebuild-dance as phase 36 / phase 41: `legacy_alter_table=ON`
-- so the rename keeps the literal table name; the pool-level FK-off
-- inside `db::open_with` covers the DROP without firing the
-- `invite_libraries` + `invite_groups` cascades (those tables
-- reference invites with ON DELETE CASCADE — we want to preserve
-- their rows since the new `invites` table will keep the same ids).

PRAGMA legacy_alter_table = ON;

CREATE TABLE invites_new (
    id          INTEGER PRIMARY KEY,
    code_hash   TEXT    NOT NULL UNIQUE,
    email       TEXT,
    created_by  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at  INTEGER,
    consumed_by INTEGER REFERENCES users(id) ON DELETE SET NULL,
    consumed_at INTEGER,
    sent_at     INTEGER,
    created_at  INTEGER NOT NULL
);

INSERT INTO invites_new (
    id, code_hash, email, created_by, expires_at, consumed_by, consumed_at, sent_at, created_at
)
SELECT
    id, code_hash, email, created_by, expires_at, consumed_by, consumed_at, sent_at, created_at
FROM invites;

DROP TABLE invites;
ALTER TABLE invites_new RENAME TO invites;

PRAGMA legacy_alter_table = OFF;
