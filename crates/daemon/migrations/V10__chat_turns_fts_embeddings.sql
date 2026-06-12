-- FTS index for cross-session conversation recall
CREATE VIRTUAL TABLE IF NOT EXISTS chat_turns_fts USING fts5(
    id UNINDEXED,
    content,
    content='chat_turns',
    content_rowid='rowid',
    tokenize='unicode61'
);

-- Backfill existing rows (safe to run multiple times via INSERT OR IGNORE on embeddings)
INSERT INTO chat_turns_fts(rowid, id, content)
SELECT rowid, id, content FROM chat_turns
WHERE id NOT IN (SELECT id FROM chat_turns_fts);

-- Keep FTS in sync with chat_turns
CREATE TRIGGER IF NOT EXISTS chat_turns_ai AFTER INSERT ON chat_turns BEGIN
    INSERT INTO chat_turns_fts(rowid, id, content) VALUES (new.rowid, new.id, new.content);
END;

CREATE TRIGGER IF NOT EXISTS chat_turns_ad AFTER DELETE ON chat_turns BEGIN
    INSERT INTO chat_turns_fts(chat_turns_fts, rowid, id, content)
    VALUES ('delete', old.rowid, old.id, old.content);
END;

CREATE TRIGGER IF NOT EXISTS chat_turns_au AFTER UPDATE ON chat_turns BEGIN
    INSERT INTO chat_turns_fts(chat_turns_fts, rowid, id, content)
    VALUES ('delete', old.rowid, old.id, old.content);
    INSERT INTO chat_turns_fts(rowid, id, content) VALUES (new.rowid, new.id, new.content);
END;

-- Add source discriminator to embeddings so we can distinguish memory items vs chat turns
ALTER TABLE embeddings ADD COLUMN source TEXT NOT NULL DEFAULT 'memory_item';
