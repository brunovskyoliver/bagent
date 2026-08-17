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

**Session Export**:
A user-requested representation of the retained user-visible content and provenance of one Automation Session, including every Truncation Disclosure. It excludes opaque connector tokens and raw execution data.
_Avoid_: Diagnostic Export, database dump

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

**Validated Source**:
A web source successfully fetched and admitted to support, corroborate, or conflict with Final Output. Discovery candidates and unfetched search results are not Validated Sources.
_Avoid_: Search result, discovery snippet

**Connector Reference**:
A privacy-safe opaque reference validated by trusted connector execution and retained separately from activity so an authorized source or action can be revisited. It reveals neither connector-native identity nor Evidence Content.
_Avoid_: Raw connector ID, activity detail

**Approval Record**:
The privacy-safe provenance of one requested side effect and its user decision, expiry, or restart abandonment. It preserves the action category and request identity without raw arguments, payloads, credentials, or private identities.
_Avoid_: Raw approval request, inferred denial

**Approval Withdrawal**:
The terminal invalidation of a pending approval because its Work was cancelled before the user decision or deadline won. It is distinct from denial, expiry, and daemon-restart abandonment.
_Avoid_: User denial, approval expiry, cancellation acknowledgement

**Fresh Approval**:
A new, action-specific user decision required for a gated action in its current execution context. Historical approvals and Continuation Provenance never satisfy it.
_Avoid_: Reused approval, inherited permission

**Run Outcome**:
The terminal classification of an Automation Run as completed, partial, failed, skipped, cancelled, or abandoned. It describes satisfaction of the task rather than incidental retry or tool-attempt status.
_Avoid_: Tool outcome, progress state

**Completion Attention**:
The mutable unread or viewed state of a terminal Automation Session, kept separately from its immutable content. Scheduler-only skipped runs do not require attention.
_Avoid_: Run Outcome, session mutation

**Truncation Disclosure**:
The durable notice that retained Automation Session content is incomplete, including which section was bounded and the original and retained extent. Truncation is never silent.
_Avoid_: Omission, complete result

**Persistence Allowlist**:
The closed set of user-visible content and privacy-safe metadata permitted to become durable Automation Session data. Unknown or non-allowlisted fields are discarded before persistence.
_Avoid_: Post-hoc redaction, debug capture

**Event Allowlist**:
The closed subset of privacy-safe identity, lifecycle, queue, activity, approval, and availability facts permitted in daemon events. Content-bearing changes are announced by identity and fetched through an authorized projection rather than embedded in the event.
_Avoid_: Persistence Allowlist, arbitrary event payload, content stream

**Automation Session Retention**:
The automatic boundary that keeps at most fifty Automation Sessions per automation and no session longer than ninety days. Active work and pending approvals are never pruned.
_Avoid_: Permanent history, unread preservation

**Session Deletion**:
The explicit removal of one terminal Automation Run and its one-to-one Automation Session content while leaving only a privacy-safe audit tombstone. A Current Chat previously seeded from that session remains separate.
_Avoid_: Clear Current Chat, delete automation

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

**Work**:
One executable unit corresponding to exactly one accepted Conversation Turn or one admitted Automation Run, with a stable identity and one authoritative lifecycle from submission through terminal outcome. A scheduler-only skipped occurrence is recorded without Work because execution was never admitted.
_Avoid_: Model request, UI interaction mode, scheduler occurrence

**Work Identity**:
The stable opaque identity of one Work, distinct from the identities of its originating Current Chat, Conversation Turn, Automation Definition, Automation Run, or Automation Session.
_Avoid_: Session ID, model request ID, origin identity

**Work Origin**:
The immutable classification and provenance that ties one Work to exactly one Conversation Turn or admitted Automation Run without merging their domain identities.
_Avoid_: Session, mutable execution context, inferred trigger

**Work State**:
The closed, revisioned lifecycle state of one Work as queued, waiting for a model, running, waiting for approval, cancelling, or in an immutable completed, partial, failed, cancelled, or abandoned outcome.
_Avoid_: UI mode, inferred progress, independent status flags

**Work Revision**:
The monotonically increasing version of one Work after an authoritative mutation, used to reject stale mutation and projection without replacing the global Event Cursor.
_Avoid_: Event sequence, schema version, daemon generation

**Work Snapshot**:
The privacy-safe authoritative view of Work and related projected state that is consistent through one Event Cursor.
_Avoid_: Event log, UI cache, Current Chat snapshot

**Event Cursor**:
The durable position through which authoritative events are known to be committed and from which later events may be resumed or a gap detected.
_Avoid_: Connection identity, Work revision, receipt timestamp

**Daemon Generation**:
The opaque identity of one daemon process lifetime, changed on restart without changing durable Work identity or Event Cursor continuity.
_Avoid_: Work generation, model process identity, event sequence

**Cancellation Intent**:
The monotonic, acknowledged request to stop one nonterminal Work at its next safe point. It does not itself prove that execution stopped; only a terminal cancelled outcome does.
_Avoid_: Immediate cancellation, terminal outcome, UI dismissal

**Execution Slot**:
The bounded admission capacity held by Work while it may execute model or tool activity, distinct from queue position and from a Model Residency Lease.
_Avoid_: Model Residency Lease, scheduler claim, thread

**Model Residency**:
The period during which a model's weights remain loaded and ready, distinct from availability of the model service itself. An active model generation protects residency through a Model Residency Lease; otherwise residency may end after inactivity or memory pressure without changing task correctness.
_Avoid_: Permanent model ownership, evidence cache

**Model Residency Lease**:
The non-preemptible protection held for one active model generation so its required Model Residency cannot be changed until that generation ends.
_Avoid_: Loaded-model ownership, execution slot, cancellation immunity

