-- Phase 70: per-kind job concurrency overrides.
--
-- Each job kind in `crates/server/src/tasks/registry.rs` ships a
-- conservative default `concurrency` (1 for CPU-bound ffmpeg work, 2–4
-- for network-bound). On hefty boxes those defaults leave 80% of the
-- CPU idle. Storing overrides here lets the admin UI tune per-kind
-- caps live — the settings PATCH path calls
-- `KindLimiter::resize(name, cap)` to swap the in-flight semaphore
-- without restarting the worker pool.
--
-- Shape: JSON object `{ "<job_kind>": <positive integer>, ... }`.
-- Empty object = use registry defaults for every kind (the shipped
-- behaviour before this column existed). Unknown keys are tolerated
-- (forward-compat for kinds removed across versions).

ALTER TABLE server_settings
ADD COLUMN job_kind_concurrency TEXT NOT NULL DEFAULT '{}';
