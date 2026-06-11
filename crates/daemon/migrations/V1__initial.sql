CREATE TABLE IF NOT EXISTS audit_entries (
    id         INTEGER PRIMARY KEY,
    actor      TEXT    NOT NULL DEFAULT 'user',
    action     TEXT    NOT NULL,
    payload    TEXT,
    model      TEXT,
    language   TEXT    DEFAULT 'und',
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS approvals (
    id         INTEGER PRIMARY KEY,
    request    TEXT    NOT NULL,
    decision   TEXT,
    decided_at INTEGER,
    created_at INTEGER NOT NULL DEFAULT (unixepoch())
);
