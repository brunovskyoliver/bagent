# Activity Peek Stage Rail decision

Decision ticket: [Prototype Activity Peek and the invariant status pill](https://github.com/brunovskyoliver/bagent/issues/18)

Map: [Rebuild the bagent notch UX around model residency and automation sessions](https://github.com/brunovskyoliver/bagent/issues/15)

Selected: 2026-07-30 through live user feedback

## Decision

Use **Stage Rail** for Activity Peek.

Stage Rail is a compact, read-only presentation inside bagent's one existing
notch panel. Its primary hierarchy is a four-stage rail:

```text
Model → Think → Tool → Done
```

One focused, privacy-safe activity caption sits below the rail. The invariant
status pill remains at one top-right coordinate while the bridge changes
height. The panel remains fixed and non-activating; no window, menu-bar item,
popover, notification surface, or focus-stealing interaction is added.

The user selected Stage Rail after comparing it live with Spotlight and Run
Stack. The selection itself is direct user feedback. The product rationale
inferred from the comparison is that Stage Rail makes model residency and the
work lifecycle visible without giving every concurrent run a permanently
expanded row. Its known cost is less direct multi-run selection; the interaction
contract below resolves that with the anchored active-count control and focused
run cycling, not a stacked layout.

## Geometry

The implementation must retain the production one-panel contract:

- AppKit continues to own one fixed `BagentPanel`.
- Fixed panel width is `2 × 260 pt + notchWidth`.
- Fixed panel height is `menuBarHeight + 280 pt`.
- `NotchWrapShape` remains the only black notch-wrapping surface.
- Physical displays use the measured notch gap; non-notch displays retain the
  existing 221 pt synthetic notch gap.
- Activity Peek uses 248 pt left and right wings, below the 260 pt ceiling.
- Only `bridgeHeight` changes between Activity Peek states. AppKit never resizes
  the panel for Activity Peek.

Bridge targets:

| Presentation | Bridge height |
|---|---:|
| Idle/collapsed | 0 pt |
| Model loading | 78 pt |
| Foreground or background thinking | 78 pt |
| Mail, web, filesystem, Odoo, Codex, or generic tool activity | 78 pt |
| Unread, partial, or failed completion | 98 pt |
| Foreground chat with background work continuing | 126 pt |
| Two concurrent Automation Runs | 150 pt |
| Pending approval | 176 pt |

Every target is below the 280 pt bridge ceiling. Content is laid out only after
the target shape is known; geometry leads content rather than expanding around
already-visible text.

### Invariant status-pill anchor

The status pill is a fixed 74 × 18 pt capsule:

- top inset: 9 pt from the fixed panel's top;
- trailing inset: 12 pt from the visible 248 pt right-wing edge;
- panel-local origin:
  `x = maxPanelWidth - (260 - 248) - 74 - 12`, `y = 9`;
- neither bridge height nor focused activity changes this origin or size;
- label changes cross-fade within the capsule and never reflow its frame.

The pill uses the existing off-white text tokens and a small semantic dot. It
has a `Color.white.opacity(0.06)` fill, no material, blur, shadow, or inner
shadow.

## Information contract

Activity Peek may show only:

- one static phase label from the Stage Rail;
- one allowlisted activity category;
- a bounded generic action label such as `Checking Mail`;
- the saved, user-authored Automation display name, truncated to one line;
- active-run count, queue position when available, and normalized terminal
  state.

It must never show hidden reasoning, chain-of-thought, prompts, automation task
text, raw tool arguments, connector identifiers, evidence content, credentials,
source passages, provider errors, or tool/model output. Unknown tool names map
to the generic category rather than being rendered verbatim.

## Priority and focus

The focused caption is selected deterministically:

1. A pending approval preempts the rail and every other activity.
2. An active foreground Current Chat preempts background Automation Runs.
3. A foreground tool belonging to that Current Chat remains focused.
4. Otherwise, keep the Automation Run most recently focused by the user.
5. If none was focused, show the oldest active Automation Run, preserving FIFO
   stability rather than switching on every event.
6. When no work is active, show the newest unacknowledged terminal Automation
   Session.
7. With no active or unread work, collapse to idle.

Foreground priority does not pause or hide the existence of background work.
The status pill continues to show the active Automation Run count, and the
caption's secondary line says that background work continues without exposing
its content.

## Interaction

Activity Peek is read-only except for navigation and acknowledgement:

- With exactly one selectable activity, clicking the bridge opens that Current
  Chat output or Automation Session using the existing notch output/detail
  surface.
- With several active Automation Runs, clicking the bridge cycles the focused
  caption in stable FIFO order and shows `1 of N`, `2 of N`, and so on.
- Clicking the invariant `N ACTIVE` status pill opens the existing Automations
  surface ordered with active runs first; this is not a new chooser or popover.
- Return performs the same action as clicking the currently focused caption
  when Activity Peek already has keyboard focus.
- Passive appearance, focus cycling, and completion arrival never activate
  bagent or take focus from the frontmost application.
- A pending approval replaces Activity Peek with the existing approval surface;
  approval remains the only place where a gated write can be allowed.

## State transitions

The rail is presentation state derived from the future unified background-work
event contract; it is not a second runtime authority.

```text
idle
  ├─ cold demand ─────────────→ Model
  ├─ warm foreground demand ──→ Think
  └─ warm Automation demand ──→ Think

Model ── ready ────────────────→ Think
Model ── normalized failure ───→ Done(failed)

Think ── tool started ─────────→ Tool(category)
Tool  ── tool completed ───────→ Think
Think/Tool ── completed ───────→ Done(unread)
Think/Tool ── partial ─────────→ Done(partial)
Think/Tool ── failed ──────────→ Done(failed)

any visible state ── approval ─→ approval preemption
approval resolved ─────────────→ recompute from authoritative work state
Done acknowledged + no work ───→ idle
```

Repeated tools change the Tool icon/category in place; they do not append a
transcript. If model weights are already resident, work starts at Think.
Transport health alone must not light Model or imply residency.

Concurrent runs share one rail. The focused caption changes, but the active
count is the aggregate number of active Automation Runs. Foreground chat uses
the same rail with foreground priority while the aggregate count continues to
represent background Automation Runs.

## Status-pill labels and completion

Allowlisted labels:

| Aggregate state | Pill |
|---|---|
| Idle with resident weights | `RESIDENT` |
| Model lifecycle transition | `LOADING` |
| One active item | `ACTIVE` |
| Two active Automation Runs | `2 ACTIVE` |
| No active work, unread success | `UNREAD` |
| No active work, unread partial result | `PARTIAL` |
| No active work, unread failure | `FAILED` |
| Pending approval | `APPROVE` |

While work remains active, `ACTIVE`/`N ACTIVE` wins the pill label. Any
unacknowledged terminal result is represented by a small marker on Done until
active work drains. Among terminal-only states, severity is `FAILED` then
`PARTIAL` then `UNREAD`.

Opening a terminal Automation Session acknowledges that completion. Cycling
focus does not. After acknowledgement, the next unacknowledged terminal session
is focused; when none remain and no work is active, the bridge collapses.
Completion never creates a macOS notification or another surface.

## Icon mapping

| Activity | Icon |
|---|---|
| Model loading/residency transition | `cpu` |
| Foreground chat thinking | `bubble.left.fill` |
| Background Automation Run thinking | `clock.arrow.2.circlepath` |
| Mail | `envelope.fill` |
| Web | `globe` |
| Filesystem | `folder.fill` |
| Odoo | existing project Odoo connector badge |
| Codex | `chevron.left.forwardslash.chevron.right` |
| Generic/unknown tool | `wrench.and.screwdriver.fill` |
| Completion | `checkmark.circle.fill` |
| Approval preemption | `hand.raised.fill` or the existing approval shield |

Only the currently focused tool icon is emphasized. Other stage icons remain
faint positional landmarks.

## Motion

Normal motion:

- surface morph: existing 0.58 s ease-in-out;
- content reveal: begins at 62% of the morph, approximately 0.36 s;
- status label: 0.16 s opacity cross-fade without position or size change;
- Model, web, and Automation icons: 1.8 s continuous rotation;
- Mail: ±7° top-anchored swing over 0.38 s;
- filesystem and Codex: ±2 pt vertical travel over 0.72 s;
- Odoo/chat: 0.90–1.08 scale breath over 0.72 s;
- generic tool: ±12° tool rotation over 0.38 s;
- completion: one 0.24 s check scale/opacity reveal, then static;
- the active rail segment changes color and opacity; the rail itself does not
  slide.

Motion is meaningful only while the corresponding activity is current. Changing
the focused run replaces the category animation rather than stacking animations.

## Reduced motion

When `accessibilityReduceMotion` is true:

- geometry snaps to its target with no spatial morph;
- content uses a 0.12 s opacity-only reveal;
- rail segments and icons never rotate, translate, swing, or scale;
- the current activity icon is static and distinguished by semantic color;
- the status dot may use a slow opacity-only pulse;
- completion uses a single opacity fade and remains static;
- focus cycling cross-fades the caption without horizontal movement.

The reduced-motion mode preserves every label, stage, count, priority rule,
click target, acknowledgement rule, and accessibility value.

## Accessibility

- Treat the rail and focused caption as one accessibility element whose label
  states phase, activity, foreground/background origin, run position, active
  count, and terminal state.
- Give the invariant status pill an explicit label and value, for example
  `Status, two active Automation Runs`.
- Mark decorative connectors between rail stages as hidden.
- Category icons use explicit labels and never rely on color alone.
- State updates may use `updatesFrequently`, but must not move VoiceOver focus
  or activate the application.
- Click targets retain keyboard equivalents when the existing notch surface is
  already focused.

## Deferred architecture boundary

This decision specifies presentation and interaction only. It does not choose
Swift/Rust ownership, invent a production event schema, or implement model
residency. The unified background-work state-machine ticket must provide one
authoritative event model carrying privacy-safe phase, origin, stable run
identity, aggregate count, queue position, normalized category, priority, and
terminal acknowledgement state. Activity Peek consumes that model and never
derives residency from service health or the current `isThinking` flag.

## Prototype disposition

The selected Stage Rail behavior was validated in the isolated native SwiftUI
prototype across collapsed, tool-active, concurrent, completed, approval,
foreground-priority, partial/failed, and reduced-motion scenarios. Per the
prototype contract, the throwaway package and generated evidence were deleted
after this decision was recorded. No prototype code is promoted into the
production app.
