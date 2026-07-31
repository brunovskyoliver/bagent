# Automation activity, result, and continuation interaction decision

Decision ticket:
[Prototype automation activity, result, and continuation flows](https://github.com/brunovskyoliver/bagent/issues/21)

Map:
[Rebuild the bagent notch UX around model residency and automation sessions](https://github.com/brunovskyoliver/bagent/issues/15)

Selected through live prototype feedback on 2026-07-31.

## Decision

Use **Dvojpanelový prehľad** for Automation activity, results, history, and
continuation.

The selected structure is a persistent master/detail presentation inside the
existing notch bridge:

```text
┌──────────────────────┬────────────────────────────────────────────┐
│ Active and new       │ Focused Automation Session                │
│                      │                                            │
│ selected row         │ Stage Rail / Result Summary / primary      │
│ History              │ action / context commands                 │
└──────────────────────┴────────────────────────────────────────────┘
```

The left master column preserves the user's place across active work, unread
completions, and history. The right detail column shows the focused run without
turning every concurrent run into a permanently expanded row. Full-width child
destinations temporarily replace the split when their content needs the whole
bridge, then return to the same selected master row.

The user selected Dvojpanelový prehľad directly after comparing it with
Fronta relácií and Výsledková os. No additional verbal rationale was given.
The product rationale established by the selected structure is persistent
context, fast switching between overlapping work and history, and a stable
place for session-specific actions without adding another window.

This is an interaction and information-hierarchy decision. It does not
implement Automation Session persistence, the unified background-work state
machine, daemon APIs, scheduler changes, model residency, or production
SwiftUI.

## Normative policy boundary

Every requirement in
[Automation Session storage, retention, and chat continuation](AUTOMATION_SESSION_POLICY_DECISION.md)
remains normative. This interaction must not weaken or reinterpret that policy.
In particular:

- each Automation Run owns one isolated Automation Session;
- terminal Automation Session content is immutable;
- Completion Attention is mutable and separate from session content;
- Result Summary and Final Output are distinct products;
- scheduler-only skipped runs never offer continuation;
- Continue creates a separate Current Chat from a bounded visible
  Continuation Seed;
- historical approvals never authorize Current Chat work;
- Session Export and Diagnostic Export are different products;
- deletion, retention expiry, and `/clear` retain their separate meanings;
- detached Automation Sessions survive deletion of their Automation
  Definition;
- all persistence allowlists, truncation limits, and privacy prohibitions
  remain unchanged.

The term `session` must not appear unqualified in new product copy, APIs, or
implementation contracts. Use **Automation Session** or **Current Chat**.

## Entry points and Stage Rail integration

The selected flow consumes the already-decided
[Activity Peek Stage Rail](ACTIVITY_PEEK_STAGE_RAIL_DECISION.md). It does not
replace or fork it.

### Activity Peek bridge

- With exactly one active Automation Run, clicking the bridge opens that
  active Automation Session.
- With several active Automation Runs, clicking the bridge cycles focus in
  stable FIFO order and displays the focused position, for example `1 z 2`.
- Return performs the same action when Activity Peek already owns keyboard
  focus.
- Focus cycling is navigation only. It never marks a terminal completion
  viewed.
- Passive focus changes and new events never activate bagent or steal focus
  from the frontmost application.

### Invariant status pill

- `ACTIVE` or `N ACTIVE` wins while Automation Runs are active.
- Clicking the pill opens the existing Automations surface with active runs
  first and the oldest active run selected.
- With no active work, `FAILED`, `PARTIAL`, or `UNREAD` follows the Stage Rail
  terminal-attention priority.
- The pill is a direct entry point, not a menu, popover, chooser, or new
  surface.

### Other entry points

- `/automations` opens the active-first master/detail surface.
- Existing Automation Definition detail and run-history entry points open the
  same surface with the relevant definition or Automation Session selected.
- Opening a terminal Automation Session from any entry point marks its
  Completion Attention viewed.
- Merely selecting or previewing a row does not mark it viewed.

### Approval preemption

A pending live approval replaces Stage Rail and every Automation surface with
the existing approval surface. It is the only place a gated write can be
allowed. When the approval resolves, the panel recomputes its presentation
from authoritative state and restores the prior selection when it still
exists.

## Active-first master column

The master column has two bounded groups.

### Active and new

Order rows as follows:

1. active Automation Runs in stable FIFO claim order, with stable Automation
   Run identity as the tie-breaker;
2. unread terminal Automation Sessions by `finished_at` newest first, with
   stable Automation Run identity as the tie-breaker.

Different Automation Definitions may contribute active rows. A detached unread
Automation Session participates normally and carries an explicit `Odpojená`
label. Scheduler-only skipped runs do not appear as unread attention.

The visible master is a paged viewport, not a `ScrollView`. It shows at most
four selectable rows at notch scale. Up and Down move through the complete
ordered backing collection and advance the viewport when selection leaves the
visible page. The active count and unread count remain available to
accessibility even when some rows are off-page.

Unread and viewed must differ by more than color:

- unread rows use stronger type, an explicit accessibility value, and a small
  attention marker;
- viewed rows use normal type and no attention marker;
- active rows use the running glyph and `Beží`;
- outcome glyphs and text distinguish completed, partial, failed, cancelled,
  abandoned, and skipped.

### History

`História` is a stable final master row. It opens a bounded historical run list
for the selected historical Automation Definition identity. The list includes
viewed and unread terminal Automation Sessions, detached history, and
scheduler-only skipped runs.

History is ordered by `scheduled_for` newest first, with stable Automation Run
identity as the tie-breaker. It uses the same paged keyboard viewport rather
than nested scrolling.

Deleting an Automation Definition removes future scheduling but does not remove
or relabel its surviving Automation Sessions as errors. They become detached
history and retain their Task Snapshot name.

## Right detail column

The right column is always about the selected master row.

### Active Automation Session

Show, in order:

1. the shared four-stage `Model → Think → Tool → Done` rail;
2. Automation display name and active position when more than one run exists;
3. one current privacy-safe activity caption;
4. foreground/background context and aggregate active count when relevant;
5. the primary action `Otvoriť priebeh`;
6. `Príkazy` only when an allowed active-session command exists.

The active view is not an activity transcript. It may show only the allowlisted
Stage Rail phase, activity category, bounded generic action label, automation
name, queue position, and normalized state.

### Terminal preview

Selecting a terminal row produces a preview without acknowledging it. Show:

1. Run Outcome, completion time, and unread/viewed state;
2. historical Automation name from the Task Snapshot;
3. `Odpojená` when the Automation Definition no longer exists;
4. the separately labelled **Zhrnutie výsledku**;
5. explicit shortfall or normalized no-output condition;
6. the primary action `Otvoriť reláciu`;
7. the secondary action `Príkazy`.

Result Summary never expands in place into Final Output and never masquerades
as the full result.

`Príkazy` is available from the preview so an unread Automation Session may be
exported without first being opened. Opening `Príkazy` alone does not change
Completion Attention.

### Opening a terminal Automation Session

`Otvoriť reláciu` acknowledges the completion and shows the terminal detail.
The ordinary split remains visible where the content fits. The detail
hierarchy is:

1. Run Outcome and Run Provenance;
2. Result Summary;
3. explicit shortfall or absence of Final Output;
4. Final Output primary destination when present;
5. Session Activity Timeline;
6. Validated Sources and unavailable Connector References;
7. redacted historical Approval Records;
8. Continue in new chat when allowed;
9. Session Export, Diagnostic Export, and Delete Automation Session.

Failed, cancelled, and abandoned Automation Sessions show deterministic Result
Summary copy and `Finálny výstup nevznikol`. They may still offer Continue.
Scheduler-only skipped sessions show their normalized scheduler reason, never
show Final Output, and never offer Continue.

## Full-width child destinations

The following temporarily replace the split inside the same black bridge:

- full Final Output;
- expanded Session Activity Timeline;
- Validated Sources and Connector Reference availability;
- redacted approval history;
- Continue replacement confirmation;
- continued Current Chat provenance;
- Delete Automation Session confirmation;
- export explanations when they do not fit the command view.

Each destination has a labelled Back control. Back restores the same selected
master row and previous detail depth. No destination creates a sheet, popover,
menu, notification window, or secondary chooser.

## Final Output and activity hierarchy

### Final Output

- Label the surface **Finálny výstup**.
- It contains the retained privacy-reviewed user-visible answer, not Result
  Summary or model transport output.
- It reuses the existing bounded output surface and is the only Automation
  Session child allowed an internal `ScrollView`.
- The scroll position remains local to that opened destination and does not
  move master selection.
- A 64 KiB truncation is visible at the heading and again at the omission
  marker, with original and retained extent.
- Closing and reopening starts at the beginning unless production already has
  an authoritative persisted reading-position policy. This decision does not
  create one.

### Session Activity Timeline

- Label the surface **Bezpečná časová os aktivity**.
- Show chronological Logical Activities, not attempts or reasoning.
- Each row may contain only allowlisted category, normalized operation,
  timing, outcome, Evidence Contribution, counts, retry grouping, duplicate
  suppression, and normalized failure.
- The bounded timeline is paged if it cannot fit; it never nests another
  scroller beside Final Output.
- Safety-relevant and terminal records remain reachable after truncation.
- Every Truncation Disclosure is visible.

### Validated Sources and Connector References

- Show only admitted Validated Source title, sanitized domain/URL, citation
  identity, validation time, and supporting/corroborating/conflicting role.
- Never show discovery snippets, fetched passages, redirect traces, search
  queries, or provider responses.
- A Connector Reference appears only as its privacy-safe category and current
  availability.
- Unavailable, expired, or deleted connector targets say `Odkaz nie je
  dostupný`; no opaque token or connector-native identifier is shown.

### Historical approvals

- The heading states `Historické schválenia · neudeľujú oprávnenie`.
- Rows show only redacted category, side-effect class, timestamps, outcome,
  and origin.
- `denied`, `expired`, and `abandoned` remain distinct.
- Every continued Current Chat states that new gated work requires
  **Čerstvé schválenie**.

## Continue in new chat

Continue is available for completed, partial, failed, cancelled, and abandoned
Automation Sessions. It is absent for scheduler-only skipped sessions.

Opening Continue marks the source Automation Session viewed.

### Empty Current Chat

When Current Chat is empty, create the new Current Chat directly. The first
visible item is immutable Continuation Provenance followed by the bounded
visible Continuation Seed.

### Non-empty Current Chat

Show an explicit, full-width inline confirmation before replacement. It must
state all of the following:

- the existing Current Chat will be cleared;
- no hidden archive is created;
- the Automation Session remains immutable;
- the new chat receives a bounded visible Continuation Seed and provenance;
- later tool use follows current policy and gated writes require Fresh
  Approval.

Confirm creates a new Current Chat identity; Cancel returns to the unchanged
Automation Session. A 16 KiB Continuation Seed truncation is disclosed in the
confirmation and in the resulting Current Chat.

### Continued Current Chat

The provenance block is visible, immutable, and precedes ordinary Current Chat
turns. It contains:

- historical Automation name;
- Run Outcome;
- completion time;
- trigger and model route where useful;
- `source available`, `source expired`, or `source deleted`;
- every applicable Continuation Seed Truncation Disclosure.

It never contains an opaque Connector Reference token. It grants no authority
and provides no control that mutates the source Automation Session.

Fresh tool use follows current policy. Every gated write requires Fresh
Approval. New activity and results belong only to Current Chat and never become
input to a later recurring Automation Run.

The `/clear` concept affects only Current Chat, its turns, Continuation Seed,
Continuation Provenance, and chat-scoped records. This decision does not
implement or otherwise resolve the separate slash-command work.

## Commands, deletion, and exports

### Command presentation

`Príkazy` opens a bounded command destination in the same panel. It groups:

- Continue in new chat;
- Session Export;
- Diagnostic Export;
- Delete Automation Session;
- links to sources and historical approvals when those are not already visible
  in terminal detail.

Unavailable actions remain absent rather than looking enabled. Destructive
actions are not the default focused action.

### Delete Automation Session

The product action is exactly **Vymazať Automation Session**, never “vymazať
výsledok”.

It opens an inline full-width confirmation stating:

- the terminal Automation Session and its retained content will be removed;
- the action is not `/clear` and does not delete an Automation Definition;
- an already-seeded Current Chat remains separate;
- that Current Chat will subsequently report `source deleted`.

Cancel restores the commands destination. Confirm returns to the active-first
master, removes the row, and moves selection deterministically to the next
active or unread row, then History.

### Session Export

Session Export is described as retained user-visible content. It contains the
allowlisted Task Snapshot, Run Provenance, Run Outcome, Result Summary, Final
Output or explicit absence, timeline, Validated Sources, redacted approvals,
and Truncation Disclosures.

It excludes opaque connector tokens and raw execution data.

### Diagnostic Export

Diagnostic Export is described as privacy-safe structural data. It contains
versioned normalized states, timing, categories, counts, fingerprints,
failures, and retention metadata.

It contains no task text, Final Output, source URL, private identity, or
Connector Reference.

Exporting from an unopened terminal preview does not mark it viewed. If the
user already opened the Automation Session, it remains viewed for that reason,
not because of export.

## Keyboard behavior

Keyboard behavior is deterministic and mirrors visible controls.

### Activity Peek

- Return performs the focused bridge action.
- With several active runs, Return cycles FIFO exactly like a bridge click.
- Tab may focus the invariant status pill; Return opens active-first
  Automations.

### Master/detail

- Up and Down move master selection through the complete active-first backing
  order and page the bounded viewport.
- Right moves focus from the master row to the first enabled detail action.
- Left from a detail action returns focus to the selected master row.
- Return on a master row invokes its primary action.
- Tab and Shift-Tab traverse visible detail controls in visual order.
- Space or Return activates the focused control.
- Commands never intercept arrows owned by a focused text editor or Final
  Output scroller.

### Back and Escape

Escape performs one deterministic level:

1. cancel inline confirmation;
2. leave a full-width child destination;
3. leave terminal or active detail for the active-first master;
4. leave History for the active-first master;
5. collapse the Automations surface.

Escape never deletes, clears Current Chat, cancels a run, or acts as implicit
approval denial.

Within Final Output, Page Up, Page Down, Home, End, and ordinary scrolling
remain native to the bounded output view. Escape leaves the output destination.

## Geometry and visual rules

The one-panel contract remains absolute:

- one fixed non-activating `BagentPanel`;
- fixed width `2 × 260 pt + notchWidth`;
- fixed height `menuBarHeight + 280 pt`;
- one `NotchWrapShape`;
- no AppKit resizing during navigation or animation;
- no new window, sheet, popover, menu-bar item, notification window, or
  secondary chooser.

Activity Peek retains every bridge target from the Stage Rail decision.

The Automation Session master/detail shell uses:

- 248 pt visible left and right wings;
- 252 pt bridge for the active-first split and ordinary detail;
- up to 280 pt only for full Final Output, bounded full-width children, or
  confirmation copy that cannot fit at 252 pt;
- 16 pt horizontal bridge insets, producing a 685 pt content width on the
  221 pt synthetic-notch reference geometry;
- 190 pt master column;
- 1 pt white-opacity divider with 8 pt gutters;
- the remaining width for detail.

Content is measured for the selected target before reveal. Shape geometry leads
content. A child may select the 280 pt target before appearing; content must not
force the bridge to grow after it is visible.

The Stage Rail invariant status pill remains:

- 74 × 18 pt;
- top inset 9 pt;
- trailing inset 12 pt from the visible 248 pt right-wing edge;
- panel-local origin
  `x = maxPanelWidth - (260 - 248) - 74 - 12`, `y = 9`.

No list selection, command, outcome, confirmation, or child destination may
move or resize the pill.

Use the existing off-white text tokens (`0.80`, `0.55`, `0.42`) and
white-opacity fills and controls. There is no material, blur, inner shadow, or
decorative background. State never depends on color alone.

## Motion and reduced motion

Normal motion:

- the fixed AppKit panel never moves;
- surface morph uses the existing 0.58 s ease;
- content appears only after the shape leads it;
- master selection changes in place;
- detail replacement may use a bounded horizontal move plus opacity;
- full-width child entry and Back use matching directional transitions;
- status-pill labels cross-fade without reflow;
- Stage Rail activity motion follows its existing decision.

With Reduce Motion enabled:

- geometry snaps to its target;
- content uses a 0.12 s opacity-only reveal;
- master selection and detail replacement do not translate;
- Stage Rail icons remain static;
- focus changes cross-fade without horizontal motion;
- every hierarchy, label, action, count, acknowledgement rule, scroll
  affordance, and focus destination remains identical.

## Accessibility

- The Automation surface is one named container.
- Reading and keyboard order is: invariant status pill, master heading, visible
  master rows, detail heading, detail content, detail actions.
- The master column is a named navigation group. Each row exposes Automation
  name, active/terminal state, outcome, unread/viewed value, detached state,
  and position in the complete collection.
- The selected master row exposes selected state without relying on color.
- Stage Rail and its focused caption remain one accessibility element using
  the Stage Rail label contract.
- The divider and decorative connectors are hidden.
- Detail headings are announced after explicit navigation without moving
  VoiceOver focus during passive events.
- Full-width child Back controls name their destination, for example `Späť na
  Automation Session`.
- Result Summary, Final Output, shortfalls, Truncation Disclosures, historical
  approvals, and Fresh Approval warnings have distinct labels.
- Passive progress, completion arrival, FIFO cycling, source expiry, and
  Connector Reference availability changes never activate the application or
  move VoiceOver focus.
- The master viewport does not advertise scrolling. Final Output is the only
  element that advertises an internal scroll action.
- Reduced motion preserves the same accessibility tree and focus order.

## Privacy-safe presentation

No selected surface may display or expose through accessibility:

- hidden reasoning or chain-of-thought;
- system, internal, model, or tool prompts;
- Evidence Content or fetched passages;
- raw tool arguments or results;
- write payloads;
- credentials, tokens, signed URLs, or secret query parameters;
- provider errors, stack traces, or model internals;
- connector-native identifiers or private identities;
- opaque Connector Reference tokens.

The exact user-authored automation task is permitted only where the Task
Snapshot policy explicitly permits it. Unknown activity and tools map to the
generic allowlisted category.

## Acceptance scenarios

Production acceptance must exercise Dvojpanelový prehľad at actual notch-scale
geometry for:

1. one active manual run;
2. two active runs from different Automation Definitions and stable FIFO
   cycling;
3. unread completed result;
4. unread partial result with explicit shortfall;
5. failed result with no Final Output;
6. cancelled result;
7. abandoned result after daemon restart;
8. scheduler-only skipped result with no Continue;
9. detached unread Automation Session after definition deletion;
10. 64 KiB-truncated Final Output with visible disclosure and internal
    scrolling;
11. 16 KiB-truncated Continuation Seed;
12. admitted Validated Sources;
13. unavailable Connector Reference;
14. denied, expired, and abandoned historical approvals;
15. pending live approval preemption;
16. continued Current Chat whose source later expires;
17. continued Current Chat whose source is explicitly deleted;
18. Continue with empty Current Chat;
19. Continue replacement confirmation with non-empty Current Chat;
20. Delete Automation Session before and after continuation;
21. Session Export and Diagnostic Export from an unread preview;
22. keyboard and VoiceOver traversal;
23. normal and reduced motion.

Report static, automated, visual, accessibility, signed-build, and live results
as separate evidence. A rendered prototype or server-side state test is not
production browser, signed-build, VoiceOver, or live-daemon proof.

## Implementation boundary and exposed architecture questions

Production implementation must rewrite this behavior under production
standards. No prototype code is promoted.

This decision does not resolve
[Design the unified background-work state machine and event contract](https://github.com/brunovskyoliver/bagent/issues/23).
That ticket still owns authoritative runtime states, events, queue and
cancellation semantics, persistence transactions, daemon/Swift wire types,
race-safe acknowledgement, retention, and Current Chat APIs.

This selection makes the UI consumer contract precise but does not graduate new
fog or justify another ticket. The existing architecture and staged
implementation questions on the map remain sufficient.

## Prototype disposition

The throwaway native SwiftUI prototype compared:

- Fronta relácií: queue-first drill-down;
- Výsledková os: chronology-first outcome flow;
- Dvojpanelový prehľad: persistent master/detail with contextual commands.

It used fake in-memory state only and contacted no daemon, BaseRT process,
connector, TCC service, Keychain, or automation database. It was exercised
across the acceptance scenarios above at the fixed 741 × 319 pt synthetic-notch
reference size, including reduced motion and keyboard navigation.

Dvojpanelový prehľad was selected through direct user feedback. Per the
prototype contract, the throwaway package and temporary screenshots are
deleted rather than promoted.
