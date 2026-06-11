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

## What NOT to put in the wrap

The notch wrap is a **1-second UI surface**: glanceable, tappable, always visible. Do not put:

- Free-form text input (→ belongs in `ExpandedChatView` input bar)
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
