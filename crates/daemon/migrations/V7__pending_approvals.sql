-- Pending approvals: one row per in-flight tool approval request.
-- Decision is null until the user acts (or the 60s countdown expires).
CREATE TABLE IF NOT EXISTS pending_approvals (
    id              TEXT PRIMARY KEY,
    tool_name       TEXT NOT NULL,
    description     TEXT,
    dry_run_preview TEXT,
    rule_name       TEXT,
    decision        TEXT CHECK(decision IN ('allow', 'deny')),
    decided_at      TEXT,
    expires_at      TEXT NOT NULL,
    created_at      TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS pending_approvals_open
    ON pending_approvals(decision, expires_at);
