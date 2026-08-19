-- Stage 5 bounded authoritative Notch Projection reads.
CREATE INDEX IF NOT EXISTS idx_works_notch_active
    ON works (identity)
    WHERE state NOT IN ('completed', 'partial', 'failed', 'cancelled', 'abandoned');

CREATE INDEX IF NOT EXISTS idx_works_notch_terminal_conversation
    ON works (updated_at DESC, identity ASC)
    WHERE origin_kind = 'conversation'
      AND state IN ('completed', 'partial', 'failed');

CREATE INDEX IF NOT EXISTS idx_work_automation_sessions_unread
    ON work_automation_sessions (automation_session_identity)
    WHERE attention_state = 'unread';
