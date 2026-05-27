# V2 Schema — Greenfield Design

> Status: **planning draft.** Defines the design principles + sketch
> for V2's database schema. Final table definitions land in
> `crates/db/migrations/0001_initial.sql` in Phase 1.

## Design principles

1. **External IDs are first-class.** Every importable identity (movie,
   show, episode, person) is keyed on its provider IDs (TMDB, IMDB,
   TVDB, AniList) from row 1. V1 makes the provider IDs columns on a
   shared item identity; V2 makes them the foundation. This is what
   unlocks a clean V1→V2 user-data importer and what lets the metadata
   pipeline tolerate provider rename / merge events.
2. **Episodes are first-class rows, not denormalized children.** V1
   treats episodes ambiguously — sometimes as items, sometimes via
   joins. V2 has dedicated `episodes` rows from the start, and the
   `play_state` table is episode-only for shows and movie-only for
   movies. The CHECK constraint V1 added late in life is V2's first
   line of code.
3. **Files are siblings of identities, not subordinates.** A media
   file is a physical artifact on disk. An identity (movie or
   episode) may have zero or many files. V1 routinely confuses these
   — searches return identities, browse shows files, soft-delete
   reconciliation needs both. V2 names the distinction explicitly:
   `media_files` table holds disk artifacts; `movies` / `episodes`
   hold identities; a join table maps one to the other.
4. **Soft-delete is opt-in per table, not a global pattern.**
   `media_files.removed_at` exists because mount races and partial
   unmounts demand it. `users.removed_at` does not — user deletion is
   either a real delete or a transactional account-disabled flag.
   Don't bolt soft-delete on every table just because it's easy.
5. **Settings are typed.** V1 ships a `server_settings` table with
   typed columns per setting (Phase TBD). V2 keeps this approach. No
   `key TEXT, value TEXT` settings table; every operator-visible knob
   is a real column with a real type, validated at the patch boundary.
6. **State tables are explicit.** V1 introduced `trakt_collection_state`
   / `trakt_watchlist_state` / `trakt_last_activities` as separate
   tables when sync state needed durability. V2 adopts this from the
   start: any sync mirror, cursor, or "what we've already pushed"
   state lives in its own table, named so it's clear it's not the
   primary record.
7. **Audit log is narrow.** V1 deliberately scoped audit_log to
   security-relevant ops (role changes, credential edits). V2 keeps
   that scope. Activity logging for the UI is a separate concern
   served by event records, not audit.
8. **Pagination is universal.** Every list endpoint paginates from
   day one. No `LIMIT 1000` defaults. (V1 mostly does this; V2 makes
   it a schema-level expectation via consistent `created_at` /
   `sort_key` columns on every list-yielding table.)
9. **FTS is built in, not bolted on.** V1's search FTS table came in
   Phase 82 as a rewrite. V2 ships with the FTS5 virtual table from
   migration 0001, populated by triggers. Same bm25 ranking and
   column weights V1 settled on after iteration (title=10,
   original=5, cast=3, summary=1).

## Identity model

The trickiest design call in V2 is the identity model. V1 has a single
`items` table for movies + shows (with `kind` discriminator) and an
`episodes` table for episodes. V2 should consider whether to split:

**Option A: Single `items` table, like V1.**
- Pros: One place to query for "anything in any library." Search is
  natural. Less join complexity.
- Cons: `kind` discriminator means many columns are non-applicable for
  many rows. CHECK constraints proliferate. Foreign keys from `episodes`
  to `items` (parent show) work fine but the type of the parent is
  implicit.

**Option B: Separate `movies` + `shows` tables.**
- Pros: Each table's columns are all applicable. Foreign keys are
  typed (an episode's `show_id` references `shows.id`, not `items.id`
  with a runtime check).
- Cons: Cross-type queries (home page rails, search) need UNION ALL
  or a view. Browse-by-library needs to know which table to read
  per library kind.

**Option C: Hybrid — `media_titles` (common identity bag) + per-kind
extension tables (`movie_details`, `show_details`).**
- Pros: Cross-kind queries hit `media_titles` only. Kind-specific
  columns live in their own table.
- Cons: Two-table reads for any detail page. Joins.

**Recommendation: Option B, with a materialized view for cross-kind
reads.** The migration overhead is real but pays back in every query
that doesn't need the discriminator dance. Search FTS index can union
across the two tables in its triggers. Browse logic dispatches on
library.kind already.

Final call deferred to Phase 1 prototyping — write the search +
browse + home queries against both Option A and Option B sketches,
pick the one that's clearer.

## Table sketch (informal)

> Not a migration. A conversation about what's there.

### Library

- `libraries` — id, name, kind (movies/shows/anime), language,
  hidden, settings JSON for kind-specific config.
- `library_paths` — library_id, path. Multiple per library.
- `library_agents` — library_id, agent_name, priority, enabled,
  is_primary.

### Identities

- `movies` — id, tmdb_id, imdb_id, title, original_title, summary,
  year, runtime_minutes, poster_path, backdrop_path, language,
  popularity, …
- `shows` — id, tmdb_id, tvdb_id, imdb_id, anilist_id, title,
  original_title, summary, year, status, network, language, …
- `seasons` — id, show_id, season_number, name, summary, poster_path,
  episode_count, air_date.
- `episodes` — id, show_id, season_id, episode_number,
  absolute_number, title, summary, air_date, runtime_minutes,
  still_path.
