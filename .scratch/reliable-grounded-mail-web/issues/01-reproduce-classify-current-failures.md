# Reproduce and classify current Mail and web failures

Type: research
Status: resolved
Blocked by: none

## Question

Across a controlled set of Mail and web prompts, where does the current flow fail: intent recognition, tool routing, policy admission, connector execution, evidence completeness, model synthesis, or user-visible reporting? Produce a reproducible evidence report that includes the exact screenshot prompt, current implementation paths, redacted per-turn traces, executed tool sequences, connector outcomes, and final answers, without changing runtime behavior.

## Answer

The evidence is captured in [Current Mail and web failure evidence](../assets/01-current-failure-evidence.md).

The screenshot is primarily an orchestration failure exposed by Qwen3 4B. The broad Mail router recognizes the request, but the required-read detector recognizes only summary wording, so “read me the 3 latest emails” skips deterministic prefetch. The model then performs three inbox listings, no body reads, and returns an unfinished promise. Policy was not the blocker and live positive controls show the Mail connector can return readable messages.

Interactive web correctness has the same structural flaw: search/fetch requirements are prompt guidance, not enforced evidence transitions. Recorded prompts produced plausible or fabricated researched answers without any web tool call, while an automation positive control proves both web tools can execute.

The UI reports connector-call completion rather than evidence or task completion, so three duplicate list calls become “3 steps completed.” Current traces omit per-round decisions and structured result/completeness data. All 15 focused agent-loop tests pass, but none covers the exact wording, terminal incomplete evidence, or mandatory web acquisition.
