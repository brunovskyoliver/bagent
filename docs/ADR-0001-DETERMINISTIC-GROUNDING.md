# ADR 0001: Deterministic grounding is canonical

Date: 2026-07-29

Status: Accepted. Stage 9 makes this canonical path the production default for its
existing supported evidence intents.

## Context

Stage 8 measured strict free-form synthesis at 42 accepted answers out of 90 and
structured synthesis at 32 out of 90. Both campaigns produced 90 out of 90 safe
terminal answers only because invalid model output was withheld and deterministic
rendering completed the turn. Model acceptance therefore measured wording success,
not evidence correctness.

## Decision

Every validated Evidence Bundle produces a complete `CanonicalGroundedAnswer` before
model admission. It independently preserves user-visible text, coverage, citations,
conflicts, shortfalls, source identities, completeness, and outcome status.

Qwen3.6-35B-A3B is optional wording polish using the remediated 4,096-context, KV4,
256-token, batch-size-1 configuration and 25%/8 GiB memory gate. The retained
free-form contract may replace canonical wording only when validation against both
the bundle and canonical invariants succeeds. It may not alter facts, numbers, dates,
uncertainty, conflicts, shortfalls, coverage, or citation targets. Structured
synthesis remains disabled. The 4B model is not used as a grounding-quality fallback.

Only structural polish status is retained: skipped, accepted, rejected, timed out,
unavailable, or memory-ineligible. Evidence Outcome is derived exclusively from the
validated bundle and canonical answer.

## Consequences

Correctness and terminal availability no longer depend on model acceptance or model
residency. Invalid model output is never exposed. A poisoned BaseRT residency is
restarted for later clean-process polish, while the current turn returns its already
available canonical answer.

Stage 8 passed the reproducible release gate at
`96756b27c1d5798887e3baa379d5b0ab449bbc09`. Stage 9 is authorized to select this
accepted path by default without changing its grounding or polish contracts.
