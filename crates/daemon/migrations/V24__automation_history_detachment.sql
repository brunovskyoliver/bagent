-- Stage 6: an Automation Definition owns future scheduling only. Historical
-- Automation Runs must survive definition deletion.

ALTER TABLE automations ADD COLUMN definition_revision INTEGER NOT NULL DEFAULT 1;

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
    created_at     TEXT NOT NULL
);

INSERT INTO automation_runs_stage6
    (id, automation_id, scheduled_for, started_at, finished_at, status,
     result_summary, is_catch_up, is_manual, created_at)
SELECT id, automation_id, scheduled_for, started_at, finished_at, status,
       result_summary, is_catch_up, is_manual, created_at
FROM automation_runs;

DROP TABLE automation_runs;
ALTER TABLE automation_runs_stage6 RENAME TO automation_runs;

CREATE INDEX idx_automation_runs_recent
    ON automation_runs (automation_id, created_at DESC);
CREATE INDEX idx_automation_runs_active
    ON automation_runs (automation_id, status);
