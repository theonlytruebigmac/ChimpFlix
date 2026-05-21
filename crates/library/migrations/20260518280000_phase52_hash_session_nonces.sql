-- Phase 52: session nonces stored as SHA-256 hashes, not raw bytes.
--
-- The security audit (2026-05-18) flagged that `sessions.nonce` held the
-- raw 32-byte cookie nonce verbatim. Combined with the HMAC secret in the
-- vault, a DB read alone yielded forgeable cookies for every active user.
--
-- Fix: the cookie still carries the raw nonce (no client-side change);
-- the DB now stores `SHA-256(nonce)`. The extractor hashes the
-- caller-supplied nonce and compares against the stored value. A stolen
-- `chimpflix.db` is no longer enough to mint a working cookie.
--
-- Schema doesn't change (the `nonce` column is still 32 bytes — same
-- type, same length, just hashes instead of raw nonces). But existing
-- rows hold *raw nonces*, which would now fail to match any cookie. We
-- delete them all so users get a single forced re-login rather than a
-- mysterious "logged out for no reason" the next time they visit. The
-- column is renamed in source-of-truth comments only (the SQLite
-- column name stays `nonce` for migration simplicity).

DELETE FROM sessions;

-- Comment for future-readers via PRAGMA-readable schema metadata.
-- (SQLite doesn't support COMMENT ON, so this lives in the migration
-- file and the SessionRow docstring.)
