# Reliable Grounded Mail and Web Agent Flows

Label: wayfinder:map
Status: resolved

## Destination

A tested, decision-complete redesign specification for bagent's Mail and web-search orchestration, backed by reproducible evidence that separates BaseRT model limitations from agent-flow defects. The specification must be implementable without further decisions about routing, evidence completeness, fallback behavior, observability, or acceptance criteria.

## Notes

- Domain: read-only Apple Mail and web-search agent flows in `bagent`.
- Use `wayfinder`, `grilling`, and `domain-modeling` while resolving decision tickets; use `research` for evidence gathered outside the working tree.
- Compare all installed BaseRT chat models currently identified by `basert list`: `basecompute/Qwen3-4B-Instruct-2507`, `basecompute/Qwen3-8B`, and `basecompute/Qwen3.6-35B-A3B`.
- The flow owns required evidence acquisition and completeness. Models interpret and summarize evidence; native tool-calling must not be the sole correctness mechanism.
- Prefer `basecompute/Qwen3.6-35B-A3B` for final evidence synthesis because of its measured speed. Design a native-template-compatible transcript with one initial system message; use Qwen3 4B as the availability fallback.
- Prioritize grounded correctness, then establish explicit latency and tool-count budgets from measurements.
- Preserve the shared agent loop, policy gate, approval behavior, stateless chat direction, and untrusted-content boundary unless a ticket proves a change is necessary.
- Starting evidence: the live prompt `can you read me the 3 latest emails?` produced three `mail_list_inbox` audit calls and no `mail_read`. `desired_mail_read_count` only recognizes summary wording, so the deterministic prefetch path did not activate and Qwen3 4B repeatedly ignored follow-up guidance.

## Decisions so far

- [Reproduce and classify current Mail and web failures](issues/01-reproduce-classify-current-failures.md) — Mail “read me” wording bypasses deterministic reads, web acquisition is prompt-only, and activity/traces do not represent evidence completeness; policy and connector availability were not the primary failure.
- [Measure native tool-calling across installed BaseRT models](issues/02-measure-basert-tool-calling.md) — No installed model reliably owns mandatory evidence transitions: 4B is the best current transcript fit but production-sensitive, 8B is slow and copies invalid rowids, and 35B-A3B rejects mid-transcript system guidance.
- [Define required-evidence contracts for Mail and web](issues/03-define-evidence-contracts.md) — Mail distinguishes header listing from body reading with explicit batching and shortfalls; web requires fetched, cited, and sometimes corroborated evidence under bounded exploration; all external content remains untrusted data.
- [Design diagnostic traces and activity semantics](issues/05-design-diagnostic-traces.md) — UI progress is evidence-based rather than call-based; privacy-safe traces separate execution from evidence contribution, group retries, expose sanitized diagnostics, and retain no prompt or evidence content.
- [Characterize web search and fetch evidence quality](issues/08-characterize-web-evidence-quality.md) — Current prose results collapse provider/fetch states and can mis-cite redirects; typed candidates, fetch outcomes, extraction quality, failure codes, and stronger SSRF validation are required before web evidence can be enforced.
- [Design the deterministic orchestration boundary](issues/04-design-deterministic-orchestration.md) — A shared typed Evidence Orchestrator deterministically plans, gates, executes, validates, and bundles required evidence; 35B receives a single tool-free synthesis request and one bounded repair opportunity.
- [Set model-selection, latency, and fallback policy](issues/06-set-model-latency-fallback-policy.md) — Deterministic intent and evidence work surround lazy warm 35B synthesis with explicit cold/warm deadlines, one bounded 4B fallback, no interactive 8B role, and deterministic zero-evidence recovery.
- [Finalize the executable redesign specification](issues/07-finalize-redesign-specification.md) — The resolved contracts are consolidated into a typed implementation specification with deterministic fixtures, staged rollout, rollback boundaries, and fifteen acceptance gates; no design decision remains open.

## Not yet specified

<!-- No unresolved fog at the current frontier. -->

## Out of scope

- Redesigning Notes, filesystem, WhatsApp, Odoo, Codex, shell, or other connector flows.
- Side-effecting tools, write approvals, and autonomous action policies except where existing policy must remain compatible.
- Implementing the redesign or changing production behavior during the wayfinding effort.
- UI redesign beyond accurately representing tool activity, failure, and evidence completeness.
