# Spike: Notch Geometry

**Date:** 2026-06-11
**Device:** MacBook Pro Mac17,2, Apple M5, 32 GB RAM, macOS 25.5.0

---

## Measured Values (Mac17,2 / M5)

```
Screen frame (logical pts):   w=1800   h=1169
Visible frame:                x=0      y=0      w=1800   h=1130
Menu bar height:              39 pt

NSScreen.auxiliaryTopLeftArea:
  x=0      y=1131   w=791    h=38

NSScreen.auxiliaryTopRightArea:
  x=1012   y=1131   w=788    h=38

Derived notch region:
  x=791    y=1131   w=221    h=38
  (notchLeft = 0+791 = 791, notchRight = 1012, notchWidth = 1012-791 = 221)
```

### Key Measurements

| Property | Value |
|---|---|
| Screen width | 1800 pt |
| Screen height | 1169 pt |
| Notch width | **221 pt** |
| Notch height | **38 pt** (= menu bar height) |
| Notch center X | **901.5 pt** (=791 + 221/2) |
| Left auxiliary area width | 791 pt |
| Right auxiliary area width | 788 pt |
| Safe inset from notch edges | 10 pt recommended |

---

## NSPanel Positioning Strategy

### Option A: Center pill in notch region (recommended for MVP)

```swift
let notchX: CGFloat = 791.0       // auxiliaryTopLeft.width
let notchWidth: CGFloat = 221.0   // derived
let notchHeight: CGFloat = 38.0
let screenHeight: CGFloat = 1169.0

// Pill dimensions (collapsed state)
let pillW: CGFloat = 120.0
let pillH: CGFloat = 22.0

// Position: centered in notch, vertically centered in menu bar
let pillX = notchX + (notchWidth - pillW) / 2.0   // = 791 + 50.5 = 841.5
let pillY = screenHeight - notchHeight + (notchHeight - pillH) / 2.0  // = 1131 + 8 = 1139

// In screen coordinates (y=0 at bottom-left on macOS):
// pillOrigin = NSPoint(x: pillX, y: pillY)
```

### Option B: Full-width notch overlay (more aggressive)

Use `NSPanel` sized to exactly the notch: `x=791, y=1131, w=221, h=38`.
Covers the notch entirely. May conflict with system notch rendering.
Not recommended without testing on multiple units.

### Expanded Panel Position

```swift
// Expanded chat panel drops below notch
let panelW: CGFloat = 400.0
let panelH: CGFloat = 520.0
let panelX = notchX + (notchWidth - panelW) / 2.0  // centered on notch
let panelY = screenHeight - notchHeight - panelH    // just below menu bar
```

---

## NSPanel Configuration

```swift
let panel = NSPanel(
    contentRect: NSRect(x: pillX, y: pillY, width: pillW, height: pillH),
    styleMask: [.borderless, .nonactivatingPanel, .hudWindow],
    backing: .buffered,
    defer: false
)
panel.level = .mainMenu           // sits above regular windows, at menu bar level
panel.isOpaque = false
panel.backgroundColor = .clear
panel.hasShadow = false           // no shadow on collapsed pill
panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
panel.isMovable = false           // pill doesn't move
```

**Note:** `.mainMenu` level confirmed to render above regular app windows. Test against:
- [ ] Full-screen apps (expected: panel hides correctly — check `NSScreen.screens` changes)
- [ ] Mission Control (expected: panel stays pinned)
- [ ] External display (expected: panel only on primary/notch display)

---

## Coordinate System Note

macOS coordinate system: **y=0 at bottom-left**. The notch area is at the **top** of the screen, so `y` values are near `screenHeight`.

When setting `NSPanel.setFrameOrigin(_:)`, pass the **bottom-left corner** of the window frame. For a panel at the top of the screen: `y = screenHeight - panelHeight - topInset`.

---

## Non-Notch Mac Fallback

When `NSScreen.main?.auxiliaryTopLeftArea == nil`, fall back to `NSStatusItem`:

```swift
if NSScreen.main?.auxiliaryTopLeftArea == nil {
    // no notch — use status bar icon
    statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
    statusItem.button?.image = NSImage(systemSymbolName: "sparkles", ...)
}
```

---

## Open Questions / TODOs

- [ ] Verify behavior on MacBook Air M2/M3 (no notch — `auxiliaryTopLeft` should be nil).
- [ ] Test on MacBook Pro 14" M4 (Mac16,x) — notch may differ slightly from M5.
- [ ] Test panel z-order when a full-screen app is active on the notch display.
- [ ] Verify `NSScreen.auxiliaryTopLeftArea` returns correct value on external display (notch display as secondary).
- [ ] Check if `.hudWindow` styleMask is needed or if `.borderless` alone is sufficient.
- [ ] Measure physical pixel values: Mac17,2 display is likely 2× retina, so 221 logical pt = 442 physical px.
