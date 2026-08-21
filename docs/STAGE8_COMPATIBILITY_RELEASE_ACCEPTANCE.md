# Stage 8 Compatibility and Final Release Acceptance

Status: current campaign evidence. This document is separate from the
historical conditional record in
`docs/STAGE8_CANONICAL_FINAL_ACCEPTANCE.md`; that record is preserved and its
old conditional verdict is not reused for this campaign.

## Campaign boundary

- Fixed Stage 7C comparison commit: `45c26b1c1d3bd482b144525723a9c71a1fe57ced`
- Implementation ticket: [#38](https://github.com/brunovskyoliver/bagent/issues/38)
- Wayfinder map: [#15](https://github.com/brunovskyoliver/bagent/issues/15)
- Closed dependency: [#36](https://github.com/brunovskyoliver/bagent/issues/36)
- Required bundle identifier: `sk.bagent.app`
- Required Team ID: `QUB47S3XTF`
- Signed/live qualification OS: macOS 26 only
- Compile-only targets: macOS 14 and 15, only when the existing configuration permits

The campaign does not grant, deny, revoke, reset, or otherwise mutate live TCC
permissions. It does not use `tccutil reset`, grant Accessibility to the
production bagent app, or perform a drag-to-System-Settings mutation. Those
checks are explicitly omitted and are never PASS. The owner of port 8080 and
the user's production database are outside all disposable qualification
fixtures.

## Candidate and environment

The final candidate commit, signed bundle hash, toolchain versions, command
timestamps, test counts, cleanup, and final runtime state are recorded in the
A60 section below after the release commit is frozen.

Observed qualification environment for signed/live gates:

- macOS 26.5.2, build 25F84, Apple silicon
- Apple Development identity: `Apple Development: obrunovsky7@gmail.com (D63PW2838J)`
- Team ID: `QUB47S3XTF`
- Rust/Cargo and Swift toolchain versions are recorded by the reproducibility gate

## A51–A60 matrix

The A51–A60 matrix in
`docs/IMPLEMENTATION_SEQUENCE_ACCEPTANCE_GATES_DECISION.md` is authoritative.
Each row below records an executed command and its result. No omitted,
conditional, blocked, or inferred result is called PASS.

| Gate | Executed evidence | Current result |
| --- | --- | --- |
| A51 | `scripts/acceptance/final-authority-inventory.sh`; canonical authority subgates and production inventory | PASS: capability detector found 12 seeded forbidden matches, including the retired prompt/debug result path; four authority subgates passed; production inventory found 0 findings and 8 canonical assertions |
| A52 | `cargo test -p bagentd --test persistence_migration clean_and_v14 -- --exact` | PASS: 1 executed, 1 passed, 0 failed; disposable empty and V14 databases converged to the canonical schema and invariants, with kind-only approval provenance and no private automation identity |
| A53 | `cargo test -p bagentd --test persistence_migration interrupted_migration -- --exact`; `cargo test -p bagentd --test work_concurrency crash_recovery -- --exact` | PASS: 2 executed, 2 passed, 0 failed; before-transaction, during-copy, after-commit, before-admission, retry, canonical recovery, and crash-recovery checks ran |
| A54 | `scripts/acceptance/stage8-rollback-qualification.sh apps/macos/bagent.app` | PASS: disposable old/new signed candidates and databases; fixed-base refusal, pre-Work verified backup, post-Work archive-and-restore, hashes, and cleanup recorded; integration backup `39b40193d8583f0abe188a38dad1f9f8aa3ed011277b55cef182d7a43ac5e9f2`, pre-Work `623c1252a32eeac496129f0e4b45ae46da8bd1d5793ae3276e86289ca1f588f1`, archive `61d44bf0e6717743e97f832efea9144c65bb1d9c1e5a582e32ac5ecda766b7d0` |
| A55 | `scripts/acceptance/stage8-privacy-scan.sh`; `cargo test -p bagentd --test privacy_contract -- --nocapture`; `swift test --package-path apps/macos --filter ProjectionPrivacyTests`; `swift test --package-path apps/macos --filter UIRelaunchHandoffTests` | PASS: 9 surfaces, 9 synthetic canaries, 9 scanner detections, 0 sanitized projection matches; disposable captures securely deleted; Rust privacy contract 1 executed and passed, Swift privacy projection 4 executed and passed, UI relaunch handoff privacy 5 executed and passed |
| A56 | `scripts/acceptance/stage8-visual-qualification.sh apps/macos/bagent.app` | PASS: signed macOS 26 candidate; 11 notch states; 57 settings fixtures × 2 widths across light/dark/high-contrast/large-text/reduced-motion; status-pill anchor and identity verified |
| A57 | `scripts/acceptance/stage8-accessibility-qualification.sh apps/macos/bagent.app` | PASS: signed macOS 26 live AX fixture; 2 notch states, 5 notch assertions, 57 settings routes, 634 settings assertions, 0 skipped live assertions; hosted XCTest AX check is recorded as SKIPPED, not PASS |
| A58 | `scripts/acceptance/stage8-active-load-relaunch.sh apps/macos/bagent.app`; `scripts/acceptance/ui-relaunch-handoff.sh apps/macos/bagent.app` | PASS: A18–A21, poison/8080 isolation, and signed A49 UI-only relaunch all executed nonzero; daemon/BaseRT stability, Work/model convergence, protected port sentinel, and historical A50 scanner verified |
| A59 | `scripts/acceptance/stage8-live-smoke.sh apps/macos/bagent.app` | PASS: signed macOS 26 candidate with real disposable daemon/BaseRT; preload, foreground chat, two automations, safe activity/tool presentation, result open, continuation, scoped `/clear`, permission reread, UI-only relaunch, idle retirement, later reload, and port isolation verified; safe external source ended in `verification_shortfall` (`acquired=0`, `requested=2`, `source_count=0`, 497 token bytes, capture SHA-256 `90c80a8fa11fe3b844879facb6f7b07230d533dc904ecdc56718925178636954`); no TCC mutation |
| A60 | `scripts/acceptance/stage8-reproducibility.sh <frozen-commit>` | PENDING until the final implementation commit is frozen and validated from a separate clean checkout |

### A51 final authority cleanup

The implementation removes computed compatibility accessors, legacy Work
mutations/routes/events, direct BaseRT lifecycle paths outside Model Runtime,
duplicate UI authority flags, obsolete result injection, and the old five-page
settings implementation. V23 is an explicit forward migration that removes
obsolete lifecycle tables and columns. Runtime flags cannot reactivate the old
authority. The canonical Legacy Run Record reader and normal 50/90 retention
behavior remain covered by deterministic tests.

### A52–A55 deterministic migration, recovery, rollback, and privacy

Migration fixtures are disposable and never use the user's production
database. Empty and V14 inputs converge through the same canonical schema
checksum and invariant checks, including Legacy Run Records, Current Chat,
Work/session conversion, authority ownership, and privacy. Failpoints cover
pre-commit retry and post-commit recovery. Rollback is before first post-cutover
Work only; after cutover it uses archive-and-restore and the old binary never
reads the new database. Privacy canaries cover event, UI, logs, diagnostics,
export, migration, rollback, crash, and failure surfaces without retaining
disposable captures.

### A56–A58 signed visual, accessibility, and active-load evidence

Signed/live visual and accessibility qualification is macOS 26 only and follows
`docs/UI_DESIGN.md`. The invariant status pill remains top-right. Keyboard,
focus, VoiceOver names/readouts, announcements, contrast, enlarged-text, and
reduced-motion checks are represented by executed deterministic and signed
fixture evidence. No TCC state is changed. Active-load relaunch changes only
the UI consumer PID except for the explicit disposable port-8082 poison case;
the port-8080 owner is never touched.

### A59 observational live smoke

The run recorded the initial disposable process/runtime baseline and restored
application, daemon, BaseRT, automation, lease, session, port, and preference
state. The safe external-source limitation is handled under
`docs/ADR-0002-REPRODUCIBLE-STAGE8-RELEASE-GATE.md`; no unsupported answer was
accepted.

## A60 reproducibility record

The initial A60 run is performed against the frozen implementation candidate;
the final evidence commit reruns A60 so the exact commit recorded below is the
one delivered. The final clean-checkout run must retain:

- frozen candidate commit and exact clean-checkout identity;
- OS, architecture, Rust/Cargo, Swift, Xcode, and signing identity;
- exact commands, UTC timestamps, nonzero execution counts, log hashes, and
  the final evidence-record hash;
- clean builds, full Rust and Swift suites, formatting, lint, diff, links,
  authority, privacy, migration, regression, localization, signed bundle,
  nested-code, identity, and strict codesign checks; and
- disposable checkout/build cleanup and the final runtime/port state.

## Review and closeout

Two independent reviews against fixed base
`45c26b1c1d3bd482b144525723a9c71a1fe57ced` are required: Standards, and Stage
8 specification/A51–A60 compliance. Both must report zero unresolved
findings. The ticket is closed only after the final commit is pushed and local
HEAD, upstream, and remote are identical; the worktree must be clean. No merge
or pull request is part of this stage.

## TDD and review ledger

- A51 authority cleanup: seeded forbidden-path inventory failed red before
  cleanup and passed green with 12 detected seeds and 0 production findings.
- A59 activity privacy: missing safe tool lifecycle and fallback cases failed
  red, then passed after generic allowlisted activity projection was added.
- A52 privacy boundary: both migration and live approval-origin tests failed
  red when a private automation name was retained, then passed after the
  canonical origin became kind-only.
- Final Standards review against the fixed Stage 7C commit: PASS, zero
  blockers, high findings, or lower-severity acceptance defects unresolved.
- Final Stage 8/A51-A60 specification review against the fixed Stage 7C
  commit: PASS, zero blockers, high findings, or lower-severity acceptance
  defects unresolved.
