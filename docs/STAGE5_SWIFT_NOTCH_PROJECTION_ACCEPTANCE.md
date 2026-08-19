# Stage 5 Swift Notch Projection acceptance

Date: 2026-08-19  
Issue: #31  
Map: #15  
Dependency: closed #30

## Scope

Stage 5 replaces the writable Swift presentation flags and the legacy global-event consumer with one pure `NotchProjection`. The reducer accepts a fenced authoritative Work snapshot, ordered revisioned Work events, and local navigation intent. `NotchInteractionMode` is stored only in the reducer result.

The daemon projection exposes only opaque Work structure, an allowlisted current activity category, saved Automation names, queue/FIFO structure, pending approval identity, and authoritative Model Runtime phase. Tool activity is committed through the Work Coordinator and clears after each call. Raw names, arguments, prompts, evidence, outputs, provider errors, credentials, and private identities are not part of the compact schema.

Stage 6 Automation Session history, retention, continuation, deletion, and split-view work remains out of scope.

## Fixed surface

- One existing non-activating `BagentPanel`; AppKit keeps its maximum frame fixed.
- Maximum width remains `2 × 260 pt + notchWidth`; maximum height remains `menuBarHeight + 280 pt`.
- Physical notch geometry remains measured; the existing 221 pt synthetic gap remains the fallback.
- `NotchWrapShape` remains the only visible black shape.
- `refreshSurface()` remains the state-to-geometry resolver.
- Activity Peek uses 248 pt wings and the accepted 0/78/98/126/150/176 pt bridge heights.
- The status pill is fixed at 74 × 18 pt with origin `x = maxPanelWidth - (260 - 248) - 74 - 12`, `y = 9`.
- No material, blur, shadow, window, popover, sheet, menu-bar item, or notification was added.

## Gate evidence

| Gate | Evidence | Result |
|---|---|---|
| A27 | `swift test --package-path apps/macos --filter NotchProjectionTests` | PASS: every Work state, exact revision, deterministic replay, ordered events, duplicate suppression, gap and revision rejection. |
| A28 | `swift test --package-path apps/macos --filter EventConsumerRecoveryTests` | PASS: cursor, schema, revision, generation, server-gap, and reconnect recovery use one stable consumer fence and exactly one replacement snapshot. |
| A29 | `scripts/acceptance/notch-mode-authority.sh` | PASS: no writable parallel notch lifecycle flags and no Swift `/events` consumer. |
| A30 | `swift test --package-path apps/macos --filter StageRailTests`; `scripts/acceptance/capture-notch-states.sh` | PASS: focus/pill priority, FIFO cycling, all bridge heights, fixed anchor, normal/Reduce Motion contract, and 11 rendered state fixtures. |
| A31 | `swift test --package-path apps/macos --filter ProjectionPrivacyTests`; daemon allowlist unit test | PASS: unknown fields fail closed, unknown activities become generic, and forbidden canaries do not enter labels, accessibility values, or errors. |
| A32 | `scripts/acceptance/accessibility-audit.sh` | PASS: signed bundle verification, Button/Return semantics, explicit labels and values, decorative hiding, stable source focus, semantic rail fonts, reduced-motion values, contrast checks, and the rendered state catalog. |

The generated PNG catalog is written under `apps/macos/.build/notch-state-catalog/` and is not committed. It covers idle, queued, loading, thinking, tool, approval, streaming, completion, failure, cancellation, and interruption. The catalog was visually inspected for clipping, shape/pill anchoring, readable selected and unselected rail stages, and absence of extra surfaces.

## Repository validation

The final validation set is:

```bash
swift test --package-path apps/macos
swift build --package-path apps/macos
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
scripts/acceptance/notch-mode-authority.sh
scripts/acceptance/capture-notch-states.sh
scripts/acceptance/accessibility-audit.sh
git diff --check
```

The signed fixture bundle is assembled locally at `apps/macos/bagent.app`. It is not launched and does not replace or attach to an installed bagent runtime.
