-- V12: WhatsApp connector tables.
-- Messages are stored in the existing `messages` table (source='whatsapp').
-- Adds metadata_json to messages for per-row chat/from_me/has_media data.
-- New tables: whatsapp_contacts, whatsapp_chats, whatsapp_sync_state.
-- Embeddings for WhatsApp messages reuse the existing `embeddings` table
--   (source='whatsapp', namespace='whatsapp_messages').

-- Extend the messages table with optional metadata JSON
-- (used for WhatsApp chat_id, from_me, has_media, to fields).
-- SQLite requires one ALTER TABLE per column.
ALTER TABLE messages ADD COLUMN metadata_json TEXT;

-- WhatsApp contact cache.
CREATE TABLE IF NOT EXISTS whatsapp_contacts (
    id           TEXT    PRIMARY KEY,
    name         TEXT,
    push_name    TEXT,
    phone        TEXT,
    is_business  INTEGER NOT NULL DEFAULT 0,
    updated_at   TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- WhatsApp chat cache (recent chats list).
CREATE TABLE IF NOT EXISTS whatsapp_chats (
    id              TEXT    PRIMARY KEY,
    name            TEXT,
    is_group        INTEGER NOT NULL DEFAULT 0,
    last_message_at TEXT,
    unread_count    INTEGER NOT NULL DEFAULT 0,
    updated_at      TEXT    NOT NULL DEFAULT (datetime('now'))
);

-- WhatsApp sync state (singleton row, id CHECK enforces at most one row).
CREATE TABLE IF NOT EXISTS whatsapp_sync_state (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    last_sync_at TEXT,
    status       TEXT    NOT NULL DEFAULT 'stopped',
    last_error   TEXT
);

-- Seed the connectors row so last_sync_at can be tracked.
INSERT OR IGNORE INTO connectors (kind, enabled) VALUES ('whatsapp', 1);

-- Index for fast WhatsApp message lookups (source + received_at).
CREATE INDEX IF NOT EXISTS idx_messages_whatsapp
    ON messages (source, received_at)
    WHERE source = 'whatsapp';
