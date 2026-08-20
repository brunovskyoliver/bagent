# Automations: Extract reusable agent execution with unattended safety

GitHub issue: #3

## What to build

Run foreground chat and future automations through one reusable agent loop, tool dispatcher, privacy path, rules engine, and approval boundary.

## Acceptance criteria

- [ ] Extract an internal execution service instead of invoking the chat HTTP route or duplicating the loop.
- [ ] Support a pluggable event sink and trusted execution-origin metadata while preserving existing round and tool-call budgets.
- [ ] Pass actual serialized tool arguments into policy checks.
- [ ] Fail closed for unattended unknown or unmapped operations and explicitly classify allowed local read-only tools.
- [ ] Require fresh approval or preserve existing forbidden behavior for writes, shell, Codex, and other side effects.
- [ ] Keep foreground chat behavior green with regression tests and add unattended admission tests.

## Blocked by

- None — can start immediately.
