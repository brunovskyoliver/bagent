# Design the deterministic orchestration boundary

Type: grilling
Status: resolved
Blocked by: 01, 03, 08

## Question

Given the observed failure taxonomy and required-evidence contracts, what responsibilities and interfaces belong to intent classification, evidence planning, policy-gated execution, completeness validation, optional model-led exploration, and final synthesis? Specify how the shared agent loop handles retries, duplicate suppression, budgets, connector failures, partial evidence, and prompt injection while keeping mandatory evidence acquisition independent of native model tool-calling.

## Answer

The agreed domain vocabulary is recorded in [`CONTEXT.md`](../../../CONTEXT.md).

Architecture boundary:

- Add one typed `EvidenceOrchestrator`, shared by foreground chat and automations, for recognized Mail and web Evidence Intents. Unrelated connectors retain the existing general agent loop.
- Deterministic classification establishes the minimum Evidence Plan for clear intents. A model may clarify or add requirements to an ambiguous intent but may never downgrade deterministic evidence requirements.
- If plausible plans differ materially in privacy, cost, or evidence scope, ask one concise clarification rather than choosing the broader plan.
- Mandatory Mail listing/reading and minimum web search/fetch transitions are deterministic. Native model tool-calling is not a correctness dependency.
- 35B-A3B may request optional exploration only by selecting Validated References or proposing another bounded search. It cannot supply raw rowids, arbitrary URLs, tool names, or unchecked arguments.

Execution:

- Every normalized operation passes through the existing policy gate immediately before execution; planning grants no authority.
- Use a canonical Operation Key to group attempts and suppress duplicates before connector execution.
- Permit one retry only for transient timeout, reset, HTTP 429, or 5xx outcomes. Prefer alternate web providers/candidates; permanent, denied, invalid, unsupported, empty, and validation failures are not retried.
- Every actual search/fetch attempt consumes the two-search/five-fetch budget; suppressed duplicates do not.
- Mail bodies are read sequentially newest-first. Up to two independent validated web fetches may run concurrently.
- Connector results are typed. The orchestrator—not the model—owns provider/execution status, candidate and evidence identity, final URLs, extraction/truncation status, evidence counts, budgets, duplicate detection, policy result, and citation eligibility.
- Partial evidence is preserved with explicit shortfalls. Zero usable evidence returns a deterministic Recovery Outcome without invoking a model.
- Instruction-like Evidence Content becomes an Evidence Exclusion before synthesis, unless the user explicitly asks to analyze it as quoted data.

Synthesis:

- Convert validated data into a versioned, bounded Evidence Bundle containing the original request, Evidence Intent, completeness, requested/acquired/missing counts, evidence/source IDs, eligible citation URLs, conflicts, shortfalls, and untrusted Evidence Content.
- Web content is reduced to source-linked Evidence Passages with surrounding context; every truncation remains explicit. Optional passage expansion uses Validated References inside budget.
- Final synthesis is a separate 35B-A3B request with one initial system message and no tools. Do not replay the tool transcript or append mid-transcript system guidance.
- Validate the answer against evidence IDs, eligible citations, required item coverage, conflicts, and shortfall disclosure before display.
- On validation failure, permit one fresh tool-free Synthesis Repair over the same Evidence Bundle plus machine-readable errors. If repair fails, render the validated evidence deterministically.

All phases emit the previously defined Diagnostic Trace and Evidence Phase/Outcome events through shared infrastructure.
