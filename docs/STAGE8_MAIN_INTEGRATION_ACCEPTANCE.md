# Stage 8 main-integration acceptance

Date: 2026-08-24

Revision under test: the merge of `e72d28bf` (roadmap) and `342be486` (main).

This record covers the integration campaign only. It does not restate or reuse
the verdicts in `docs/STAGE8_COMPATIBILITY_RELEASE_ACCEPTANCE.md`; that record
proves `e72d28bf` and is preserved unchanged.

## Support limitations

Unchanged from the Stage 8 campaign: signed and live qualification covers
macOS 26 only; macOS 14 and 15 are compile-only targets; live TCC grant, denial,
revocation and drag-to-System-Settings mutation stay outside the campaign and
are never reported as PASS. The owner of port 8080 and the user's production
database are outside every fixture.

## Migration renumbering

`main` shipped V15-V18 and those migrations are applied on real databases, so
they are kept byte-identical. The roadmap's V15-V23 are renumbered to V19-V27.
The user's database was inspected read-only before any work and carries main's
sequence exactly:

```
15 automation_reference_blocked
16 conversational_reference_resolution
17 reconcile_provider_authorization
18 notifications
```

## A54 rollback: rewritten around the released predecessor

`scripts/acceptance/stage8-rollback-qualification.sh` builds its old candidate
from the fixed Stage 7C commit `45c26b1c`. That build numbers
`work_coordinator_foundations` as V15. The integration deliberately renumbers it
to V19 so main's already-shipped V15 keeps its number, which makes any database
created by a pre-merge roadmap build incompatible with the merged build:

```
migration error: applied migration V15__work_coordinator_foundations
is different than filesystem one V15__automation_reference_blocked
```

Re-running with `STAGE8_FIXED_BASE=342be486` does not work either: main's
`bagentd` has no `stage7a-acceptance` feature, which the fixture requires.

No pre-merge roadmap build was ever released, and the user's database carries
main's sequence, so no real database is affected. The gate as written presumes
an older roadmap build whose numbering this integration invalidates on purpose.
The gate now defaults its old candidate to `342be486` and compiles only the
acceptance features that base declares, so a genuine released predecessor is
buildable. `STAGE8_FIXED_BASE` still overrides it. With that change A54 passes.
No pre-merge roadmap build was ever released and the user's database carries
main's sequence, so no real database is affected.

## Executed gates

