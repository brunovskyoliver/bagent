-- Additive Stage 6 safe-record columns for databases that already applied V19.
CREATE TABLE IF NOT EXISTS automation_run_outcomes (
    automation_run_identity TEXT PRIMARY KEY,
    automation_session_identity TEXT NOT NULL UNIQUE,
    outcome TEXT NOT NULL,
    finished_at TEXT NOT NULL
);

ALTER TABLE automation_sessions
    ADD COLUMN validated_sources_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE automation_sessions
    ADD COLUMN connector_references_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE automation_sessions
    ADD COLUMN historical_approvals_json TEXT NOT NULL DEFAULT '[]';

CREATE TABLE IF NOT EXISTS automation_session_open_commands (
    command_identity TEXT PRIMARY KEY,
    automation_session_identity TEXT NOT NULL,
    expected_revision INTEGER NOT NULL,
    committed_at TEXT NOT NULL
);
