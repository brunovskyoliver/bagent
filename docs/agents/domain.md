# Domain docs

This is a single-context repository.

## Before exploring

Read:

- root `CONTEXT.md` for canonical evidence and grounding language; and
- relevant system ADRs matching `docs/ADR-*.md`.

Proceed silently when a referenced domain document does not exist. Create
domain documentation lazily only when a domain-modeling session resolves a new
term or a qualifying architectural decision.

## Layout

```text
/
├── CONTEXT.md
└── docs/
    ├── ADR-0001-DETERMINISTIC-GROUNDING.md
    └── ADR-0002-REPRODUCIBLE-STAGE8-RELEASE-GATE.md
```

There is no `CONTEXT-MAP.md` and no per-module context split.

## Vocabulary

Use the terms defined in `CONTEXT.md` in issue titles, questions, decisions,
test names, and implementation plans. Do not substitute synonyms that the
glossary explicitly rejects.

## ADR conflicts

If a proposed decision contradicts an existing ADR, surface the conflict
explicitly rather than silently overriding it.