**Model Runtime Generation**:
The opaque identity of one bagent-owned port-8082 model process lifetime, changed only by a changed-process restart and kept separate from Daemon Generation.
_Avoid_: Daemon Generation, Work Revision, independent port-8080 runtime

**Command Acknowledgement**:
The durable idempotent result of accepting or recognizing one command, distinct from proof that the requested Work has reached a terminal outcome.
_Avoid_: Work completion, HTTP delivery, event receipt

**Automation Run**:
One scheduled or manually triggered execution occurrence of an automation, with its own lifecycle and terminal state, isolated from every earlier and later execution.
_Avoid_: Recurring conversation, shared run

**Automation Definition**:
The mutable saved task and schedule that may create future Automation Runs. Deleting it stops future execution without deleting existing Automation Sessions.
_Avoid_: Automation Session, Task Snapshot

**Automation Session**:
The immutable task context, observable activity, tool outcomes, and final response owned one-to-one by an Automation Run. It may seed a new Current Chat but is never extended by that chat.
_Avoid_: Result summary, current chat

**Task Snapshot**:
The immutable copy of the user-authored automation task and identifying definition context captured for one Automation Session. It excludes internal prompts, hidden reasoning, credentials, and Evidence Content.
_Avoid_: Current automation definition, system prompt

**Run Provenance**:
The privacy-safe identity, timing, trigger, schedule, model-route, and terminal metadata that explains how one Automation Run occurred. It excludes prompts, model internals, raw provider errors, credentials, and Evidence Content.
_Avoid_: Activity transcript, debug log

**Final Output**:
The complete privacy-reviewed user-visible answer belonging to an Automation Session. It is absent when a run produces no safe answer and is distinct from its Result Summary.
_Avoid_: Result Summary, raw model output

**Result Summary**:
The short, glanceable description of an Automation Session used in compact result lists. It never substitutes for Final Output.
_Avoid_: Full result, Automation Session

**Session Activity Timeline**:
The chronological privacy-safe sequence of Logical Activities belonging to an Automation Session, with normalized categories, outcomes, counts, timing, and failures. It excludes individual attempts, raw tool data, identities, Evidence Content, and hidden reasoning.
_Avoid_: Tool transcript, reasoning trace

**Conversation Turn**:
One user request and its resulting assistant work within the current chat session.
_Avoid_: Automation Run, model request

**Current Chat**:
The user-controlled conversation that receives new Conversation Turns and survives app or daemon restart until explicitly cleared. Clearing it starts a fresh Current Chat without deleting Automation Sessions or saved long-term memory.
_Avoid_: Automation Session, conversation archive

**Current Chat Draft**:
The user-authored text prepared for the next Current Chat turn but not yet submitted. It remains separate from completed turns and from internal, system, model, or tool prompts.
_Avoid_: Conversation Turn, prompt log

**Slash Command Candidate**:
The complete, unmodified Current Chat input when it is one whitespace-free token beginning with `/` at the first character. Only a candidate matching a known command or alias produces suggestions; its entered text is never normalized by suggestion display.
_Avoid_: Command mode, rewritten input

**Slash Command**:
A recognized local instruction identified by a canonical name or an explicitly accepted alias. It is handled by bagent itself and never becomes model input or a Conversation Turn.
_Avoid_: Slash-prefixed prompt, model command

**Clear Current Chat**:
The explicit `/clear` action that removes only Current Chat content and chat-scoped continuation context before creating a new empty Current Chat. It never deletes Automation Sessions or saved long-term memory.
_Avoid_: Delete Automation Session, forget memory

**Saved Long-Term Memory**:
User-authorized distilled facts or preferences retained independently of Current Chat and Automation Sessions. Neither unattended work nor Continuation Seeds create it automatically.
_Avoid_: Conversation archive, Automation Session

**Continuation**:
The one-way creation of a new Current Chat from one terminal Automation Session. It marks the source viewed but never reopens, extends, or merges with that session.
_Avoid_: Resume run, append to session

**Continuation Seed**:
The bounded, visible, privacy-safe context copied from one Automation Session into a new Current Chat. Historical approvals in the seed convey provenance but never authority.
_Avoid_: Session transcript, inherited conversation

**Continuation Provenance**:
The durable one-way link from a continued Current Chat to its source Automation Session. The link may report that its source expired or was deleted, but never grants authority or mutates the source.
_Avoid_: Shared session, inherited approval

**Activity Peek**:
A transient, compact notch presentation of one privacy-safe current activity and its tool category while work continues in the background. It exposes neither hidden reasoning nor evidence content and does not take focus.
_Avoid_: Chain of thought, activity transcript, notification window

**Permission Grant Assist**:
An in-notch guide that opens the relevant macOS privacy pane, presents bagent as a draggable application, rechecks the authoritative system grant, and offers a UI-only relaunch when the grant requires it.
_Avoid_: Custom permission dialog, automatic grant, daemon restart

**UI-only Relaunch**:
The bounded replacement of the notch presentation process while the daemon, BaseRT, Automation Runs, and model leases retain their existing ownership and lifecycle.
_Avoid_: App restart, daemon restart, runtime restart

**UI Relaunch Handoff**:
The versioned, short-lived, single-use transfer of allowlisted presentation state from the old notch UI to its intended replacement during a UI-only Relaunch.
_Avoid_: Current Chat snapshot, runtime checkpoint, process archive

**UI Event Consumer**:
The single notch UI authority that applies daemon events to presentation state. Its identity remains distinct from reconnectable event-stream transport connections.
_Avoid_: Event connection, duplicate UI subscriber

**Deterministic Rendering**:
A model-free presentation of validated evidence and shortfalls used when synthesis is ineligible or every admitted synthesis attempt fails.
_Avoid_: Ungrounded completion, silent failure