| Gate | Command | Result |
| --- | --- | --- |
| Format | `cargo fmt --all -- --check` | PASS |
| Lint | `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| Rust tests | `cargo test --workspace --no-fail-fast` | PASS, 733 executed |
| Swift build | `swift build --package-path apps/macos` | PASS |
| Swift tests | `swift test --package-path apps/macos` | PASS, 173 XCTest + 45 Swift Testing |
| Whitespace | `git diff --check` | PASS |
| Bundle | `make -C apps/macos bundle` | PASS, signed with the Apple Development identity |
| Signed bundle | `scripts/acceptance/signed-bundle-verification.sh` | PASS, now also verifying nested `bagent-browser-mcp` |
| Codesign | `codesign --verify --deep --strict` | PASS |
| A51 authority | `scripts/acceptance/final-authority-inventory.sh` | PASS, 0 findings, 8 canonical assertions, 13 seeded red matches |
| A52 migration | `cargo test -p bagentd --test persistence_migration` | PASS, 5 executed |
| A53 restart | `scripts/acceptance/stage8-migration-restart.sh` | PASS |
| A54 rollback | `scripts/acceptance/stage8-rollback-qualification.sh` | PASS against the released predecessor (see below) |
| A55 privacy | `scripts/acceptance/stage8-privacy-scan.sh apps/macos/bagent.app` | PASS, 0 canary matches in production captures |
| A56 visual | `scripts/acceptance/stage8-visual-qualification.sh apps/macos/bagent.app` | PASS, 11 notch states, 2 split-view states, 57 fixtures x 2 widths x 6 variants, 74 transition frames |
| A57 accessibility | `scripts/acceptance/stage8-accessibility-qualification.sh apps/macos/bagent.app` | PASS |
| A58 active-load relaunch | `stage8-active-load-relaunch.sh`, `ui-relaunch-handoff.sh` | PASS |
| A59 live smoke | `scripts/acceptance/stage8-live-smoke.sh apps/macos/bagent.app` | **NOT PASSING** (see below) |
| Settings catalog | `scripts/acceptance/settings-catalog.sh` | PASS, 4 variants x 2 widths x 57 fixtures |
| Accessibility audit | `scripts/acceptance/accessibility-audit.sh` | PASS |
| Work authority | `scripts/acceptance/work-authority.sh` | PASS |
| Model Runtime authority | `scripts/acceptance/model-runtime-authority.sh` | PASS |
| Current Chat authority | `scripts/acceptance/current-chat-authority.sh` | PASS |
| Settings authority | `scripts/acceptance/settings-authority.sh` | PASS |
| Notch mode authority | `scripts/acceptance/notch-mode-authority.sh` | PASS |
| Cutover rollback | `scripts/acceptance/work-cutover-rollback.sh` | PASS |
| Documentation links | `scripts/acceptance/documentation-links.sh` | PASS |
| Localization | `scripts/acceptance/settings-localization.sh` | PASS |

## A59 live smoke: NOT PASSING

A59 is an observational live gate: it requires a disposable 4B automation to
produce both a Think and a Tool stage inside one 12-second projection window,
and at least one Automation Session to carry a non-empty activity timeline.

Measured on this machine, macOS 26.5.2:

- roadmap `e72d28bf`: 2 of 3 runs pass
- integration commit: 0 of 5 runs pass, failing at the Think/Tool observation or
  the activity-timeline assertion

The gate exposed one genuine defect, now fixed: an externally managed BaseRT hit
the managed-port restart guard and poisoned the runtime, so every later demand
failed and all disposable automations died with a model error. That fix moved
the failure from the first projection assertion to later ones.

The code paths the remaining assertions cover are byte-identical to the roadmap:
`safe_activity_from_event`, `safe_activity_timeline`, the drain, and the
terminalization ordering all match `e72d28bf` exactly, and the recorded
`tool_call` audit entries are identical in shape. The residual difference is
that the disposable automations complete less reliably here: a run is left in
`running`, or a transient `BaseRT stream read` error occurs, so no run reports
`tool_calls_used > 0` and the timeline fallback never fires.

**A59 is therefore recorded as NOT PASSING on the integration commit.** It is
not claimed as a pass, and no rule was relaxed to make it one.

## Not yet executed

A60 clean-checkout reproducibility has not been run against the integration
commit; the old A60 record proves `e72d28bf` only. The two independent reviews
have not run. Until A59 is green or explicitly accepted as a limitation, and
A60 and the reviews complete, the integration is not qualified for merge.

## Defects found and fixed during integration

- V24 rebuilt `automation_runs` without main's `reference_outcome_code`, and
  forced a schema re-parse that exposed main's V17 continuation trigger
  comparing `confirmation.expires_at_ms`, a column its target table lacks. Every
  fresh database failed to migrate.
- `prepare_pre_cutover_backup` gated on `version >= 16`, which after renumbering
  is no longer the Unified Work cutover. A database at main's V18 ceiling would
  have skipped its verified pre-cutover backup. Red evidence recorded first.
- `finalize_stage8_cleanup` dropped `sessions` and `automation_runs` while seven
  of main's resolver tables still had foreign keys into them, leaving the schema
  unreadable and preventing daemon start.
- The A51 detector classified a whole-tree `cfg(test)` module as production and
  matched an unrelated `BrowserProfile.clear()`; both corrected without
  weakening the red-capability self-tests.
