# Automation Session storage, retention, and chat continuation

Decision ticket:
[Define Automation Session storage, retention, and chat continuation](https://github.com/brunovskyoliver/bagent/issues/20)

Map:
[Rebuild the bagent notch UX around model residency and automation sessions](https://github.com/brunovskyoliver/bagent/issues/15)

Agreed through live grilling on 2026-07-31.

## Decision

Each [Automation Run](../CONTEXT.md) owns exactly one isolated, immutable
[Automation Session](../CONTEXT.md). A recurring automation never inherits
conversation state, activity, references, sources, approvals, or results from
an earlier run.

An Automation Session is a durable, reopenable product record. It contains the
task and provenance that produced the run, the permitted privacy-safe account
of what happened, and the safe result. It is not a conversation and cannot be
resumed or extended.

**Continue in new chat** creates a separate [Current Chat](../CONTEXT.md) from a
bounded, visible Continuation Seed. New chat work, tools, approvals, and results
belong only to Current Chat. `/clear` clears only Current Chat.

This policy replaces the current behavior in which `automation_runs` retains
only lifecycle fields and one 2,000-character result string, recurring runs
share ephemeral runtime references by automation identity, and deleting an
automation deletes its run history. It specifies the destination, not the
database migration or UI implementation.

## Canonical boundaries

| Concept | Owns | Mutability | Boundary |
|---|---|---|---|
| Automation Definition | Saved task and future schedule | Mutable; deletable | Creates future Automation Runs but is not historical truth |
| Automation Run | One execution occurrence and lifecycle | Advances to one terminal Run Outcome | Never shares execution state with another run |
| Automation Session | One run's Task Snapshot, Run Provenance, safe activity, and result | Immutable after terminalization | Owned one-to-one by one Automation Run |
| Completion Attention | `unread` or `viewed`, plus `viewed_at` | Mutable | Separate from immutable session content |
| Current Chat | User-controlled Conversation Turns and chat-scoped context | Mutable until `/clear` | Separate from every Automation Session |
| Saved Long-Term Memory | Explicitly saved distilled facts or preferences | Governed by its own policy | Never created automatically by an Automation Session or Continuation Seed |

The unqualified term “session” is ambiguous and must not be used in product
contracts or new APIs. Use **Automation Session** or **Current Chat**.

## Task Snapshot

Every Automation Session must freeze the task context used for that run when
the run is claimed or recorded as skipped:

- historical automation identity;
- display name;
- exact user-authored task text;
- structured schedule and IANA time zone;
- definition revision or equivalent last-updated identity.

The mutable Automation Definition may change or be deleted later. Neither event
may alter an existing Task Snapshot.

The Task Snapshot is user-authored product data. It does not contain generated
system instructions, internal prompt layers, hidden reasoning, credentials, or
Evidence Content. Task text keeps the existing 4,000-character validation
limit; oversized definitions are rejected rather than silently truncated.

## Run Provenance

Every Automation Session must retain:

- Automation Run and Automation Session identities;
- historical Automation Definition identity and Task Snapshot revision;
- nominal `scheduled_for`;
- `claimed_at`;
- actual `started_at`, when execution began;
- `finished_at`, when the terminal outcome was committed;
- trigger type: `scheduled` or `run_now`;
- catch-up status and the original scheduled occurrence;
- requested model route;
- actual admitted route, fallback route, or deterministic renderer;
- terminal Run Outcome and normalized terminal reason.

A skipped run has no `started_at`. Provenance may record model classes and
normalized route outcomes, but never internal prompts, raw model output,
provider errors, model internals, or credentials.

## Run Outcomes

| Outcome | Meaning | Final Output |
|---|---|---|
| `completed` | The requested task was satisfied | Required |
| `partial` | A safe answer exists but identifies unmet required work | Required |
| `failed` | Execution ended without a safe answer | Absent |
| `skipped` | Execution never began because of scheduler policy | Absent |
| `cancelled` | An explicit user or authoritative system cancellation stopped execution | Absent |
| `abandoned` | Runtime ownership became indeterminate after restart, crash, or equivalent loss | Absent |

Normalized reasons distinguish scheduler cases such as `overlap` and
`stale_occurrence`. A low-level retry or failed attempt does not make the run
partial if the requested outcome was ultimately satisfied.

Denied access produces `partial` only when a useful safe Final Output can state
the shortfall. Otherwise it produces `failed`. Failed, skipped, cancelled, and
abandoned runs receive a deterministic Result Summary and never fabricated or
partial model text as Final Output.

The existing no-overlap policy remains unchanged: at most one run of an
Automation Definition executes at once. A conflicting recurring occurrence is
recorded as `skipped` with reason `overlap`. Storage must nevertheless preserve
independent identities if recovery or migration exposes conflicting historical
records.

## Final Output and Result Summary

An Automation Session stores two distinct immutable products:

1. **Final Output** is the privacy-reviewed user-visible answer.
2. **Result Summary** is a separately generated, glanceable description for
   compact lists and Activity Peek.

Opening an Automation Session shows Final Output. Expanding Result Summary must
not masquerade as the full result. When Final Output is absent, Result Summary
must be deterministic and limited to the normalized terminal condition and a
safe next step.

## Session Activity Timeline

The timeline is chronological and contains Logical Activities, not individual
model or tool attempts. Retries and suppressed duplicates remain grouped under
their originating activity.

Each activity may retain only:

- allowlisted category, such as Mail, web, filesystem, Odoo, Codex, or generic;
- normalized operation;
- start, finish, and duration;
- normalized outcome such as succeeded, partial, empty, denied, failed, or
  timed out;
- Evidence Contribution;
- privacy-safe item counts;
- grouped attempt, retry, and duplicate-suppression counts;
- normalized failure code.

For example, denied Mail access may persist as `Mail · denied · 0 items`. It may
not include a sender, subject, mailbox, connector identifier, raw argument, raw
result, or message content.

## Validated Sources and Connector References

A Validated Source is retained only when a web source was fetched and admitted
to support, corroborate, or conflict with Final Output. Its record may contain:

- sanitized canonical final URL;
- title and domain;
- fetch and validation timestamp;
- supporting, corroborating, or conflicting role;
- stable citation identity linking it to Final Output.

Discovery snippets, search queries, unfetched candidates, fetched passages,
page content, redirect traces, and provider responses are not retained.

Connector data uses a separate privacy boundary:

- the activity timeline retains only category, normalized action, outcome,
  count, and timing;
- a Connector Reference may retain an opaque daemon-validated token, connector
  category, and availability state outside the timeline;
- connector-native identifiers, people or account identities, Mail subjects,
  content, write payloads, and raw arguments are never retained;
- reuse requires authoritative revalidation of current access;
- deleted or inaccessible targets become unavailable instead of exposing stale
  content.

## Approvals

Each approval request belonging to an Automation Run produces a privacy-safe
Approval Record containing:

- normalized action category and side-effect class;
- request, expiry, and resolution timestamps;
- resolution: `allowed`, `denied`, `expired`, or `abandoned`;
- resolution origin: user, deadline, or daemon restart;
- session-scoped non-reversible request fingerprint;
- redacted display summary;
- links to the Automation Run and related Logical Activity.

Approval lifetime remains 60 seconds.

A daemon restart abandons every approval that was pending when runtime ownership
was lost. Restart is not a user denial and does not become `denied`. It takes
precedence over a deadline that would expire after restart. The originating run
also becomes `abandoned`; neither approval nor side effect may resume.

Historical approvals carry provenance only. Every gated action in a continued
Current Chat requires Fresh Approval. An earlier denial, expiry, or abandonment
does not permanently prohibit a new request.

Raw arguments, write payloads, dry-run content, credentials, and private
identities are not Automation Session data.

## Completion Attention

Runs that started and end `completed`, `partial`, `failed`, `cancelled`, or
`abandoned` become independently `unread`. Scheduler-only skipped runs remain
in history but do not create unread attention.

Opening the Automation Session marks it `viewed` and records `viewed_at`.
Cycling focus in Activity Peek does not mark it viewed. Exporting without first
opening it does not mark it viewed.

Concurrent completions are ordered by `finished_at`, followed by a stable
Automation Run identity tie-breaker. Attention state survives app and daemon
restarts.

## Storage limits and truncation

| Section | Maximum retained |
|---|---:|
| Task Snapshot task text | 4,000 characters, validated before execution |
| Final Output | 64 KiB UTF-8 |
| Result Summary | 500 characters |
| Session Activity Timeline | 256 Logical Activities and 128 KiB encoded |
| Validated Sources | 32 records |
| Connector References | 32 records |
| Approval Records | 64 records |
| Continuation Seed | 16 KiB UTF-8 |

Truncation must never be silent.

For every bounded section, Truncation Disclosure records:

- the affected section;
- original size or count;
- retained size or count;
- truncation reason.

Bounded text retains beginning and ending portions around an explicit omission
marker. Bounded lists preserve terminal and safety-relevant records before
ordinary successful activity. Reopening, Session Export, diagnostics, and
Continuation Seed must all expose the applicable disclosure.

The 16 KiB Continuation Seed limit is independent of the 64 KiB Final Output
limit. A long Final Output may therefore carry both session-storage and
continuation-context disclosures.

## Retention and deletion

Automatic retention applies both limits, whichever removes a session first:

- at most 50 Automation Sessions per historical automation identity;
- at most 90 days from terminalization.

Skipped sessions count toward retention. Unread status does not override the
90-day privacy limit. Running sessions and sessions with pending approvals are
never pruned.

Cleanup must occur after terminalization, at daemon startup, and during bounded
periodic maintenance. Audit data records only privacy-safe cleanup counts.

### Delete one Automation Session

The user-facing action is **Delete Automation Session**, not “delete result.”
It is allowed only for a terminal run with no pending approval and atomically
removes:

- the Automation Run lifecycle record;
- Task Snapshot and Run Provenance;
- Final Output and Result Summary;
- activity, sources, connector references, and approvals;
- Completion Attention and session diagnostic material.

A minimal privacy-safe audit tombstone may retain opaque run identity, deletion
timestamp, and former Run Outcome. It contains no session content.

A Current Chat already seeded from the deleted session remains separate. Its
provenance link reports `source deleted`, while its already-copied seed remains.

### Delete an Automation Definition

Deleting an Automation Definition removes its mutable task and future schedule.
It is blocked while a run is active.

Existing Automation Sessions survive as detached history until individual
deletion or automatic retention. Their Task Snapshots provide historical names
and tasks. Unread detached sessions remain eligible for Activity Peek.

## Restart behavior

| Event | Required behavior |
|---|---|
| App-only relaunch | Active Automation Runs continue in the daemon; durable sessions and attention refetch unchanged |
| Daemon restart | Active Automation Runs and pending approvals become `abandoned`; no action resumes |
| Terminal session across either restart | Content, attention, and retention metadata remain unchanged |
| Current Chat across either restart | Completed turns and continuation provenance remain until `/clear` |
| In-flight Conversation Turn across daemon restart | Earlier turns remain; the interrupted turn gets a normalized interruption marker |
| Connector Reference after restart | Opaque record remains but must be revalidated before reuse |

Draft preservation during a full daemon restart is not part of this policy.
App-only permission handoff may preserve bounded draft state under
[Settings and Permission Grant Assist](SETTINGS_PERMISSION_GRANT_ASSIST_DECISION.md).

New APIs must avoid an unqualified `/sessions` route because “session” is
ambiguous. Route naming and wire shape are later implementation choices.

## Continue in new chat

Continue is available for `completed`, `partial`, `failed`, `cancelled`, and
`abandoned` Automation Sessions. It is not available for scheduler-only skipped
sessions.

Invoking Continue:

1. marks the source Automation Session viewed;
2. creates a new Current Chat identity;
3. creates one visible immutable Continuation Seed;
4. creates one durable one-way Continuation Provenance link;
5. never reopens, mutates, or appends to the source Automation Session;
6. never merges with an existing Current Chat.

If Current Chat is non-empty, replacement requires explicit confirmation.
Replacement clears that Current Chat without creating a hidden conversation
archive.

### Continuation Seed

The seed contains only:

- source Automation Session and Automation Run identities;
- historical automation name and exact task from the Task Snapshot;
- scheduled and finished times, trigger, model route, and Run Outcome;
- Result Summary;
- retained Final Output within the seed limit;
- compact activity categories, outcomes, counts, and normalized failures;
- Validated Source title, domain, sanitized URL, and citation identity;
- historical approval categories and outcomes, marked non-authorizing;
- every applicable Truncation Disclosure.

The seed contains no Evidence Content, fetched passage, connector-native
identifier, Connector Reference token, raw argument, write payload, credential,
internal prompt, or hidden reasoning. It contains no data from another recurring
run.

Continuation Provenance is visible in Current Chat and included in Session
Export and diagnostics. It identifies the source run, historical automation
name, outcome, and completion time. If retention or deletion removes the
source, the link reports `source expired` or `source deleted`; the copied seed
remains.

### Fresh work after continuation

A continued Current Chat behaves like any other foreground Current Chat:

- fresh read and write tools follow current policy;
- Connector References are revalidated before use;
- every gated action requires Fresh Approval;
- historical approval outcomes grant no authority;
- new activity, approvals, and results belong only to Current Chat;
- nothing writes back to the source Automation Session;
- nothing becomes input to a later recurring Automation Run.

## `/clear`

`/clear` is unavailable while a Conversation Turn or approval is active. It
must not act as implicit cancellation.

When accepted, `/clear` removes only:

- persisted Current Chat turns;
- Continuation Seed;
- Continuation Provenance;
- chat-scoped references;
- completed chat approval presentation records.

It then creates a new empty Current Chat identity.

`/clear` does not delete Automation Sessions, Automation Definitions, external
side effects that already occurred, audit tombstones, or Saved Long-Term
Memory. Privacy-safe system audit remains independent of chat presentation.

## Saved Long-Term Memory

Automation Sessions, Final Output, activity, sources, approvals, and
Continuation Seeds never become Saved Long-Term Memory automatically.
Unattended Automation Runs cannot create or modify it.

A continued Current Chat excludes its Continuation Seed from passive memory
extraction. Only an explicit foreground user request may save a bounded,
distilled fact or preference under the separate memory policy.

Saved memory never contains Evidence Content, raw prompts, tool arguments,
credentials, Connector References, or approval authority. Session deletion,
retention expiry, and `/clear` do not delete independently saved memory;
deleting memory does not alter Automation Sessions.

## Privacy and redaction

Automation Session persistence is governed by a closed Persistence Allowlist.
Sanitization occurs before the first durable write and before event broadcast,
not later during export. Unknown fields are discarded.

Permitted user content is limited to:

- Task Snapshot;
- privacy-reviewed Final Output;
- Result Summary;
- sanitized Validated Source metadata.

Operational data is limited to allowlisted enums, bounded counts, safe
timestamps, normalized reasons, and session-scoped non-reversible
fingerprints.

The following are never Automation Session activity, audit detail, diagnostics,
or continuation context:

- hidden reasoning or chain-of-thought;
- internal, system, or model prompts;
- Evidence Content or fetched passages;
- raw tool arguments or results;
- credentials, tokens, signed URLs, or secret query parameters;
- connector-native identifiers or private identities;
- raw provider errors, stack traces, or model internals.

A URL that cannot be safely sanitized retains only safe source metadata and is
marked unavailable. Durable content remains local inside bagent's protected
encrypted storage boundary.

The prohibition on raw prompts means internal prompt assembly. The exact
user-authored automation task is permitted only as the explicitly defined Task
Snapshot.

## Export and diagnostic access

Each terminal Automation Session supports two explicit exports.

**Session Export** contains retained user-visible content:

- Task Snapshot and Run Provenance;
- Run Outcome;
- Final Output or its explicit absence;
- Result Summary;
- Session Activity Timeline;
- Validated Sources;
- redacted approval history;
- Truncation Disclosures.

It excludes opaque connector tokens and raw execution data.

**Diagnostic Export** is a versioned privacy-safe structural record containing
normalized states, timings, categories, counts, fingerprints, failures, and
retention metadata. It contains no task text, Final Output, source URL, private
identity, or Connector Reference.

## Acceptance scenarios

### 1. Definition changes after an older run

Monday captures Task Snapshot v1 and completes. Tuesday changes the Automation
Definition to v2. Monday always reopens and continues with v1; Tuesday's next
run captures v2. Neither inherits from the other.

### 2. Concurrent completion

The supported scheduler never executes two runs of the same Automation
Definition concurrently; the second recurring occurrence is skipped for
overlap. Different automations may finish concurrently and become independently
unread. Defensive storage never allows one session to overwrite another.

### 3. Mail access denied

Mail is denied while validated web work succeeds. The timeline records
`Mail · denied · 0 items`, the Approval Record is redacted, and no Mail identity
or content persists. The run ends partial with a useful safe answer and explicit
shortfall. Without a safe answer it ends failed.

### 4. Approval crosses daemon restart

A write approval is pending and its deadline lies after a daemon restart.
Restart marks the approval and run abandoned. The write never resumes. The
historical deadline remains provenance only.

### 5. Long Final Output

A completed run produces 100 KiB of safe output. The session retains 64 KiB
with an omission marker and records original and retained sizes. Reopen, export,
and continuation disclose truncation; continuation separately applies its
16 KiB cap.

### 6. Automation deleted before viewing

The Automation Definition is deleted after an unread completion. Scheduling
stops, but the detached Automation Session remains unread and reopenable under
normal retention.

### 7. Continue, then `/clear`

The user continues an old result, asks follow-ups, then invokes `/clear`.
Current Chat turns, seed, provenance, and chat-scoped records disappear. The
source Automation Session and Saved Long-Term Memory remain unchanged.

### 8. Continued chat invokes a write

Historical approval grants no authority. The connector is revalidated and the
exact new action requires Fresh Approval. New activity remains in Current Chat
and never mutates the source or a later recurring run.

### 9. Web sources and connector references

Two fetched admitted web sources persist as sanitized Validated Sources. A
connector-backed record persists only as an opaque Connector Reference with
category and availability. Session Export includes web sources and a redacted
connector description; Continuation Seed excludes the opaque token.

### 10. Failure produces no safe answer

The run ends failed with no Final Output. Result Summary is deterministic and
normalized. The session retains safe provenance and activity, becomes unread,
and may continue with a seed that explicitly states no safe prior answer
exists.

## Product policy versus later implementation

This document decides:

- domain boundaries and one-to-one ownership;
- immutable and mutable data;
- allowed durable content;
- outcome and attention semantics;
- retention, deletion, restart, privacy, and truncation rules;
- continuation and `/clear` behavior;
- acceptance outcomes.

This document does not decide:

- SQLite table layout, migration version, indexes, or transaction strategy;
- Rust repository types or Swift wire structs;
- exact daemon endpoint and SSE envelope names;
- UI layout, animations, or placement of result, delete, export, confirmation,
  and Continue controls;
- unified runtime queue, cancellation, model-residency, or event state graph;
- implementation sequencing or acceptance harnesses.

No ADR is required. The policy is the ticket's expected durable decision asset,
is directly visible in the domain glossary, and does not choose a surprising
hard-to-reverse implementation mechanism.

## Questions exposed for existing tickets

### Prototype automation activity, result, and continuation flows

[Prototype automation activity, result, and continuation flows](https://github.com/brunovskyoliver/bagent/issues/21)
can now test these precise interaction questions:

1. How does the notch distinguish Result Summary from Final Output and expose
   Truncation Disclosure without turning Activity Peek into a transcript?
2. How are multiple independently unread completions ordered, opened, and
   acknowledged while focus cycling remains non-acknowledging?
3. How does Continue confirm replacement of a non-empty Current Chat and make
   Continuation Provenance visible?
4. How do failed sessions with no Final Output present a useful Continue action?
5. How are detached sessions, skipped history, Session Export, and Session
   Deletion reached without adding another surface?

### Design the unified background-work state machine and event contract

[Design the unified background-work state machine and event contract](https://github.com/brunovskyoliver/bagent/issues/23)
must settle:

1. Which authoritative states and events atomically create the Automation Run,
   Task Snapshot, Run Provenance, terminal Automation Session, and Completion
   Attention?
2. How are cancellation and restart abandonment distinguished from failure,
   and when is safe terminalization still possible?
3. Which allowlisted event schema sanitizes activity, model route, sources,
   approvals, and normalized failures before both persistence and broadcast?
4. Which ownership boundary removes the current recurring-run runtime-reference
   sharing and revalidates opaque Connector References?
5. How are stable concurrent-completion ordering, acknowledgement, pruning, and
   explicit deletion made race-safe?
6. Which Current Chat persistence and API boundaries assemble the 16 KiB
   Continuation Seed, preserve one-way provenance, and implement `/clear`
   without an ambiguous `/sessions` contract?

These are refinements of the existing prototype and unified-state-machine
tickets. This decision does not justify a new ticket, dependency edge, ADR, or
fog graduation.
