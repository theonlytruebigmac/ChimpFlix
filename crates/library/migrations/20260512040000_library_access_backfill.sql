-- Preserve current "everyone sees everything" behavior by granting all
-- existing users access to all existing libraries. From here on, owners
-- bypass the check entirely (they always see everything) and non-owners
-- only see libraries they have an explicit access row for.

INSERT INTO library_access (user_id, library_id)
SELECT u.id, l.id
FROM users u
CROSS JOIN libraries l
WHERE NOT EXISTS (
    SELECT 1 FROM library_access
    WHERE user_id = u.id AND library_id = l.id
);
