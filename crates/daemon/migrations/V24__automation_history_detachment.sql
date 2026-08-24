-- Stage 6: an Automation Definition owns future scheduling only. Historical
-- Automation Runs must survive definition deletion.

ALTER TABLE automations ADD COLUMN definition_revision INTEGER NOT NULL DEFAULT 1;

-- main's V17 continuation trigger binds a continuation to its parent tombstone
-- but compares `confirmation.expires_at_ms`, a column `reference_confirmation_tombstones`
-- does not have. The trigger therefore aborts every continuation insert, and the
-- table rebuild below forces a full schema re-parse that surfaces it. Recreate it
-- with the three parent columns that actually exist, preserving the binding intent.
DROP TRIGGER IF EXISTS reference_confirmation_continuations_validate_v17;

CREATE TRIGGER reference_confirmation_continuations_validate_v17
BEFORE INSERT ON reference_confirmation_continuations_v17
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM reference_confirmation_tombstones confirmation
        WHERE confirmation.confirmation_id = NEW.confirmation_id
          AND confirmation.session_id = NEW.session_id
          AND confirmation.initiating_turn_id = NEW.initiating_turn_id
    ) THEN RAISE(ABORT, 'continuation parent binding is invalid') END;
END;

CREATE TABLE automation_runs_stage6 (
    id             TEXT PRIMARY KEY,
    automation_id  TEXT NOT NULL,
    scheduled_for  TEXT NOT NULL,
    started_at     TEXT,
    finished_at    TEXT,
    status         TEXT NOT NULL,
    result_summary TEXT,
    is_catch_up    INTEGER NOT NULL DEFAULT 0,
    is_manual      INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL,
    -- Preserved from main's V15: a blocked run carries its typed reference
    -- outcome, and the two must stay in agreement.
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

INSERT INTO automation_runs_stage6
    (id, automation_id, scheduled_for, started_at, finished_at, status,
     result_summary, is_catch_up, is_manual, created_at, reference_outcome_code)
SELECT id, automation_id, scheduled_for, started_at, finished_at, status,
       result_summary, is_catch_up, is_manual, created_at, reference_outcome_code
FROM automation_runs;

DROP TABLE automation_runs;
ALTER TABLE automation_runs_stage6 RENAME TO automation_runs;

CREATE INDEX idx_automation_runs_recent
    ON automation_runs (automation_id, created_at DESC);
CREATE INDEX idx_automation_runs_active
    ON automation_runs (automation_id, status);
