-- Phase 28 — Email-change verification.
--
-- Changing an existing email requires proving control of the NEW address
-- (token sent there) + the current password (re-auth at request time).
-- Until the user clicks the link, the old email stays in place — so
-- password reset still works through their previous address if they
-- typo the new one.
--
-- Same security shape as invites/password-reset: plaintext token shown
-- via email once, SHA-256 hash stored at rest, single-use, short TTL.

CREATE TABLE email_change_tokens (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    new_email   TEXT    NOT NULL,
    code_hash   TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    consumed_at INTEGER
);
CREATE UNIQUE INDEX idx_email_change_tokens_hash
    ON email_change_tokens(code_hash);
CREATE INDEX idx_email_change_tokens_user
    ON email_change_tokens(user_id, expires_at);
