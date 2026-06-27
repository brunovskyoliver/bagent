# bagent UI Design Reference

Living document. Future phases (7, 9, 10) and any new UI component should read this first.

---

## Display modes

| Mode | Condition | Resting UI |
|---|---|---|
| **Notch wrap** | `NSScreen.main?.auxiliaryTopLeftArea != nil` | Curved black bar hugging the physical notch |
| **Menu-bar inline** | External display or non-notch Mac | Transparent pill inside menu bar at screen center |
| **Status item fallback** | Non-notch Mac, no external display | `NSStatusItem` at right of menu bar |

The `NotchWindowController.hasNotch` flag drives the branch. External display logic (`screensChanged`) fires on `NSApplication.didChangeScreenParametersNotification` and recomputes geometry.

---

## Notch wrap anatomy

```
┌──────────────────────────────────────────────────────────────────┐  ← top of screen / menu-bar top edge
│  menu bar                                                        │
│        ╔═══════╗              ╔═══════════════╗                  │
│        ║ LEFT  ║  [  NOTCH  ] ║     RIGHT     ║                  │
│        ║  ✦   ║              ║      ⌄         ║                  │
│        ╚═══╤══╝              ╚══════╤═════════╝                  │
│            └──────────────────────┘  ← bottom bridge (hover only)│
└──────────────────────────────────────────────────────────────────┘
```

- **Left wing** — from `auxiliaryTopLeftArea.maxX - wingW` to `auxiliaryTopLeftArea.maxX`, full menu-bar height.
- **Right wing** — from `auxiliaryTopRightArea.minX` to `auxiliaryTopRightArea.minX + wingW`, same height.
- **Notch gap** — the physical camera cutout; `tr.minX - tl.maxX`. bagent draws nothing here (click-through).
- **Bottom bridge** — thin strip below the notch connecting the two wings. Hidden at idle, appears on hover, becomes the top chrome when expanded.

Sizing constants (all in points):

| State | `wingW` | `bridgeHeight` |
|---|---|---|
| Idle | 32 | 0 |
| Hover | 96 | 8 |
| Expanded | `chatWidth / 2` (200) | full chat height (520) |

---

## `NotchWrapShape`

Custom `Shape` in `NotchWrapShape.swift`. Inputs drive the path:

- `notchWidth` — fixed, from geometry.
- `notchHeight` — fixed, equals `menuBarH`.
- `wingWidth` — animatable (`AnimatablePair` left).
- `bridgeHeight` — animatable (`AnimatablePair` right).
- `outerCornerRadius` ≈ 10 pt — where the wing meets the outer menu-bar edge.
- `innerCornerRadius` ≈ 8 pt — where the wing meets the notch cutout (matches physical notch rounding on M-series).

The path draws a U-shape (open at the top) that wraps left wing → bridge → right wing. When `bridgeHeight == 0` the bridge segment degenerates to a point and the two wings are visually separate.

---

## Animation language

Three-phase expand (total ≈ 320 ms):

| Phase | Time | What happens |
|---|---|---|
| A — spread | 0–120 ms | Wings grow horizontally to full chat width; `wingWidth` springs out |
| B — drop | 80–280 ms (overlaps A) | `bridgeHeight` springs down to full chat height; outer corner radius eases from 10 → 16 |
| C — content | 180–320 ms | `ExpandedChatView` fades + scales in from 0.96 → 1.0, anchored at notch top-center |

Collapse is phases in reverse: C → B → A.

Spring params (both phases): `response: 0.32, dampingFraction: 0.72`.

**Reduced-motion fallback** (`UIAccessibility.isReduceMotionEnabled` / AppKit equivalent): skip phases A+B entirely; do a simple cross-fade (opacity only) over 180 ms.

---

## Iconography slots

| Slot | Idle | Hover | Expanded |
|---|---|---|---|
| **Left wing** | `sparkles` (0.7 opacity) | `sparkles` (1.0 opacity) | (hidden — panel chrome takes over) |
| **Right wing** | `chevron.down` (0.7 opacity) | `chevron.down` (1.0 opacity) | `xmark.circle.fill` (tap to collapse) |

Status overlays (appear on top of left-wing slot, not replacing it):

- `brain` badge — `memory_saved` ACK from daemon → fades out after 2 s.
- `shield.lefthalf.filled` badge (orange) — pending approval count > 0.

These badges live in `ChatViewModel` and are read by `NotchWrapView` directly.

---

## Reference apps

- **NotchNook** (Lo.cafe) — idle border only, hover/drag expands downward symmetrically. Inspiration for hover-expand idiom and bridge concept.
- **Alcove** (Pranjal Satija) — permanent slim icon-flanked wrap, click expands with curved top edge retained. We mirror this exact pattern.

We chose Alcove's always-visible idle state (confirmed by user preference: "thin black wrap, icons visible").

---

