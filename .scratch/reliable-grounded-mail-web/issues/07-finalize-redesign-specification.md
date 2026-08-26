# Finalize the executable redesign specification

Type: task
Status: resolved
Blocked by: 04, 05, 06

## Question

Consolidate the resolved decisions into one implementation-ready specification covering component responsibilities, interfaces and data shapes, Mail and web data flow, state transitions, safety boundaries, failure behavior, compatibility constraints, observability, test fixtures, model matrix, performance budgets, rollout order, and acceptance criteria. Is every decision needed by an implementer explicit and supported by the preceding evidence?

## Answer

The consolidated implementation contract is recorded in [Reliable grounded Mail and web flows: executable redesign specification](../assets/07-executable-redesign-specification.md).

It defines the typed Evidence Orchestrator boundary, canonical intent/plan/operation/result/bundle shapes, deterministic Mail and web state machines, policy and untrusted-content boundaries, model admission and transcript rules, runtime deadlines and residency, privacy-safe events and diagnostics, deterministic fixtures, rollout/rollback order, and fifteen acceptance gates.

The decision-completeness audit maps every destination category to a specification section. No unresolved product or architecture decision remains. Exact Rust module subdivision, enum spelling, database migration mechanics, and UI styling remain implementation choices constrained by the specification rather than missing decisions.
