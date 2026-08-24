# ADR 0002: Reproducible Stage 8 release gate

Date: 2026-07-30

Status: Accepted. The definitive campaign at
`96756b27c1d5798887e3baa379d5b0ab449bbc09` passed every authorization condition;
Stage 9 default routing is authorized under this decision.

Current campaign boundary: Stage 7C and Stage 8 signed/live qualification is
limited to macOS 26. macOS 14 and 15 remain compile targets when existing
configuration permits them, without runtime, System Settings, TCC, visual, or
accessibility qualification claims. Live TCC grant, denial, revocation, and
drag-to-System-Settings mutation are outside this campaign; omitted checks are
never PASS. Deterministic adapters, signed-bundle and drag-payload validation,
privacy tests, and daemon-preserving relaunch remain required.

## Context

Revision `8536f0115a50e2c011b39ce31ac79cc16b47d8fc` passes deterministic
grounding, workspace, signed failure-path, privacy, rollback, signing, and
lifecycle checks. Repeated signed live campaigns nevertheless cannot produce a
stable release verdict: Mail.app may expose headers while local bodies remain
unavailable, and changing public discovery results may not assemble a desired
authoritative, corroborated, or conflicting bundle.

Treating external availability as the positive release oracle conflates system
correctness with environmental luck. It also encourages repeated public
campaigns even after the product has already failed closed with the correct
verification shortfall.

## Decision

Stage 8 acceptance has two mandatory layers.

### Mandatory deterministic signed E2E

The reproducible release gate runs through the real signed app and its
authenticated daemon API. It exercises the production classifier, planner,
immediate operation policy gate, evidence orchestrator, validator, canonical
renderer, SSE event path, and Swift decoder. Deterministic responses are
injected only through the same typed Mail, web, and synthesis-provider
interfaces used at external-system boundaries.

The signed suite must prove the specified positive, partial, unavailable,
denied, empty, conflict, irrelevant, redirect, provider-failure, bounded
fallback, all-fetch-failure, polish, canonical-byte, and terminal-event cases
twice with identical structural results.

The fixture boundary requires both the `stage8-acceptance` Cargo feature and
the exact `BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1` runtime flag. Its authenticated
control route is absent otherwise. A prompt cannot select a fixture. Fixture
selection is process-local and is neither placed in prompts nor written to
diagnostics or persisted prompt traces. Every simulated operation remains
policy-gated immediately before adapter execution. Discovery provenance,
opaque Mail identifiers, validation, rendering, and UI events retain their
production contracts.

### Observational live smoke

A signed live smoke against real Mail.app and Tavily/public pages is mandatory
to run for each definitive campaign. It records external availability and
checks that the product remains grounded and safe.

An availability failure, partial result, or verification shortfall does not by
itself fail release acceptance when the deterministic positive signed E2E gate
passes. Any fabricated claim, unsupported claim, incorrect citation,
unvalidated source, disclosure of internal Mail identifiers, or unsafe answer
is a hard acceptance failure.

## Stage 9 authorization rule

Stage 9 was authorized when all of the following passed on the same exact clean
commit:

1. Every mandatory deterministic signed fixture passes twice with identical
   structural results.
2. The live observational smoke produces no unsafe, fabricated, unsupported,
   or incorrectly cited answer.
3. The candidate's routing default and explicit rollback behavior pass.
4. Privacy scans, strict signing, release validation, and runtime cleanup pass.

Live safe unavailable, partial, or verification-shortfall outcomes are
acceptable under this rule. A live positive answer is still required to meet
the same grounding, authority, independence, relevance, citation, and safety
standards as a deterministic positive answer.

## Consequences

Release authorization becomes reproducible without weakening product rules.
This decision changes test reliability only. It does not change what counts as
evidence; it does not lower grounding, first-party authority, source
independence, entity relevance, redirect/final-URL validation, citation, or
safety requirements. Stage 9 may enable the accepted evidence route by default
without changing this release gate.

Ordinary builds do not compile the fixture module. Acceptance-compiled builds
still return 404 unless explicitly activated at daemon startup, and return 401
to unauthenticated callers when activated. Production adapters and routing are
unchanged whenever the acceptance boundary is absent.
