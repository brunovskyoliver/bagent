-- Local cache of Apple Mail metadata for fast chat context injection.
-- Populated by POST /mail/sync; queried by fetch_tool_context without needing
-- repeated direct access to Apple Mail's Envelope Index.
CREATE TABLE IF NOT EXISTS mail_cache (
    rowid          INTEGER PRIMARY KEY,   -- mirrors Envelope Index ROWID
    subject        TEXT    NOT NULL DEFAULT '',
    sender         TEXT    NOT NULL DEFAULT '',
    sender_display TEXT,
    received_at    INTEGER NOT NULL DEFAULT 0,
    is_read        INTEGER NOT NULL DEFAULT 0,
    mailbox_url    TEXT    NOT NULL DEFAULT '',
    language       TEXT,
    synced_at      INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE INDEX IF NOT EXISTS mail_cache_received ON mail_cache(received_at DESC);

-- Seed rows so the sync handler can UPDATE without INSERT-or-ignore complications.
INSERT OR IGNORE INTO connectors (kind, config_json, enabled, last_sync_at)
VALUES
    ('apple_mail',  '{}', 1, 0),
    ('apple_notes', '{}', 1, 0);
