# Automations: Keep daemon alive independently of notch app

GitHub issue: #2

## What to build

Make the local daemon a managed per-user process so scheduled work continues when the Swift UI process exits, while preserving existing authenticated discovery.

## Acceptance criteria

- [ ] App launch ensures a compatible daemon is reachable without making the app its sole process owner.
- [ ] App termination does not stop scheduled work or leave an unmanaged orphan process.
- [ ] Preserve the existing local port and bearer-token discovery contract.
- [ ] Handle daemon restart, app relaunch, packaging, upgrade, and explicit shutdown deterministically.
- [ ] Document and test the supported daemon residency lifecycle.

## Blocked by

- None — can start immediately.
