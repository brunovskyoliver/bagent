# Settings and Permission Grant Assist decision

Decision ticket:
[Prototype the settings and Permission Grant Assist information architecture](https://github.com/brunovskyoliver/bagent/issues/19)

Map:
[Rebuild the bagent notch UX around model residency and automation sessions](https://github.com/brunovskyoliver/bagent/issues/15)

Selected: **Compass Rail**, through direct user feedback on 2026-07-30.

## Decision

Replace the current five-page settings sequence and monolithic Setup page with
four peer destinations in a persistent horizontal rail:

```text
General | Model and Runtime | Integrations | Privacy and Permissions
```

Compass Rail keeps all four destinations visible while one bounded page renders
below it. Configurable integrations, Permission Grant Assist, and rules and
approval policy open as focused child destinations inside the same bridge.
There is no separate settings home page and no scrolling Setup page.

The user selected Compass Rail after comparing it with Split Ledger and Focus
Hub in a live native SwiftUI prototype. No additional verbal rationale was
given. The product rationale established by the selected structure is fast
lateral movement between four equal areas, persistent location awareness, and
full-width child flows without reintroducing a catch-all Setup destination.

This is an information-architecture and interaction decision. It does not
implement production settings, model residency, connector mutations, permission
probes, signing, or UI relaunch.

## Surface and geometry

Settings remains part of bagent's one existing notch surface:

- AppKit owns the existing single fixed oversized `BagentPanel`.
- The fixed panel ceiling remains `2 × 260 pt + notchWidth` wide and
  `menuBarHeight + 280 pt` high.
- Ordinary settings use 205 pt wings and a 252 pt bridge.
- Every selected page and child destination must fit that ordinary settings
  geometry. Removing the scrolling Setup page also removes its special 280 pt
  bridge target.
- `NotchWrapShape` remains the only black surface. No new window, sheet,
  popover, modal, menu-bar item, or material is introduced.
- The visible styling remains black with the existing off-white
  `0.80`/`0.55`/`0.42` text tokens, subtle white-opacity controls, and no blur,
  material, inner shadow, or decorative background.
- Content clips to the already-computed fixed panel. AppKit must not resize the
  panel during settings navigation.

The status pill remains at one settings-local top-right coordinate:

- 74 × 18 pt;
- top inset 9 pt;
- trailing inset 12 pt from the visible 205 pt right-wing edge;
- no area, child destination, permission phase, validation message, or
  reduced-motion setting may move or resize it;
- label changes cross-fade in place without reflow.

Pending approval and WhatsApp QR pairing retain their existing preemption over
the settings surface. The status pill is not a fifth navigation destination.

## Hierarchy

```text
Settings
├── General
├── Model and Runtime
├── Integrations
│   ├── Apple Mail status
│   ├── Apple Notes status
│   ├── WhatsApp configuration
│   ├── Odoo configuration
│   └── Codex configuration
└── Privacy and Permissions
    ├── Full Disk Access assist
    ├── Screen Recording assist
    ├── Accessibility assist
    └── Rules and approval policy
```

The horizontal rail stays visible at every level. A selected top-level area uses
the existing faint/selected off-white treatment and a bounded icon plus short
label. It must not depend on color alone. The notch's left-wing icon mirrors
the selected top-level area even when a child destination is open.

Opening `/settings` selects General. Returning to settings after a UI-only
permission relaunch restores the top-level area and child destination captured
by the handoff snapshot.

## Navigation and keyboard contract

At a top-level area:

- clicking a rail item selects it;
- plain Left and Right cycle through the four areas and wrap at both ends;
- the app-level notch command handler owns those arrows even when a
  non-editable child view currently has focus;
- Tab and Shift-Tab move through visible controls in visual order;
- Return or Space activates the focused control;
- Escape collapses settings through the existing notch dismissal path.

At a focused child destination:

- the rail remains visible and can directly select another top-level area;
- plain Left and Right do not change areas, because credential fields and other
  focused child controls retain their native editing behavior;
- a labeled Back control returns exactly one level;
- Escape performs the same one-level Back action;
- after Back, focus returns to the control that opened the child;
- Escape from the restored top-level area then collapses settings.

The app-level arrow handler must yield whenever an editable `TextField`,
`SecureField`, `TextEditor`, picker, slider, or equivalent native editor owns
focus. It must not infer editability from the page name. Icon-only rail
controls, toggles, status pills, Back, test actions, and the draggable
application affordance require explicit accessibility labels and values.

No settings page contains `ScrollView`, lazily scrollable grids, nested
scrolling editors, or an accessibility element that advertises scrolling. Long
or advanced content must move to another bounded child destination or a stepped
flow.

## General

General contains only existing everyday interaction preferences:

- paste wheel enabled state, including its Accessibility dependency;
- cmux notifications enabled state;
- existing interaction guidance such as the `Option-Space` input shortcut.

The shortcut is explanatory unless production already permits editing it. This
decision adds no cloud, Ollama, legacy-provider, or invented account setting.

## Model and Runtime

Model and Runtime presents one selected local chat model and the authoritative
residency state supplied by the future daemon-owned Model Residency
coordinator. Service availability and weight residency remain distinct.

Required residency presentations:

| State | Meaning |
|---|---|
| Loaded | Weights are resident; completion readiness is still being established. |
| Loading | One single-flight load or model transition is in progress. |
| Ready | The selected model is resident and completion-ready. |
| Unloading | Active leases have drained and weights are retiring. |
| Unloaded | The BaseRT service is alive with no resident weights. |
| Unavailable | The runtime cannot admit work until bounded recovery succeeds. |

The page also contains:

- selected chat model;
- preload on input as cancellable lowest-priority demand, not a lease;
- one shared idle timeout;
- an active-lease indicator explaining that unload is prevented;
- background automation waiting for residency;
- a changed-PID recovery requirement after 35B retirement, a Metal/device/
  command-buffer fault, or an indeterminate lifecycle/completion timeout.

Loading, unloading, unavailable, and changed-PID recovery states are status,
not user-authored provider configuration. No Ollama or cloud provider appears.
The page must never imply that model discovery, service health, or
`loaded=true` alone proves a usable completion.

## Integrations

The top-level Integrations page is a compact overview, not a configuration
form. It shows:

- Apple Mail;
- Apple Notes;
- WhatsApp;
- Odoo;
- Codex;
- the relevant local-only connector service status.

Each row exposes a normalized state such as connected, needs setup, testing,
validation failed, permission required, or local service unavailable. Raw
connector identifiers, provider errors, credentials, Mail or Notes content,
and tool output never appear.

Apple Mail and Apple Notes remain local permission-backed statuses. When their
protected-resource probe is missing, the row links to the relevant Full Disk
Access child under Privacy and Permissions rather than inventing credentials.

WhatsApp, Odoo, and Codex may open focused configuration children:

- each child has Back and Escape behavior;
- only fields relevant to that integration are visible;
- secret values use safe placeholders and production Keychain-backed handling;
- placeholders never reveal whether a specific secret character or value
  exists;
- Test configuration has explicit idle, testing, valid, invalid, and
  unavailable states;
- normalized validation feedback stays within the child and fits without
  scrolling;
- WhatsApp pairing may invoke the existing QR preemption surface, not a new
  window or inline QR clone.

Odoo carries its endpoint/database/user and API-key needs; Codex carries its
local executable/path needs; WhatsApp carries its local bridge connection
state. Configuration storage, validation APIs, and secret ownership are
implementation concerns, but the UI must not write credentials to ordinary
logs or settings files.

## Privacy and Permissions

The top-level page shows three authoritative grant summaries:

- Full Disk Access;
- Screen Recording;
- Accessibility.

Selecting a missing, unknown, or relaunch-required grant opens Permission Grant
Assist as a focused child. Selecting Rules and approval policy opens a separate
advanced child that summarizes or edits bounded policy sections. The current
giant inline `rules.yaml` editor and scrolling Setup group do not return.

This decision does not add a microphone row. A microphone permission flow
requires its own product need, usage description, authoritative AVFoundation
state mapping, and acceptance coverage.

## Permission Grant Assist state machine

Permission Grant Assist is guidance around authoritative system state. It never
grants permission itself.

```text
unknown
  └── probe ──→ denied_or_missing
                  └── Allow ──→ opening_exact_pane
                                    ├── confirmed ──→ drag_ready
                                    └── failed/unconfirmed
                                          └── opening_privacy_root
                                                └── drag_ready

drag_ready ── drag began ──→ dragging_application
dragging_application ── drag ended ──→ waiting_for_system_settings
waiting_for_system_settings ── app activated ──→ rechecking

rechecking
  ├── probe false/error ──→ denied_or_missing
  ├── granted and effective ──→ granted_active
  └── granted but current UI must restart ──→ relaunch_required

relaunch_required ── Relaunch bagent ──→ relaunch_handoff
relaunch_handoff ── replacement UI ready ──→ rechecking_after_relaunch
rechecking_after_relaunch
  ├── probe true ──→ relaunch_completed_active
  └── probe false/error ──→ denied_or_missing
```

Visible labels may be shorter, but the implementation must preserve these
distinct states:

1. unknown;
2. denied or missing;
3. exact pane opening;
4. exact-pane failure with Privacy and Security root fallback;
5. ready to drag;
6. dragging the application;
7. waiting for System Settings;
8. authoritative recheck on app activation;
9. granted and active;
10. granted but UI relaunch required;
11. daemon-preserving relaunch handoff;
12. relaunch completed and permission rechecked.

The helper always states:

- macOS requires the user's action;
- bagent cannot grant permission automatically;
- opening a pane does not prove success;
- no TCC reset is performed or suggested;
- the authoritative probe, not a prompt callback or deep-link result, decides
  the state;
- category deep links require a Privacy and Security root fallback.

### Authoritative probes

- Full Disk Access has no public grant-status API. Open the exact protected Mail
  and Notes resources required by the product and expose normalized per-resource
  results. `isReadableFile` metadata alone is insufficient, and the probed
  signed process/helper identity must match the item being granted.
- Screen Recording uses `CGPreflightScreenCaptureAccess()`.
- Accessibility uses `AXIsProcessTrusted()`.

Prompt/request APIs may initiate a system interaction but never mark the row
granted. Probe on initial display and after a debounced
`NSApplication.didBecomeActiveNotification`. Repeated activation events must
not race or stack probes.

### Pane opening

`Allow` first attempts the centralized category destination for the selected
permission. If the exact destination cannot be confirmed, open the live-verified
Privacy and Security root and name the pane the user must select. Pane opening
is navigation feedback only; it does not advance to granted.

## Application drag semantics

The visible bagent icon is a drag affordance for the running application:

- the payload is the existing installed, signed `.app` at
  `Bundle.main.bundleURL`;
- it is written as an `NSURL`/`URL` pasteboard writer advertising
  `public.file-url`;
- the drag image comes from the packaged application icon, preferably through
  `NSWorkspace.icon(forFile:)`;
- the PNG artwork used by the prototype is never the payload;
- an executable, alias, file promise, source-build directory, or standalone
  image is never offered as the application to grant;
- the helper must refuse to advertise a drag payload when the running URL is
  not an existing `.app` bundle with the expected stable signed identity.

The visible instruction says to drag **bagent.app** into the named System
Settings pane and enable it. Its accessibility label is “Draggable bagent
application”; its hint states that the signed running application is the item
being dragged. Users who cannot drag can still use the pane's native Add flow;
bagent does not simulate that system action.

## UI-only relaunch handoff

When an authoritative post-activation probe reports a grant that the current UI
cannot use, show:

```text
Relaunch bagent
Only the notch UI restarts. The daemon, BaseRT, and active automations stay running.
```

The action launches a replacement UI in an explicit permission-handoff mode.
That mode must skip `DaemonLauncher.launch()`: ordinary launch currently
reinstalls and restarts the launchd daemon and therefore cannot serve as the
permission relaunch path.

The handoff snapshot is versioned and bounded. It may carry:

- Current Chat session identifier or a bounded refetch cursor;
- current draft;
- selected settings area;
- selected child destination and permission-assist phase;
- the minimum UI restoration metadata allowed by existing product policy.

It must not persist credentials, raw permission probe details, hidden reasoning,
evidence content beyond existing retention policy, connector identifiers, or
automation database rows.

The old UI exits only after the replacement acknowledges the handoff. The
replacement restores the Compass Rail destination, subscribes once to daemon
events, and runs the authoritative permission probe again. “Relaunch completed”
is never inferred from process start alone.

## Motion and reduced motion

Normal navigation uses the existing surface-led sequence:

- the fixed panel does not move;
- the selected rail treatment changes in place;
- page content follows the shape and may move horizontally with opacity;
- child entry and Back use the same bounded directional transition;
- status-pill labels cross-fade without moving.

With Reduce Motion enabled:

- geometry snaps to its target;
- pages and child destinations use a 0.12 s opacity-only transition;
- no content translates, scales, rotates, or springs;
- selection, hierarchy, status, labels, focus restoration, and every keyboard
  action remain identical.

## Accessibility

- The rail is one named navigation group with four individually named buttons.
- The selected rail item exposes selected state and does not rely on color.
- The page heading is announced after direct area navigation without moving
  VoiceOver focus during passive status updates.
- Every toggle exposes its label and On/Off value.
- Model residency, lease, queue, validation, permission, and relaunch states
  expose concise labels and values.
- Status icons have labels; decorative dividers and connectors are hidden.
- Back is named for its parent destination, for example
  “Back to Integrations”.
- Permission instructions, drag semantics, authoritative source, and
  daemon-preserving relaunch consequence are available to assistive
  technologies.
- No page advertises unavailable scroll actions.

## Implementation and acceptance boundary

For the current Stage 7C and Stage 8 campaign, signed/live qualification is
limited to macOS 26. macOS 14 and 15 remain compile targets when existing
configuration permits them, but this campaign claims no runtime, System
Settings, TCC, visual, or accessibility qualification on those versions. Live
grant, denial, revocation, and drag-to-System-Settings mutation are out of
scope. Deterministic permission adapters, signed-bundle and drag-payload
validation, privacy tests, and daemon-preserving relaunch remain required;
omitted evidence is never PASS.

Production implementation must rewrite the selected structure under production
standards rather than copy prototype code. At minimum, acceptance must separate:

1. deterministic unit coverage for hierarchy, arrow routing, edit-control
   yielding, Back/Escape, permission-state mapping, pane fallback, activation
   debounce, drag-payload validation, and bounded handoff encoding;
2. isolated Swift build and focused UI/state tests;
3. accessibility-tree validation for every area and child destination;
4. notch-scale screenshots on physical-notch and synthetic-notch displays,
   including every residency state, integration validation state, permission
   state, and reduced motion;
5. signed-bundle validation for stable identity, icon resources, nested code,
   and `public.file-url` drag payload;
6. macOS 26 observation-only checks for exact pane plus root fallback;
7. deterministic permission-transition adapters; live TCC grant, denial,
   revocation, and drag-to-System-Settings mutation are outside this campaign;
8. an active Automation Run relaunch test proving unchanged daemon and BaseRT
   PIDs, uninterrupted completion, restored UI state, one event subscription,
   and an authoritative post-relaunch probe.

Static, automated, signed-build, visual, and macOS 26 observational results
must be reported as separate evidence. macOS 14/15 runtime evidence and live
TCC mutation must be listed as omitted. Opening System Settings, rendering the
helper, or launching a replacement process is not by itself a permission or
relaunch PASS.

## Prototype disposition

The throwaway native SwiftUI prototype compared:

- Compass Rail: persistent horizontal peer navigation;
- Split Ledger: persistent vertical navigation with a narrower content column;
- Focus Hub: a settings home followed by full-width focused pages.

It used fake in-memory state only and was validated at 205 pt wings and a
252 pt bridge across root, runtime, integration, missing-permission, drag,
relaunch-required, and reduced-motion scenarios. Live accessibility validation
also covered page-container focus, editable-control yielding, arrow navigation,
Return, Back, Escape, and absence of scroll semantics.

Compass Rail was selected by the user. Per the prototype contract, the
throwaway package and temporary screenshots are deleted rather than promoted.
