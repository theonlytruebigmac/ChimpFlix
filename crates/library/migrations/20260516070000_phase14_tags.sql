-- Phase 14 (Tier 1): user-managed tags.
--
-- Distinct from `genres` (which the metadata pipeline writes from TMDB
-- categories). Tags are operator-owned — handy for free-form curation
-- like "rewatch", "comfort movies", "kid-friendly" — and persist
-- through scans + refreshes since they're not touched by enrichment.

CREATE TABLE tags (
    id      INTEGER PRIMARY KEY,
    name    TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE item_tags (
    item_id INTEGER NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    tag_id  INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (item_id, tag_id)
);
CREATE INDEX idx_item_tags_tag ON item_tags(tag_id);
