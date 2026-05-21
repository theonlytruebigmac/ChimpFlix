-- Phase 23 — Self-service password reset.
--
-- Same security shape as invites (Phase 22): plaintext token shown once
-- to the requester (delivered via SMTP), only SHA-256(token) stored at
-- rest. Tokens are single-use and short-lived (default 1h). The reset
-- endpoint deletes the row on success so a token can't be replayed.
--
-- We also invalidate every existing session for the user when a reset
-- completes — covered by application code, not this migration, but
-- documented here so the foreign-key cascade is intentional.
--
-- Adds `users.email` (nullable, unique when set) so reset requests can
-- identify the user by email instead of username. Pre-existing rows
-- get NULL; the user supplies their email at first opportunity (admin
-- settings, or via invite pre-bind at register time).

ALTER TABLE users ADD COLUMN email TEXT;
CREATE UNIQUE INDEX idx_users_email_unique
    ON users(LOWER(email)) WHERE email IS NOT NULL;

CREATE TABLE password_reset_tokens (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash   TEXT    NOT NULL,
    requested_ip TEXT,
    user_agent  TEXT,
    created_at  INTEGER NOT NULL,
    expires_at  INTEGER NOT NULL,
    consumed_at INTEGER
);
CREATE UNIQUE INDEX idx_password_reset_tokens_hash
    ON password_reset_tokens(code_hash);
CREATE INDEX idx_password_reset_tokens_user
    ON password_reset_tokens(user_id, expires_at);
