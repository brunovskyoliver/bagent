# Automations: Lock scheduling semantics and build recurrence core

GitHub issue: #1

## What to build

Define typed automation schedules with deterministic validation and next-occurrence calculation, independent of persistence, HTTP, and UI.

## Acceptance criteria

- [ ] Support run once, every N hours, daily, weekdays, selected weekdays, and weekly schedules using IANA time zones.
- [ ] Reject empty names/prompts, intervals below one hour, invalid zones, malformed weekdays, impossible local times, and schedules with no next occurrence.
- [ ] Use the approved policies: 24-hour catch-up window, one catch-up maximum, earlier instant for ambiguous DST times, and next valid local time for nonexistent DST times.
- [ ] Define overlap, active-edit/disable/delete, failure, bounded-concurrency, result-size, and retention policies as typed behavior.
- [ ] Cover recurrence, DST transitions, validation, catch-up, stale occurrences, and injected-clock behavior with deterministic tests.

## Blocked by

- None — can start immediately.
