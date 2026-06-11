CREATE TABLE IF NOT EXISTS messages (
    id          INTEGER PRIMARY KEY,
    source      TEXT    NOT NULL,
    external_id TEXT,
    language    TEXT    DEFAULT 'und',
    subject     TEXT,
    body        TEXT,
    sender      TEXT,
    received_at INTEGER,
    indexed_at  INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS sessions (
    id         INTEGER PRIMARY KEY,
    started_at INTEGER NOT NULL DEFAULT (unixepoch()),
    ended_at   INTEGER,
    model      TEXT,
    turn_count INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS connectors (
    id           INTEGER PRIMARY KEY,
    kind         TEXT    NOT NULL UNIQUE,
    config_json  TEXT,
    enabled      INTEGER DEFAULT 1,
    last_sync_at INTEGER
);
