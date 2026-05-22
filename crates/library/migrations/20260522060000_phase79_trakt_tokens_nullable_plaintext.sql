-- Phase 79 — Allow NULL on the legacy plaintext Trakt token columns.
--
-- Phase 15 created `user_trakt_tokens.access_token` / `refresh_token`
-- as TEXT NOT NULL. Phase 53 added the encrypted-blob columns and made
-- `upsert_trakt_tokens` insert NULL into the plaintext columns so a
-- token rotation never leaves the old plaintext value behind. That
-- contradicts the original NOT NULL constraint, so every Trakt link
-- attempt 500s with `NOT NULL constraint failed:
-- user_trakt_tokens.access_token`.
--
-- SQLite cannot relax a column's NOT NULL via ALTER COLUMN, so we do
-- the rebuild-and-swap dance. Existing rows already populated by the
-- phase-53 backfill are preserved verbatim.

CREATE TABLE user_trakt_tokens_new (
    user_id             INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    access_token        TEXT,
    refresh_token       TEXT,
    access_token_enc    BLOB,
    access_token_nonce  BLOB,
    refresh_token_enc   BLOB,
    refresh_token_nonce BLOB,
    scope               TEXT,
    expires_at          INTEGER NOT NULL,
    linked_at           INTEGER NOT NULL,
    last_synced_at      INTEGER
);

INSERT INTO user_trakt_tokens_new
    (user_id, access_token, refresh_token,
     access_token_enc, access_token_nonce,
     refresh_token_enc, refresh_token_nonce,
     scope, expires_at, linked_at, last_synced_at)
SELECT
    user_id, access_token, refresh_token,
    access_token_enc, access_token_nonce,
    refresh_token_enc, refresh_token_nonce,
    scope, expires_at, linked_at, last_synced_at
FROM user_trakt_tokens;

DROP TABLE user_trakt_tokens;
ALTER TABLE user_trakt_tokens_new RENAME TO user_trakt_tokens;
