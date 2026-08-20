# Automations: Open live automation list and detail inside notch

GitHub issue: #9

## What to build

Make /automations open a compact live management surface inside the existing notch, with no additional window or panel.

## Acceptance criteria

- [ ] Register canonical /automations only after the surface is available.
- [ ] Show approximately three upcoming automations with short name, next-run time, and compact status.
- [ ] Provide detail with schedule, latest status, latest concise result, enable/disable, run-now, edit entry point, and inline delete confirmation.
- [ ] Update an open list/detail from daemon events and handle records changed or deleted elsewhere.
- [ ] Use NotchInteractionMode as the single source of truth and preserve approval and WhatsApp QR preemption order.
- [ ] Keep all geometry within existing ceilings and route geometry changes through the established refresh path.
- [ ] Support keyboard-only operation, reduced motion, and explicit accessibility labels.

## Blocked by

- #4
- #5
- #7
