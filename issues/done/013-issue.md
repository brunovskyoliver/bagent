# Automations: Finish run history, retention, and operational hardening

GitHub issue: #13

## What to build

Keep automation history useful, bounded, redacted, and deterministic across failures and concurrent lifecycle changes.

## Acceptance criteria

- [ ] Expose recent runs and latest concise result through API and notch detail, reusing the existing output presentation for explicitly requested full output.
- [ ] Retain the latest 50 runs per automation and audit each cleanup without deleting append-only audit history.
- [ ] Handle model/Ollama/connector/network failures, approval denial/timeout, repeated failures, and daemon shutdown during a run without infinite retries or silent disablement.
- [ ] Define and test queued/running behavior when an automation is edited, disabled, or deleted.
- [ ] Exercise two different automations due together and the same automation becoming due while active.
- [ ] Verify run/event/audit payloads do not expose stack traces, secrets, full connector payloads, unnecessary prompts, or model internals.

## Blocked by

- #8
- #12
