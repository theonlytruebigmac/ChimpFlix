-- Phase 13a: enroll every existing library in the TVDB agent chain so the
-- backfill agent runs against existing data the same way TVMaze does for
-- shows. New libraries get the same row seeded by queries::create_library;
-- this migration covers anything created before the agent shipped.

INSERT INTO library_agents (library_id, agent_name, priority, enabled, config_json)
SELECT id, 'tvdb', 2, 1, '{}' FROM libraries
WHERE NOT EXISTS (
    SELECT 1 FROM library_agents
    WHERE library_agents.library_id = libraries.id
      AND library_agents.agent_name = 'tvdb'
);
