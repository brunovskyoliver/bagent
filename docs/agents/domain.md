# Domain docs

How the engineering skills should consume this repository's domain documentation when exploring the codebase.

## Layout

This is a single-context repository:

```text
/
├── CONTEXT.md
├── docs/
│   └── adr/
└── crates/
```

## Before exploring

- Read the root `CONTEXT.md`.
- Read ADRs under `docs/adr/` that touch the area being explored.
- If either location does not exist, proceed silently. The domain-modeling skill creates domain documentation lazily when terminology or a durable architectural decision is actually resolved.

## Use the glossary's vocabulary

When output names a domain concept in an issue title, proposal, hypothesis, or test, use the term defined in `CONTEXT.md`. Do not drift to a synonym the glossary explicitly avoids.

If a needed concept is missing, reconsider whether it belongs to the existing language or record the gap for a domain-modeling session.

## Flag ADR conflicts

If proposed work contradicts an existing ADR, surface the conflict explicitly instead of silently overriding it.
