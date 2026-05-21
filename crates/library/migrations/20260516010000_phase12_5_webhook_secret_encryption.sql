-- Phase 12.5: encrypt webhook signing secrets at rest.
--
-- Adds parallel `secret_enc` / `secret_nonce` columns alongside the
-- existing `secret` plaintext column. Startup runs a backfill that
-- encrypts any plaintext rows and nulls the legacy column; a later
-- migration can drop `secret` outright once we're confident every
-- deployment has run the backfill.

ALTER TABLE webhooks ADD COLUMN secret_enc BLOB;
ALTER TABLE webhooks ADD COLUMN secret_nonce BLOB;
