-- Phase 45 — split GPU vs CPU max-concurrent transcode caps.
--
-- The existing `transcoder_max_concurrent` is the overall ceiling
-- (regardless of encoder). Operators with a GPU often want CPU
-- (software libx264 / libx265) encodes capped separately — a single
-- CPU encode pegs N cores and noticeably degrades a parallel GPU
-- session. Default 1 matches "one CPU encode at a time on top of the
-- overall cap" semantics.

ALTER TABLE server_settings ADD COLUMN transcoder_max_cpu_concurrent INTEGER NOT NULL DEFAULT 1;
