-- Stage 7A: daemon-authoritative Current Chat and atomic replacement.

-- V19 briefly exposed a second writable chat-identity table to the Automation
-- Session seam. Stage 7A replaces it with the singleton daemon authority.
-- V19 had no singleton pointer, and its Swift-local clear path did not remove
-- these rows. Recency therefore cannot prove which legacy identity remained
-- current. Remove every unowned target and let the daemon issue a fresh
-- authoritative identity rather than resurrecting cleared content or
-- promoting a Swift-created identity.
CREATE TABLE IF NOT EXISTS automation_current_chats (
    current_chat_identity TEXT PRIMARY KEY,
    content_empty INTEGER NOT NULL CHECK (content_empty IN (0, 1))
);
CREATE TABLE IF NOT EXISTS automation_continuation_provenance (
    identity TEXT PRIMARY KEY,
    source_automation_session_identity TEXT NOT NULL,
    target_current_chat_identity TEXT NOT NULL UNIQUE,
    command_identity TEXT NOT NULL UNIQUE,
    seed TEXT NOT NULL,
    seed_bytes INTEGER NOT NULL CHECK (seed_bytes <= 16384),
    source_deleted INTEGER NOT NULL DEFAULT 0 CHECK (source_deleted IN (0, 1)),
    created_at TEXT NOT NULL
);
DELETE FROM automation_continuation_provenance
WHERE target_current_chat_identity IN (
    SELECT current_chat_identity FROM automation_current_chats
);
DROP TABLE IF EXISTS automation_current_chats;

CREATE TABLE IF NOT EXISTS current_chats (
    identity        TEXT PRIMARY KEY,
    revision        INTEGER NOT NULL CHECK (revision >= 1),
    turn_count      INTEGER NOT NULL DEFAULT 0 CHECK (turn_count BETWEEN 0 AND 500),
    content_bytes   INTEGER NOT NULL DEFAULT 0 CHECK (content_bytes BETWEEN 0 AND 16777216),
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS current_chat_authority (
    singleton             INTEGER PRIMARY KEY CHECK (singleton = 1),
    current_chat_identity TEXT NOT NULL UNIQUE
        REFERENCES current_chats(identity) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS current_chat_turns (
    identity              TEXT PRIMARY KEY,
    current_chat_identity TEXT NOT NULL
        REFERENCES current_chats(identity) ON DELETE CASCADE,
    ordinal               INTEGER NOT NULL,
    user_message          TEXT NOT NULL,
    assistant_output      TEXT,
    state                 TEXT NOT NULL CHECK (state IN ('active', 'completed', 'interrupted')),
    interruption_reason   TEXT CHECK (
        interruption_reason IS NULL OR interruption_reason IN (
            'daemon_restart', 'content_bound', 'execution_failed'
        )
    ),
    submitted_at          TEXT NOT NULL,
    completed_at          TEXT,
    encoded_bytes         INTEGER NOT NULL CHECK (encoded_bytes >= 0),
    UNIQUE (current_chat_identity, ordinal)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_current_chat_one_active_turn
    ON current_chat_turns(current_chat_identity) WHERE state = 'active';

CREATE TABLE IF NOT EXISTS current_chat_drafts (
    current_chat_identity          TEXT PRIMARY KEY
        REFERENCES current_chats(identity) ON DELETE CASCADE,
    text                           TEXT NOT NULL,
    edited_at                      TEXT NOT NULL,
    pending_attachment_references TEXT NOT NULL DEFAULT '[]',
    CHECK (length(CAST(text AS BLOB)) <= 16384)
);

CREATE TABLE IF NOT EXISTS current_chat_submitted_attachments (
    current_chat_identity TEXT NOT NULL
        REFERENCES current_chats(identity) ON DELETE CASCADE,
    conversation_turn_identity TEXT NOT NULL
        REFERENCES current_chat_turns(identity) ON DELETE CASCADE,
    attachment_identity TEXT NOT NULL,
    filename TEXT NOT NULL,
    mime TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    available INTEGER NOT NULL DEFAULT 1 CHECK (available IN (0, 1)),
    PRIMARY KEY (conversation_turn_identity, attachment_identity)
);

CREATE TABLE IF NOT EXISTS current_chat_validated_sources (
    current_chat_identity TEXT NOT NULL
        REFERENCES current_chats(identity) ON DELETE CASCADE,
    source_identity TEXT NOT NULL,
    title TEXT NOT NULL,
    domain TEXT NOT NULL,
    available INTEGER NOT NULL DEFAULT 1 CHECK (available IN (0, 1)),
    PRIMARY KEY (current_chat_identity, source_identity)
);

CREATE TABLE IF NOT EXISTS current_chat_connector_references (
    current_chat_identity TEXT NOT NULL
        REFERENCES current_chats(identity) ON DELETE CASCADE,
    reference_identity TEXT NOT NULL,
    connector_kind TEXT NOT NULL,
    availability TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY (current_chat_identity, reference_identity)
);

CREATE TABLE IF NOT EXISTS current_chat_approval_presentations (
    current_chat_identity TEXT NOT NULL
        REFERENCES current_chats(identity) ON DELETE CASCADE,
    approval_identity TEXT NOT NULL,
    category TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('allowed', 'denied', 'expired', 'withdrawn', 'abandoned')),
    PRIMARY KEY (current_chat_identity, approval_identity)
);

CREATE TABLE IF NOT EXISTS current_chat_clear_commands (
    command_identity          TEXT PRIMARY KEY,
    old_current_chat_identity TEXT NOT NULL,
    old_revision              INTEGER NOT NULL,
    confirmed_non_empty       INTEGER NOT NULL CHECK (confirmed_non_empty IN (0, 1)),
    new_current_chat_identity TEXT NOT NULL UNIQUE,
    committed_at              TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS current_chat_lifecycle_audit (
    command_identity          TEXT PRIMARY KEY,
    canonical_action          TEXT NOT NULL CHECK (canonical_action = 'clear_current_chat'),
    old_current_chat_identity TEXT NOT NULL,
    new_current_chat_identity TEXT NOT NULL,
    normalized_outcome        TEXT NOT NULL CHECK (normalized_outcome = 'committed'),
    committed_at              TEXT NOT NULL
);
