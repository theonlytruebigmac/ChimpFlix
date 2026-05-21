-- Phase 3: Plex-equivalent library fields + per-library metadata agent
-- priority list.
--
-- The new library columns mirror the surface Plex exposes in the Edit
-- Library dialog (episode sort/order, certification country, visibility).
-- We default to Plex's defaults so existing libraries don't behave
-- differently after this migration runs.
--
-- library_agents stores the ordered chain of metadata providers per
-- library. The seed inserts mirror today's hardcoded behavior (TMDB
-- primary, TVMaze fallback for shows) so the refactor is a no-op until
-- the user reorders the chain. `config_json` is reserved for per-agent
-- options like region codes or include-adult flags.

ALTER TABLE libraries ADD COLUMN episode_sort_order      TEXT NOT NULL DEFAULT 'oldest_first';  -- 'oldest_first' | 'newest_first'
ALTER TABLE libraries ADD COLUMN episode_naming          TEXT NOT NULL DEFAULT 'tmdb';          -- 'tmdb' | 'original' | 'absolute'
ALTER TABLE libraries ADD COLUMN certification_country   TEXT NOT NULL DEFAULT 'US';
ALTER TABLE libraries ADD COLUMN visibility              TEXT NOT NULL DEFAULT 'home_and_search';  -- 'home_and_search' | 'search_only' | 'hidden'

CREATE TABLE library_agents (
    library_id  INTEGER NOT NULL REFERENCES libraries(id) ON DELETE CASCADE,
    agent_name  TEXT    NOT NULL,                  -- 'tmdb' | 'tvmaze' | future
    priority    INTEGER NOT NULL,                  -- lower = runs first
    enabled     INTEGER NOT NULL DEFAULT 1,
    config_json TEXT    NOT NULL DEFAULT '{}',
    PRIMARY KEY (library_id, agent_name)
);

CREATE INDEX idx_library_agents_priority
    ON library_agents(library_id, priority)
    WHERE enabled = 1;

-- Seed defaults so the refactored pipeline behaves identically to today.
-- Movie libraries: TMDB only. Show libraries: TMDB first, TVMaze fallback.
INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
SELECT id, 'tmdb', 0, 1, '{}' FROM libraries;

INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
SELECT id, 'tvmaze', 1, 1, '{}' FROM libraries WHERE kind = 'shows';
