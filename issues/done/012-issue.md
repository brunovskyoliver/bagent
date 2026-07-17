# Automations: Add weekday and weekly recurrence end to end

GitHub issue: #12

## What to build

Let users create, edit, execute, and inspect weekday, selected-weekday, and weekly automations across the complete stack.

## Acceptance criteria

- [ ] Support weekdays, selected weekdays, and weekly-on-one-weekday schedules without raw cron syntax.
- [ ] Reject malformed weekday values and empty selected-weekday sets.
- [ ] Provide compact inline weekday controls with explicit accessibility labels and full keyboard operation.
- [ ] Show recurrence consistently in review, list, detail, API responses, and events.
- [ ] Cover local-time advancement, DST boundaries, restart, catch-up, overlap, and next-occurrence calculation with deterministic tests.

## Blocked by

- #11
