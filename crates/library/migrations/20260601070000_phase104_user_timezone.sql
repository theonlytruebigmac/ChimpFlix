-- Per-user IANA timezone (e.g. "America/New_York", "Europe/London").
-- Used by crates/server/src/notifier.rs::in_quiet_hours to interpret a
-- user's quiet-hours window in their local wall-clock time rather than
-- raw UTC. Default 'UTC' preserves the historical behaviour for every
-- existing row (quiet hours were previously compared against UTC hours).
-- The value must be a valid IANA tz name; validated at /auth/me, not by
-- the DB. An unparseable value falls back to UTC at read time.
ALTER TABLE users ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';
