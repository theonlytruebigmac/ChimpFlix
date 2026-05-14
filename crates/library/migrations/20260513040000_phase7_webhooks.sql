-- Phase 7: outbound webhooks.
--
-- A webhook subscribes to a set of event names (stored as JSON array in
-- event_mask) and receives a POST with the event payload. Every attempt is
-- recorded in webhook_deliveries so the admin UI can show delivery history
-- and a failed-with-retries timeline.

CREATE TABLE webhooks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    name            TEXT NOT NULL,
    url             TEXT NOT NULL,
    secret          TEXT,
    event_mask      TEXT NOT NULL,                -- JSON array of event names
    enabled         INTEGER NOT NULL DEFAULT 1,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
CREATE INDEX idx_webhooks_enabled ON webhooks(enabled);

CREATE TABLE webhook_deliveries (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    webhook_id      INTEGER NOT NULL REFERENCES webhooks(id) ON DELETE CASCADE,
    event           TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    status_code     INTEGER,
    response_body   TEXT,
    error           TEXT,
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_retry_at   INTEGER,
    delivered_at    INTEGER,
    created_at      INTEGER NOT NULL
);
CREATE INDEX idx_webhook_deliveries_pending
    ON webhook_deliveries(next_retry_at)
    WHERE delivered_at IS NULL AND next_retry_at IS NOT NULL;
CREATE INDEX idx_webhook_deliveries_webhook
    ON webhook_deliveries(webhook_id, created_at DESC);
