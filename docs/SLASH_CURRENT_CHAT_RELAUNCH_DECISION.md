# Slash commands, Current Chat clearing, restoration, and UI-only relaunch

Decision ticket:
[Specify slash commands, Current Chat clearing, and UI relaunch restoration](https://github.com/brunovskyoliver/bagent/issues/22)

Map:
[Rebuild the bagent notch UX around model residency and automation sessions](https://github.com/brunovskyoliver/bagent/issues/15)

Agreed through live grilling on 2026-07-31.

## Decision

bagent uses one explicit editing and execution contract for Slash Commands,
one daemon-authoritative atomic operation for Clear Current Chat, one durable
Current Chat restoration contract, and one bounded UI Relaunch Handoff for
permission-driven UI-only relaunch.

The contract preserves these boundaries:

- entered text is never changed merely because suggestions are visible;
- Tab and click complete text but never execute;
- Return and keypad Enter execute only an exact recognized Slash Command;
- partial and unknown slash text remains an ordinary prompt;
- Clear Current Chat replaces only Current Chat;
- daemon, BaseRT, Automation Runs, and model leases remain background-owned
  during UI-only relaunch;
- Current Chat, Automation Session, and UI Relaunch Handoff are distinct
  concepts;
- every product contract names Current Chat or Automation Session explicitly.

This document specifies product policy and consumer-visible state machines. It
does not choose SQLite layout, wire names, Swift/Rust module ownership, or the
unified background-work event schema.

## Canonical language

The normative glossary is [CONTEXT.md](../CONTEXT.md). This decision relies on:

- **Current Chat**;
- **Current Chat Draft**;
- **Conversation Turn**;
- **Slash Command Candidate**;
- **Slash Command**;
- **Clear Current Chat**;
- **Automation Run**;
- **Automation Session**;
- **Continuation Seed** and **Continuation Provenance**;
- **Saved Long-Term Memory**;
- **Permission Grant Assist**;
- **UI-only Relaunch**;
- **UI Relaunch Handoff**;
- **UI Event Consumer**.

The exact user-authored Current Chat Draft is distinct from internal, system,
model, and tool prompts. Where this document permits the draft, it does not
permit those other prompt classes.

## Facts in the current implementation

These are current-state gaps, not the destination design:

- `SlashCommandRegistry.suggestions` derives case-insensitive prefix matches
  from a slash-prefixed, whitespace-free input.
- `ChatViewModel.acceptSlashSuggestion` currently canonicalizes and executes in
  one operation. `NotchWindowController` invokes it for Tab, Return, and click,
  and `ChatViewModel.send` accepts a visible partial suggestion before exact
  recognition. Therefore `/auto` plus Return currently becomes and executes
  `/automations`.
- Ordinary `TextField` typing and paste bind directly to `inputText`; no
  typing-time path was found that independently strips `/`. The confirmed loss
  or replacement is action-time completion/execution coupling.
- Exact recognition currently trims surrounding whitespace, contrary to the
  strict exactness selected here.
- `/clear` is not registered. The existing `ChatViewModel.clear` clears
  in-memory Swift presentation, drops the saved identifier, and asynchronously
  requests a new identifier without atomically deleting the old Current Chat.
- Swift retains only the current identifier across UI restart. Completed turns,
  drafts, attachments, approval presentation, and connector references are not
  restored.
- Daemon Current Chat listing is empty, turn retrieval is disabled, the chat
  path does not durably append Current Chat turns, and startup purges legacy
  chat-turn data. Runtime connector references are daemon-memory-only.
- Ordinary `AppDelegate` launch invokes `DaemonLauncher.launch`, whose current
  path reinstalls and restarts the daemon. There is no permission-specific
  UI-only handoff.
- One `ChatViewModel` cancels its prior event task before starting another, but
  there is no cross-process UI Event Consumer fencing or cursor handoff.

The focused Swift test target could not execute during this investigation
because unrelated `TavilyConfigurationSyncTests` no longer compile against the
current production types. This does not weaken the static isolation above.

## Slash editing state machine

### Eligibility

A Slash Command Candidate exists only when the complete raw Current Chat Draft:

1. begins with `/` at character zero;
2. is one token with no whitespace or newline;
3. is not carrying uncommitted native marked text.

Leading whitespace, trailing whitespace, embedded whitespace, ordinary text,
and multi-token text are not candidates. A candidate produces suggestions only
when it is a prefix of a registered canonical command or explicit alias.

Matching is Unicode case-insensitive. Diacritics are not folded. Diacritic and
non-diacritic alternatives must be registered as explicit aliases. The draft's
original spelling, casing, diacritics, and `/` remain unchanged until the user
edits it or explicitly completes a suggestion.

### Editing states

```text
ordinary_draft
  └─ text becomes matching Slash Command Candidate ─→ suggestions_visible

suggestions_visible
  ├─ ordinary text edit still matches ───────────────→ suggestions_visible
  ├─ edit no longer matches / whitespace appears ───→ ordinary_draft
  ├─ begin IME/dictation/autocomplete composition ───→ native_composition
  ├─ Escape ─────────────────────────────────────────→ suggestions_dismissed
  ├─ Tab or suggestion click ────────────────────────→ explicitly_completed
  └─ exact Return/keypad Enter ──────────────────────→ execution_requested

native_composition
  ├─ commit matching candidate ──────────────────────→ suggestions_visible
  └─ commit other text / cancel ─────────────────────→ ordinary_draft

suggestions_dismissed
  ├─ text unchanged ─────────────────────────────────→ suggestions_dismissed
  └─ text changes and matches ───────────────────────→ suggestions_visible

explicitly_completed
  └─ one canonical text edit, suggestions hidden ────→ ordinary_draft
```

Suggestion display is read-only. It never changes draft text, caret, selection,
undo history, or focus.

### Completion

Tab or click on a suggestion:

- replaces the entire Slash Command Candidate with the command's canonical
  spelling;
- treats an alias exactly like its canonical command;
- creates one undoable edit;
- places the caret at the end;
- clears selection;
- preserves Current Chat input focus;
- hides suggestions;
- does not execute.

Tab with no suggestion performs forward focus traversal. Shift-Tab always
performs backward focus traversal. Neither inserts a tab, submits, or changes
response-history state.

### Native editing ownership

Caret movement, Home, End, mouse selection, selection replacement, cut, copy,
paste, undo, redo, and ordinary text editing remain native. Suggestions derive
from the complete draft, not the caret position, and recompute only after text
changes.

While IME composition, dictation, or system autocomplete owns uncommitted
marked text:

- suggestions are hidden;
- no Slash Command keyboard action runs;
- the native editor exclusively owns the event;
- suggestions recompute from the exact committed result.

Pasting text follows the same eligibility rules as typing. Pasting `/settings`
may reveal a suggestion but never completes or executes. Pasted paths, URLs,
aliases, casing, and diacritics remain exact user input.

### Suggestion and response-history precedence

Visible suggestions exclusively own plain Up and Down and change only
`highlighted suggestion`. Response-history browsing is eligible only when:

- suggestions are hidden;
- Current Chat Draft is empty;
- native composition is inactive;
- no Conversation Turn is active.

Up and Down never replace or mutate a non-empty draft.

## Slash execution state machine

One typed command registry is the sole authority for:

- canonical command identity and spelling;
- explicit aliases;
- suggestion metadata;
- destination;
- confirmation requirement;
- local execution routing.

An exact command means the complete raw Current Chat Draft equals a canonical
command or explicit alias under Unicode case-insensitive comparison. No leading
or trailing whitespace is ignored.

```text
draft_ready
  ├─ Return + exact read-only command ───────────────→ executing_navigation
  ├─ Return + exact confirmed command ──────────────→ awaiting_confirmation
  ├─ Return + partial/unknown slash text ───────────→ ordinary_prompt_submit
  └─ modified Return/Enter ─────────────────────────→ native_behavior

executing_navigation
  ├─ destination opens ─────────────────────────────→ destination_open
  └─ destination unavailable ───────────────────────→ command_failure

command_failure
  └─ preserve exact draft + focus + normalized error; never model fallback

awaiting_confirmation
  ├─ Cancel/Escape ─────────────────────────────────→ original_Current_Chat
  └─ confirm ───────────────────────────────────────→ command_specific_commit
```

Plain Return and keypad Enter are equivalent. Return or Enter combined with
Command, Option, Control, or Shift never executes a Slash Command.

Plain Escape precedence while Current Chat input is active:

1. native composition cancellation;
2. hide visible suggestions without changing draft, caret, selection, or
   focus;
3. response-history exit when browsing;
4. ordinary current-surface Back or collapse behavior.

Modified Escape is not a Slash Command event.

Slash Commands are recognized only when editable Current Chat input owns focus
and neither an active Conversation Turn nor a pending approval exists. Settings,
Automations, completed output, and other non-editable surfaces do not interpret
keys as Slash Commands. Approval preemption wins.

### Command persistence and failure

- Slash Command text, alias text, partial candidates, completion actions, and
  suggestion highlights never enter Current Chat turns or model history.
- Read-only navigation commands such as `/settings` and `/automations` create no
  durable command record.
- Clear Current Chat may create only a privacy-safe lifecycle audit containing
  canonical action, timestamp, normalized outcome, and opaque old/new Current
  Chat identities.
- A navigation failure preserves the exact command as Current Chat Draft,
  restores input focus, hides suggestions, and shows normalized retryable
  failure.
- A command failure never falls through to ordinary prompt submission.

Read-only navigation commands need no confirmation. `/clear` follows the
confirmation policy below. Any future destructive, external-side-effecting, or
authority-changing command must define a confirmation contract before it can
enter the registry.

## Keyboard and event precedence

Higher rows preempt lower rows.

| Priority | Context | Event | Required result |
|---:|---|---|---|
| 1 | Pending approval | Any command/navigation key | Approval surface owns the event |
| 2 | Native marked text | Editing and Escape | Native composition owns the event |
| 3 | Clear confirmation | Tab/Shift-Tab | Traverse Cancel and Clear |
| 3 | Clear confirmation | Return/Space | Activate only focused control |
| 3 | Clear confirmation | Escape | Cancel; never clear |
| 4 | Automations or Settings focused editor | Native editing keys | Focused native editor owns the event |
| 5 | Visible slash suggestions | Up/Down | Change highlight only |
| 5 | Visible slash suggestions | Tab | Complete; never execute |
| 5 | Visible slash suggestions | click | Complete; preserve input focus; never execute |
| 5 | Visible slash suggestions | Escape | Hide suggestions; preserve draft |
| 6 | Current Chat input | plain Return/keypad Enter | Exact command executes; otherwise submit ordinary prompt |
| 6 | Current Chat input | modified Return/Enter | Native/system behavior; never command execution |
| 7 | Empty Current Chat input | Up/Down | Browse completed response history |
| 8 | Other focused surface | key event | That surface's documented navigation or native behavior |

The command registry does not intercept arrows owned by a focused text editor,
settings control, Automation detail, Final Output scroller, or native
composition.

## Clear Current Chat

### Confirmation eligibility

Clear Current Chat is unavailable while any Conversation Turn or approval is
active. The UI hides or disables the action, and the daemon rejects stale or
racing requests with a normalized conflict. Rejection:

- changes nothing;
- does not cancel work;
- does not allow, deny, expire, or abandon an approval;
- requires a fresh invocation after Current Chat becomes idle.

Confirmation is required when Current Chat contains any:

- completed or interrupted Conversation Turn;
- Continuation Seed or Continuation Provenance;
- chat-scoped Validated Source or Connector Reference;
- submitted or pending attachment;
- completed approval presentation.

The `/clear` command text does not by itself make an otherwise empty Current
Chat non-empty. Clearing an empty Current Chat immediately creates a fresh
identity and acknowledgement.

For a non-empty Current Chat, `/clear` opens an inline confirmation. It states:

- Current Chat content and chat-scoped continuation context will be removed;
- no hidden archive will be created;
- Automation Sessions and Automation Definitions remain;
- Saved Long-Term Memory remains;
- external side effects remain;
- this action does not delete an Automation Session, delete an Automation
  Definition, cancel work, or forget Saved Long-Term Memory.

When continuation context exists, the confirmation also states that the copied
Continuation Seed and Provenance will be removed while the source Automation
Session and its viewed state remain unchanged.

Cancel is initially focused. Destructive confirmation is never the default
focus.

### Atomic state machine

```text
idle_Current_Chat
  ├─ active Conversation Turn or approval ──────────→ clear_unavailable
  ├─ exact /clear + empty ──────────────────────────→ clear_requested
  └─ exact /clear + non-empty ──────────────────────→ clear_confirmation

clear_confirmation
  ├─ Cancel/Escape ─────────────────────────────────→ idle_Current_Chat
  └─ confirm ───────────────────────────────────────→ clear_requested

clear_requested
  └─ submit idempotency key to daemon ──────────────→ committing

committing
  ├─ transaction commits + response arrives ───────→ replacement_acknowledged
  ├─ transaction fails before commit ──────────────→ rolled_back
  └─ connection/response lost ─────────────────────→ outcome_unknown

outcome_unknown
  └─ refetch + same idempotency key ────────────────→ committed_or_rolled_back

replacement_acknowledged
  └─ swap Swift presentation to returned identity ─→ empty_Current_Chat
```

The daemon performs one transaction that removes the old Current Chat's
allowlisted records and creates one replacement Current Chat identity. Swift
keeps presenting the old Current Chat until the committed replacement is
acknowledged.

Each confirmed clear carries a client-generated idempotency key. A lost
response or daemon disappearance produces an unknown outcome, not a local
success or failure guess. The UI refetches authoritative state and retries only
with the same key. It never invents an identity or performs a second independent
clear.

A storage failure rolls the full operation back. Partial storage success is not
a valid state. Orphaned attachment bytes whose only links were removed become
eligible for bounded cleanup after the authoritative ownership change; cleanup
does not redefine clear success.

### Removed and retained data

| Removed from old Current Chat | Retained independently |
|---|---|
| Completed and interrupted turns | Automation Sessions |
| Continuation Seed | Automation Runs and Definitions |
| Continuation Provenance | Saved Long-Term Memory |
| Chat-scoped Validated Sources | External side effects |
| Chat-scoped Connector References | Automation retention/deletion records |
| Submitted attachment links | Privacy-safe lifecycle audit |
| Current Chat Draft | Independent system audit required by policy |
| Caret and selection state | Source Automation Session viewed state |
| Pending attachment references | Detached Automation Session history |
| Response-history position | |
| Slash suggestion/highlight state | |
| Completed approval presentation | |

Successful clear presents the new empty Current Chat, focuses input with the
caret at position zero, and shows a brief non-durable `Current Chat cleared`
acknowledgement. The acknowledgement is not a Conversation Turn.

## Durable Current Chat restoration

### Authority and reopen

The daemon is authoritative for:

- Current Chat identity and durable revision;
- completed Conversation Turns;
- normalized interruption markers;
- Continuation Seed and Continuation Provenance;
- privacy-safe submitted attachment metadata;
- chat-scoped source and Connector Reference availability;
- privacy-safe completed approval presentation.

On ordinary UI reopen, the UI refetches a bounded Current Chat snapshot. The
notch starts collapsed unless a pending approval requires preemption. It does
not replay old daemon events, reopen the prior output/settings surface, or
restore response-history browsing.

### Durable allowlist

| Data | Retention and bound |
|---|---|
| Current Chat identity/revision | Until successful Clear Current Chat |
| Completed Conversation Turns | Until clear; total limits below |
| Interruption marker | Until clear |
| Continuation Seed | Existing 16 KiB policy; until clear |
| Continuation Provenance | Until clear |
| Submitted attachment metadata and links | With owning Conversation Turn |
| Chat-scoped Validated Source metadata | Until clear, bounded by later schema policy |
| Connector Reference | Opaque daemon-owned record until clear; revalidate before reuse |
| Completed approval presentation | Privacy-safe and non-authorizing until clear |
| Current Chat Draft | 16 KiB UTF-8; seven days from last edit |
| Pending attachment references | Same seven-day expiry as Current Chat Draft |

Current Chat has no time-based expiry and is never silently pruned or summarized.
It accepts at most:

- 500 completed Conversation Turns; or
- 16 MiB of encoded retained Current Chat content;

whichever is reached first. At the bound, a new turn is rejected with guidance
to export or Clear Current Chat. Old turns are not silently removed.

Current Chat Draft accepts at most 16 KiB UTF-8. The editor rejects additional
input at the bound rather than truncating. Submission, successful Clear Current
Chat, or explicit draft deletion removes it immediately. Otherwise it expires
seven days after the last edit.

Ordinary UI reopen restores the draft with the caret at its end and no
selection. Exact caret and selection restoration belongs only to a valid,
short-lived UI Relaunch Handoff.

Pending attachments survive ordinary reopen with the draft. Submitted
attachment metadata survives with its Conversation Turn. Restoration never
rereads an arbitrary original path automatically. Missing, expired, deleted, or
inaccessible content is visibly unavailable and may be removed or replaced.

Connector References remain daemon-owned opaque records. The UI receives only
allowlisted category and current availability. Reuse requires authoritative
revalidation; stale or inaccessible targets never fall back to guessed
identifiers.

### Restart state machine

```text
ordinary_UI_quit
  └─ daemon remains authoritative ──────────────────→ UI_refetch_on_reopen

UI_refetch_on_reopen
  ├─ Current Chat available ─────────────────────────→ restore_bounded_state
  └─ daemon unavailable ─────────────────────────────→ reconnecting

daemon_restart_with_no_active_turn
  └─ refetch durable Current Chat ───────────────────→ restore_bounded_state

daemon_restart_during_Conversation_Turn
  ├─ retain submitted user message ──────────────────→ interrupted_turn
  ├─ discard incomplete assistant output ────────────→ interrupted_turn
  ├─ abandon pending approval and runtime ownership ─→ interrupted_turn
  └─ append normalized interruption marker ──────────→ restore_bounded_state
```

The interruption marker states that the response was interrupted by daemon
restart. It is not an assistant answer and offers explicit retry. Incomplete
assistant output is non-authoritative and is not retained. No tool, approval,
side effect, or model request resumes automatically.

Completed approval presentations remain privacy-safe provenance. Pending
approval at daemon restart becomes `abandoned`, never `denied` or silently
resumed.

## Permission-driven UI-only relaunch

### Eligibility

UI-only relaunch is unavailable during:

- an active Conversation Turn;
- any pending approval, including one belonging to an Automation Run.

It is allowed while Current Chat is otherwise idle, including:

- empty input;
- non-empty Current Chat Draft;
- completed output;
- Settings and Permission Grant Assist;
- active Automation Runs.

Approval preemption makes the relaunch control unreachable until the approval
resolves.

### UI Relaunch Handoff allowlist

The handoff is a closed allowlist:

| Allowed field | Purpose |
|---|---|
| Schema version | Compatibility gate |
| Created-at and expires-at | 60-second lifetime |
| Nonce and source UI identity | Replay and origin binding |
| Intended replacement identity | Recipient binding |
| Current Chat identity | Authoritative refetch target |
| Bounded refetch cursor/revision | Gap-free restoration |
| Current Chat Draft, max 16 KiB | Preserve unsent user-authored text |
| Caret and selection offsets | Exact short-lived editing restoration |
| Pending attachment references | Restore unsent attachment presentation |
| Selected settings area | Restore Compass Rail location |
| Selected child destination | Restore focused settings hierarchy |
| Permission Grant Assist phase | Restore visible permission workflow |
| Semantic focus identifier | Restore safe focus when still valid |

Everything else is refetched or recomputed.

### Forbidden handoff data

The handoff must not contain:

- credentials, tokens, signed URLs, or secret values;
- secret-presence or secret-shape indicators;
- raw permission probe values or raw protected-resource data;
- internal, system, model, or tool prompts;
- hidden reasoning or chain-of-thought;
- Evidence Content or fetched passages;
- Current Chat transcript content or assistant output;
- connector tokens, connector-native identifiers, or private identities;
- Automation Session, Automation Run, or automation database rows;
- raw daemon events or provider errors;
- clipboard content;
- diagnostics, debug traces, logs, or stack traces;
- native object archives, view objects, or responder-chain objects.

The bounded user-authored Current Chat Draft is the only prompt-like text
allowed.

The payload never travels in process arguments, environment variables, logs,
`UserDefaults`, pasteboard, or an unprotected file. Process launch carries only
an opaque one-time token. The payload lives in a user-scoped protected transient
store and is erased on successful consumption or expiry. Exact storage and API
module selection are deferred.

### Lifetime and validation

The handoff:

- has an explicit schema version;
- expires 60 seconds after creation;
- is bound to the initiating UI and intended replacement;
- is atomically single-use;
- rejects unknown versions, expiry, identity mismatch, replay, or second
  consumption;
- never partially restores after validation failure.

### Relaunch state machine

```text
permission_relaunch_required
  ├─ ineligible work/approval exists ────────────────→ relaunch_unavailable
  └─ user invokes relaunch ──────────────────────────→ handoff_prepared

handoff_prepared
  └─ launch replacement in UI-only mode ────────────→ replacement_staging

replacement_staging
  ├─ validate/consume handoff ───────────────────────→ refetch_and_restore
  ├─ invalid/expired/incompatible ───────────────────→ replacement_rejected
  └─ no readiness by 10 s ──────────────────────────→ takeover_timed_out

refetch_and_restore
  ├─ restore hidden usable first frame ──────────────→ ready_for_transfer
  └─ authoritative refetch unavailable by 10 s ─────→ takeover_timed_out

ready_for_transfer
  └─ fenced ownership transfer ─────────────────────→ replacement_authoritative

replacement_authoritative
  ├─ replacement acknowledges active presentation ─→ old_UI_exits
  └─ acknowledgement fails ─────────────────────────→ old_UI_reactivated

old_UI_exits
  └─ replacement authoritative probe ───────────────→ rechecking_permission

rechecking_permission
  ├─ granted and effective ─────────────────────────→ granted_active
  └─ denied/missing/indeterminate ──────────────────→ denied_or_missing_guidance
```

### Background-service ownership

Replacement UI-only mode performs no:

- daemon launch, installation, restart, shutdown, or ownership transition;
- BaseRT launch, restart, shutdown, or ownership transition;
- Automation Run start, duplicate, cancellation, or resumption;
- model-load, unload, or lease mutation.

It attaches to existing services and refetches state. Temporary service
unavailability presents reconnecting state rather than causing the UI to assume
ownership.

An active Automation Run, daemon PID, BaseRT PID, and model leases remain
unchanged by UI-only relaunch. Runtime failures are separate events and must not
be attributed to relaunch without evidence.

### UI Event Consumer and duplicate prevention

“Exactly-once event subscription” is expressed precisely as:

- one active UI Event Consumer owns presentation;
- reconnectable transport connections are not the consumer identity;
- authoritative state is refetched before event application;
- a bounded daemon cursor resumes after the refetch boundary;
- stable event identities are deduplicated;
- the old consumer stops after acknowledged takeover.

The replacement prepares its first usable frame hidden and non-interactive.
Ownership transfer is fenced:

1. replacement validates, refetches, restores, and reserves successor authority;
2. old UI becomes non-interactive, hides, and stops consuming;
3. successor authority activates;
4. replacement becomes visible and acknowledges active presentation;
5. old UI exits.

At most one UI is visible or accepts input. A stale process cannot reacquire an
older ownership generation. If activation acknowledgement fails, the successor
is revoked and a newer fence reactivates the old UI.

### Readiness, timeout, and crash recovery

Replacement readiness requires all of:

- handoff validation and consumption;
- Current Chat refetch;
- restoration of allowlisted draft, settings, attachment, and focus state;
- successor UI Event Consumer authority;
- a hidden usable first frame.

The old UI never exits merely because another process started.

If readiness is not acknowledged within 10 seconds:

- the old UI remains or becomes authoritative;
- attempted takeover is revoked;
- the old state remains intact;
- a normalized retryable failure appears;
- a late replacement remains hidden and exits;
- retry creates a fresh handoff.

Crash behavior:

- if old UI crashes after issuing a valid handoff, the intended replacement may
  finish takeover using the same unexpired token;
- if replacement crashes before acknowledgement, old UI remains authoritative
  until timeout;
- if both disappear, a later ordinary UI launch restores durable Current Chat
  and draft state, but not expired Permission Grant Assist navigation state.

### Focus and permission recheck

Focus is restored by semantic identity only. If the identified control no
longer exists or is inappropriate in the restored phase, focus moves to the
restored destination heading. Native responder objects are never serialized.

If Current Chat input was the handoff focus target, its exact caret and
selection restore only when the draft revision matches. Permission-phase
changes never auto-focus a credential editor or move VoiceOver focus during
passive probe updates.

After takeover, replacement restores Permission Grant Assist in a visible
`Rechecking` phase and runs the authoritative probe from the replacement
process identity. If permission remains denied, missing, or indeterminate:

- guidance returns in place;
- no success is claimed;
- no automatic relaunch loop begins;
- Current Chat Draft, attachments, settings location, and safe focus remain.

### Daemon restart during handoff

UI ownership fencing is independent of daemon availability.

- Before takeover acknowledgement, replacement waits only until the existing
  10-second deadline for authoritative refetch. If unavailable, takeover fails
  and old UI remains.
- After acknowledged takeover, replacement remains the UI authority, shows
  reconnecting, and refetches durable Current Chat when the daemon returns.
- Active runtime work follows daemon-restart policy. An Automation Run may
  become abandoned under that policy; UI-only relaunch neither causes nor masks
  the outcome.

## Accessibility and reduced motion

### Slash suggestions

- Expose one named `Slash command suggestions` list.
- Each item exposes canonical command, short description, and selected state.
- Announce result count when the list appears.
- Announce the selected item only after user-initiated highlight movement.
- Announce explicit completion as a text edit and dismissal once.
- Typing does not repeatedly announce the entire list.
- Passive suggestion changes do not move VoiceOver focus.

### Clear Current Chat

- Confirmation initially focuses Cancel.
- Tab and Shift-Tab traverse Cancel and Clear in visual order.
- Return or Space activates only the focused control.
- Escape cancels.
- VoiceOver names Current Chat as the deletion target and names Automation
  Sessions and Saved Long-Term Memory as retained.
- Success acknowledgement is announced once and is not a Conversation Turn.

### UI-only relaunch

- Permission Grant Assist communicates that only the notch UI restarts.
- Handoff and passive probe updates never move VoiceOver focus unexpectedly.
- Restored headings and focus controls have stable semantic labels.
- Reconnecting, takeover failure, Rechecking, granted, and denied-or-missing
  states expose concise labels and values.

With Reduce Motion:

- suggestion and confirmation appearance is opacity-only;
- successful-clear acknowledgement is opacity-only;
- replacement transfer has no spatial cross-fade or morph;
- old UI hides before replacement appears;
- settings restoration and permission recheck use the existing 0.12 s
  opacity-only settings transition;
- keyboard, focus, hierarchy, state, timing, and accessibility semantics remain
  unchanged.

## Acceptance scenarios

Production acceptance must include at least these outcomes:

1. **Character-by-character `/automations`**: exact text is preserved; prefix
   suggestions narrow; nothing executes until exact plain Return.
2. **`/auto` plus Return**: submits unchanged ordinary prompt.
3. **`/auto` plus Tab**: completes to `/automations`, remains editable, does not
   execute.
4. **`/auto` plus Escape**: hides suggestions, preserves `/auto`; later Return
   submits it normally.
5. **`/auto` plus click**: completes, preserves focus, does not execute.
6. **Paste `/settings`**: may show a suggestion; never completes or executes.
7. **Path or URL beginning `/`**: initial `/` may suggest; unmatched prefix
   removes suggestions and submits unchanged.
8. **Leading/trailing whitespace**: prevents command recognition and submits
   ordinary prompt.
9. **Casing and aliases**: exact registered variants execute one canonical
   identity; unregistered diacritic folding does not occur.
10. **IME/dictation/autocomplete**: native composition is uninterrupted;
    suggestions recompute only after commit.
11. **Caret, selection, paste, and undo**: native editing remains intact;
    explicit completion is one undoable edit.
12. **Response-history competition**: visible suggestions own Up/Down;
    non-empty draft never enters response history.
13. **Tab without suggestion**: traverses focus; does not submit.
14. **Return/keypad Enter/modifiers**: plain exact commands execute; modified
    variants never execute.
15. **Destination failure**: exact draft survives with normalized retry; no
    model fallback.
16. **`/clear` during streaming**: unavailable locally and rejected by daemon;
    streaming continues.
17. **`/clear` during approval**: approval remains unchanged; user must invoke
    again after resolution.
18. **Empty Current Chat clear**: creates fresh identity immediately and
    acknowledges.
19. **Non-empty Current Chat clear**: cancel-first confirmation accurately
    names removed and retained domains.
20. **Clear after Continue**: seed, provenance, and follow-ups disappear;
    source Automation Session remains.
21. **Daemon disappears before clear commit**: full rollback.
22. **Daemon disappears after clear commit but before response**: reconcile with
    the same idempotency key; no second identity.
23. **Partial storage failure**: no partial deletion is visible.
24. **Successful clear**: empty focused input, caret zero, transient accessible
    acknowledgement.
25. **Ordinary UI reopen**: completed Current Chat and draft restore; UI starts
    collapsed absent approval.
26. **Draft expiry/overflow**: seven-day expiry; 16 KiB bound rejects overflow.
27. **Current Chat retention bound**: 500 turns or 16 MiB rejects new work;
    existing turns remain.
28. **Missing attachment**: unavailable state; no arbitrary path reread.
29. **Stale Connector Reference**: revalidation yields unavailable; no guessed
    identity.
30. **Daemon restart during turn**: user message and interruption marker remain;
    partial assistant output and resumable authority do not.
31. **Permission relaunch with draft**: draft, attachments, caret, and selection
    restore without submission.
32. **Permission relaunch during Automation Run**: daemon/BaseRT PIDs, run, and
    leases remain unchanged; one UI Event Consumer resumes presentation.
33. **Replacement never acknowledges**: old UI remains after 10 seconds; late
    replacement exits.
34. **Old and replacement processes overlap**: replacement remains hidden until
    fenced transfer; only one visible interactive notch.
35. **Expired, replayed, or incompatible handoff**: no partial restore; old UI
    remains.
36. **Old UI crashes after handoff launch**: intended replacement may consume
    the still-valid bound token.
37. **Both UIs crash**: ordinary restoration recovers durable Current Chat but
    not expired permission navigation.
38. **Permission remains denied**: replacement returns to guidance without
    success or relaunch loop.
39. **Daemon restarts before handoff readiness**: takeover times out unless
    authoritative refetch returns within 10 seconds.
40. **Daemon restarts after takeover**: replacement remains UI authority and
    reconnects; runtime work follows daemon-restart policy.
41. **VoiceOver**: suggestion, confirmation, focus, recheck, and failure
    announcements are bounded and never steal passive focus.
42. **Reduce Motion**: identical state/focus behavior with no spatial motion.

Acceptance must report deterministic tests, persistence/transaction tests,
accessibility validation, signed-build process-handoff proof, visual notch-scale
proof, and live daemon/BaseRT/Automation Run proof as separate evidence.
Rendering a prototype, opening a replacement process, or seeing a permission
pane is not sufficient live proof.

## Product policy versus deferred implementation

This document decides:

- Slash Command eligibility, editing, completion, execution, failure, keyboard,
  persistence, and accessibility behavior;
- Clear Current Chat confirmation, atomicity, idempotency, removal, retention,
  failure, and presentation;
- Current Chat durable restoration, bounds, interruption, attachment,
  Connector Reference, draft, and approval-presentation policy;
- UI-only relaunch eligibility, handoff allowlist and prohibitions, lifetime,
  ownership, readiness, timeout, crash, duplicate-process, focus, permission
  recheck, and daemon-restart behavior;
- concrete acceptance outcomes.

The later
[Design the unified background-work state machine and event contract](https://github.com/brunovskyoliver/bagent/issues/23)
still decides:

- SQLite tables, migrations, indexes, and transaction implementation;
- Current Chat repository and retention-worker module ownership;
- Swift and Rust types;
- endpoint, cursor, idempotency, event, and error envelope names;
- daemon event schema and delivery mechanism;
- UI ownership-fence and protected transient-store mechanism;
- unified Conversation Turn, approval, Automation Run, model lease, restart,
  and cancellation state graph;
- staged implementation and rollback sequence;
- exact acceptance harnesses.

Those choices must implement this consumer contract without weakening its
privacy, atomicity, or ownership boundaries.

No ADR is required. The durable decision asset is the appropriate record: it
defines product policy, exposes its rationale and trade-offs directly, and
deliberately avoids a surprising hard-to-reverse implementation mechanism.

## Map impact

This decision makes the Current Chat and UI-relaunch consumer contract precise
for
[Design the unified background-work state machine and event contract](https://github.com/brunovskyoliver/bagent/issues/23).
It does not require a new ticket, new dependency, or fog graduation. Existing
architecture and staged-implementation fog on the map remains sufficient.
