-- V13: Scheduled automations + bounded run history.
-- Schedules are typed JSON (bagent-automations AutomationSchedule); instants
-- are UTC RFC3339 strings; the user-selected IANA zone is kept alongside so
-- recurrence is always calculated in local time.
-- audit_entries stays the append-only record — automation_runs is bounded
-- (pruned to a per-automation cap) and prunable without touching audits.

CREATE TABLE automations (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    prompt              TEXT NOT NULL,
    enabled             INTEGER NOT NULL DEFAULT 1,
    timezone            TEXT NOT NULL,
    schedule_json       TEXT NOT NULL,
    next_run_at         TEXT,               -- NULL when exhausted (run-once done)
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    last_run_at         TEXT,
    last_run_status     TEXT,
    last_result_summary TEXT
);

-- Scheduler wake-up: enabled automations ordered by due time.
CREATE INDEX idx_automations_due ON automations (enabled, next_run_at);

CREATE TABLE automation_runs (
    id             TEXT PRIMARY KEY,
    automation_id  TEXT NOT NULL REFERENCES automations(id) ON DELETE CASCADE,
    scheduled_for  TEXT NOT NULL,
    started_at     TEXT,
    finished_at    TEXT,
    status         TEXT NOT NULL,           -- running/completed/partial/failed/skipped_overlap/skipped_stale/abandoned
    result_summary TEXT,
    is_catch_up    INTEGER NOT NULL DEFAULT 0,
    is_manual      INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL
);

-- Recent-history reads and the active-run (overlap/conflict) check.
CREATE INDEX idx_automation_runs_recent ON automation_runs (automation_id, created_at DESC);
CREATE INDEX idx_automation_runs_active ON automation_runs (automation_id, status);
