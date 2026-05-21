-- Phase 32: network policy dials for an internet-exposed deployment.
--
-- Plex's Settings → Network page exposes a few values that matter for
-- an instance reachable from the open web. These are the subset we can
-- wire today without standing up entire new subsystems (relay, GDM,
-- and TLS bits stay out — TLS is terminated upstream, GDM is for TV
-- discovery we don't support, and relay is a Plex-cloud-only feature).
--
-- Added:
--   - `transcoder_reaper_idle_threshold_ms` — how long after the last
--     keepalive we kill an orphaned transcode session. Was hard-coded
--     to 90_000 in main.rs; default keeps that value. Lowering helps
--     mobile users (force-close doesn't fire a clean unload), raising
--     helps slow-network users whose keepalives sometimes hiccup.
--   - `max_remote_streams_per_user` — cap on concurrent sessions per
--     user when their request originates from outside the LAN. 0 means
--     unlimited (current behavior). When >0, sessions originating from
--     a `lan_networks` IP don't count and only "remote" sessions are
--     rate-limited.
--   - `lan_networks` — comma-separated CIDR list (e.g.
--     `192.168.0.0/16,10.0.0.0/8`). Used by the per-user remote cap
--     above to decide which requests count as "remote" and by the
--     auth bypass check below.
--   - `auth_bypass_cidrs` — comma-separated CIDR list of trusted
--     networks that may call the API without a session cookie. When a
--     request's client IP matches, the auth extractor returns the
--     server owner user instead of rejecting with 401. Useful for
--     Home Assistant / cron jobs / LAN automation against an
--     internet-exposed server. Empty (default) disables bypass.

ALTER TABLE server_settings
    ADD COLUMN transcoder_reaper_idle_threshold_ms INTEGER NOT NULL DEFAULT 90000;

ALTER TABLE server_settings
    ADD COLUMN max_remote_streams_per_user INTEGER NOT NULL DEFAULT 0;

ALTER TABLE server_settings
    ADD COLUMN lan_networks TEXT NOT NULL DEFAULT '';

ALTER TABLE server_settings
    ADD COLUMN auth_bypass_cidrs TEXT NOT NULL DEFAULT '';
