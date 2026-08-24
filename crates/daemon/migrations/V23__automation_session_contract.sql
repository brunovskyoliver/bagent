-- Stage 6: immutable Automation Session product data and independently
-- mutable Completion Attention. WorkCoordinator may apply this migration to
-- databases that do not yet contain the legacy automation tables.

CREATE TABLE IF NOT EXISTS automation_work_states (
    work_identity TEXT PRIMARY KEY,
    automation_run_identity TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS automation_task_snapshots (
    automation_session_identity TEXT PRIMARY KEY,
    automation_run_identity TEXT NOT NULL UNIQUE,
    automation_identity TEXT NOT NULL,
    display_name TEXT NOT NULL,
    task_text TEXT NOT NULL CHECK (length(task_text) <= 4000),
    schedule_json TEXT NOT NULL,
    timezone TEXT NOT NULL,
    definition_revision INTEGER NOT NULL CHECK (definition_revision >= 0)
);

CREATE TABLE IF NOT EXISTS automation_run_outcomes (
    automation_run_identity TEXT PRIMARY KEY,
    automation_session_identity TEXT NOT NULL UNIQUE,
    outcome TEXT NOT NULL,
    finished_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS automation_sessions (
    automation_session_identity TEXT PRIMARY KEY,
    automation_run_identity TEXT NOT NULL UNIQUE,
    outcome TEXT NOT NULL CHECK (outcome IN (
        'completed', 'partial', 'failed', 'skipped', 'cancelled', 'abandoned'
    )),
    finished_at TEXT NOT NULL,
    result_summary TEXT,
    final_output TEXT,
    final_output_available INTEGER NOT NULL CHECK (final_output_available IN (0, 1)),
    activity_timeline_json TEXT NOT NULL,
    truncation_disclosures_json TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS automation_session_attention (
    automation_session_identity TEXT PRIMARY KEY
        REFERENCES automation_sessions(automation_session_identity) ON DELETE CASCADE,
    attention_state TEXT NOT NULL CHECK (attention_state IN ('none', 'unread', 'viewed'))
);

CREATE TABLE IF NOT EXISTS automation_session_open_commands (
    command_identity TEXT PRIMARY KEY,
    automation_session_identity TEXT NOT NULL,
    expected_revision INTEGER NOT NULL,
    committed_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS automation_terminal_outbox (
    automation_session_identity TEXT PRIMARY KEY
        REFERENCES automation_sessions(automation_session_identity) ON DELETE CASCADE,
    automation_run_identity TEXT NOT NULL UNIQUE,
    outcome TEXT NOT NULL,
    emitted_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS automation_session_tombstones (
    automation_session_identity TEXT PRIMARY KEY,
    deleted_at TEXT NOT NULL,
    former_outcome TEXT NOT NULL
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

CREATE TABLE IF NOT EXISTS automation_definitions (
    automation_identity TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS automation_current_chats (
    current_chat_identity TEXT PRIMARY KEY,
    content_empty INTEGER NOT NULL CHECK (content_empty IN (0, 1))
);

CREATE TABLE IF NOT EXISTS automation_session_pending_approvals (
    automation_session_identity TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS automation_retention_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deleted_count INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_automation_sessions_finished
    ON automation_sessions (finished_at DESC, automation_run_identity ASC);
CREATE INDEX IF NOT EXISTS idx_automation_session_attention_unread
    ON automation_session_attention (attention_state, automation_session_identity);
