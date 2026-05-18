-- Phase 18: per-session encoder-quality preset for the transcoder.
--
-- The hardware encoders ship with a speed-vs-quality dial (NVENC p1..p7,
-- libx264 ultrafast..veryslow, AMF speed/balanced/quality, ...). Until
-- now we hard-coded the "balanced" point on each branch. This column
-- lets the operator move the dial server-wide based on whether their
-- box has spare CPU/GPU headroom (push to quality) or is at capacity
-- (push to speed). Default `balanced` keeps existing deployments
-- behaving exactly as before this migration.

ALTER TABLE server_settings
    ADD COLUMN transcoder_encoder_preset TEXT NOT NULL DEFAULT 'balanced';
    -- One of: 'speed' | 'balanced' | 'quality'.
