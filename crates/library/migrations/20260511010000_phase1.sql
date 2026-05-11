-- Phase 1: scanner upsert support.
--
-- The scanner upserts items by (library_id, kind, sort_title). Limitation:
-- two distinct movies with the same title in the same library will collide.
-- Acceptable for v0.1 — multi-version support is a future enhancement.

CREATE UNIQUE INDEX uq_items_library_kind_title
    ON items(library_id, kind, sort_title);
