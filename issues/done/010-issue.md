# Automations: Create and edit run-once automations inside notch

GitHub issue: #10

## What to build

Let users complete the full notch-native creation and editing flow for one-time natural-language automations.

## Acceptance criteria

- [ ] Use a typed editor state for task, schedule, review, saving, result, and delete confirmation rather than unrelated booleans.
- [ ] Capture name, natural-language task, later today/tomorrow/selected date, selected local time, IANA time zone, and enabled state.
- [ ] Keep controls inline inside the notch; do not use a window, sheet, popover, external date picker, or scrolling dashboard.
- [ ] Show a concise review summary and claim success only after daemon persistence succeeds.
- [ ] Handle invalid/stale schedules, daemon unavailability, save failure, concurrent SSE updates, and time-zone conversion errors.
- [ ] Preserve Slovak and English text, diacritics, keyboard operation, accessibility labels, reduced motion, and geometry ceilings.

## Blocked by

- #8
- #9
