-- Phase 80 — Federated auth providers per user (starting with Plex).
--
-- Each row in `user_auth_providers` binds a ChimpFlix user to an
-- external identity. Today the only `provider` value is `'plex'`; a
-- future Google OAuth integration will use `'google'` against the same
-- shape so we don't have to migrate again.
--
-- Constraints:
--   * `(provider, external_id)` is unique — one Plex account links to
--     exactly one ChimpFlix user. Trying to link the same Plex identity
--     to a second user surfaces as a clean UNIQUE-violation rather than
--     silently overwriting.
--   * `(user_id, provider)` is unique — a single ChimpFlix user keeps
--     at most one Plex link active. Replacing a link means unlinking
--     and re-linking.
--
-- `external_email` / `external_username` are *snapshots* taken at link
-- time. They aren't kept in sync with the upstream account — if the
-- user renames themselves on Plex we don't follow. They exist purely
-- so the admin UI can show "linked to alice@plex.tv" without making a
-- network round-trip.
CREATE TABLE user_auth_providers (
    id                INTEGER PRIMARY KEY,
    user_id           INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider          TEXT NOT NULL,
    external_id       TEXT NOT NULL,
    external_email    TEXT,
    external_username TEXT,
    linked_at         INTEGER NOT NULL,
    last_login_at     INTEGER,
    UNIQUE (provider, external_id),
    UNIQUE (user_id, provider)
);
CREATE INDEX idx_user_auth_providers_user ON user_auth_providers(user_id);

-- Relax `users.password_hash` NOT NULL so a Plex-only signup (via
-- invite) can create a user without a local password. The login path
-- treats NULL as "password login disabled — sign in with a linked
-- provider, or use the password-reset email flow to set one". The
-- `Owner` role guard in `queries::create_user_no_password` keeps
-- refusing to mint an owner without a password so plex.tv being down
-- can never lock out the only admin.
--
-- SQLite doesn't support `ALTER COLUMN`, so we rebuild-and-swap. The
-- table is small (one row per local account) and the migration pool
-- runs with `foreign_keys=OFF` so the FK cascades from sessions /
-- audit_log / play_state / etc. aren't fired against the in-progress
-- table swap.
CREATE TABLE users_new (
    id                      INTEGER PRIMARY KEY,
    username                TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash           TEXT,
    role                    TEXT NOT NULL,
    display_name            TEXT,
    avatar_path             TEXT,
    created_at              INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    default_audio_lang      TEXT,
    default_subtitle_lang   TEXT,
    email                   TEXT,
    notify_via_email        INTEGER NOT NULL DEFAULT 0,
    last_login_at           INTEGER,
    last_login_ip           TEXT,
    previous_login_at       INTEGER,
    previous_login_ip       TEXT
);

INSERT INTO users_new (
    id, username, password_hash, role, display_name, avatar_path,
    created_at, updated_at,
    default_audio_lang, default_subtitle_lang,
    email, notify_via_email,
    last_login_at, last_login_ip, previous_login_at, previous_login_ip
)
SELECT
    id, username, password_hash, role, display_name, avatar_path,
    created_at, updated_at,
    default_audio_lang, default_subtitle_lang,
    email, notify_via_email,
    last_login_at, last_login_ip, previous_login_at, previous_login_ip
FROM users;

DROP TABLE users;
ALTER TABLE users_new RENAME TO users;

-- Stable per-install identifier we present to plex.tv. Generated
-- lazily on the first `/auth/plex/start` call and persisted here so
-- subsequent restarts keep the same identity. Resetting it invalidates
-- in-flight PINs but not already-linked accounts — those are keyed by
-- the *Plex* user uuid, not our client identifier.
--
-- Plex auth itself isn't behind a feature flag: the invite system
-- already gates who can sign up (an unrecognized Plex identity with no
-- invite is rejected by `/auth/plex/poll`), so a redundant master
-- switch would just be one more thing for operators to misconfigure.
-- If a future deployment wants to fully disable Plex auth, dropping
-- this column to NULL is the operator switch (the runtime treats an
-- absent identifier as "not configured").
ALTER TABLE server_settings ADD COLUMN plex_client_identifier TEXT;
