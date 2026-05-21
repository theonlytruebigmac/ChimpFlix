-- Phase 54 — pre-roll volume.
--
-- Operator-set output level for the pre-roll sting. 0..=100 (percent of
-- the source's authored level). Defaults to 100 so existing installs
-- behave identically; many operators upload pre-rolls that were
-- mastered at theater levels and want to dial them down so a sting
-- doesn't blow out viewers' speakers before the show audio normalises.

ALTER TABLE server_settings ADD COLUMN preroll_volume INTEGER NOT NULL DEFAULT 100;
