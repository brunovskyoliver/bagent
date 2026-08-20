-- V15: typed terminal outcomes for unattended reference safety blocks.
-- Additive only. Existing rows keep their status and receive NULL reason
-- fields; there is deliberately no historical backfill.

ALTER TABLE automation_runs
    ADD COLUMN reference_outcome_code TEXT
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
    CHECK ((status = 'blocked') = (reference_outcome_code IS NOT NULL));

ALTER TABLE automations
    ADD COLUMN last_reference_outcome_code TEXT
    CHECK (
        last_reference_outcome_code IS NULL
        OR last_reference_outcome_code IN (
            'missing_referent',
            'ambiguous',
            'confirmation_required',
            'private_source_denied',
            'expired',
            'unsupported',
            'resolver_unavailable'
        )
    )
    CHECK ((last_run_status = 'blocked') = (last_reference_outcome_code IS NOT NULL));
