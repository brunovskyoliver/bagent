# bagent UI Design Reference — the notch surface

**Read this before adding any UI.** bagent has exactly one surface: a black shape
that wraps the notch. There is no chat window, no settings window, no menu-bar
status item, and no voice overlay. Everything the user sees is drawn inside that
one shape, and anything new belongs inside it too.

---

## The one-panel rule

| Thing | Where it lives |
|---|---|
| AppKit window | a single `BagentPanel` (`statusPanel`) — borderless, non-activating, `.statusBar` level |
| Its frame | **fixed** at `pillFrame` = `2 × maxWingWidth + notchWidth` × `menuBarH + maxBridgeHeight` |
| The visible shape | `NotchWrapShape`, animated by SwiftUI **inside** that fixed frame |
| Content | `StatusPillView` → `NotchWrapView` → `InlineNotchContent` |

The frame is deliberately oversized and never resized. AppKit resizing during a
shape animation clips the bottom arcs into sharp corners, so the panel stays put
and only the SwiftUI path moves. **Do not call `setFrame` to make the notch grow** —
animate `wingWidth` / `bridgeHeight` instead.

`computeGeometry()` recomputes `pillFrame` only on
`NSApplication.didChangeScreenParametersNotification` (`screensChanged`).

### Notch vs. non-notch displays

The surface is always notch-style. Physical-notch displays use the measured
`auxiliaryTopLeftArea` / `auxiliaryTopRightArea`; external and non-notch displays
draw a centered synthetic 221 pt notch gap (`syntheticNotchWidth`, measured from
Mac17,2). `hasNotch` records which case applies — it changes the geometry source,
never the idiom.

---

## Anatomy

```
┌──────────────────────────────────────────────────────────────────┐  ← top of screen
│  menu bar                                                        │
│        ╔═══════╗              ╔═══════════════╗                  │
│        ║ LEFT  ║  [  NOTCH  ] ║     RIGHT     ║                  │
│        ╚═══╤═══╝              ╚══════╤════════╝                  │
│            └───────── bridge ────────┘                           │
└──────────────────────────────────────────────────────────────────┘
```

- **Left wing** — connector icons / page icon. Grows to fit its icon row: every
  state clamps `targetWing = max(targetWing, requiredLeftWingWidth)`.
- **Right wing** — status dot (hidden when idle + collapsed, so an idle notch is
  pure black), cmux dot.
- **Notch gap** — the physical cutout. bagent draws nothing here.
- **Bridge** — the strip below the notch. Height 0 at idle; it *is* the content
  area in every open state.

