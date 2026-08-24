-- Stage 5 authoritative, privacy-allowlisted current activity.
CREATE TABLE IF NOT EXISTS work_activity_projection (
    work_identity TEXT PRIMARY KEY REFERENCES works(identity) ON DELETE CASCADE,
    category TEXT NOT NULL CHECK (category IN (
        'mail', 'web', 'filesystem', 'odoo', 'codex', 'chat', 'automation', 'generic_tool'
    ))
);
