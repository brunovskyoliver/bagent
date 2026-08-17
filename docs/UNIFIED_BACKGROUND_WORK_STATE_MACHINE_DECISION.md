# Unified background-work state machine and event contract

Decision ticket:
[Design the unified background-work state machine and event contract](https://github.com/brunovskyoliver/bagent/issues/23)

Map:
[Wayfinder: Rebuild the bagent notch UX around model residency and automation sessions](https://github.com/brunovskyoliver/bagent/issues/15)

Accepted through live HITL grilling on 2026-08-17.

## Decision

bagent will use one daemon-owned Work Coordinator as the sole authority for
foreground Conversation Turns and admitted Automation Runs. It owns Work
identity and origin, lifecycle, queue admission, Execution Slots, Model
Residency Leases, approval waiting, cancellation, persistence, commands,
snapshots, and ordered events.

Swift is a client of that authority. One deep Notch Projection module reduces
authoritative snapshots and events plus local user intent into the sole stored
`NotchInteractionMode`, Stage Rail, the invariant status pill, thinking/tool
activity, and Dvojpanelový prehľad. Swift never creates a competing Work
lifecycle or infers authoritative state from transport, `isThinking`, model
discovery, or view visibility.

The contract unifies execution without merging the domain identities of a
Conversation Turn, Current Chat, Automation Run, Automation Session, or
Automation Definition. Scheduler-only skipped occurrences retain their
Automation Run and terminal Automation Session history but create no Work,
because execution was never admitted.

This is an architecture and product-contract decision. It specifies the
implementation destination, not production code, migration sequencing, or
acceptance execution.

## Normative inputs

This decision implements, and does not reopen, the accepted contracts in:

- [BaseRT residency transitions under chat and automation load](BASERT_RESIDENCY_TRANSITIONS_RESEARCH.md)
- [Activity Peek Stage Rail decision](ACTIVITY_PEEK_STAGE_RAIL_DECISION.md)
- [Settings and Permission Grant Assist decision](SETTINGS_PERMISSION_GRANT_ASSIST_DECISION.md)
- [Automation Session storage, retention, and chat continuation](AUTOMATION_SESSION_POLICY_DECISION.md)
- [Automation activity, result, and continuation interaction decision](AUTOMATION_ACTIVITY_RESULT_CONTINUATION_DECISION.md)
- [Slash commands, Current Chat clearing, restoration, and UI-only relaunch](SLASH_CURRENT_CHAT_RELAUNCH_DECISION.md)
- [bagent UI Design Reference](UI_DESIGN.md)
- the canonical language in [CONTEXT.md](../CONTEXT.md)

In particular, the one-panel rule, Stage Rail, invariant status-pill anchor,
approval preemption, Dvojpanelový prehľad, immutable terminal Automation
Sessions, Current Chat continuation, UI-only relaunch fencing, privacy
allowlists, and changed-PID BaseRT safety boundaries remain normative.

## Authority and deep-module seams

### Work Coordinator

The Work Coordinator is a deep Rust module. Its external interface is the only
seam through which a caller may create or mutate Work:

```text
submit(command) -> Command Acknowledgement
snapshot() -> Work Snapshot
events(after Event Cursor) -> ordered authoritative events or gap
projection(identity, expected revision?) -> authorized content projection
```

The interface includes its invariants, legal transitions, idempotency,
ordering, failure modes, and privacy contract. Queue implementation, storage
transactions, model-runtime coordination, approval timers, cancellation
drivers, and outbox publication remain inside the module. Scheduler, chat
handlers, approval handlers, and recovery code call this interface rather than
writing lifecycle rows or broadcasting ad-hoc JSON directly.

The module may use internal seams for deterministic tests:

- a clock and identity source;
- a transactional persistence adapter;
- a Model Runtime adapter;
- tool and side-effect adapters;
- an event publisher awakened by the durable outbox.

Those are internal seams, not additional product authorities.

### Model Runtime coordinator

The daemon-owned Model Runtime coordinator is the only implementation allowed
to load, ready, lease, retire, poison, or restart bagent's BaseRT process on
port 8082. The Work Coordinator expresses typed model demand through its
interface. Swift, schedulers, chat handlers, tests, and speculative preload
callers never contact BaseRT lifecycle operations directly.

Port 8080 remains unrelated and outside this contract.

### Swift Notch Projection

The Notch Projection is a deep Swift module with a pure reducer interface:

```text
reduce(previous presentation, authoritative snapshot/event, local user intent)
    -> Notch presentation
```

Its output contains the sole stored `NotchInteractionMode` and the complete
derived presentation. Compatibility accessors such as `isThinking` may exist
only as computed views during migration; they cannot be writable authority.
The reducer interface is also the Swift test surface.

## Work identity and immutable origin

The daemon creates an opaque Work Identity atomically with accepted work. It
is never derived from a session identifier, request identifier, UI process,
model request, or scheduler occurrence.

Every Work has exactly one immutable origin:

```text
conversation {
  current_chat_identity,
  conversation_turn_identity
}

automation {
  automation_run_identity,
  automation_session_identity,
  historical_automation_identity,
  frozen_definition_revision
}
```

The identities remain distinct. Work does not replace any origin identity and
an origin cannot be changed after creation.

Submitting Current Chat input atomically creates the Conversation Turn, its
Work, the immutable user message, initial Work Revision, and initial outbox
event. Claiming executable automation atomically creates the Automation Run,
Automation Session shell and Task Snapshot, Work and origin, initial revision,
and event.

A scheduler-only skipped occurrence creates a terminal Automation Run and
Automation Session with its normalized skipped reason, but no Work Identity.
It never appears in an execution queue, consumes no Execution Slot, and never
offers Continue.

## Closed Work lifecycle

### States

The closed authoritative Work State is:

```text
queued
waiting_for_model
running
waiting_for_approval
cancelling
terminal(completed | partial | failed | cancelled | abandoned)
```

Current activity, queue position, Execution Slot, Model Residency Lease, and
projection availability are typed facts within the applicable state. They are
not independent lifecycle booleans.

### Legal transitions

| From | To | Cause and invariant |
|---|---|---|
| creation | `queued` | Work and origin commit atomically |
| `queued` | `waiting_for_model` | daemon admits Work and its origin-specific Execution Slot |
| `queued` | `cancelling` | Cancellation Intent wins before dispatch |
| `waiting_for_model` | `running` | required model is ready and the generation receives its lease |
| `waiting_for_model` | `cancelling` | cancellation wins before model dispatch |
| `running` | `waiting_for_model` | the next model generation must wait for dispatch or a different residency |
| `running` | `waiting_for_approval` | one approval request commits after the preceding generation reaches a safe end |
| `waiting_for_approval` | `running` or `waiting_for_model` | allow, deny, or expiry resolves and execution resumes or queues its next generation |
| `waiting_for_approval` | `cancelling` | cancellation withdraws the approval |
| any nonterminal | `cancelling` | monotonic Cancellation Intent is recorded |
| `running` | terminal | authoritative outcome commits |
| `cancelling` | `cancelled` | a safe point proves no further execution can begin |
| any prior-generation nonterminal | `abandoned` | daemon startup recovery owns the transition |
| any nonterminal | `failed` | an unrecoverable known failure ends execution safely |

`completed` requires the requested work to be satisfied. `partial` requires a
safe output that identifies unmet required work. `failed` has no safe output.
`cancelled` requires proven terminal cancellation. `abandoned` means runtime
ownership became indeterminate across a daemon generation boundary.

Terminal Work is immutable. It cannot resume, requeue, receive another
Cancellation Intent, acquire a slot or lease, or change terminal outcome.
Mutable Completion Attention remains a separate Automation Session concern.

Every authoritative Work mutation increments Work Revision exactly once. A
transaction either commits the new revision and its event together or exposes
neither.

## Queue admission, priority, and fairness

The daemon enforces three origin-specific capacities:

- at most one admitted Conversation Turn;
- at most two admitted Automation Runs, from different Automation
  Definitions under the existing no-overlap rule;
- speculative preload owns no Work and no Execution Slot.

An admitted Work holds an Execution Slot through model/tool activity and
approval waiting. Queue order is stable:

- Automation Runs are FIFO by claim time, then Work Identity;
- a foreground Conversation Turn may jump ahead at the next safe dispatch
  boundary;
- after one foreground dispatch while any Automation Run waits, the next
  eligible dispatch must be the oldest waiting Automation Run;
- speculative preload is eligible only when no executable Work is active or
  queued.

Already-running model or tool activity is never preempted. Foreground priority
therefore means first eligibility at the next safe boundary, not interruption.
The bounded foreground jump prevents continuous chat from starving automation.

Queue-wait and execution deadlines are distinct. An execution deadline starts
only when its operation is dispatched; time spent queued cannot consume it.
Queue deadlines, where defined, produce their own normalized outcome and never
masquerade as a model timeout.

## Model Runtime state and residency

### Closed Model Runtime state

The daemon-owned state graph is:

```text
unavailable
unloaded
loading(model)
loaded_not_ready(model)
ready(model)
retiring(model)
poisoned(model)
restarting
```

Discovery, process health, registry membership, `loaded=true`, RSS, and free
memory are observations, not residency authority. Only `ready(model)` may
grant a Model Residency Lease.

Same-model demand joins an in-flight load. Different-model demand remains
queued until the transition settles and every active lease drains. Only one
lifecycle transition is in flight.

### Model Residency Lease

Each model generation acquires a non-preemptible lease bound to:

- Work Identity;
- model class;
- Model Runtime Generation.

The lease ends only when the coordinator observes BaseRT's terminal completion
or verifies a healthy changed-PID boundary with zero loaded weights. Client
disconnect, cancellation, or timeout cannot release it. Cancellation prevents
new dispatch but must await definitive completion or changed-PID recovery.

No retirement, model switch, or lifecycle mutation may reach BaseRT while a
lease is active. A critical-pressure signal records retirement intent and acts
only after leases drain.

### Admission, retirement, and poison

A cold 35B load requires both at least 25% free memory and at least 8 GiB
estimated available. The gate is not reapplied to a healthy warm residency.

An unused speculative 4B preload may remain opportunistically warm under the
shared idle timeout. Executable different-model demand or memory pressure
requests retirement at the next idle boundary.

A verified 4B API unload may provide ordinary same-PID idle retirement when
measured headroom returns. That process must still cross a changed-PID boundary
before later 35B admission.

The following poison the current Model Runtime Generation:

- indeterminate load, unload, readiness, or completion timeout;
- client cancellation with indeterminate server completion;
- Metal, device, or command-buffer failure;
- failed lifecycle proof.

`poisoned` admits no work and transitions only through `restarting`. A valid
restart requires a changed PID, health, zero loaded weights, and a new Model
Runtime Generation. Changed PID is also mandatory after 35B retirement and
before every new 35B residency.

Daemon Generation and Model Runtime Generation are independent. UI-only
relaunch changes neither.

### No backup-model synthesis

Ordinary Conversation Turns may use the configured 4B chat model. Canonical
evidence synthesis uses 35B only when admitted. If 35B is inadmissible,
unavailable, timed out, or faulted, the same validated Evidence Bundle goes
directly to Deterministic Rendering. The unused 35B-to-4B backup-model path is
not part of the production contract.

Deterministic Rendering may still yield `completed` or `partial` when it safely
satisfies the evidence contract. Run Provenance records
`deterministic_renderer` as the actual route. A 35B fault still requires poison
recovery before later model work.

## Approval waiting and preemption

Creating an approval and entering `waiting_for_approval` is one transaction.
The Work retains its Execution Slot, limiting simultaneous unattended approval
load, but holds no Model Residency Lease after the preceding generation has
definitively ended.

Approval lifetime remains 60 seconds. Multiple approvals are presented by:

1. earliest expiry;
2. request time;
3. stable approval identity.

The pending approval surface preempts every Swift presentation. This is
presentation preemption, not runtime preemption: already-running model or tool
activity continues to its safe boundary. When an approval resolves, the Work
resumes at the next safe dispatch boundary and Swift recomputes the prior valid
destination.

Approval outcomes remain distinct:

- `allowed` — user allowed the request;
- `denied` — user denied it;
- `expired` — its deadline won;
- `withdrawn` — Work cancellation invalidated it;
- `abandoned` — daemon restart destroyed runtime ownership.

Historical outcomes grant no authority. Every later gated action requires
Fresh Approval.

## Cancellation

Cancellation is a monotonic two-step contract.

First, the daemon records Cancellation Intent, increments Work Revision,
enters `cancelling`, commits its event, and returns a Command Acknowledgement.
That acknowledgement proves only that the request was recognized.

Second, Work becomes terminal `cancelled` only after a safe point proves no
further model or tool execution can begin:

- queued or pre-dispatch model demand cancels immediately and releases its
  demand and slot;
- approval waiting atomically withdraws its pending approval and cancels;
- active model generation stops future dispatch, then awaits definitive
  completion or crosses a changed-PID boundary if indeterminate;
- a read-only tool must return or prove subprocess termination;
- an approved side effect already dispatched is never interrupted mid-effect;
  its authoritative outcome is recorded before cancellation terminalizes.

External effects already performed are not rolled back or hidden. If the
external outcome is indeterminate, the normalized record states that
uncertainty; terminal cancellation still requires proof that bagent will issue
no further execution.

Client disconnect, UI dismissal, transport timeout, and loss of visibility do
not create Cancellation Intent. Swift displays `cancelling` until an event or
snapshot proves `cancelled`. Partial assistant deltas never become Final Output.

If daemon restart wins before a cancellation safe point, startup recovery
commits `abandoned`, not `cancelled`.

## Persistence and atomicity

The physical SQLite migration sequence belongs to the downstream sequencing
ticket, but the persistence implementation must provide these logical records
and constraints:

- Work and immutable origin;
- current Work State and monotonically increasing Work Revision;
- resource/activity facts owned by that revision;
- durable authoritative event outbox;
- durable command-result ledger;
- Automation Run, Automation Session, attention, and approval records;
- Current Chat, Conversation Turn, content projection, continuation, and
  interruption records;
- daemon and Model Runtime generation metadata.

Foreign keys and uniqueness must enforce one origin per Work, no Work for a
skipped occurrence, at most one active run per Automation Definition, one
Automation Session per Automation Run, and one event per committed Work
revision.

Allowlisted Automation Session activity may accumulate while Work runs.
Terminalization atomically:

1. freezes the Automation Session;
2. commits Run Outcome and Final Output or its explicit absence;
3. moves Work to its terminal state;
4. creates Completion Attention when policy requires it;
5. writes the terminal outbox event.

After terminalization, Automation Session content is immutable. Attention,
retention, source availability, and deletion are separate records and commands.

Current Chat assistant deltas are staged and ephemeral. Only committed output
or a normalized interruption/failure marker becomes durable Current Chat
content. A failed terminalization exposes no partially frozen session. Startup
recovery deterministically marks unfinished prior-generation Work abandoned.

Continue is an idempotent transaction that marks the source viewed, replaces
Current Chat when authorized, copies its bounded visible Continuation Seed,
and creates one-way Continuation Provenance. It creates no Work; the next
submitted Conversation Turn does. `/clear`, Continue, acknowledgement,
retention, and deletion have separate command identities and transactions.

## Authoritative event envelope

Each authoritative event is committed to the durable outbox in the same
transaction as the mutation:

```json
{
  "schema_version": 1,
  "event_cursor": "opaque-durable-position",
  "daemon_generation": "opaque-process-generation",
  "committed_at": "RFC3339 UTC",
  "event_kind": "closed_typed_kind",
  "work_identity": "optional-opaque-identity",
  "work_revision": "optional-monotonic-revision",
  "payload": {}
}
```

`schema_version` is an integer major version. Additive optional fields remain
compatible. Removing a field, making one required, or changing meaning
requires a new major version. Incompatible clients are rejected explicitly;
Rust never emits a partially understood hybrid.

Event Cursor is globally monotonic, allocated transactionally, survives daemon
restart, and is also the SSE event ID. `(schema_version, event_cursor)` is the
stable event identity. Delivery is at least once.

Daemon Generation changes for every daemon process lifetime without resetting
Event Cursor or Work identity. Work Revision appears only for events mutating
one Work. Aggregate model, attention, or availability events still have an
Event Cursor but may omit Work identity and revision.

## Event Allowlist and authorized projections

The event stream is structural and content-free. Its closed allowlist permits:

- opaque identities and immutable origin classification;
- Work lifecycle, revision, queue position, priority, and aggregate counts;
- model class, residency phase, lease count, and Model Runtime Generation;
- allowlisted activity category and normalized outcome;
- approval identity, category, deadline, and resolution state;
- Completion Attention and projection-availability changes;
- normalized failure codes and bounded timestamps.

Events never contain user text, draft, task text, assistant output, Result
Summary, prompts, reasoning, evidence, source URLs, private identities,
Connector References, raw arguments or results, provider errors, credentials,
signed URLs, write payloads, or approval payloads.

Content-bearing mutations announce only identity and projection revision.
Authenticated, purpose-specific projection interfaces provide:

- Current Chat content;
- Automation Session content;
- redacted approval presentation;
- authorized Validated Source and Connector Reference availability;
- live assistant deltas through an ephemeral Work Output Projection stream.

Live deltas are presentation hints, not durable events or Work authority. After
disconnect or restart, Swift refetches committed content. It never rebuilds
truth from partial deltas.

All event kinds and payloads are typed Rust values. There is no arbitrary JSON
publisher escape hatch. Unknown or non-allowlisted fields are rejected before
the outbox write, not removed after broadcast.

## Snapshot, resume, gaps, and restart

### Work Snapshot

A Work Snapshot is one privacy-safe, transactionally consistent view through a
single Event Cursor. It contains:

- schema version and Daemon Generation;
- structural nonterminal and relevant terminal Work projections;
- Work Revisions and immutable origins;
- queue and Execution Slot facts;
- pending approval projections;
- Completion Attention and projection revisions;
- Model Runtime state, generation, demand, and lease facts;
- the active UI Event Consumer fence where authorized.

It contains no forbidden event or content-projection data.

### Initial attach and resume

Swift fetches a snapshot, atomically installs it, and subscribes strictly after
the returned Event Cursor. The durable outbox closes the snapshot/subscription
race.

On reconnect, Swift first requests events after its last applied cursor.
Duplicate cursors and non-increasing Work Revisions are ignored. Any of these
forces event application to stop and a fresh snapshot to replace authority:

- missing Event Cursor;
- Work Revision jump;
- incompatible schema;
- cursor outside the bounded retention window;
- changed Daemon Generation.

A changed Daemon Generation always causes a snapshot even when cursors remain
continuous, because volatile runtime ownership changed. Cursor expiry is a
normal snapshot fallback, not an inferred failure.

### Daemon restart

Before serving clients, startup recovery atomically:

- changes Daemon Generation;
- marks every prior-generation nonterminal Work `abandoned`;
- marks pending approvals `abandoned`;
- records a normalized interruption marker for an affected Conversation Turn;
- commits recovery events;
- withdraws trust in the prior Model Runtime Generation.

No queued, running, approval-waiting, or cancelling Work resumes automatically.
New model work requires a verified healthy changed-PID boundary because prior
BaseRT execution is indeterminate.

### UI-only relaunch

UI-only relaunch performs no Work or runtime transition. The successor:

1. validates and consumes the bounded handoff;
2. refetches a Work Snapshot;
3. resumes after its Event Cursor;
4. acquires the successor UI-consumer fence;
5. becomes visible only after the existing handoff acknowledgement protocol.

Daemon and Model Runtime generations, Work, slots, leases, and Automation Runs
remain unchanged. On ordinary UI reopen, presentation starts collapsed unless
approval preemption applies. A valid handoff may restore only its allowlisted
destination and editing state.

## Swift projection into the notch

The pure Notch Projection reducer consumes only accepted snapshots/events and
local user intent.

### Presentation priority

1. Pending approval overrides rendering and owns command/navigation input.
2. Active foreground Work controls foreground Model, Think, Tool, cancelling,
   and output presentation.
3. Background Work projects into Activity Peek without taking focus.
4. With no active work, terminal attention projects Done using failed,
   partial, then unread priority.
5. With no active or unread work, the bridge collapses.

Approval resolution recomputes the previous destination if still valid.
Neither passive events nor focus cycling activate bagent or move VoiceOver
focus.

### Stage Rail

| Authoritative fact | Stage Rail projection |
|---|---|
| `waiting_for_model` or model lifecycle transition | Model |
| running model generation | Think |
| allowlisted current tool activity | Tool |
| terminal attention | Done |

Unknown activity maps to the generic tool category. The rail never displays
hidden reasoning, evidence, raw tool data, or content projection fields.

### Invariant status pill

The accepted fixed anchor and geometry remain unchanged. Its deterministic
label priority is:

1. `APPROVE` for pending approval;
2. `ACTIVE` or `N ACTIVE` while Work is active;
3. `LOADING` for a model transition without active executable Work;
4. `FAILED`, then `PARTIAL`, then `UNREAD` for terminal attention;
5. `RESIDENT` for idle ready weights;
6. hidden for fully idle/unloaded state.

The active Automation Run count remains visible while foreground Work is
focused.

### Dvojpanelový prehľad

The reducer derives active-first rows, unread rows, history, counts, and stable
selection from the same snapshot:

- active Automation Runs remain FIFO by claim time and Work Identity;
- unread terminal Automation Sessions remain newest-finished first with stable
  identity tie-breaker;
- selecting or cycling never acknowledges Completion Attention;
- opening a terminal Automation Session issues the revisioned acknowledgement
  command;
- a refetch preserves selection only when the identity and destination remain
  valid, otherwise it moves deterministically to the next accepted row or
  History.

`NotchInteractionMode` is one Swift projection, not a Rust lifecycle. Local
intent owns input, Settings, Automations navigation, focus, and confirmation;
it cannot mutate Work without an acknowledged command.

## Command contract and concurrency

Every mutation uses a typed command envelope:

```json
{
  "command_schema_version": 1,
  "command_identity": "client-generated-opaque-identity",
  "ui_consumer_fence": "active-fence-or-daemon-actor",
  "target_identity": "opaque-target",
  "expected_revision": 42,
  "command_kind": "closed_typed-kind",
  "payload": {}
}
```

The daemon writes a privacy-safe Command Acknowledgement transactionally with
the mutation and resulting Event Cursor. Repeating the same Command Identity
and identical payload returns the original acknowledgement. Reusing an
identity with different content is rejected.

State-changing commands compare-and-swap the applicable Work, Current Chat,
Automation Session attention, approval, or Automation Definition revision. A
stale target revision or stale UI-consumer fence is rejected with current
structural revision information. The daemon never redirects a stale command to
newer state.

Acknowledgement outcomes are closed and distinguish at least:

- committed;
- already committed;
- stale revision;
- stale consumer fence;
- terminal target;
- unavailable/deleted target;
- already resolved;
- conflict.

Cancellation acknowledgement proves Cancellation Intent, not terminal
cancellation.

Events may arrive before the HTTP acknowledgement. Shared cursor and revision
identities make either order safe. A lost response may be retried after daemon
restart with the same Command Identity.

The persistence transaction selects one winner for:

- cancellation versus terminalization;
- approval versus expiry or withdrawal;
- Continue versus deletion or retention;
- attention acknowledgement versus deletion;
- UI takeover versus a stale old-UI command.

The loser receives the corresponding deterministic acknowledgement and causes
no side effect. Scheduler mutations use the same interface with a daemon-owned
actor identity rather than a UI fence.

The command-result ledger retains structural acknowledgements for 90 days,
never prunes nonterminal command ownership, and stores no raw content,
credentials, private identities, or forbidden event data.

## Deterministic acceptance contract

Implementation must be verified through the Work Coordinator interface using
a fake clock, deterministic identities, isolated SQLite database, in-memory
Model Runtime adapter, and controlled tool and side-effect adapters. Tests do
not use the production database, live port 8082, TCC state, Keychain, or app
processes.

### Required suites

1. Every legal and illegal Work transition, terminal immutability, and Work
   Revision monotonicity.
2. One-foreground/two-automation admission, FIFO ordering, bounded foreground
   jumps, starvation prevention, and speculative preload priority.
3. Lease exclusion, retirement deferral, poison handling, and mandatory
   changed-PID proof.
4. Approval ordering, retained slots, allow/deny/expire/withdraw/abandon
   outcomes, and presentation preemption.
5. Cancellation at every state and safe point, including indeterminate model
   execution and already-dispatched side effects.
6. Transactional creation and terminalization of Work, Automation Run,
   Automation Session, Current Chat, attention, approval, and outbox records.
7. Event versioning, cursor continuity, duplicate delivery, stale revisions,
   gaps, resume expiry, and changed Daemon Generation.
8. Command duplication, lost responses, stale revisions/fences, and every
   specified concurrency race.
9. Pure Swift reducer coverage for Stage Rail, status pill, thinking/tool
   activity, approval restoration, Dvojpanelový prehľad, reconnecting, and
   UI-only relaunch.
10. Schema-level privacy coverage proving forbidden content cannot serialize
    into events, snapshots, acknowledgements, or Diagnostic Export.

### Required failure injection

- SQLite busy, full, and I/O failure before every transactional commit point;
- daemon crash before commit, after commit before publish, and during every
  nonterminal Work State;
- SSE disconnect, lag, duplication, event-before-ack, and snapshot/event race;
- BaseRT load, readiness, completion, and unload timeout;
- unchanged PID, false health, residual 35B memory, Metal/device/command-buffer
  fault, and restart failure;
- tool cancellation, subprocess termination failure, delayed side-effect
  completion, and indeterminate external outcome;
- simultaneous cancellation/terminalization, approval/expiry/withdrawal,
  Continue/deletion/retention, and acknowledgement/deletion;
- old and replacement UI crash at every handoff-fence step.

Tests assert observable results through module interfaces rather than internal
state. Test execution must be nonzero and reported by exact suite/count; an
exact filter that runs zero tests is invalid evidence.

Deterministic tests, signed-build proof, visual/accessibility validation, and
live daemon/BaseRT/Automation proof remain separate claims. The downstream
[Lock implementation sequencing and acceptance gates](https://github.com/brunovskyoliver/bagent/issues/24)
ticket owns their staged order and release gates.

## Race outcomes

The following outcomes are normative examples:

| Race | Required winner semantics |
|---|---|
| terminal commit before cancellation | cancellation returns terminal target; outcome is unchanged |
| Cancellation Intent before terminal commit | execution observes intent and reaches a cancellation safe point |
| approval decision before expiry | decision commits; expiry is already resolved |
| expiry before allow | allow is already resolved and cannot authorize the side effect |
| cancellation before approval decision | approval becomes withdrawn; decision is already resolved |
| daemon restart during cancellation | prior Work becomes abandoned, never falsely cancelled |
| event before command response | Swift applies event and recognizes the later acknowledgement by cursor/revision |
| duplicate event | cursor/revision suppression produces no projection change |
| outbox commit before daemon crash | restarted publisher delivers the durable event at least once |
| Continue before source deletion | copied seed remains; later source state becomes deleted |
| deletion before Continue | Continue returns unavailable and creates no Current Chat |
| old UI command after successor fence | stale consumer fence; no mutation |

## Privacy and diagnostics

Persistence Allowlist, Event Allowlist, content-projection allowlists, Session
Export, and Diagnostic Export are distinct contracts. Passing one allowlist
does not admit data to another.

Logs and diagnostics may retain structural identities only as bounded opaque
or non-reversible values. They never include task text, Current Chat content,
Final Output, private identities, raw events, raw commands, model prompts,
evidence, credentials, connector tokens, or write payloads.

Unknown event or projection fields fail closed. Sanitization occurs before the
first durable write and before broadcast. Post-hoc redaction is not an
authorization mechanism.

## Current implementation gap

The current tree has multiple shallow authorities that this contract replaces:

- foreground chat uses in-memory Swift `isThinking` and an attached response
  SSE stream;
- Automation Runs use a separate two-permit semaphore and a small lifecycle
  enum;
- the daemon-wide `/events` channel is an ephemeral broadcast that silently
  skips lagged events and has no cursor, generation, revision, or resume;
- ad-hoc JSON publishers bypass a closed Event Allowlist;
- `BaseRtClient` protects lifecycle only within one shared connector instance,
  while the synthesis manager separately owns 35B leases;
- pending approval, startup abandonment, Current Chat persistence, and UI
  event-consumer fencing are not one transactional Work contract;
- Swift stores writable parallel presentation flags alongside
  `NotchInteractionMode`.

These are migration facts, not authorization to modify production code in this
ticket.

## Trade-offs

The decision deliberately accepts:

- a transactional outbox and command-result ledger;
- revision-aware Swift command handling and snapshot fallback;
- bounded Automation throughput while approvals occupy slots;
- restart latency after ambiguous BaseRT state;
- abandonment rather than silent resumption across daemon restart;
- additional content-projection fetches;
- a substantial deterministic failure-injection harness;
- deterministic rendering instead of a lower-quality backup-model synthesis.

In return, one deep interface localizes lifecycle knowledge; UI relaunch cannot
duplicate work; events are resumable and privacy-safe; cancellation never lies;
approval authority cannot revive; BaseRT lifecycle cannot race active
generation; and every concurrency result is deterministic.

## Scope and map impact

No production code, dependency, BaseRT/TCC state, application process,
acceptance infrastructure, or other ticket is changed by this decision. No ADR
is required: the durable decision asset directly records the accepted product
and architecture trade-offs.

This resolution clears the architecture fog already represented by
[Lock implementation sequencing and acceptance gates](https://github.com/brunovskyoliver/bagent/issues/24).
It creates no new ticket or dependency. That ticket becomes the next unblocked
map work but is not claimed or begun here.
