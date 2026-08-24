-- Stage 8: prepare the canonical release schema.
--
-- The old lifecycle tables remain present until the daemon has established the
-- changed-PID boundary. `cutover::finalize_stage8_cleanup` then copies their
-- allowlisted records and drops them in one forward-only transaction.

CREATE TABLE IF NOT EXISTS automation_run_records (
    id             TEXT PRIMARY KEY,
    automation_id  TEXT NOT NULL,
    scheduled_for  TEXT NOT NULL,
    started_at     TEXT,
    finished_at    TEXT,
    status         TEXT NOT NULL,
    result_summary TEXT,
    is_catch_up    INTEGER NOT NULL DEFAULT 0 CHECK (is_catch_up IN (0, 1)),
    is_manual      INTEGER NOT NULL DEFAULT 0 CHECK (is_manual IN (0, 1)),
    created_at     TEXT NOT NULL,
    -- Carried from main's V15 so a blocked run keeps its typed reference
    -- outcome on the canonical run record.
    reference_outcome_code TEXT
        CHECK (
            reference_outcome_code IS NULL
            OR reference_outcome_code IN (
                'missing_referent',
                'ambiguous',
                'confirmation_required',
                'private_source_denied',
                'expired',
                'unsupported',
                'resolver_unavailable'
            )
        )
        CHECK ((status = 'blocked') = (reference_outcome_code IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS idx_automation_run_records_recent
    ON automation_run_records (automation_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_automation_run_records_active
    ON automation_run_records (automation_id, status);

CREATE TABLE IF NOT EXISTS work_approval_requests (
    identity       TEXT PRIMARY KEY,
    work_identity  TEXT,
    tool_name      TEXT NOT NULL,
    description    TEXT NOT NULL,
    expires_at     TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    origin_json    TEXT,
    decision       TEXT,
    decided_at     TEXT
);

CREATE INDEX IF NOT EXISTS idx_work_approval_requests_pending
    ON work_approval_requests (decision, expires_at, created_at);

CREATE TABLE IF NOT EXISTS stage8_cleanup_state (
    singleton       INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_generation INTEGER NOT NULL CHECK (schema_generation = 23),
    committed_at    TEXT
);

INSERT OR IGNORE INTO stage8_cleanup_state (singleton, schema_generation)
VALUES (1, 23);

-- This Stage 6 placeholder never held authoritative approval state. Remove it
-- in the forward migration so the bootstrap schema cannot resurrect it.
DROP TABLE IF EXISTS automation_session_pending_approvals;
