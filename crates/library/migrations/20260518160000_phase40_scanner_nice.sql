-- Phase 40 — optional `nice -n N` wrapper around scanner/background ffmpeg.
--
-- The deferred analysis (see project memory) noted that calling
-- libc::nice() from a tokio task is racy: the task hops between
-- threads, and spawned children inherit whichever thread's nice was
-- active at fork. The cleaner fix — and the one this phase ships —
-- is the `nice` wrapper command: `nice -n N <ffmpeg> ...args`
-- forces the child into the requested priority unambiguously, no
-- matter which tokio worker forks it.
--
-- Read at server startup. Toggling requires a restart (we plumb
-- the value into a single `FfmpegConfig` held by AppState; runtime
-- swaps would mean re-wiring every helper).
--
-- 0 = off (no nice wrapper); 1..=19 = standard "nice" range.

ALTER TABLE server_settings ADD COLUMN scanner_nice_level INTEGER NOT NULL DEFAULT 0;
