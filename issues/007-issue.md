# Automations: Broadcast daemon events and surface background approvals

GitHub issue: #7

## What to build

Deliver automation lifecycle and approval changes to the macOS app outside foreground chat streams, including reconnect-safe approval preemption.

## Acceptance criteria

- [ ] Add one authenticated daemon-wide SSE endpoint with concise redacted typed event envelopes.
- [ ] Publish automation changes, run lifecycle, next-run changes, overlap/missed outcomes, and approval creation.
- [ ] Add Swift subscription and reconnect handling using the existing networking stack.
- [ ] Fetch pending approvals at app startup and after reconnect so durable approvals are not missed.
- [ ] Open the notch and preempt every ordinary surface when an automation approval arrives.
- [ ] Show the automation identity and concise requested action while preserving the existing 60-second timeout and controls.
- [ ] Refetch authoritative records after events instead of treating event payloads as a second source of truth.

## Blocked by

- #6
