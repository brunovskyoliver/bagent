-- Stage 4: atomically establish unified Work authority and bounded legacy history.

CREATE TABLE IF NOT EXISTS work_cutover (
    singleton                    INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_generation            INTEGER NOT NULL CHECK (schema_generation = 16),
    cutover_committed_at         TEXT NOT NULL,
    first_post_cutover_work_at   TEXT,
    pre_cutover_backup_sha256    TEXT
);

CREATE TABLE IF NOT EXISTS legacy_run_records (
    legacy_run_identity          TEXT PRIMARY KEY,
    historical_automation_identity TEXT NOT NULL,
    outcome                      TEXT NOT NULL CHECK (outcome IN (
        'completed', 'partial', 'failed', 'cancelled', 'abandoned'
    )),
    summary                      TEXT NOT NULL,
    summary_available            INTEGER NOT NULL CHECK (summary_available IN (0, 1)),
    viewed                       INTEGER NOT NULL DEFAULT 1 CHECK (viewed = 1),
    completion_attention         INTEGER NOT NULL DEFAULT 0 CHECK (completion_attention = 0),
    continuation_available       INTEGER NOT NULL DEFAULT 0 CHECK (continuation_available = 0),
    created_at                   TEXT NOT NULL,
    finished_at                  TEXT
);

CREATE INDEX IF NOT EXISTS idx_legacy_run_records_automation
    ON legacy_run_records (historical_automation_identity, created_at DESC);

ALTER TABLE work_approvals ADD COLUMN decision_revision INTEGER NOT NULL DEFAULT 0;

INSERT OR IGNORE INTO work_cutover
    (singleton, schema_generation, cutover_committed_at)
VALUES (1, 16, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
