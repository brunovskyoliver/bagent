# Automations: Add hourly and daily recurrence end to end

GitHub issue: #11

## What to build

Let users create, edit, execute, and inspect every-N-hours and daily-at-local-time automations across persistence, API, scheduler, and notch UI.

## Acceptance criteria

- [ ] Encode structured hourly and daily recurrence without exposing raw cron syntax.
- [ ] Validate the one-hour minimum and calculate daily recurrence in the selected local time zone rather than adding 24 UTC hours.
- [ ] Advance next-run state atomically after claim/completion and avoid catch-up loops.
- [ ] Expose recurrence through typed API and Swift models, editor controls, review summary, list/detail, and live events.
- [ ] Cover DST transitions, restart, sleep/wake, catch-up, overlap, disable/edit behavior, and failure advancement with deterministic tests.

## Blocked by

- #10
