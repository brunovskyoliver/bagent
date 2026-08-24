-- Stage 2 additive Work Coordinator foundations.
-- These tables are intentionally unused by production lifecycle paths until
-- the unified Work cutover. They must not be shadow-written from legacy code.

CREATE TABLE IF NOT EXISTS works (
    identity                  TEXT PRIMARY KEY,
    origin_kind               TEXT NOT NULL CHECK (origin_kind IN ('conversation', 'automation')),
    origin_primary_identity   TEXT NOT NULL,
    origin_secondary_identity TEXT NOT NULL,
    origin_historical_identity TEXT,
    origin_definition_revision INTEGER,
    state                     TEXT NOT NULL CHECK (state IN (
        'queued', 'waiting_for_model', 'running', 'waiting_for_approval',
        'cancelling', 'completed', 'partial', 'failed', 'cancelled', 'abandoned'
    )),
    revision                  INTEGER NOT NULL CHECK (revision >= 1),
    created_at                TEXT NOT NULL,
    updated_at                TEXT NOT NULL,
    UNIQUE (origin_kind, origin_primary_identity, origin_secondary_identity),
    CHECK (
        (origin_kind = 'conversation' AND origin_historical_identity IS NULL AND origin_definition_revision IS NULL)
        OR
        (origin_kind = 'automation' AND origin_historical_identity IS NOT NULL AND origin_definition_revision IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS work_command_results (
    command_identity TEXT PRIMARY KEY,
    command_hash     TEXT NOT NULL,
    acknowledgement TEXT NOT NULL,
    committed_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS work_event_outbox (
    cursor            INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version    INTEGER NOT NULL CHECK (schema_version = 1),
    daemon_generation TEXT NOT NULL,
    committed_at      TEXT NOT NULL,
    event_kind        TEXT NOT NULL,
    work_identity     TEXT REFERENCES works(identity) ON DELETE RESTRICT,
    work_revision     INTEGER,
    payload           TEXT NOT NULL,
    UNIQUE (work_identity, work_revision)
);

CREATE INDEX IF NOT EXISTS idx_work_event_outbox_work
    ON work_event_outbox (work_identity, work_revision);

CREATE TABLE IF NOT EXISTS work_coordinator_metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS work_current_chats (
    identity TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS work_conversation_turns (
    identity              TEXT PRIMARY KEY,
    current_chat_identity TEXT NOT NULL REFERENCES work_current_chats(identity) ON DELETE RESTRICT,
    work_identity         TEXT NOT NULL UNIQUE REFERENCES works(identity) ON DELETE RESTRICT
);

CREATE TABLE IF NOT EXISTS work_automation_runs (
    automation_run_identity       TEXT PRIMARY KEY,
    automation_session_identity   TEXT NOT NULL UNIQUE,
    historical_automation_identity TEXT NOT NULL,
    frozen_definition_revision    INTEGER NOT NULL CHECK (frozen_definition_revision >= 0),
    work_identity                 TEXT NOT NULL UNIQUE REFERENCES works(identity) ON DELETE RESTRICT,
    active                        INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_work_automation_one_active_definition
    ON work_automation_runs (historical_automation_identity) WHERE active = 1;

CREATE TABLE IF NOT EXISTS work_automation_sessions (
    automation_session_identity TEXT PRIMARY KEY,
    automation_run_identity     TEXT NOT NULL UNIQUE REFERENCES work_automation_runs(automation_run_identity) ON DELETE RESTRICT,
    attention_state             TEXT NOT NULL DEFAULT 'none' CHECK (attention_state IN ('none', 'unread', 'viewed')),
    frozen                      INTEGER NOT NULL DEFAULT 0 CHECK (frozen IN (0, 1))
);

CREATE TABLE IF NOT EXISTS work_approvals (
    identity       TEXT PRIMARY KEY,
    work_identity  TEXT NOT NULL REFERENCES works(identity) ON DELETE RESTRICT,
    category       TEXT NOT NULL,
    state          TEXT NOT NULL CHECK (state IN ('pending', 'allowed', 'denied', 'expired', 'withdrawn', 'abandoned')),
    created_at     TEXT NOT NULL,
    resolved_at    TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_work_approval_one_pending
    ON work_approvals (work_identity) WHERE state = 'pending';

CREATE TABLE IF NOT EXISTS work_projections (
    work_identity TEXT PRIMARY KEY REFERENCES works(identity) ON DELETE RESTRICT,
    revision      INTEGER NOT NULL CHECK (revision >= 1),
    available     INTEGER NOT NULL DEFAULT 0 CHECK (available IN (0, 1))
);

CREATE TABLE IF NOT EXISTS work_continuations (
    identity                    TEXT PRIMARY KEY,
    source_automation_session   TEXT NOT NULL REFERENCES work_automation_sessions(automation_session_identity) ON DELETE RESTRICT,
    target_current_chat_identity TEXT NOT NULL REFERENCES work_current_chats(identity) ON DELETE RESTRICT,
    created_at                  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS work_interruption_markers (
    conversation_turn_identity TEXT PRIMARY KEY REFERENCES work_conversation_turns(identity) ON DELETE RESTRICT,
    daemon_generation          TEXT NOT NULL,
    reason                     TEXT NOT NULL CHECK (reason = 'daemon_restart'),
    created_at                 TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS work_model_runtime_recovery (
    singleton                INTEGER PRIMARY KEY CHECK (singleton = 1),
    model_runtime_generation TEXT,
    trusted                  INTEGER NOT NULL CHECK (trusted IN (0, 1))
);

INSERT OR IGNORE INTO work_model_runtime_recovery
    (singleton, model_runtime_generation, trusted) VALUES (1, NULL, 0);
