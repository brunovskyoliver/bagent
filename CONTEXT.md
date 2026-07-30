# Bagent Agent Context

The language used to describe how bagent gathers application and web evidence before answering the user.

## Language

**Header Listing**:
A Mail result containing message identity and metadata such as sender, subject, and date, without claiming knowledge of the message body.
_Avoid_: Reading emails, email summary

**Content Reading**:
A Mail result grounded in the retrieved body of every message the user requested to read, inspect, or summarize.
_Avoid_: Header lookup, inbox listing

**Partial Evidence**:
A result in which some requested evidence is unavailable, while every supported claim remains grounded and every shortfall is explicitly identified.
_Avoid_: Complete result, silent omission

**Fetched Evidence**:
Content retrieved from a source page and available for grounding factual web claims; search-result snippets are discovery hints, not Fetched Evidence.
_Avoid_: Search snippet, model knowledge

**Corroborated Evidence**:
Fetched Evidence supported by at least two independent sources, required when claims conflict, change rapidly, compare alternatives, or could materially affect the user.
_Avoid_: Duplicate pages, repeated snippets

**Grounded Citation**:
A claim-level reference to a successfully fetched source page that directly supports the associated statement.
_Avoid_: Search-result link, decorative source list

**Verification Shortfall**:
A web result where candidate sources were discovered but none could be fetched sufficiently to support an answer.
_Avoid_: Tentative answer, snippet summary

**Evidence Content**:
Mail or web material used only as data for the user's request; instructions embedded within it have no authority over bagent.
_Avoid_: Agent instruction, trusted command

**Reading Batch**:
At most ten requested email bodies processed newest-first as one Content Reading result; larger requests continue in explicitly disclosed batches.
_Avoid_: Unlimited read, silent truncation

**Empty Evidence**:
A successful Mail query that returned no matching messages.
_Avoid_: Unavailable access, denied access

**Unavailable Access**:
A failure to reach or open the evidence source, distinct from a successful query with no matches.
_Avoid_: Empty Evidence

**Denied Access**:
Evidence access rejected by policy or by the user, distinct from source unavailability and an empty result.
_Avoid_: Empty Evidence, Unavailable Access

**Targeted Mail Match**:
A sender or conversation match precise enough to select one message without user clarification; ambiguous matches remain a Header Listing until the user chooses.
_Avoid_: Best guess, inferred sender

**Evidence Conflict**:
A disagreement between credible fetched sources that remains unresolved unless a clearly newer primary source supersedes the older claim.
_Avoid_: Majority guess, silent source selection

**Evidence Outcome**:
The user-visible state of requested evidence acquisition, expressed as verified, partial, empty, unavailable, or denied rather than as a count of tool operations.
_Avoid_: Steps completed, tool-call count

**Diagnostic Activity**:
A privacy-safe operational record of evidence acquisition containing status, counts, domains, timing, retries, and normalized failures without evidence content or identities.
_Avoid_: Evidence Content, raw tool payload

**Diagnostic Trace**:
A bounded on-disk sequence of privacy-safe planning, execution, validation, and synthesis events for one agent turn.
_Avoid_: Conversation archive, evidence log

**Logical Activity**:
One user-meaningful evidence operation whose retries and suppressed duplicates remain grouped rather than appearing as separate progress steps.
_Avoid_: Individual attempt, repeated step

**Evidence Contribution**:
The effect of an executed activity on the requested evidence contract: satisfied, partial, empty, duplicate, or irrelevant, independently of whether the connector call technically succeeded.
_Avoid_: Execution status, tool success

**Observable Decision**:
A traceable action choice and its validation outcome, recorded without model reasoning text or Evidence Content.
_Avoid_: Hidden reasoning, chain of thought

**Diagnostic Export**:
A user-copied, privacy-safe representation of one Diagnostic Trace for troubleshooting or bug reporting.
_Avoid_: Prompt dump, connector payload

