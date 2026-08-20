# Automations: Schedule and recover one-time automations

GitHub issue: #8

## What to build

Execute enabled one-time automations at their persisted due instant, including daemon restart and Mac sleep/wake recovery without replay storms.

## Acceptance criteria

- [ ] Run a daemon-owned scheduler with persisted next-run timestamps, efficient interruptible sleeping, and immediate wake after schedule changes.
- [ ] Atomically claim due work and prevent overlapping runs of the same automation, including run-now.
- [ ] Allow at most two different automations to execute concurrently without holding SQLite transactions during execution.
- [ ] Perform at most one catch-up within 24 hours and record older one-time occurrences as intentionally skipped.
- [ ] Recover abandoned runs as failed, release stale claims, and never revive prior approvals.
- [ ] Recalculate after sleep/wake and clock changes, cancel cleanly on daemon shutdown, and avoid short-interval polling.
- [ ] Do not immediately retry failed executions; persist the safe redacted outcome and audit trail.

## Blocked by

- #2
- #6
- #7
