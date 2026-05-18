-- Phase 12a: external subtitle agent.
--
-- Holds subtitle tracks fetched from external agents (OpenSubtitles
-- today; future: Subscene, Addic7ed) plus operator-uploaded sidecar
-- files. Embedded subtitle streams remain on `media_streams`; this is
-- the parallel surface the unified player picker merges into a single
-- list at playback time.

CREATE TABLE external_subtitles (
    id              INTEGER PRIMARY KEY,
    item_id         INTEGER REFERENCES items(id) ON DELETE CASCADE,
    episode_id      INTEGER REFERENCES episodes(id) ON DELETE CASCADE,
    language        TEXT NOT NULL,                -- ISO 639-2 / 3-letter code
    source          TEXT NOT NULL,                -- 'opensubtitles' | 'manual'
    source_file_id  TEXT,                          -- OpenSubtitles file_id; dedup key
    file_path       TEXT NOT NULL,                 -- absolute on-disk path
    forced          INTEGER NOT NULL DEFAULT 0,    -- 1 = "forced narrative"
    sdh             INTEGER NOT NULL DEFAULT 0,    -- 1 = hearing-impaired
    created_at      INTEGER NOT NULL,
    CHECK ((item_id IS NULL) <> (episode_id IS NULL)),
    UNIQUE(source, source_file_id)
);
CREATE INDEX idx_extsubs_item    ON external_subtitles(item_id)    WHERE item_id    IS NOT NULL;
CREATE INDEX idx_extsubs_episode ON external_subtitles(episode_id) WHERE episode_id IS NOT NULL;
