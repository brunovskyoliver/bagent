# Automations: Execute an automation safely with Run Now

GitHub issue: #6

## What to build

Execute a saved automation immediately through the shared agent loop and persist a concise, auditable result under the same safety rules as scheduled execution.

## Acceptance criteria

- [ ] Expose a typed run-now endpoint with not-found, disabled, validation, and active-run conflict handling.
- [ ] Carry trusted automation ID, name, run ID, scheduled/start timestamps, time zone, catch-up flag, and unattended status into execution.
- [ ] Claim overlap atomically and release database locks before model or connector work begins.
- [ ] Persist started, completed, partial, failed, denied, and approval-timeout outcomes with a redacted result of at most 2,000 characters.
- [ ] Attach automation/run provenance to each pending approval and preserve fresh single-action approval semantics.
- [ ] Prove read-only unattended execution works and gated writes cannot execute without approval.

## Blocked by

- #3
- #5
