-- Phase 77 — Surface TVDB episode id on episodes.
--
-- Mirror of phase 76's `items.tvmaze_id` add. `apply_episode_data`
-- writes `episodes.tvdb_id` from `EpisodeData.tvdb_id` (populated by
-- `TvdbAgent::fetch_episode`), but the original schema only had
-- `episodes.tmdb_id`. Every episode-level write from TVDB / OMDb /
-- AniList through `apply_episode_data` hit `no such column: tvdb_id`
-- and bailed.

ALTER TABLE episodes ADD COLUMN tvdb_id INTEGER;

CREATE INDEX idx_episodes_tvdb ON episodes(tvdb_id) WHERE tvdb_id IS NOT NULL;
