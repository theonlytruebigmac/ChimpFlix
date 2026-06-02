-- Phase 100: per-account lock / disable.
--
-- `locked` : when 1, the account is disabled — the /auth/login handler
--            rejects it AFTER the password check (so a locked username
--            can't be enumerated from the pre-credential path), with a
--            clear "account is locked" error. 0 = active (the default).
--            Owners are never lockable (guarded in the admin route + the
--            login gate skips the check for owners), so the system can
--            never be left with no usable owner.
ALTER TABLE users ADD COLUMN locked INTEGER NOT NULL DEFAULT 0;
