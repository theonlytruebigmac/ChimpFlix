-- Phase 75 — Per-episode credits with source attribution.
--
-- Companion to phase 74 (`item_credits.source`). Where item-level
-- credits already exist, this adds the symmetric per-episode table
-- so when a future agent surfaces episode-level cast (TVDB v4
-- exposes guest stars via `/episodes/{id}/extended`; AniDB has
-- per-episode staff) the apply layer has somewhere to land the rows.
--
-- Schema deliberately mirrors `item_credits` (with `episode_id`
-- instead of `item_id`). Same source-scoped DELETE pattern means a
-- TMDB credits pass can refresh its own episode-cast list without
-- wiping TVDB's, and vice versa.
--
-- The table is empty until an agent populates it. The metadata
-- crate's `EpisodeData.people` field already exists for that purpose;
-- the `apply_episode_credits_for_source` helper (queries.rs) writes
-- rows here once a `MetadataAgent::fetch_episode` returns a non-empty
-- `people` Vec.

CREATE TABLE episode_credits (
    id              INTEGER PRIMARY KEY,
    episode_id      INTEGER NOT NULL REFERENCES episodes(id) ON DELETE CASCADE,
    person_id       INTEGER NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    role_kind       TEXT NOT NULL,                 -- 'cast' | 'guest' | 'director' | 'writer' | 'crew'
    role            TEXT NOT NULL,                 -- free-form ("Actor", "Director", "Guest Star")
    character_name  TEXT,                          -- populated for role_kind = 'cast' / 'guest'
    sort_order      INTEGER NOT NULL DEFAULT 0,
    source          TEXT NOT NULL DEFAULT 'tmdb'
);

CREATE INDEX idx_episode_credits_ep ON episode_credits(episode_id, sort_order);
CREATE INDEX idx_episode_credits_ep_source ON episode_credits(episode_id, source);
CREATE INDEX idx_episode_credits_person ON episode_credits(person_id);
