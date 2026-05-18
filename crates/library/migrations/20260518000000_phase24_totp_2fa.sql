-- Phase 24 — TOTP-based two-factor auth.
--
-- Storage shape mirrors the rest of the credential vault: encrypted
-- BLOB + nonce, with NULL nonce signalling plaintext mode (which only
-- happens when CHIMPFLIX_SECRET_KEY isn't set). The TOTP secret is
-- functionally a password — anyone with it can mint valid codes — so
-- encryption-at-rest matters.
--
-- `verified_at` distinguishes between "the user started enrollment and
-- got a QR code" (pre-verify) and "the user proved possession of their
-- authenticator". Only the latter counts as 2FA-enabled.
--
-- Recovery codes are 10 short hex strings, shown to the user exactly
-- once at enrollment. Each one is single-use; we store only the
-- SHA-256 hash and mark `consumed_at` when used.
--
-- The `totp_enforcement` server setting controls the global policy:
--   * `disabled` — no one can enroll
--   * `optional` (default) — users may enroll if they want
--   * `required` — all users must enroll before they can complete login

CREATE TABLE user_totp (
    user_id      INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    secret_enc   BLOB    NOT NULL,
    secret_nonce BLOB,                       -- NULL = vault in plaintext mode
    verified_at  INTEGER,                    -- NULL until first valid code
    created_at   INTEGER NOT NULL
);

CREATE TABLE user_recovery_codes (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash   TEXT    NOT NULL,
    consumed_at INTEGER,
    created_at  INTEGER NOT NULL
);
CREATE INDEX        idx_user_recovery_codes_user ON user_recovery_codes(user_id);
CREATE UNIQUE INDEX idx_user_recovery_codes_hash ON user_recovery_codes(code_hash);

-- Global enforcement policy. NULL stays as "optional" via app default
-- so the admin can leave it untouched.
ALTER TABLE server_settings ADD COLUMN totp_enforcement TEXT;
UPDATE server_settings SET totp_enforcement = 'optional' WHERE id = 1;
