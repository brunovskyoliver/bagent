# Post-Stage 9 web-discovery stabilization

Date: 2026-07-30

## Scope

This correction is limited to Tavily credential handoff, authenticated configuration status,
pending-provider behavior, and logical web-fetch presentation. It does not change evidence
relevance, grounding, citations, independence, SSRF/redirect/final-URL validation, Mail behavior,
model lifecycle, canonical rendering, rollback, routing intents, or ordinary fixture exposure.

## Proven cause boundary

App startup launched daemon replacement and `ChatViewModel.refreshHealth()` concurrently. The
only Tavily POST ran once after a health loop, discarded every error with `try?`, and was never
repeated by health polling, SSE reconnection, app relaunch, or daemon PID replacement. The trace
proves that the serving daemon remained unconfigured; the code proves there was no recovery if
that one POST failed or targeted the daemon being replaced. Retained artifacts contain neither a
configuration status nor the swallowed error, so they cannot distinguish those two historical
subcases. The exact recoverable root cause is the lossy one-shot handoff to the current daemon,
not Keychain identity, provider selection, or evidence validation.

The failing diagnostic independently exposed a presentation defect. Post-fetch duplicate
detection compared content fingerprints even after passage validation made distinct pages
irrelevant; equivalent empty/generic fingerprints overwrote `irrelevant` or failed contributions
with `duplicate`.

## Corrected contract

- The app owns a bounded, PID-aware configuration synchronizer independent of BaseRT readiness.
- One failed configuration may be retried once for the same daemon. A changed PID resets the
  bounded attempt state. App launch performs one bounded resend even when an existing daemon
  reports configured, so an updated Keychain value replaces stale in-memory state.
- Keychain lookup failure is distinct from credential absence: it records normalized
  `configuration_failed`, never sends `null`, and never exposes or persists the credential.
- The credential is read only from Keychain, carried only in an authenticated loopback request,
  and retained only in daemon memory.
- Authenticated health/status returns normalized state only. The initial state is `pending`;
  explicit `null` becomes `absent`; a valid in-memory value becomes `configured`; rejected
  configuration becomes `configuration_failed`.
- Pending or failed handoff selects Tavily first and returns normalized connector unavailability,
  followed by at most one DuckDuckGo fallback. Explicit absence retains the existing keyless
  Wikipedia/DuckDuckGo policy.
- A fetch failure remains failed, distinct irrelevant results remain irrelevant, and only a
  canonical operation/result duplicate is grouped as suppressed duplicate work.

## Signed acceptance

`scripts/post-stage9-web-stabilization-acceptance.py` verifies a changed daemon PID, configured
authenticated status, `401` at the unauthenticated status boundary, Tavily as the first live
discovery provider, structural credential-pattern absence in bundle/app-data/log/plist/process
state, and the signed deterministic all-fetch-failure shortfall from the Stage 8 acceptance report.
That report is bound to the expected source commit and signed app binary SHA-256. The script never
reads or emits the Keychain value and writes only structural results.
