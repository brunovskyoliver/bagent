# Stage 7B Compass Rail acceptance

Status: validation complete; administrative commit, push, issue closure, and Wayfinder summary are pending.

## Scope and provenance

- Fixed starting commit: `752395c4112322d56ebb9af0c99b0b73548882fb`
- Implementation ticket: [#34](https://github.com/brunovskyoliver/bagent/issues/34)
- Native blocker: closed Stage 7A [#33](https://github.com/brunovskyoliver/bagent/issues/33)
- Wayfinder map: [#15](https://github.com/brunovskyoliver/bagent/issues/15)
- Branch: `t3code/basert-notch-automation-ux`
- Ticket state: open pending administrative closure; no Stage 7C work started

The ticket was searched across open and closed issues before creation, then created once with `ready-for-agent` and `wayfinder:task`, assigned to `brunovskyoliver`, linked as a native dependency on #33, and added under map #15.

## Authority and route inventory

The settings surface now has one writable route, `ChatViewModel.compassRailRoute`, with these four peers in order:

1. General
2. Model and Runtime
3. Integrations
4. Privacy and Permissions

The child destinations are WhatsApp, Odoo, Codex, Full Disk Access, Screen Recording, Accessibility, and Rules and approval policy. `/settings` resets to General. The rail remains present for children, while the left-wing icon derives from the route's top-level area. The deterministic catalog contains 11 routes, all 8 model-runtime phases with baseline, active-lease, waiting-for-residency, and changed-PID recovery variants, 5 validation states, and 2 permission states.

Stage 7B presents existing projections and commands for model runtime, connectors, permissions, rules, and automation state. It does not add a second authority or change recurrence semantics. It does not implement Stage 7C permission rechecks, System Settings routing, drag payloads, TCC mutation, or relaunch behavior.

## Removed legacy settings

- `NotchSettingsPage` and its five routes, including Setup
- setup-only 280 pt bridge geometry
- scrolling settings content and raw `rules.yaml` editing
- legacy connector settings layout
- obsolete provider/Ollama/fallback presentation
- duplicate settings-selection state

Ordinary settings remain in the fixed 205 pt wings and 252 pt bridge. The 74 × 18 pt status pill stays at y=9 and 12 pt from the visible settings wing edge for synthetic narrow and wide notch fixtures.

## Gate evidence

### A43

Command:

```sh
swift test --package-path apps/macos --filter CompassRailTests
```

Result: PASS — 11 tests, 0 failures.

Covered: four-peer order, exact top-level and permission-assist routes, General opening, selection persistence through an authoritative projection update, direct rail selection from children, wrapping, editable-control yielding, child Left/Right yielding, Back/Escape, focus restoration, approval/QR preemption, left-wing mirroring, fixed status-pill geometry, synthetic narrow/wide fixtures, and absence of Setup/fifth-peer routes.

### A44

Command:

```sh
scripts/acceptance/settings-authority.sh
```

Result: PASS — 5 measured production surfaces, 12 assertions, 0 failures. The disposable seeded Setup route was rejected, proving red capability.

The recurrence checks passed without changing automation behavior:

- `swift test --package-path apps/macos --filter AutomationRecurrenceDraftTests`: 3 passed
- `swift test --package-path apps/macos --filter AutomationsSurfaceTests`: 1 passed
- `swift test --package-path apps/macos --filter AutomationsSurfaceStateTests`: 6 passed
- `cargo test -p bagent-automations`: 17 passed; doc tests 0 passed

The existing `AutomationsSurfaceStateTests` suite was not renamed or replaced; the named acceptance contract runs the required filter and checks the authoritative automation surface plus recurrence label.

### A45 visual fixture

Command:

```sh
scripts/acceptance/settings-catalog.sh
```

Result: PASS — a disposable signed bundle rendered 4 variants × 2 synthetic panel widths × 57 route/state fixtures = 456 PNGs at fixed 318 pt height, with 7 keyboard/focus/accessibility assertions. The variants were default, larger text, increased contrast, and Reduce Motion. The widths were 701 pt and 941 pt; settings geometry remained 205 pt wings and 252 pt bridge in both fixtures.

- default
- larger supported text size
- increased contrast
- Reduce Motion

The fixture renders all 8 model-runtime phases across baseline, active-lease, waiting-for-residency, and changed-PID recovery, plus 5 validation states and 2 permission states, with 57 route/state fixtures per synthetic width. High contrast uses the disposable fixture's explicit `compassRailHighContrast` environment and visual contrast modifier, and the fixture asserts the standard and high-contrast palette ratios; no system display preference was changed. Keyboard routing and focus ownership are exercised by the deterministic Compass Rail contract and fixture interaction assertions. Sampled bitmap edge probes and fitting bounds provide bounded-layout evidence.

The bundle was signed with `Apple Development: obrunovsky7@gmail.com (D63PW2838J)` and passed `codesign --verify --strict`.

### A45 live accessibility fixture

Command:

```sh
BAGENT_STAGE7B_AX_ACCEPTANCE_DIR='/tmp/bagent-stage7b-acceptance-34' scripts/acceptance/settings-accessibility.sh --run
```

Result: PASS — the stable disposable Apple Development-signed fixture queried the live Accessibility API across 57 route/state cases with 1,043 live elements and 634 assertions; skipped count was 0. The fixture's JSON evidence was privacy-safe and contained no prompts or model output. The exact fixture path remained `/tmp/bagent-stage7b-acceptance-34/settings-ax-fixture.app`.

- bundle identifier: `sk.bagent.stage7b.ax.fixture`
- Team ID: `QUB47S3XTF`
- designated requirement: `identifier "sk.bagent.stage7b.ax.fixture" and anchor apple generic and certificate leaf[subject.CN] = "Apple Development: obrunovsky7@gmail.com (D63PW2838J)" and certificate 1[field.1.2.840.113635.100.6.2.1] /* exists */`
- evidence: `/tmp/bagent-stage7b-acceptance-34/live-ax-evidence/live-ax.json`
- pregrant check: the exact signed fixture initially reported `accessibility_available=false` and required the manual grant; the final run reported `true`
- verified: four peer buttons in order, selected state, headings, Back labels, values, focus order and keyboard routing, editable-control arrow ownership, Back focus return, and absence of scroll semantics
- the fixture performed no daemon, BaseRT, TCC, System Settings, port, lease, or product-state mutation

The source-level accessibility assertions remain present alongside the live result: one Compass Rail group, four named peer buttons, selected state independent of color, headings, Back labels, concise values, hidden decorative icons/dividers, Reduce Motion handling, and no scroll semantics.

## Regression results

- Full Swift suite: PASS — 117 tests, 0 failures, 1 existing permission-gated skip
- Full Rust workspace: PASS
- `cargo fmt --all -- --check`: PASS
- strict Clippy: PASS with `-D warnings`
- Swift release build: PASS
- `git diff --check`: PASS
- notch mode authority: PASS
- Stage Rail signed/accessibility authority: PASS with its existing AX-permission skip
- Current Chat authority: PASS
- signed Stage 7A Current Chat relaunch fixture: PASS — `A41_SIGNED_UI_RELAUNCH_PASS`, real lease count 1, real admitted Work, unchanged disposable daemon and BaseRT PIDs across daemon restart, unchanged protected ports
- Model Runtime authority: PASS
- Work authority: PASS
- localization catalog: PASS — valid JSON, 115 keys, all new Compass Rail copy checked in `en` and `ja`

The disposable lease diagnosis was repeated and minimized before the fix. The fixture-only policy boundary was corrected, and a regression now proves the Stage 7A acceptance fixture uses the isolated chat execution policy. No protected runtime, database, TCC state, accessibility preference, or display preference was changed.

## Review state

Two independent final reviews were run against fixed commit `752395c4112322d56ebb9af0c99b0b73548882fb`:

1. Standards review against repository rules
2. Spec review against Stage 7B and accepted decisions

The prior review findings were resolved in the final live fixture and production monitor seam. Final independent reviews against fixed commit `752395c4112322d56ebb9af0c99b0b73548882fb` both passed with zero unresolved findings:

- Standards review: PASS — zero unresolved findings
- Spec review: PASS — zero unresolved findings

The signed visual fixture uses an explicit disposable high-contrast environment because macOS SwiftUI exposes no writable native high-contrast environment; its contrast assertions and live AX checks are recorded separately. The repaired signed Stage 7A fixture proves a real admitted Work and lease. After evidence capture, the disposable fixture app and task temporary directories were removed. Its Accessibility entry must be removed manually in System Settings → Privacy & Security → Accessibility; no TCC reset or mutation was performed.
