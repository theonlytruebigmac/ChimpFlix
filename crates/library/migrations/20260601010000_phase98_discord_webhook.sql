-- Per-user Discord notification webhook. When set, notifications that
-- reach the user (after applying their per-kind prefs + quiet hours, same
-- as the email channel) are also POSTed to this Discord webhook URL as an
-- embed. NULL = not configured (the default — no Discord delivery). The
-- value must be a Discord webhook URL; the shape is validated at the
-- /auth/me endpoint, not by the DB. Delivered by
-- crates/server/src/notifier.rs::send_discord.
ALTER TABLE users ADD COLUMN discord_webhook_url TEXT;
