-- Phase 5B: Chat attachments

CREATE TABLE IF NOT EXISTS attachments (
    id             TEXT PRIMARY KEY,
    sha256         TEXT NOT NULL,
    filename       TEXT NOT NULL,
    mime           TEXT NOT NULL,
    kind           TEXT NOT NULL,      -- 'image' | 'pdf' | 'text' | 'other'
    bytes_path     TEXT NOT NULL,      -- absolute path under Application Support/bagent/attachments/
    extracted_text TEXT,               -- plain-text preview (max ~8000 chars)
    size_bytes     INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_attachments_sha256 ON attachments(sha256);

-- Links a chat turn to the attachments it included.
CREATE TABLE IF NOT EXISTS chat_turn_attachments (
    chat_turn_id  TEXT NOT NULL,
    attachment_id TEXT NOT NULL,
    PRIMARY KEY (chat_turn_id, attachment_id)
);

CREATE INDEX IF NOT EXISTS idx_cta_turn ON chat_turn_attachments(chat_turn_id);
