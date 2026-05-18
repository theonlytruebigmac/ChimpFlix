-- Phase 12.5: credential vault.
--
-- Holds encrypted-at-rest "named secrets" — currently TMDB / TVDB / AniList
-- API keys and the session HMAC. ChaCha20-Poly1305 in the application
-- layer; this table only stores the ciphertext and nonce.
--
-- A NULL nonce signals plaintext mode (vault constructed without
-- CHIMPFLIX_SECRET_KEY). The app refuses to mix modes — see
-- chimpflix_common::vault for the read/write logic.

CREATE TABLE secrets (
    name        TEXT PRIMARY KEY,
    value_enc   BLOB NOT NULL,
    nonce       BLOB,                                       -- NULL = plaintext mode
    updated_at  INTEGER NOT NULL,
    updated_by  INTEGER REFERENCES users(id) ON DELETE SET NULL
);
