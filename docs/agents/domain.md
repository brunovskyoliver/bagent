# Domain docs

How the engineering skills should consume this repository's domain documentation
when exploring the codebase.

## Layout

This is a single-context repository. There is no `CONTEXT-MAP.md` and no
per-module context split.

```text
/
├── CONTEXT.md
├── docs/
│   ├── ADR-0001-DETERMINISTIC-GROUNDING.md
│   ├── ADR-0002-REPRODUCIBLE-STAGE8-RELEASE-GATE.md
│   └── adr/
│       └── 0001-own-webkit-browser-profile.md …
└── crates/
```

Two ADR locations coexist and both are authoritative:

- `docs/ADR-*.md` — system-wide decisions (grounding, release gating).
- `docs/adr/NNNN-*.md` — subsystem decisions, currently the browser ADRs.

## Before exploring

- Read the root `CONTEXT.md` for canonical evidence and grounding language.
- Read the ADRs from either location that touch the area being explored.
- If a referenced domain document does not exist, proceed silently. The
  domain-modeling skill creates domain documentation lazily, only when a session
  actually resolves a new term or a qualifying architectural decision.

## Use the glossary's vocabulary

When output names a domain concept in an issue title, question, decision,
proposal, hypothesis, test name, or implementation plan, use the term defined in
`CONTEXT.md`. Do not drift to a synonym the glossary explicitly rejects.

If a needed concept is missing, reconsider whether it belongs to the existing
language or record the gap for a domain-modeling session.

## Flag ADR conflicts

If proposed work contradicts an existing ADR, surface the conflict explicitly
instead of silently overriding it.
