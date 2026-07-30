# Set model-selection, latency, and fallback policy

Type: grilling
Status: resolved
Blocked by: 02, 04

## Question

Using the installed-model measurements and deterministic orchestration design, which models and transcript shapes are compatible enough to be admitted for classification, optional exploration, and synthesis; what p50 and p95 latency, tool-count, round, and timeout budgets apply; and when must bagent return a disclosed partial result or fall back to another installed model? Include the Qwen3.6 restriction on mid-transcript system messages and the Qwen3 8B hidden-reasoning cost. Correct grounding must remain mandatory while avoidable interactive latency is bounded.

## Answer

Model admission:

- Deterministic parsing owns recognized Mail and web Evidence Intent classification. No model is required on this critical path.
- For otherwise unclassified wording, Qwen3 4B may propose a typed intent, but bagent must validate it against the user's request. Failed validation or ambiguity that materially changes privacy, cost, or evidence scope requires clarification.
- Qwen3.6 35B-A3B is the preferred synthesis model. It receives a fresh, tool-free transcript containing exactly one initial system message and a bounded Evidence Bundle. Tool transcripts and mid-transcript system messages are forbidden because its native template rejects that shape.
- Qwen3.6 35B-A3B may make at most one optional exploration proposal using only Validated References. The proposal must fit the existing Evidence Plan and global operation budget; it cannot start a recursive model-controlled loop.
- Qwen3 4B is the availability and synthesis fallback. It receives the same tool-free, bounded transcript contract as 35B.
- Qwen3 8B is not admitted to interactive classification, exploration, or synthesis. Its measured hidden-reasoning behavior took 100.8–141.7 seconds for Mail and copied invented identifiers.

Residency and latency:

- Load 35B lazily on the first eligible synthesis request, keep it warm for 20 minutes after use, and unload it earlier under memory pressure.
- Expose cold loading as a distinct Evidence Phase with a 45-second readiness timeout. Once ready, warm synthesis has a target p50 of at most 8 seconds, target p95 of at most 15 seconds, and a hard timeout of 20 seconds.
- The p50/p95 values are admission targets derived from directional 5.2–13.5 second warm measurements, not statistically established service levels. Acceptance testing must replace them with measured distributions.
- If 35B fails to load, encounters memory pressure, becomes unavailable, or exceeds its synthesis timeout, make one 4B fallback synthesis attempt capped at 25 seconds. Do not retry 35B within the same turn.
- If 4B also fails, render the validated Evidence Bundle deterministically and disclose that model summarization was unavailable.

Operation and round limits:

- Mail permits one planning round, one inbox listing, and at most ten sequential body reads per Reading Batch.
- Web permits at most two searches and five fetches, with no more than two fetches executing concurrently.
- One retry is allowed only for a transient timeout, connection reset, HTTP 429, or HTTP 5xx. Every actual attempt, including retries, consumes the applicable global budget; suppressed duplicates do not.
- Model exploration is limited to one proposal round. Synthesis is one 35B attempt, followed only by the previously specified single Synthesis Repair when validation fails. A timeout or availability failure uses the one 4B fallback instead of repairing the missing 35B answer.

Fallback and partial-result rules:

- Complete evidence is eligible for synthesis.
- Useful Partial Evidence remains eligible only when the Evidence Bundle constrains claims to acquired evidence and the output explicitly discloses every shortfall.
- Zero usable evidence, Denied Access, or entirely rejected or unavailable fetched content bypasses both synthesis models and returns a deterministic Recovery Outcome.
- No model may fill evidence gaps from parametric memory. Model failure never causes evidence reacquisition outside the original budget.

These limits preserve fast 35B synthesis while making evidence correctness independent of every installed model.
