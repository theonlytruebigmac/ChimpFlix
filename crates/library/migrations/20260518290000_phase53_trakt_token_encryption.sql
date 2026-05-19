-- Phase 53: encrypt per-user Trakt OAuth tokens at rest.
--
-- Audit (2026-05-18) flagged that `user_trakt_tokens.access_token` and
-- `refresh_token` were stored as TEXT. Unlike app-level integration
-- keys (which already go through the vault), these are per-user
-- bearer credentials granting read+write on each user's Trakt account
-- (watch history, ratings, lists, scrobbles). A DB leak = Trakt
-- account hijack for every linked user.
--
-- Strategy mirrors Phase 12.5 (webhook secret encryption): add
-- parallel `_enc` / `_nonce` columns alongside the existing plaintext
-- columns; the startup backfill in main.rs encrypts any remaining
-- plaintext rows on every boot until they're all converted. A later
-- migration can drop the legacy plaintext columns once we're
-- confident every deployment has run the backfill.

ALTER TABLE user_trakt_tokens ADD COLUMN access_token_enc BLOB;
ALTER TABLE user_trakt_tokens ADD COLUMN access_token_nonce BLOB;
ALTER TABLE user_trakt_tokens ADD COLUMN refresh_token_enc BLOB;
ALTER TABLE user_trakt_tokens ADD COLUMN refresh_token_nonce BLOB;