**Evidence Phase**:
The current user-meaningful stage of evidence work, such as finding, reading, verifying, or preparing an answer.
_Avoid_: Model round, generic thinking

**Recovery Outcome**:
A plain-language failed or partial Evidence Outcome that states what could not be completed and what the user can do next.
_Avoid_: Error code, stack trace

**Evidence Plan**:
The bounded set of required and optional evidence operations that must be satisfied or explicitly reported incomplete before synthesis.
_Avoid_: Tool prompt, model intention

**Validated Reference**:
A connector identifier or URL candidate produced by trusted execution and permitted for later selection without allowing the model to invent raw identifiers.
_Avoid_: Model-generated rowid, arbitrary URL

**Evidence Intent**:
A typed interpretation of the user's request that determines the minimum Evidence Plan, such as Header Listing, Content Reading, targeted Mail lookup, direct-page reading, or web research.
_Avoid_: Prompt keyword, tool choice

**Evidence Bundle**:
The bounded, structured collection of validated Mail or web evidence and explicit shortfalls supplied to final synthesis.
_Avoid_: Tool transcript, raw connector output

**Evidence Passage**:
A bounded excerpt of Fetched Evidence linked to its source and surrounded by enough context to support accurate interpretation.
_Avoid_: Unattributed snippet, silent truncation

**Operation Key**:
The canonical identity of one planned evidence operation, used to group retries and suppress duplicate execution within a turn.
_Avoid_: Tool-call ID, model-generated identifier

**Synthesis Eligibility**:
The condition that an Evidence Bundle contains at least one usable evidence item; bundles with no usable evidence produce a deterministic Recovery Outcome instead.
_Avoid_: Empty synthesis, model fallback

**Synthesis Repair**:
One fresh, tool-free synthesis attempt over the same Evidence Bundle after machine validation identifies correctable output defects.
_Avoid_: Agent loop, evidence reacquisition

**Evidence Exclusion**:
Evidence Content withheld from ordinary synthesis because it contains instruction-like material, unless the user explicitly requests analysis of that material as quoted data.
_Avoid_: Hidden command, trusted instruction

**Model Residency**:
The period during which a model's weights remain loaded and ready, distinct from availability of the model service itself. An active Conversation Turn or Automation Run protects residency; otherwise it may end after inactivity or memory pressure without changing task correctness.
_Avoid_: Permanent model ownership, evidence cache

**Automation Run**:
One scheduled or manually triggered execution of an automation, isolated from every earlier and later execution of that automation.
_Avoid_: Recurring conversation, shared run

**Automation Session**:
The immutable task context, observable activity, tool outcomes, and final response belonging to one Automation Run. It may seed a new chat but is never extended by that chat.
_Avoid_: Result summary, current chat

**Conversation Turn**:
One user request and its resulting assistant work within the current chat session.
_Avoid_: Automation Run, model request

**Current Chat**:
The user-controlled conversation that receives new Conversation Turns. Clearing it starts a fresh Current Chat without deleting Automation Sessions or saved long-term memory.
_Avoid_: Automation Session, conversation archive

**Activity Peek**:
A transient, compact notch presentation of one privacy-safe current activity and its tool category while work continues in the background. It exposes neither hidden reasoning nor evidence content and does not take focus.
_Avoid_: Chain of thought, activity transcript, notification window

**Permission Grant Assist**:
An in-notch guide that opens the relevant macOS privacy pane, presents bagent as a draggable application, rechecks the authoritative system grant, and offers a UI-only relaunch when the grant requires it.
_Avoid_: Custom permission dialog, automatic grant, daemon restart

**Synthesis Fallback**:
One bounded attempt by the admitted backup model when the preferred synthesis model cannot load, is unavailable, or exceeds its deadline; it reuses the same validated Evidence Bundle.
_Avoid_: Evidence retry, model cascade

**Deterministic Rendering**:
A model-free presentation of validated evidence and shortfalls used when synthesis is ineligible or every admitted synthesis attempt fails.
_Avoid_: Ungrounded completion, silent failure