Two animatable inputs drive `NotchWrapShape`: `wingWidth` and `bridgeHeight`
(plus `bulgeDepth` / `bulgeSweep` for the paste wheel's dome).

---

## States

`NotchInteractionMode` (on `ChatViewModel`) is the single source of truth for what
the notch is doing. There are no parallel `isExpanded` / `isInputShowing` flags —
they were removed; do not reintroduce them.

| Mode | Entered by | Shows |
|---|---|---|
| `.collapsed` | `collapse()`, Esc, click-away | nothing (black notch) |
| `.input` | `⌥Space` / pill tap → `presentInputOnly()` | text field + source bubbles |
| `.thinking` | submit → `collapseInputForThinking()` | thinking indicator |
| `.output` | first assistant token → `presentOutputChat()` | streamed response |
| `.settings` | `/settings` → `openNotchSettings()` | settings pages |
| `.automations` | `/automations` → `openAutomations()` | automation list / detail / step editor |

Two surfaces preempt the mode switch entirely, in `InlineNotchContent.body`:

1. **A pending approval** — `pendingApprovals.first` beats everything. The daemon
   auto-denies after 60 s, so `NotchWindowController` also opens the notch on its
   own when one arrives (`approvalCancellable`). This is the only place a gated
   write can be allowed; never gate it behind another surface.
2. **WhatsApp QR pairing** — `showWhatsappPairing`.

### Sizing

All points, in `NotchWrapMetrics`:

| State | `wingW` | `bridgeHeight` |
|---|---|---|
| Idle | 32 | 0 |
| Hover | 72 | 22 |
| Input | 221 | 72 |
| Output | 154 (min 72) | 96, grows to 280 |
| Settings | 205 | 252 (setup page: 280) |
| Automations | 205 | 214 |
| cmux banner | 84 | 19 |
| Paste wheel | 196 | 36 + 96 bulge dome |
| **Ceiling** | **260** | **280** |

Never exceed `maxWingWidth` / `maxBridgeHeight` — they define the fixed panel
frame, and anything larger is clipped rather than shown.

### Assistant transcript

- While work is running, the output surface shows one gray current-action row.
- Completed activity collapses to a one-line step count and expands on click
  into chronological tool/search rows; it never exposes hidden chain-of-thought.
- The answer streams independently at an adaptive word cadence.
- Retained web sources appear as numbered links. Inline `[N]` citations are
  clickable only when `N` maps to a daemon-validated HTTP(S) source.

---

## Animation

`refreshSurface()` is the one place that resolves state → target geometry. Call it
whenever something that affects size changes; add an `onChange` rather than
setting `wingWidth` / `bridgeHeight` by hand.

- Surface morph: `surfaceDuration` 0.58 s ease.
- Paste wheel: snappier `.spring(response: 0.34, dampingFraction: 0.82)` — the
  0.5 s hold already cost the user time, so the reveal must feel instant.
- Content fades in at ~62% of the surface morph, so the shape leads and the text
  follows. Never fade content in at t=0; it looks like the notch is bulging around
  already-present text.
- **Reduced motion** (`accessibilityReduceMotion`): every animation has a
  no-motion branch — opacity-only or `nil`. Match this in new code.

---

## Look

Text is soft off-white, **never pure white**:

| Token | Value | Use |
|---|---|---|
| `notchTextPrimary` | `white 0.80` | primary lines |
| `notchTextSecondary` | `white 0.55` | subtitles, labels |
| `notchTextFaint` | `white 0.42` | placeholders, hints |

Surfaces inside the notch are `Color.white.opacity(0.06)` fills with 5–6 pt
corners; buttons are `white.opacity(0.12)`. The background is the notch's own
black — do not add materials, blur, or shadows inside the shape.

---

## Settings

`/settings` opens `.settings` mode. `NotchSettingsContent` renders one page at a
time; `←`/`→` or the header icons switch pages, and the left wing mirrors the
current page icon.

| Page | Carries |
|---|---|
| `general` | paste wheel, cmux notifications |
| `permissions` | Full Disk, mic, screen recording, Accessibility |
| `model` | chat model picker |
| `connectors` | connector status (read-only) |
| `setup` | Odoo credentials, Codex path, WhatsApp pairing, `rules.yaml` editor |

`setup` is the only page that scrolls and the only one at full bridge height —
credentials plus the rules editor do not fit otherwise.

---

## Automations

`/automations` opens `.automations` mode. `AutomationsSurfaceState` (an enum on
`ChatViewModel`, rendered by `AutomationsNotchContent`) selects what shows
inside the mode: `.list` (≈3 upcoming rows + `+`), `.detail`,
`.deleteConfirmation`, and the step editor
(`.editorTask → .editorSchedule → .editorRecurrence → .editorReview →
.editorSaving`). The editor is divided into steps instead of scrolling; every
step fits the 214-pt bridge. Escape steps back one level before collapsing;
↑/↓ + Return drive the list. Long run results are shown by reusing the normal
`.output` presentation (tap the result box in detail). Approval and WhatsApp
QR preemption order is unchanged. See `docs/AUTOMATIONS.md`.

---

## What does NOT go in the notch

It is a **1-second surface**: glanceable, tappable, always visible.

- No long-lived free-form text outside `.input`.
- No long labels or multi-word status messages.
- Nothing needing more than a second of attention.
- No scrollable content — `setup` is the deliberate exception.
- **No new windows.** If a feature seems to need one, it belongs in a settings
  page or it does not belong in bagent.

---

## Accessibility

- Wings carry `accessibilityLabel`; the bridge is decorative.
- Reduced motion via `accessibilityReduceMotion` — shape morphs degrade to
  opacity.
- Toggles and page buttons carry explicit labels (icon-only controls otherwise
  read as "button").

---

## Reference apps

- **Alcove** (Pranjal Satija) — always-visible slim icon-flanked wrap. The idiom
  bagent follows.
- **NotchNook** (Lo.cafe) — hover-expand and the bridge concept.
