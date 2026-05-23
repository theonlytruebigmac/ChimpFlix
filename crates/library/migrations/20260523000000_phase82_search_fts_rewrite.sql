-- Rebuild items_fts so cast_names actually live-syncs from item_credits
-- changes, and switch off `content=''` (contentless) so updates to
-- existing rows stop orphaning postings.
--
-- Two latent bugs in the original triggers
-- (`20260512000000_fts_triggers.sql`) drove this:
--
-- 1. Cast-sync gap. The items INSERT/UPDATE triggers always set
--    `cast_names = ''`. The only place cast names ever made it into
--    FTS was the migration backfill, so any item created or re-
--    enriched after install was unsearchable by actor name.
--
-- 2. FTS5 contentless `delete` corruption. Contentless FTS5 requires
--    the original indexed values for ALL columns when issuing the
--    `'delete'` command. The UPDATE trigger passed `''` for
--    cast_names (per #1) — so any item whose cast had been backfilled
--    leaked stale cast postings on every subsequent UPDATE.
--
-- Fix: drop contentless mode (small storage hit on cast_names; trivial
-- trigger correctness), rebuild the index from the source of truth,
-- install triggers on item_credits so cast edits propagate.

DROP TRIGGER IF EXISTS items_ai;
DROP TRIGGER IF EXISTS items_au;
DROP TRIGGER IF EXISTS items_ad;
DROP TABLE IF EXISTS items_fts;

CREATE VIRTUAL TABLE items_fts USING fts5(
    title,
    original_title,
    summary,
    cast_names
    -- No `content=''` — FTS5 keeps its own copies of the column values.
    -- Costs ~2x storage for cast_names (a few MB at typical library
    -- size) but makes UPDATE/DELETE trivially correct: SQLite can
    -- remove the right postings without us having to remember the
    -- previous indexed values.
);

-- Backfill from the source of truth. Same shape as
-- `20260512000000_fts_triggers.sql:5-19` plus a freshly correct
-- cast_names pull.
INSERT INTO items_fts(rowid, title, original_title, summary, cast_names)
SELECT
    i.id,
    i.title,
    COALESCE(i.original_title, ''),
    COALESCE(i.summary, ''),
    COALESCE(
        (SELECT GROUP_CONCAT(p.name, ' ')
         FROM item_credits ic
         JOIN people p ON p.id = ic.person_id
         WHERE ic.item_id = i.id),
        ''
    )
FROM items i;

-- Per-item triggers. Non-contentless FTS5 lets us UPDATE specific
-- columns instead of having to issue a 'delete'+'insert' dance, which
-- closes the corruption window.

CREATE TRIGGER items_ai AFTER INSERT ON items BEGIN
    INSERT INTO items_fts(rowid, title, original_title, summary, cast_names)
    VALUES (
        new.id,
        new.title,
        COALESCE(new.original_title, ''),
        COALESCE(new.summary, ''),
        COALESCE(
            (SELECT GROUP_CONCAT(p.name, ' ')
             FROM item_credits ic
             JOIN people p ON p.id = ic.person_id
             WHERE ic.item_id = new.id),
            ''
        )
    );
END;

CREATE TRIGGER items_au AFTER UPDATE ON items BEGIN
    -- cast_names is owned by the item_credits triggers below — leave
    -- it alone here so a no-op item touch (e.g. `updated_at` bump)
    -- doesn't lose a freshly-synced cast list to a race.
    UPDATE items_fts
    SET title          = new.title,
        original_title = COALESCE(new.original_title, ''),
        summary        = COALESCE(new.summary, '')
    WHERE rowid = new.id;
END;

CREATE TRIGGER items_ad AFTER DELETE ON items BEGIN
    DELETE FROM items_fts WHERE rowid = old.id;
END;

-- item_credits triggers. Cast is a derived field on items_fts; any
-- mutation on item_credits has to re-aggregate the GROUP_CONCAT for
-- the affected item and write it back.

CREATE TRIGGER item_credits_ai AFTER INSERT ON item_credits BEGIN
    UPDATE items_fts
    SET cast_names = COALESCE(
        (SELECT GROUP_CONCAT(p.name, ' ')
         FROM item_credits ic
         JOIN people p ON p.id = ic.person_id
         WHERE ic.item_id = new.item_id),
        ''
    )
    WHERE rowid = new.item_id;
END;

CREATE TRIGGER item_credits_au AFTER UPDATE ON item_credits BEGIN
    -- An UPDATE could in theory move a credit between items (re-
    -- pointing item_id). Refresh both sides defensively — the WHERE
    -- on rowid no-ops when the id is the same.
    UPDATE items_fts
    SET cast_names = COALESCE(
        (SELECT GROUP_CONCAT(p.name, ' ')
         FROM item_credits ic
         JOIN people p ON p.id = ic.person_id
         WHERE ic.item_id = old.item_id),
        ''
    )
    WHERE rowid = old.item_id;
    UPDATE items_fts
    SET cast_names = COALESCE(
        (SELECT GROUP_CONCAT(p.name, ' ')
         FROM item_credits ic
         JOIN people p ON p.id = ic.person_id
         WHERE ic.item_id = new.item_id),
        ''
    )
    WHERE rowid = new.item_id;
END;

CREATE TRIGGER item_credits_ad AFTER DELETE ON item_credits BEGIN
    UPDATE items_fts
    SET cast_names = COALESCE(
        (SELECT GROUP_CONCAT(p.name, ' ')
         FROM item_credits ic
         JOIN people p ON p.id = ic.person_id
         WHERE ic.item_id = old.item_id),
        ''
    )
    WHERE rowid = old.item_id;
END;
