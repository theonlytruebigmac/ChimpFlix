-- Phase 42 — pre-roll video.
--
-- One operator-uploaded video that plays before the main content on
-- every watch session. Stored as a single file under
-- `<data_dir>/preroll/preroll.<ext>`; the toggle gates whether the
-- player surfaces it.
--
-- Keeping this to a single video (not a playlist) avoids the
-- "where do operator files live + reordering UX" question that
-- previously deferred this feature. A multi-video playlist can land
-- later if anyone asks.

ALTER TABLE server_settings ADD COLUMN preroll_path TEXT;
ALTER TABLE server_settings ADD COLUMN preroll_enabled INTEGER NOT NULL DEFAULT 0;
