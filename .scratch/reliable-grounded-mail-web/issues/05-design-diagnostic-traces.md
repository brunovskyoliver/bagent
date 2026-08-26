# Design diagnostic traces and activity semantics

Type: grilling
Status: resolved
Blocked by: 01

## Question

What redacted diagnostic record and user-visible activity model will make each agent turn explainable without exposing Mail bodies, fetched page content, secrets, or private prompts? Specify per-round model decisions, planned versus executed tools, normalized result status, evidence counts, retries, duplicate suppression, failure reasons, completeness, timing, and how the UI distinguishes meaningful completed work from repeated or ineffective calls.

## Answer

The agreed vocabulary is recorded in [`CONTEXT.md`](../../../CONTEXT.md).

User-visible activity:

- The collapsed line shows the final Evidence Outcome, such as `Read 3 of 3 emails`, `Read 2 of 3 emails · partial`, `Web verified · 2 sources`, `Mail access denied`, or `Couldn't verify sources`.
- While running, show the current Evidence Phase and measurable progress: finding, reading N of M, searching, verifying N of M, or preparing the grounded answer.
- Do not show tool-call counts, model rounds, or generic thinking when a concrete phase is known.
- The expanded view shows Logical Activities with operation type, evidence counts, source domains, duration, retry count, duplicate suppression, Evidence Contribution, and normalized failure reason.
- A technically successful connector call advances progress only when its Evidence Contribution is `satisfied` or `partial`; `empty`, `duplicate`, and `irrelevant` calls do not.
- Retries and suppressed duplicates are grouped under one Logical Activity and never inflate collapsed progress.
- Collapsed failures are plain-language Recovery Outcomes. Technical failure codes, failed phase, attempts, and timing stay in the expanded view.

Structured Diagnostic Trace:

- Correlate every turn, evidence plan, logical activity, attempt, observable model decision, validation decision, evidence delta, synthesis start/end, and terminal completeness outcome.
- Record model ID, normalized operation, argument hash, execution status (`succeeded`, `failed`, `denied`, `timed_out`), Evidence Contribution (`satisfied`, `partial`, `empty`, `duplicate`, `irrelevant`), counts, source domains, durations, retry/suppression data, and normalized failure codes.
- Record why the orchestrator accepted, retried, replaced, suppressed, or rejected an Observable Decision, but never hidden reasoning or `reasoning_content`.
- Never persist user prompts, Mail bodies or identities, webpage text, raw tool arguments, answer text, tokens, credentials, or secrets.
- Retain traces for at most seven days and the latest 1,000 turns, whichever is smaller, with size-based rotation.
- Provide a one-click Diagnostic Export as sanitized JSON containing the same structural, status, completeness, model, and timing data.

The existing `steps completed` UI and string-marker success inference must be replaced because connector execution success is not task or evidence completion.
