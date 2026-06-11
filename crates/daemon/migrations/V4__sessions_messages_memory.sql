-- Session tracking
CREATE TABLE IF NOT EXISTS sessions (
    id            TEXT PRIMARY KEY,
    started_at    TEXT NOT NULL,
    ended_at      TEXT,
    language      TEXT,
    summary       TEXT,
    metadata_json TEXT
);

-- Server-persisted chat turns (distinct from connector messages table)
CREATE TABLE IF NOT EXISTS chat_turns (
    id             TEXT PRIMARY KEY,
    session_id     TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    role           TEXT NOT NULL CHECK(role IN ('user','assistant','system')),
    content        TEXT NOT NULL,
    language       TEXT NOT NULL DEFAULT 'und',
    model          TEXT,
    created_at     TEXT NOT NULL,
    parent_turn_id TEXT REFERENCES chat_turns(id)
);

CREATE INDEX IF NOT EXISTS chat_turns_session ON chat_turns(session_id, created_at);

-- Agent memory: facts, preferences, corrections, style profiles
CREATE TABLE IF NOT EXISTS memory_items (
    id            TEXT PRIMARY KEY,
    namespace     TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK(kind IN ('fact','entity','summary','preference','correction','sk_glossary','style_profile','instruction')),
    language      TEXT NOT NULL DEFAULT 'und',
    text          TEXT NOT NULL,
    source_ref    TEXT,
    metadata_json TEXT,
    last_used_at  TEXT,
    use_count     INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    expires_at    TEXT
);

CREATE INDEX IF NOT EXISTS memory_items_namespace ON memory_items(namespace, kind);
CREATE INDEX IF NOT EXISTS memory_items_language  ON memory_items(language);
CREATE INDEX IF NOT EXISTS memory_items_last_used ON memory_items(last_used_at);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    id UNINDEXED,
    text,
    content='memory_items',
    content_rowid='rowid',
    tokenize='unicode61'
);

-- Triggers to keep memory_fts in sync
CREATE TRIGGER IF NOT EXISTS memory_items_ai AFTER INSERT ON memory_items BEGIN
    INSERT INTO memory_fts(rowid, id, text) VALUES (new.rowid, new.id, new.text);
END;

CREATE TRIGGER IF NOT EXISTS memory_items_ad AFTER DELETE ON memory_items BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, id, text) VALUES ('delete', old.rowid, old.id, old.text);
END;

CREATE TRIGGER IF NOT EXISTS memory_items_au AFTER UPDATE ON memory_items BEGIN
    INSERT INTO memory_fts(memory_fts, rowid, id, text) VALUES ('delete', old.rowid, old.id, old.text);
    INSERT INTO memory_fts(rowid, id, text) VALUES (new.rowid, new.id, new.text);
END;
