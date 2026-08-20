# Automations: Persist and manage run-once automations through API

GitHub issue: #5

## What to build

Let authenticated local clients create, read, list, edit, enable, disable, and delete durable run-once automations with typed validation and bounded history storage.

## Acceptance criteria

- [ ] Add refinery-managed automation and automation-run storage using the existing SQLite approach and efficient due/history indexes.
- [ ] Provide typed create, read, list, patch, enable, disable, delete, and recent-run API contracts with existing authentication and error conventions.
- [ ] Use UTC instants where appropriate while retaining the selected IANA time zone and structured schedule fields.
- [ ] Keep database transactions short and return conflict when deletion is attempted during an active run.
- [ ] Write concise redacted lifecycle audit records without duplicating full prompts or connector payloads.
- [ ] Bound automation run history without pruning append-only audit entries.

## Blocked by

- #1
