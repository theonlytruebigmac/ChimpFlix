-- Phase 51 — configurable "Recently Added" window.
--
-- The Card component used a hardcoded 14-day window for the
-- "Recently Added" ribbon. This makes the duration operator-tunable;
-- 0 disables the badge entirely (some operators prefer their library
-- view uncluttered), 365 is the upper bound (anything longer is just
-- "always badged" and defeats the point).

ALTER TABLE server_settings
    ADD COLUMN recently_added_days INTEGER NOT NULL DEFAULT 14
        CHECK (recently_added_days BETWEEN 0 AND 365);
