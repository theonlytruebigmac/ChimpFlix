-- Phase 43 — operator-controlled HEVC (H.265) output for transcode sessions.
--
-- Three modes:
--   * 'off' (default): every transcode session outputs H.264.
--   * 'when_client_supports': sessions output HEVC when the player
--     reports HEVC support; fall back to H.264 otherwise. Safe to leave
--     on — playback never breaks because of this.
--   * 'always': force HEVC for every transcode. Will break playback on
--     clients that can't decode it (Firefox, older Chrome). Set only on
--     deployments where all clients are known to be Safari / new Edge.
--
-- HEVC inside MPEG-TS is fraught (browsers expect HEVC-in-fMP4); when
-- the output codec is HEVC the container is forced to fMP4 regardless
-- of `transcoder_container_format`.

ALTER TABLE server_settings ADD COLUMN transcoder_hevc_encoding_mode TEXT
    NOT NULL DEFAULT 'off'
    CHECK (transcoder_hevc_encoding_mode IN ('off', 'when_client_supports', 'always'));
