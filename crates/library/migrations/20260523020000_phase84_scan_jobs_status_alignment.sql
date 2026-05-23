-- Align `scan_jobs.status` on the convention every other read site
-- expects. `mark_scan_completed()` historically wrote `'completed'`,
-- but stats / dashboard / purge queries all filter on `'succeeded'`.
-- The mismatch made admin "Last scanned" render as "never" forever
-- for every library that had ever scanned successfully.
--
-- The write path is fixed in queries.rs (now writes `'succeeded'`).
-- This migration brings historical rows in line so the operator's
-- existing scan history surfaces too — without it, the drawer would
-- still show "never" until the next scan completes.

UPDATE scan_jobs
SET status = 'succeeded'
WHERE status = 'completed';
