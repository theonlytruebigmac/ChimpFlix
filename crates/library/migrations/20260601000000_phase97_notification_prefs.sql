-- Per-user notification preferences: per-kind mute + email channel + a
-- quiet-hours window, stored as a JSON object keyed by notification kind
-- (e.g. {"job.failed": {"enabled": false, "email": true,
-- "quiet_start_hour": 22, "quiet_end_hour": 7}}). Empty object = all
-- defaults (every kind on, email follows users.notify_via_email, no quiet
-- hours). Security kinds (user.2fa.*) ignore this and always notify.
-- Parsed + enforced by crates/server/src/notifier.rs::delivery_for.
ALTER TABLE users ADD COLUMN notification_prefs_json TEXT NOT NULL DEFAULT '{}';
