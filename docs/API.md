# ChimpFlix API

> Base path: `/api/v1`. JSON for request and response bodies. WebSocket
> at `/api/v1/ws`. Auth via httpOnly signed cookie set by
> `POST /auth/login`.
>
> **Currency:** the prose below (auth, items, play-state, streaming,
> images, websocket) is the original v0.1 design draft. The endpoint
> surface has grown substantially since — see the [**Endpoint
> reference**](#endpoint-reference) at the bottom for the complete
> list grouped by category. Bodies for the newer endpoints are
> deliberately left as a pointer to source; the handler modules under
> `crates/server/src/api/` are short, self-explanatory, and the
> source of truth.

## Conventions

- **Times** are Unix epoch milliseconds (integers) in JSON.
- **Durations** are milliseconds (integers).
- **IDs** are integers (rowid). Exposed in URLs. Auth gates access.
- **Pagination**: `?page=N&page_size=M`. Default page_size 50, max 200.
  Response includes `total`, `page`, `page_size`.
- **Errors**: HTTP status + `{ "error": { "code": "string", "message": "human", "details": {...} } }`.
  Error codes are stable, lower_snake_case (`not_found`, `forbidden`,
  `validation_failed`, etc.).
- **CSRF**: any non-GET/HEAD request requires `Origin` header matching the
  configured public origin. Enforced in middleware.
- **Rate limit**: per-IP for unauth routes (login, setup, register).
  Per-user for auth'd. Limits return `429` with `Retry-After`.

## Auth

### `POST /auth/setup`

First-run owner creation. Only works while `SELECT count(*) FROM users = 0`.

```json
// Request
{ "username": "zach", "password": "..." }
// 201 Created
{ "user": { "id": 1, "username": "zach", "role": "owner" } }
```

### `POST /auth/login`

```json
{ "username": "zach", "password": "..." }
// 204 No Content (sets cookie)
```

### `POST /auth/logout`

```json
// 204 No Content (clears cookie, deletes session row)
```

### `GET /auth/me`

```json
{
  "user": { "id": 1, "username": "zach", "role": "owner", "display_name": null }
}
```

### `POST /auth/invites` (owner)

```json
{ "expires_in_seconds": 86400 }
// 201
{ "invite": { "code": "x7Hg...", "expires_at": 1715... } }
```

### `GET /auth/invites` (owner)

List of unconsumed invites.

### `DELETE /auth/invites/:code` (owner)

Revoke an unconsumed invite.

### `POST /auth/register`

```json
{ "code": "x7Hg...", "username": "alice", "password": "..." }
// 201
{ "user": { "id": 2, "username": "alice", "role": "user" } }
```

## Users (owner)

### `GET /users`

```json
{ "users": [ { "id": 1, "username": "zach", "role": "owner", ... } ] }
```

### `PATCH /users/:id`

Update display name, password (owner can reset any password; users can
update their own).

### `DELETE /users/:id` (owner)

Cannot delete the last owner.

## Libraries (owner for write; users for read of granted libs)

### `GET /libraries`

```json
{
  "libraries": [
    {
      "id": 1, "name": "Movies", "kind": "movies",
      "paths": ["/media/movies"],
      "last_scan_at": 1715..., "scan_interval_s": 3600
    }
  ]
}
```

### `POST /libraries` (owner)

```json
{ "name": "Movies", "kind": "movies", "paths": ["/media/movies"] }
// 201
```

### `PATCH /libraries/:id` (owner)

Update name, paths, scan interval.

### `DELETE /libraries/:id` (owner)

### `POST /libraries/:id/scan` (owner)

Enqueue a scan job. Returns the job ID.

```json
// 202
{ "job": { "id": 42, "status": "queued" } }
```

### `GET /libraries/:id/scans`

Recent scan jobs for a library.

### `POST /libraries/:id/access` (owner)

```json
{ "user_id": 2 }
// grants access
```

### `DELETE /libraries/:id/access/:user_id` (owner)

## Items (movies, shows)

### `GET /items`

Browse / list. Filters:

- `library` — library ID
- `kind` — `movie` | `show`
- `genre` — genre name
- `year` — integer
- `unwatched` — `true`/`false` (filters by current user's play state)
- `sort` — `recently_added` (default), `title`, `year`, `rating`
- `q` — substring on title (use `/search` for full-text)

```json
{
  "items": [
    {
      "id": 101, "kind": "movie", "title": "Arrival", "year": 2016,
      "duration_ms": 6900000, "rating_audience": 7.9,
      "poster_url": "/api/v1/images/items/101/poster",
      "backdrop_url": "/api/v1/images/items/101/backdrop",
      "play_state": { "position_ms": 1200000, "watched": false }
    }
  ],
  "total": 1234, "page": 1, "page_size": 50
}
```

### `GET /items/:id`

Single item, full detail. For shows, includes seasons summary (counts).

```json
{
  "item": {
    "id": 101, "kind": "movie", "title": "Arrival",
    "summary": "...", "tagline": "...", "year": 2016,
    "duration_ms": 6900000, "rating_age": "PG-13", "rating_audience": 7.9,
    "genres": ["Sci-Fi", "Drama"],
    "directors": [ { "id": 11, "name": "Denis Villeneuve" } ],
    "writers":   [ { "id": 12, "name": "Eric Heisserer" } ],
    "cast":      [ { "id": 13, "name": "Amy Adams", "character": "Louise Banks" } ],
    "files": [
      {
        "id": 201, "container": "mkv", "size_bytes": 12000000000,
        "duration_ms": 6900000, "width": 3840, "height": 2160, "hdr_format": "hdr10",
        "streams": [
          { "kind": "video", "codec": "hevc", "frame_rate": 23.976 },
          { "kind": "audio", "codec": "truehd", "language": "eng", "channels": 8 },
          { "kind": "subtitle", "codec": "pgs", "language": "eng" }
        ]
      }
    ],
    "play_state": { "position_ms": 0, "watched": false },
    "tmdb_id": 329865, "imdb_id": "tt2543164"
  }
}
```

For a show:

```json
{
  "item": {
    "id": 500, "kind": "show", "title": "Severance", ...,
    "seasons": [
      { "id": 510, "season_number": 1, "episode_count": 9, "poster_url": "..." },
      { "id": 520, "season_number": 2, "episode_count": 10, "poster_url": "..." }
    ]
  }
}
```

### `GET /seasons/:id`

```json
{
  "season": {
    "id": 510, "show_id": 500, "season_number": 1, "title": "Season 1",
    "episodes": [
      {
        "id": 5101, "episode_number": 1, "title": "Good News About Hell",
        "duration_ms": 3300000, "summary": "...",
        "thumb_url": "/api/v1/images/episodes/5101/thumb",
        "play_state": { "position_ms": 0, "watched": true }
      }
    ]
  }
}
```

### `GET /episodes/:id`

Full episode detail (includes its `media_files` like the movie shape).

### `GET /items/:id/related`

Naive v0.1: same-genre, same-decade, excluding the item itself. 12 results.

## Play state

### `POST /play-state`

Bulk upsert. Frontend pushes every ~10s while playing.

```json
{
  "updates": [
    { "episode_id": 5101, "position_ms": 1200000, "duration_ms": 3300000, "watched": false }
  ]
}
// 204
```

### `POST /play-state/scrobble`

Explicit "mark as watched" when a player crosses the threshold (90% by
default). Increments view_count.

```json
{ "episode_id": 5101 }
// 204
```

### `GET /play-state/on-deck`

Continue Watching: items with progress > 0 and < threshold, plus next-up
episodes for shows where the user is in the middle of a season.

```json
{ "items": [ { /* item or episode shape with play_state */ } ] }
```

### `GET /play-state/recently-added`

Per current user, filtered to libraries they can access.

## Search

### `GET /search?q=...`

FTS5 across items and episodes. Returns mixed list.

```json
{
  "items": [ { "kind": "movie", ... } ],
  "episodes": [ { "id": 5101, "show_title": "Severance", ... } ]
}
```

## Streaming

### `POST /stream/sessions`

Start a session. Returns the chosen profile (direct play vs transcode) and
a session ID used by subsequent requests.

```json
// Request
{
  "media_file_id": 201,
  "client": {
    "supported_video_codecs": ["h264", "hevc", "av1"],
    "supported_audio_codecs": ["aac", "ac3", "eac3"],
    "supported_containers": ["mp4", "mkv"],
    "max_bandwidth_bps": 25000000,
    "max_resolution": "2160p"
  },
  "start_position_ms": 0
}
// 201
{
  "session": {
    "id": "sess_abc123",
    "mode": "direct" | "transcode",
    "direct_url": "/api/v1/stream/201/direct",        // if mode=direct
    "hls_master_url": "/api/v1/stream/sessions/sess_abc123/master.m3u8",  // if mode=transcode
    "media_file_id": 201,
    "duration_ms": 6900000
  }
}
```

### `GET /stream/:file_id/direct`

Direct file with HTTP Range. The auth check confirms the user can access
the file's parent item/episode. Cookies-based auth (a `<video>` tag sends
cookies on same-origin requests).

### `GET /stream/sessions/:id/master.m3u8`

HLS master playlist with one or more variants (resolution rungs).

### `GET /stream/sessions/:id/v/:variant/index.m3u8`

Variant playlist. Server writes the live manifest as segments are produced.

### `GET /stream/sessions/:id/v/:variant/seg-:n.ts`

Single MPEG-TS segment.

### `PATCH /stream/sessions/:id`

Seek. Server restarts the ffmpeg process with a new `-ss` offset and
renumbers segments. The frontend re-fetches the variant manifest.

```json
{ "seek_to_ms": 1800000 }
// 204
```

### `DELETE /stream/sessions/:id`

Explicit cleanup. Idempotent.

### `GET /stream/:file_id/subs/:stream_index.vtt`

Extracted subtitle as WebVTT. The server extracts on demand and caches.
External (sidecar) subs serve directly with conversion if needed.

## Images

### `GET /images/items/:id/poster`

### `GET /images/items/:id/backdrop`

### `GET /images/episodes/:id/thumb`

Query params: `?w=300&h=450&fit=cover`. Server resizes on demand and caches
the result. Original is fetched from TMDB or embedded extraction on first
request. Long browser cache (`Cache-Control: public, max-age=31536000, immutable`)
because URLs are content-addressed by `?v=<updated_at>`.

## WebSocket

### `WS /api/v1/ws`

Single connection per browser tab. Auth via the session cookie sent in
the upgrade request — no token in query string.

**Client → server messages:**

```json
{ "type": "subscribe", "topic": "scans" }
{ "type": "unsubscribe", "topic": "scans" }
{ "type": "keepalive" }       // every 30s, server reaps idle WS at 60s
```

**Topics (v0.1):**

- `scans` — scan job lifecycle (owner only)
- `transcode:<session_id>` — progress for one of YOUR active sessions
- `play-state` — cross-device sync of YOUR play state
- `library` — additions/removals you can access

**Server → client event shapes:**

```json
{ "type": "scan.started", "topic": "scans", "data": { "job_id": 42, "library_id": 1 } }
{ "type": "scan.progress", "topic": "scans",
  "data": { "job_id": 42, "files_seen": 1200, "files_added": 5 } }
{ "type": "scan.completed", "topic": "scans",
  "data": { "job_id": 42, "files_added": 5, "files_updated": 0, "files_removed": 0 } }

{ "type": "transcode.progress", "topic": "transcode:sess_abc123",
  "data": { "current_ms": 30000, "speed_x": 1.4 } }
{ "type": "transcode.ended", "topic": "transcode:sess_abc123",
  "data": { "reason": "stopped" | "completed" | "error", "message": "..." } }

{ "type": "play-state.updated", "topic": "play-state",
  "data": { "item_id": 101, "position_ms": 1200000, "watched": false, "source_session": "sess_abc123" } }

{ "type": "library.item_added", "topic": "library",
  "data": { "item": { /* item shape */ } } }
{ "type": "library.item_updated", "topic": "library",
  "data": { "item_id": 101 } }
{ "type": "library.item_removed", "topic": "library",
  "data": { "item_id": 101 } }
```

## Health & info

### `GET /health`

```json
{ "status": "ok", "version": "0.1.0-dev", "uptime_s": 12345 }
```

No auth required. Used by Docker healthcheck.

### `GET /server-info`

Auth required.

```json
{
  "version": "0.1.0-dev",
  "transcoder": { "hw_accel": ["videotoolbox"], "active_sessions": 2, "max_sessions": 8 },
  "library_counts": { "movies": 1234, "shows": 89, "episodes": 1500 }
}
```

## Out of scope for v0.1

- OpenAPI spec generation (planned for v0.2 once shape stabilizes).
- API tokens / programmatic clients (cookies-only for now).
- Pagination cursors (offset pagination is fine for v0.1 library sizes).
- Per-field selective response (no GraphQL, no sparse fieldsets).
- Public/anonymous endpoints beyond `/health`.

---

## Endpoint reference

Complete route list grouped by category. All routes are prefixed with
`/api/v1` except where noted. **Admin** group requires the
`OwnerAuth` extractor; everything else accepts any signed-in user
(some delegate finer-grained access checks per-resource).

### Auth & users

- `POST   /auth/setup` — first-run owner creation
- `POST   /auth/login`, `POST /auth/logout`
- `GET    /auth/status`, `GET /auth/me`, `PATCH /auth/me`
- `POST   /auth/me/password`
- `POST   /auth/me/email/request-change`, `POST /auth/me/email/confirm`
- `GET    /auth/me/sessions`, `DELETE /auth/me/sessions/{id}`
- `POST   /auth/sessions/revoke-others`
- `POST   /auth/register`, `GET /auth/users`, `PATCH /auth/users/{id}`, `DELETE /auth/users/{id}`
- `GET    /auth/invites`, `POST /auth/invites`, `DELETE /auth/invites/{id}`
- `POST   /auth/password-reset/request`, `POST /auth/password-reset/confirm`
- `GET    /auth/2fa/status`, `POST /auth/2fa/enroll`, `POST /auth/2fa/verify`, `POST /auth/2fa/disable`, `POST /auth/2fa/recovery-codes/regenerate`
- `POST   /auth/2fa/login`

### Libraries & items

- `GET    /libraries`, `POST /libraries`, `GET /libraries/{id}`, `PATCH /libraries/{id}`, `DELETE /libraries/{id}`
- `GET    /libraries/{id}/access`, `PUT /libraries/{id}/access`
- `POST   /libraries/{id}/scan`, `GET /libraries/{id}/scans`
- `GET    /libraries/{id}/stats`
- `POST   /libraries/{id}/verify`, `POST /libraries/{id}/purge`
- `POST   /libraries/{id}/detect-markers`, `POST /libraries/{id}/refresh-metadata`, `POST /libraries/{id}/generate-previews`
- `GET    /scans/{id}`
- `GET    /items`, `GET /items/trending`
- `GET    /items/{id}`, `PATCH /items/{id}`
- `DELETE /items/{id}/media` (gated by library `allow_media_deletion`)
- `GET    /items/{id}/trailer`, `GET /items/{id}/similar`, `GET /items/{id}/reviews`
- `POST   /items/{id}/refresh`, `GET /items/{id}/match-search`, `POST /items/{id}/match-apply`
- `PATCH  /items/{id}/credits`
- `GET    /items/{id}/tmdb-posters`, `POST /items/{id}/poster/from-tmdb`
- `POST   /items/{id}/poster`, `GET /items/{id}/poster/blob`
- `POST   /items/{id}/backdrop`, `GET /items/{id}/backdrop/blob`
- `POST   /items/{id}/detect-markers`
- `GET    /items/{id}/rating`, `PUT /items/{id}/rating`, `DELETE /items/{id}/rating` (Trakt)
- `GET    /items/{id}/external-subtitles`
- `GET    /items/{id}/tags`, `POST /items/{id}/tags`, `DELETE /items/{id}/tags/{tag_id}`
- `GET    /tags`
- `GET    /seasons/{id}`
- `GET    /episodes/{id}`, `DELETE /episodes/{id}/media`
- `GET    /episodes/{id}/rating`, `PUT /episodes/{id}/rating`, `DELETE /episodes/{id}/rating`
- `GET    /episodes/{id}/external-subtitles`

### Collections (manual + smart + auto)

- `GET    /collections`, `GET /collections/{id}`
- `GET    /collections/{id}/poster/blob`, `GET /collections/{id}/backdrop/blob`

### Streaming (routes)

- `POST   /stream/sessions`, `POST /stream/prewarm`
- `DELETE /stream/sessions/{id}`, `POST /stream/sessions/{id}/close` (sendBeacon alias)
- `POST   /stream/sessions/{id}/pause`, `POST /stream/sessions/{id}/resume`
- `GET    /stream/sessions/{id}/master.m3u8`
- `GET    /stream/sessions/{id}/{variant}/{name}` (segments + variant playlists)
- `GET    /stream/{file_id}/direct`

### Media-file ancillary

- `GET    /external-subtitles/{id}/file`

### Play state (routes)

- `POST   /play-state`, `POST /play-state/scrobble`, `POST /play-state/watched`
- `GET    /play-state/on-deck`, `GET /play-state/history`
- `GET    /play-state/config`

### Per-user

- `GET    /my-list`, `POST /my-list/{item_id}`, `DELETE /my-list/{item_id}`
- `GET    /prefs/hidden-libraries`, `PUT /prefs/hidden-libraries`
- `GET    /notifications`, `GET /notifications/unread-count`, `POST /notifications/{id}/read`, `POST /notifications/read-all`

### Trakt

- `POST   /trakt/link/start`, `POST /trakt/link/poll`, `GET /trakt/status`, `POST /trakt/unlink`
- `POST   /trakt/sync-now`

### Pre-roll

- `GET    /preroll/blob`

### Admin — server + observability

- `GET    /admin/dashboard`, `GET /admin/library-health`
- `GET    /admin/audit`, `GET /admin/logs`, `GET /admin/alerts`
- `GET    /admin/settings`, `PATCH /admin/settings`
- `GET    /admin/settings/email`, `PUT/DELETE /admin/settings/email/password`, `POST /admin/settings/email/test`
- `GET    /admin/agents`, `GET /admin/secrets`, `PUT/DELETE /admin/secrets/{name}`, `POST /admin/secrets/{name}/test`
- `GET    /admin/network`, `PATCH /admin/network`, `POST /admin/network/test-reachability`
- `GET    /admin/privacy`, `PATCH /admin/privacy`

### Admin — transcoder

- `GET    /admin/transcoder/capabilities`
- `GET    /admin/transcoder/presets`, `POST /admin/transcoder/presets`, `PATCH/DELETE /admin/transcoder/presets/{id}`
- `GET    /admin/optimized`, `POST /admin/optimized`, `DELETE /admin/optimized/{id}`

### Admin — scheduled tasks

- `GET    /admin/tasks`, `POST /admin/tasks`, `PATCH/DELETE /admin/tasks/{id}`, `POST /admin/tasks/{id}/run`, `GET /admin/tasks/{id}/runs`

### Admin — libraries + collections

- `GET    /admin/libraries/{id}/agents`, `PUT /admin/libraries/{id}/agents`
- `POST   /admin/collections`, `PATCH/DELETE /admin/collections/{id}`
- `POST   /admin/collections/{id}/items` (add), `PUT /admin/collections/{id}/items` (reorder), `DELETE /admin/collections/{id}/items/{item_id}`
- `POST   /admin/collections/{id}/poster`, `POST /admin/collections/{id}/backdrop`
- `POST   /admin/smart-collections`, `PUT /admin/smart-collections/{id}/rule`
- `POST   /admin/items/bulk/refresh-metadata`, `POST /admin/items/bulk/add-tag`, `POST /admin/items/bulk/remove-tag`, `POST /admin/items/bulk/detect-markers`

### Admin — users + access

- `GET    /admin/sessions`, `DELETE /admin/sessions/{id}`
- `GET    /admin/users/{id}/sessions`, `DELETE /admin/users/{id}/sessions`
- `POST   /admin/users/{id}/2fa/reset`, `POST /admin/users/{id}/unlock-attempts`
- `GET    /admin/access`, `PUT /admin/access`
- `GET    /admin/access-groups`, `POST /admin/access-groups`, `GET/PATCH/DELETE /admin/access-groups/{id}`
- `PUT    /admin/access-groups/{id}/libraries`, `PUT /admin/access-groups/{id}/members`
- `GET    /admin/users/{id}/access-groups`, `PUT /admin/users/{id}/access-groups`

### Admin — webhooks

- `GET    /admin/webhooks`, `POST /admin/webhooks`, `PATCH/DELETE /admin/webhooks/{id}`
- `POST   /admin/webhooks/{id}/test`, `GET /admin/webhooks/{id}/deliveries`

### Admin — backups + maintenance

- `POST   /admin/backup` (one-shot snapshot)
- `GET    /admin/backups`, `GET /admin/backups/{filename}/download`, `DELETE /admin/backups/{filename}`
- `POST   /admin/backups/{filename}/stage-restore`, `POST /admin/backups/cancel-restore`
- `POST   /admin/maintenance/verify-all`, `POST /admin/maintenance/purge-all`
- `POST   /admin/maintenance/vacuum`, `POST /admin/maintenance/clear-transcode-cache`

### Admin — pre-roll

- `GET    /admin/preroll`, `POST /admin/preroll` (multipart upload), `DELETE /admin/preroll`

### Health, info & WebSocket (routes)

- `GET    /health` (no auth; also at `/health` outside `/api/v1` for k8s/docker probes)
- `GET    /server-info`
- `WS     /ws`
