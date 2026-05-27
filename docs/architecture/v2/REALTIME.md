# V2 Realtime — Event Hub + Server-Sent Events

> Status: **RFC skeleton.** Mostly a port; clean the event taxonomy.

## Scope

Server → client push notifications. Scan progress, job state changes,
activity feed updates, marker detection completion, disk alerts.

## V1 starting point

- In-memory event hub (`crate::hub`) with `tokio::sync::broadcast`
  channels.
- Single SSE endpoint at `/api/v1/events`.
- Event enum with variants per kind: `Scan`, `Job`, `Activity`,
  `Disk`, etc.
- Subscribers filter client-side.
- No long-term event history (live stream only).

## Carry forward

- In-memory hub with broadcast channels. No DB for live events; the
  durable record lands in `scan_jobs`, `jobs`, `audit_log` etc.
- SSE transport for browsers.
- Event-typed payloads (no generic JSON blobs).

## What changes

- **Server-side subscription filtering.** V1 sends every event to
  every subscriber; client filters. V2 lets subscribers declare
  what they care about (`?topics=scan,job`) so the hub only sends
  matching events. Reduces wire traffic for low-traffic clients
  (e.g. a user-facing tab only needs play_state events for their
  own user).
- **Backpressure.** Slow subscribers don't block the hub. V1's
  broadcast channel has bounded capacity; V2 keeps this and surfaces
  drops to the client as a "you missed events, refresh" signal.
- **Event taxonomy cleanup.** V1's variants grew organically. V2
  takes a fresh look at what's surfaced via realtime vs. what's
  pulled on demand.

## Event taxonomy (sketch)

- `scan.started` / `scan.progress` / `scan.completed` / `scan.failed`
- `job.queued` / `job.started` / `job.progress` / `job.completed` /
  `job.failed`
- `library.changed` (catalog mutation — invalidates browse caches)
- `play_state.changed` (per-user; subscribers filter on `user_id`)
- `activity.new` (admin activity feed)
- `disk.alert` (admin alerts surface)
- `marker.detected` (per-show or per-season)

Each event carries a timestamp, a sequence number, and a typed
payload.

## Open questions

- **WebSocket vs. SSE.** SSE is simpler and works well for one-way
  push. WebSocket would unlock bidirectional (typing indicators,
  live cursor on shared lists, casting). No bidirectional needs in
  V2's locked scope. Stay on SSE.
- **Persistent event log.** Worth keeping the last N events in a
  ring buffer for "show me activity since I last connected"? V1
  doesn't. V2 considers but probably doesn't — page reload pulls
  the durable record from the DB.
- **Auth on SSE.** Cookie-based, same as the rest of the API. Worth
  checking how V1 handles long-lived SSE connections vs. cookie
  expiration mid-stream.

## Cut list

- **WebSocket bidirectional realtime.** Not justified by current
  scope.
- **Per-event delivery acknowledgment.** SSE doesn't do this; if a
  client needs durability they pull from the DB. Don't bolt
  application-level acks onto SSE.
- **Event sourcing as the primary data model.** V2 stores state
  durably in tables; the realtime stream is a change-notification
  channel, not the system of record.
