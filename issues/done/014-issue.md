# Automations: Validate and document scheduled automations

GitHub issue: #14

## What to build

Verify the completed scheduling system end to end and update project documentation to describe actual behavior and limitations.

## Acceptance criteria

- [ ] Run formatting, linting, Rust build/tests, Swift tests, migration checks, app bundle build, and a practical launch smoke test; record actual outcomes.
- [ ] Manually validate sleep/wake, daemon restarts, clock movement, local time-zone changes, concurrent due work, active overlap, lifecycle changes, and approval preemption.
- [ ] Exercise unattended read-only work plus email, Odoo, shell, and file write approval paths.
- [ ] Document user flow, recurrence, DST, missed/catch-up, overlap, restart, approval, persistence, retention, API, SSE, audit, injection defenses, testing, and limitations.
- [ ] Reconcile historical documents with current architecture without claiming commands or scenarios that were not run.

## Blocked by

- #13
