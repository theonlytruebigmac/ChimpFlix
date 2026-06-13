-- Phase 99: invite-only / allow-signups toggle.
--
-- ChimpFlix has always been invite-only in practice (the /auth/register
-- handler required an invite code). This adds an explicit master switch
-- so an operator can choose to allow open self-registration — a new
-- account created with no invite — or lock the server down to
-- invite-only.
--
-- `allow_signups` : when ON (the default — open registration, matching
--                   the previous behaviour for anyone who wired up a
--                   self-signup form), the /auth/register handler will
--                   provision an account even with no invite code. When
--                   OFF, only invite-bearing registration succeeds; a
--                   codeless self-signup is rejected with a clear error.
--                   Invite-based registration ALWAYS works regardless of
--                   this flag.

ALTER TABLE server_settings
    ADD COLUMN allow_signups INTEGER NOT NULL DEFAULT 1;