## Future hooks (reserved icon/badge slots)

| Trigger | Slot | Phase |
|---|---|---|
| Screen context active (Phase 7) | Right wing: `viewfinder` icon while capturing | Phase 7 |
| Codex artifact ready (Phase 8) | Bottom bridge: download strip with filename | Phase 8 |
| Tool-call in flight | Left wing: progress spinner replaces sparkles | Phase 5+ |
| Approval pending | Left wing: orange shield badge | Phase 5 ✅ already wired |

Reserve the bridge area for transient content only — it should never carry permanent UI.

---

## Spotlight input surface

When the user opens bagent while no assistant output is being generated, the first surface is
an input-only command field rather than the full chat panel.

- **Idle open** — notch/status click opens a wide Spotlight-like input below the notch.
- **Voice mode enabled** — single `⌥Space` opens voice; double `⌥Space` opens this input.
- **Voice mode disabled** — single `⌥Space` opens this input.
- **Send** — input collapses back into the notch; the existing blue status dot signals pre-token work.
- **First token** — full chat opens automatically once assistant output begins.
- **Thinking manual open** — during pre-token work, notch/status click or shortcut may open the full chat manually.
- **Source modes** — the input can shrink from the right to reveal the four most-used source bubbles. Defaults are Mail, Files, WhatsApp, Odoo; `⌘1`-`⌘4` select the visible modes.

The input uses a liquid-glass-style material on current macOS builds. Native Liquid Glass should replace the fallback material when the app is built with an SDK that exposes those APIs.

---

## What NOT to put in the wrap

The notch wrap is a **1-second UI surface**: glanceable, tappable, always visible. Do not put:

- Long-lived free-form text input inside the always-visible wrap itself (idle entry belongs in the separate Spotlight input surface)
- Long labels or multi-word messages
- Anything requiring > 1 s of user attention
- Scrollable content
- Modal dialogs or confirmation flows (→ `ApprovalModalOverlay` inside expanded panel)

---

## Accessibility

- Left wing: `accessibilityLabel("bagent — apliácia")`, `accessibilityHint("Otvoriť chat")`
- Right wing: `accessibilityLabel("Rozbaliť chat")` / `"Zbaliť chat"` based on state
- Bottom bridge: not focusable (decorative)
- Full expanded panel: standard `accessibilityElement(children: .contain)` on the container
- Reduced-motion: read via `NSWorkspace.shared.accessibilityDisplayShouldReduceMotion`; when `true`, skip shape morph, use `opacity` transition only

---

## Voice Input (Phase 5G)

Two distinct surfaces, one shared `SpeechController` (WhisperKit, on-device).

### Voice-only overlay (⌥Space, collapsed)
- Pops from the notch using the **same charge→pop timing as the chat panel**
  (`pillHovered` spring, then panel after 150 ms) — `NotchWindowController.presentVoice()`.
- `VoiceOverlayView` (360×240): `.regularMaterial` rounded rect, subtle white stroke.
  - `waveform` SF Symbol with `.symbolEffect(.variableColor.iterative.dimInactiveLayers.reversing, options: .repeating, isActive:)`.
    *(`.repeating` is the macOS-14 form of `.repeat(.continuous)`, which is macOS 15+.)*
  - `SiriWaveView` — `TimelineView(.animation)` + `Canvas`, 3 layered translucent sine
    bands whose height tracks `speech.amplitude` (from WhisperKit `bufferEnergy`).
  - Live transcript: last ~2 sentences; each `Text` keyed by `.id(sentence)` with an
    asymmetric fade+move transition (insertion from bottom, removal to top), animated with
    `.spring(response: 0.32, dampingFraction: 0.78)`.
- Auto-finalizes on ~1.2 s silence → **morphs into the chat window** (`voiceToChatHandoff`):
  overlay hides, chat expands, transcript is submitted via the normal `send()` pipeline.
- Escape / click-away cancels (`onExitCommand` + global mouse monitor, mirroring the chat panel).

### Inline mic (chat input bar)
- `VoiceAttachControl`: the existing `+` attachments button with a `mic.fill` button that
  springs **up above it** on hover or while recording
  (`.spring(response: 0.28, dampingFraction: 0.68)`) — two stacked icons, mic on top.
- Clicking the mic does **not** open the overlay; it records inline, pulses the mic with
  `.symbolEffect(.pulse.byLayer, options: .repeating, isActive:)`, and live-fills the text
  field. The final transcript is editable like typed text; send normally.

### Hotkey
- Single ⌥Space (collapsed) → voice overlay **instantly**.
- Second ⌥Space within ~350 ms → dismiss voice, open chat (`AppDelegate.handleHotkey`).
- ⌥Space while chat open → collapse (unchanged).

### Reduced motion
- `SiriWaveView` falls back to a static amplitude capsule (no `Canvas`/`TimelineView`).
- Transcript transitions drop to `nil` animation.
