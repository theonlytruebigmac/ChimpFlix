-- Phase 22 — Invite hardening.
--
-- Three changes:
--
--   1. Replace the plaintext `code` column with `code_hash` (SHA-256 of
--      the random token). The plaintext token is shown to the admin
--      exactly once at issuance — never again. SQL dumps no longer leak
--      working invite codes; an admin who needs to re-share an invite
--      revokes + creates fresh.
--
--   2. Add `email` so the server knows where to deliver the invite link
--      via SMTP (when configured) instead of relying on the admin
--      manually copy-pasting it. NULL means "no email; just give me the
--      copy link".
--
--   3. New `invite_libraries` join table — optional pre-binding of
--      library access. When the invite is consumed, these get inserted
--      into `library_access` for the new user, so they land with the
--      right set already granted.
--
-- Implementation note: SQLite refuses `ALTER TABLE … DROP COLUMN` when
-- the column has a UNIQUE constraint (the implicit one from the
-- original `code TEXT NOT NULL UNIQUE`). So we drop and recreate the
-- table instead of ALTER-ing. Existing invites (if any) are wiped —
-- we can't recover plaintext to hash, and this is a dev-stage codebase
-- where the realistic count is zero.

DROP TABLE IF EXISTS invites;

CREATE TABLE invites (
    id          INTEGER PRIMARY KEY,
    code_hash   TEXT    NOT NULL UNIQUE,
    email       TEXT,
    created_by  INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at  INTEGER,
    consumed_by INTEGER REFERENCES users(id),
    consumed_at INTEGER,
    sent_at     INTEGER,
    created_at  INTEGER NOT NULL
);

CREATE TABLE invite_libraries (
    invite_id   INTEGER NOT NULL REFERENCES invites(id)   ON DELETE CASCADE,
    library_id  INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    PRIMARY KEY (invite_id, library_id)
);
CREATE INDEX idx_invite_libraries_library ON invite_libraries(library_id);
