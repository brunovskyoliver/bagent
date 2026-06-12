-- Phase 5C: Mail message attachment metadata cache

CREATE TABLE IF NOT EXISTS mail_attachments (
    message_rowid  INTEGER NOT NULL,
    idx            INTEGER NOT NULL,   -- part_index within the MIME tree
    filename       TEXT NOT NULL,
    mime           TEXT NOT NULL,
    size           INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (message_rowid, idx)
);

CREATE INDEX IF NOT EXISTS idx_mail_att_rowid ON mail_attachments(message_rowid);
