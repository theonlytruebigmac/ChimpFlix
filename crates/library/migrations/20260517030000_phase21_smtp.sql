-- Phase 21 — SMTP / email delivery settings.
--
-- ChimpFlix-managed email goes through this single SMTP relay. Invite
-- emails, password-reset links, and admin notifications all share this
-- config — there's no per-feature SMTP override (intentionally; running
-- multiple relays is operator-surface complexity we don't need).
--
-- The SMTP password is stored in the credential vault under key
-- "smtp_password" (encrypted at rest when CHIMPFLIX_SECRET_KEY is set).
-- Only host/port/from/security/auth-username live on this singleton row;
-- the password is fetched via vault_get() at send time.
--
-- All columns are nullable — leaving them empty means "email disabled".
-- The Mailer abstraction treats absent config as a no-op rather than an
-- error so feature code can call send_*() unconditionally.

ALTER TABLE server_settings ADD COLUMN email_smtp_host         TEXT;
ALTER TABLE server_settings ADD COLUMN email_smtp_port         INTEGER;
ALTER TABLE server_settings ADD COLUMN email_smtp_username     TEXT;
-- 'starttls' (default, port 587) | 'tls' (implicit, port 465) | 'none'
ALTER TABLE server_settings ADD COLUMN email_smtp_security     TEXT;
-- e.g. "noreply@example.com"
ALTER TABLE server_settings ADD COLUMN email_from_address      TEXT;
-- Display name shown as "From: <name> <address>".
ALTER TABLE server_settings ADD COLUMN email_from_name         TEXT;
