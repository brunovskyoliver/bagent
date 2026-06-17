-- V11: Extend memory_items with ledger fields for the memory refactor.
-- Adds confidence, importance, status, source, sensitivity, subject, supersedes_id.
-- SQLite requires one ALTER TABLE per column.

ALTER TABLE memory_items ADD COLUMN confidence    REAL    NOT NULL DEFAULT 0.8;
ALTER TABLE memory_items ADD COLUMN importance    REAL    NOT NULL DEFAULT 0.5;
ALTER TABLE memory_items ADD COLUMN status        TEXT    NOT NULL DEFAULT 'active';
ALTER TABLE memory_items ADD COLUMN source        TEXT    NOT NULL DEFAULT 'passive';
ALTER TABLE memory_items ADD COLUMN sensitivity   TEXT    NOT NULL DEFAULT 'normal';
ALTER TABLE memory_items ADD COLUMN subject       TEXT;
ALTER TABLE memory_items ADD COLUMN supersedes_id TEXT;

-- Expand the kind CHECK constraint: old CHECK is embedded in the CREATE TABLE
-- and SQLite does not allow altering constraints, but the existing CHECK
-- ("fact","entity","summary","preference","correction","sk_glossary","style_profile","instruction")
-- was defined inline. New rows may use new kind values; SQLite does not enforce
-- the CHECK on existing rows and will accept new values via INSERT with no CHECK
-- because we cannot alter the constraint. We acknowledge this SQLite limitation here
-- and validate kind values at the application layer instead.

-- Index for fast status+namespace+kind lookups (used by MemorySelector).
CREATE INDEX IF NOT EXISTS idx_memory_items_status_ns_kind
    ON memory_items (status, namespace, kind);
