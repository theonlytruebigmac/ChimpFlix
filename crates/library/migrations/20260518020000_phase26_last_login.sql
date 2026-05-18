-- Phase 26 — Last-login tracking for the login-page UX surface.
--
-- We capture the previous successful login's timestamp + IP before
-- overwriting them with the current login, so the next page after
-- authentication can show "Last signed in 3h ago from 1.2.3.4" — a
-- cheap user-side anomaly check. Nothing fancy: a single previous
-- value, not a full history. The full history lives in the existing
-- `audit_log` (action = "user.login").
--
-- Columns are nullable so existing rows (pre-Phase-26 users) don't
-- need a backfill. The login handler treats NULL as "first login".

ALTER TABLE users ADD COLUMN last_login_at         INTEGER;
ALTER TABLE users ADD COLUMN last_login_ip         TEXT;
ALTER TABLE users ADD COLUMN previous_login_at     INTEGER;
ALTER TABLE users ADD COLUMN previous_login_ip     TEXT;
