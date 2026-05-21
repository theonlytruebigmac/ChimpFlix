-- Phase 25 — In-app notifications + per-user email opt-in.
--
-- Each notification belongs to a recipient (`user_id`). Events that
-- target "all admins" fan out at insert time — one row per current
-- owner. This keeps reads simple (one user_id → one inbox) and means
-- adding a new admin doesn't retroactively show them historical
-- notifications.
--
-- `kind` is a stable string discriminator (`user.registered`,
-- `user.2fa.disabled`, etc.). `payload_json` carries the structured
-- context the UI needs to render the message — schemas are owned by
-- the kind, evolved alongside the rendering code.
--
-- `read_at` flips from NULL → epoch ms when the recipient acknowledges.
-- We don't delete rows on ack so admins can scroll back through recent
-- history; an old-rows cleanup task can prune anything older than N
-- days when the table grows.

CREATE TABLE notifications (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind          TEXT    NOT NULL,
    payload_json  TEXT    NOT NULL DEFAULT '{}',
    read_at       INTEGER,
    created_at    INTEGER NOT NULL
);
CREATE INDEX idx_notifications_user_created
    ON notifications(user_id, created_at DESC);
CREATE INDEX idx_notifications_user_unread
    ON notifications(user_id, created_at DESC) WHERE read_at IS NULL;

-- Per-user opt-in for email mirroring. When ON + SMTP is configured +
-- the user has an email, the notification is also sent as an email.
-- Defaulting to off avoids surprising users with mail; admins flip
-- it on from Settings → Account.
ALTER TABLE users ADD COLUMN notify_via_email INTEGER NOT NULL DEFAULT 0;
