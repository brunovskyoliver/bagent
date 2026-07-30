# Measure native tool-calling across installed BaseRT models

Type: research
Status: resolved
Blocked by: none

## Question

How reliably do Qwen3 4B, Qwen3 8B, and Qwen3.6 35B-A3B select and sequence the existing Mail and web tools when given identical prompts, tool schemas, fixtures, budgets, and repeated runs? Record malformed calls, duplicate calls, ignored guidance, grounding accuracy, completion rate, latency, and resource cost separately from deterministic orchestration behavior.

## Answer

The protocol, primary-source constraints, directional live matrix, and interpretation are recorded in [BaseRT native tool-calling comparison](../assets/02-basert-tool-calling-matrix.md).

Under identical synthetic evidence and production request settings, Qwen3 4B completed Mail and web 3/3, but its known real bagent failure proves that this capability is sensitive to the production transcript/data and is not a reliability guarantee. Qwen3 8B failed Mail 3/3 after 101–142 seconds by selecting invented rowids; its production-budget web run passed. Qwen3.6 35B-A3B failed both required contracts 3/3: it emitted text resembling a tool result instead of `mail_read`, skipped required web search, and its native template rejected bagent's mid-transcript system guidance before BaseRT fell back to generic ChatML.

All structured calls that were emitted parsed successfully. The dominant failures were sequencing, evidence identifier copying, transcript compatibility, and terminal completeness—not SSE transport. No installed model is reliable enough to own mandatory evidence transitions. Deterministic orchestration must acquire and validate required evidence; model selection should optimize synthesis quality and latency over an already-complete evidence bundle.
