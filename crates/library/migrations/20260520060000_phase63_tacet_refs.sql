-- Phase 63 — Tacet season fingerprint references
--
-- One row per (show_id, season_number). Holds the bincoded
-- `Vec<tacet::matching::ReferenceFingerprint>` for the intro window
-- and credits window of that season, plus the timestamp of the
-- last successful bootstrap.
--
-- A season's references are built by the `bootstrap_season_refs`
-- job once at least 3 episodes have audio that tacet can decode
-- successfully. Future per-episode detection (`detect_markers_file`)
-- loads the row and calls `tacet::detection::detect_single_episode`.
--
-- We store the references as opaque BLOBs because the inner type is
-- owned by tacet and may change schema across upstream versions.
-- bincode plus tacet's own forward-compatible serde defaults handle
-- the migration story — older rows decode into a newer tacet build
-- as long as fields are only added, not removed.

CREATE TABLE show_season_intro_refs (
    show_id          INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    season_number    INTEGER NOT NULL,
    intro_refs_blob  BLOB NOT NULL,
    credits_refs_blob BLOB NOT NULL,
    refs_built_at    INTEGER NOT NULL,
    PRIMARY KEY (show_id, season_number)
);

CREATE INDEX idx_show_season_intro_refs_built
    ON show_season_intro_refs(refs_built_at);
