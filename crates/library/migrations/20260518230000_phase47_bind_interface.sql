-- Phase 47 — operator-controlled bind address (preferred network interface).
--
-- Multi-NIC hosts can pin the HTTP listener to a specific interface
-- via settings instead of editing `BIND_ADDR`. Empty (default) means
-- "honor the env" → "0.0.0.0:8080" → bind all. A non-empty value
-- (e.g., `192.168.1.50:8080` or `[::1]:8080`) overrides the env at
-- startup. Read at bind time; changes need a restart (the listener
-- is already attached when settings hot-reload).

ALTER TABLE server_settings ADD COLUMN bind_interface TEXT NOT NULL DEFAULT '';
