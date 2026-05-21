-- Phase 13c: enroll every existing anime library in the AniList agent
-- chain as the primary metadata source. Idempotent — new anime
-- libraries get the row from queries::create_library.

INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
SELECT id, 'anilist', 0, 1, '{}' FROM libraries
WHERE kind = 'anime'
  AND NOT EXISTS (
    SELECT 1 FROM library_agents
    WHERE library_agents.library_id = libraries.id
      AND library_agents.agent_name = 'anilist'
  );

-- Demote TMDB on existing anime libraries so the priority list reflects
-- AniList-primary ordering.
UPDATE library_agents
   SET priority = 1
 WHERE agent_name = 'tmdb'
   AND library_id IN (SELECT id FROM libraries WHERE kind = 'anime');
