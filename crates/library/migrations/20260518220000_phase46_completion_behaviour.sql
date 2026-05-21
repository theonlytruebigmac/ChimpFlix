-- Phase 46 — video play completion behaviour.
--
-- Plex offers a 3-way picker for what counts as "watched":
--   * threshold_pct (default)     — scrobble at video_played_threshold_pct%.
--   * first_credits_marker        — scrobble when the first credits marker
--                                   (auto-detected by `detect_markers`) starts.
--                                   Falls back to threshold_pct when the file
--                                   has no credits marker.
--   * earliest_of_both            — scrobble at whichever comes first. The
--                                   intuitive default for a library where
--                                   detect_markers has been run on most files
--                                   but not all.
--
-- Drives both the on-deck filter (rail removes items past completion) and
-- the player's auto-scrobble.

ALTER TABLE server_settings ADD COLUMN video_completion_behaviour TEXT
    NOT NULL DEFAULT 'threshold_pct'
    CHECK (video_completion_behaviour IN
        ('threshold_pct', 'first_credits_marker', 'earliest_of_both'));