- `people` — id, tmdb_id, name, profile_path, biography.
- `cast_credits` — person_id, movie_id OR episode_id (one of), order,
  character.
- `crew_credits` — person_id, movie_id OR show_id (one of), job,
  department.

(Note the OR — V2 can use partial indexes + check constraints to keep
this clean, or split into per-target tables. Decide in Phase 1.)

### Files

- `media_files` — id, library_id, path (unique within library),
  size_bytes, container, video_codec, audio_codec, audio_channels,
  height, width, hdr_format, duration_ms, hash, scanned_at,
  removed_at (nullable), match_status (matched/unmatched/conflict),
  match_target_kind (movie/episode), match_target_id.
- `media_file_streams` — file_id, stream_index, kind (video/audio/
  subtitle), codec, language, default, forced, title.
- `subtitle_files` — file_id, path, language, format.

### Personalization

- `users` — id, username, email, role (owner/admin/user), created_at,
  password_hash (nullable for OAuth-only users), …
- `user_auth_providers` — user_id, provider (local/plex/google),
  external_id, linked_at.
- `play_state` — user_id, movie_id OR episode_id, position_ms,
  duration_ms, watched, view_count, last_played_at.
- `ratings` — user_id, movie_id OR show_id (one of), value, created_at.
- `watchlist` — user_id, movie_id OR show_id (one of), added_at.
- `watch_history` — user_id, movie_id OR episode_id, played_at,
  watched_seconds.
- `user_lists` — id, owner_user_id, name, sort_order.
- `user_list_items` — list_id, movie_id OR show_id (one of), position.

### Operational

- `jobs` — defined in `JOBS.md`. Shape depends on whether V2 keeps
  the table-as-queue model or moves to a different primitive.
- `scan_jobs` — separate from `jobs` because scan progress is a
  first-class operator view. id, library_id, status, started_at,
  finished_at, files_seen, files_added, files_updated, files_removed,
  error.
- `audit_log` — narrow security audit. actor_user_id, action,
  target_kind, target_id, at, metadata JSON.
- `webhooks` — id, name, url, secret, events, enabled,
  last_delivery_at, last_status.
- `invites` — id, code, created_by_user_id, role, expires_at,
  used_at, used_by_user_id.

### Sync state

- `trakt_collection_state` — what we've pushed to Trakt.
- `trakt_watchlist_state` — same for watchlist.
- `trakt_last_activities` — Trakt's last-activities cursor.
- `plex_account_link` — plex account → user_id mapping.

### Real-time

- (No tables. The realtime hub is in-memory; see `REALTIME.md`.
  Long-term event history, if needed, lives in `events` — Phase TBD.)

### Settings + secrets

- `server_settings` — singleton row with typed columns per setting.
- `secrets` — credential vault, encrypted at rest. Owner-only access.

## Naming conventions

- Tables: plural snake_case (`media_files`, not `media_file`).
- Primary keys: always `id INTEGER PRIMARY KEY`. No UUIDs unless a
  specific reason emerges.
- Foreign keys: `<singular_target>_id`. Always indexed.
- Timestamps: `<verb>_at` (`created_at`, `last_played_at`,
  `removed_at`). UTC, stored as `INTEGER` Unix seconds.
- Booleans: `INTEGER NOT NULL DEFAULT 0/1`, read via `!= 0` (Turso
  follows the SQLite idiom).
- Enums: TEXT with a CHECK constraint listing allowed values. No int
  enum codes — they're unreadable in `SELECT *`.
- No reserved-word column names. (`role` is fine; `order` is not —
  prefer `sort_order`.)

## Index strategy

- Every foreign key gets an index.
- Every "list these in order" query gets a covering compound index.
- `media_files.path` is UNIQUE per library.
- `play_state` indexed by `(user_id, last_played_at DESC)` for
  Continue Watching.
- FTS5 virtual table for search, populated by triggers on
  movies/shows/episodes/people.
- Partial indexes for "active" rows (e.g. `WHERE removed_at IS NULL`)
  where the table has a soft-delete column. (V1 Phase 83 introduced
  this; V2 ships it from day one.)

## Open questions

- **Movie / show split decision.** Option B vs. A above. Final call
  in Phase 1.
- **Episode crew vs. show crew.** TMDB attaches crew at the episode
  level; V1 stores some at show-level. V2 can store both with
  appropriate joins.
- **Person credit shape.** One `credits` table with optional FKs to
  movie/episode/show, or three separate tables. Tradeoff is query
  ergonomics vs. constraint cleanliness.
- **User-data importer.** Does V2 ship one? See `STORAGE.md`. If yes,
  the schema must guarantee that V1's identifiers (file paths +
  external IDs) can resolve to V2 rows.
- **Multi-version episode support.** A show with both Blu-ray and
  WEB-DL versions of the same episode — V1 supports this as multiple
  `media_files` for one `episode_id`. V2 should too. UI implications
  for `FRONTEND.md`.

## Cut list

- **EAV settings table.** Considered briefly; the typed-columns
  approach is harder to add to but easier to read and validate. Reject.
- **Soft-delete on every table.** Selective soft-delete only where
  the domain demands it.
- **Versioned schema metadata.** No "schema_version" table; migration
  history is canonical.
- **JSONB / heavy JSON column use.** Turso/SQLite JSON columns are
  fine for sparse-but-typed payloads (e.g. `metadata` on `audit_log`)
  but V2 does not lean on JSON for relational data.
