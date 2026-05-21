-- Keep items_fts in sync with items. Backfills existing rows first so
-- search works against pre-existing libraries without a rescan, then
-- installs triggers that maintain the index on subsequent writes.

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
FROM items i
WHERE NOT EXISTS (SELECT 1 FROM items_fts WHERE items_fts.rowid = i.id);

CREATE TRIGGER items_ai AFTER INSERT ON items BEGIN
    INSERT INTO items_fts(rowid, title, original_title, summary, cast_names)
    VALUES (
        new.id,
        new.title,
        COALESCE(new.original_title, ''),
        COALESCE(new.summary, ''),
        ''
    );
END;

CREATE TRIGGER items_ad AFTER DELETE ON items BEGIN
    INSERT INTO items_fts(items_fts, rowid, title, original_title, summary, cast_names)
    VALUES (
        'delete',
        old.id,
        old.title,
        COALESCE(old.original_title, ''),
        COALESCE(old.summary, ''),
        ''
    );
END;

-- After UPDATE we replace the row by deleting + inserting. FTS5 contentless
-- tables need the old indexed values supplied to `delete`, which we don't
-- always have on UPDATE — so we instead reindex from scratch with the new
-- values. The `delete-all` command rebuilds the entry cleanly.
CREATE TRIGGER items_au AFTER UPDATE ON items BEGIN
    INSERT INTO items_fts(items_fts, rowid, title, original_title, summary, cast_names)
    VALUES (
        'delete',
        old.id,
        old.title,
        COALESCE(old.original_title, ''),
        COALESCE(old.summary, ''),
        ''
    );
    INSERT INTO items_fts(rowid, title, original_title, summary, cast_names)
    VALUES (
        new.id,
        new.title,
        COALESCE(new.original_title, ''),
        COALESCE(new.summary, ''),
        ''
    );
END;
