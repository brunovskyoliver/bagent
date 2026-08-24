# Post-Stage 9 private product clarification

Date: 2026-07-30

## Decision

A later specification-research follow-up can refer generically to an SD card
whose only usable identity appeared in assistant-visible Mail-derived content.
That content is untrusted private evidence and must not become a web-provider
query.

With ordinary evidence routing enabled, a narrowly recognized unresolved
SD-card or memory-card specification request now returns this deterministic
clarification:

> Please provide a public make/model or URL for the item you want me to research.

The route exposes no prior Mail text, executes no Mail or web tool, and returns
before model inference. A bare product mention does not match. A same-message
make/model suffix does not match the unresolved form. Explicit public factual
queries continue through the existing typed web contract. The explicit `0`
rollback retains the legacy path.

## Scope

This amendment does not resolve entities from conversation history. In
particular, it does not trust identities introduced only by an assistant or
Mail result. Supporting references to earlier user-authored public identities
requires a separate contract and acceptance boundary.

The production classifier represents the outcome as a distinct typed
classification so routing does not infer it from presentation text. Chat and
automation consume the same routing decision.
