# Automations: Add typed slash commands and Swift test harness

GitHub issue: #4

## What to build

Replace ad-hoc slash-command handling with a typed registry and provide deterministic Swift tests for command suggestions and keyboard behavior.

## Acceptance criteria

- [ ] Add a Swift test target suitable for pure command and state tests.
- [ ] Register canonical /settings behavior through one typed command registry.
- [ ] Provide case-insensitive prefix filtering with at most three suggestions.
- [ ] Support arrow selection, Return and Tab acceptance, click acceptance, and Escape-first dismissal.
- [ ] Keep incomplete and unknown slash-prefixed text editable and preserve ordinary prompts, IME composition, Slovak diacritics, history, and Option-Space behavior.
- [ ] Do not register /automations until its surface exists.

## Blocked by

- None — can start immediately.
