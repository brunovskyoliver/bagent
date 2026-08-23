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

The final candidate commit is frozen before A60. Its signed bundle hash,
toolchain versions, command timestamps, test counts, cleanup, and final
runtime state are emitted by the reproducibility command and attached to the
Stage 8 ticket's resolution comment. The command's output is kept outside the
candidate commit so copying a run record into this file cannot change the
commit that A60 validates.

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
| A51 | `scripts/acceptance/final-authority-inventory.sh`; canonical authority subgates and production inventory | PASS: capability detector found 13 seeded forbidden matches, including the retired prompt/debug result path and obsolete automation event shim; four authority subgates passed; production inventory found 0 findings and 8 canonical assertions |
| A52 | `cargo test -p bagentd --test persistence_migration clean_and_v14 -- --exact` | PASS: 1 executed, 1 passed, 0 failed; disposable empty and V14 databases converged to the canonical schema and invariants, with kind-only approval provenance and no private automation identity |
| A53 | `scripts/acceptance/stage8-migration-restart.sh`; `cargo test -p bagentd --test persistence_migration interrupted_migration -- --exact`; `cargo test -p bagentd --test work_concurrency crash_recovery -- --exact` | PASS: 4 external `SIGKILL` cases and 2 exact tests executed; before-migration, during-copy, after-commit, and before-route-admission recovery preserved integrity, changed the daemon PID, admitted routes only after restart, converted 8 records without duplicates, and left protected ports unchanged |
| A54 | `scripts/acceptance/stage8-rollback-qualification.sh apps/macos/bagent.app` | PASS: disposable old/new signed candidates and databases; pre-Work verified backup and old reader, signed migration and first post-cutover Work, old-binary refusal of the post-Work database, archive-and-restore, hashes, protected-port checks, and cleanup all executed |
| A55 | `scripts/acceptance/stage8-privacy-scan.sh`; privacy contract and Swift privacy suites | PASS: 9 canaries entered the signed disposable workload, the shared scanner detected all 9 raw seeds, and the 9 actual production capture files contained 0 canary matches; disposable captures were securely deleted; Rust 1, Swift projection 4, and handoff privacy 5 tests passed |
| A56 | `scripts/acceptance/stage8-visual-qualification.sh apps/macos/bagent.app` | PASS: signed candidate rendered 11 notch-state PNGs and executed 22 normal/reduced-motion transitions; 57 settings fixtures × 2 widths across the accepted variants; status-pill anchor and identity verified |
| A57 | `scripts/acceptance/stage8-accessibility-qualification.sh apps/macos/bagent.app` | PASS: signed live AX fixture exercised active and approval states, 1 AX press action, 2 keyboard events, 1 destination change, observed AX names/values and enlarged-layout frames, 2 contrast checks, 2 posted announcements, and 0 skips |
| A58 | `scripts/acceptance/stage8-active-load-relaunch.sh apps/macos/bagent.app`; `scripts/acceptance/ui-relaunch-handoff.sh apps/macos/bagent.app` | PASS: signed UI-only relaunch preserved 1 foreground Work, 2 real run-now automation Works, and 1 canonical pending approval while daemon/BaseRT PIDs, Work revisions, protected ports, and the active UI consumer converged |
| A59 | `scripts/acceptance/stage8-live-smoke.sh apps/macos/bagent.app` | PASS: signed candidate and real disposable daemon/BaseRT; 2 foreground chats, 2 canonical automation Works, 2 links, 2 sessions, 1 live idle retirement, and 1 live reload through bounded production inference; result open, continuation, scoped `/clear`, permission reread, UI-only relaunch, unchanged process identities, and port isolation verified; external source ended in a safe `verification_shortfall` with privacy-safe capture SHA-256 `47181899a308741f2a05ecbf38106907edd75af016d2d245633a67d95ebc4884` |
| A60 | `scripts/acceptance/stage8-reproducibility.sh <frozen-final-commit>` | The final clean-checkout record, including every gate status, nonzero execution count, log hash, signed bundle hash, timestamps, protected-port baseline, cleanup, and final runtime state, is attached to the Stage 8 ticket resolution comment. This row is not a substitute for that emitted record. |

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
checksum and invariant checks, including the Automation Definition schema,
Legacy Run Records, Current Chat, Work/session conversion, authority ownership,
and privacy. Unit failpoints and external process kills cover pre-commit retry,
post-commit recovery, and route admission. Rollback is before first
post-cutover Work only; after cutover it uses archive-and-restore and the old
binary never reads the new database. Privacy canaries enter the signed
disposable relaunch workload. The gate scans the nine resulting production
capture files and securely removes them; it does not manufacture separate
sanitized placeholder files.

### A56–A58 signed visual, accessibility, and active-load evidence

Signed/live visual and accessibility qualification is macOS 26 only and follows
`docs/UI_DESIGN.md`. The invariant status pill remains top-right. The signed
fixture records AX names/values, an AX action and destination change, delivered
keyboard events, posted announcements, contrast, enlarged-layout frames, and
reduced motion. It does not claim to observe VoiceOver speech. No TCC state is
changed. Active-load relaunch changes only the UI consumer PID and preserves
one foreground Work, two automation Works, and one pending approval; the
port-8080 owner is never touched.

### A59 observational live smoke

The run recorded the initial disposable process/runtime baseline and restored
application, daemon, BaseRT, automation, lease, session, port, and preference
state. The safe external-source limitation is handled under
`docs/ADR-0002-REPRODUCIBLE-STAGE8-RELEASE-GATE.md`; no unsupported answer was
accepted.

## A60 reproducibility record

The earlier clean-checkout run below is retained as superseded development
evidence only. It is not the current release qualification because later A51,
A55, A57, and A59 fixes changed the candidate. The final A60 command is run
after the last implementation and evidence commit; its complete emitted
record is attached to the Stage 8 ticket resolution comment rather than copied
back into the commit under test.

```text
superseded_candidate=1260480bc0b91bf8a447b88fcbae885f5f4e3cfe
started_utc=2026-08-21T19:28:41Z
ended_utc=2026-08-21T19:42:29Z
os=26.5.2 (25F84)
arch=arm64
rust=rustc 1.91.1 (ed61e7d7e 2025-11-07)
cargo=cargo 1.91.1 (ea2d97820 2025-10-10)
swift=Apple Swift version 6.3.1 (swiftlang-6.3.1.1.2 clang-2100.0.123.102)
xcode=Xcode 26.4.1;Build version 17E202;
protected_8080_before=
protected_8082_before=62597,

cargo-fmt command=cargo fmt --all -- --check status=0 started=2026-08-21T19:28:41Z ended=2026-08-21T19:28:42Z nonzero_metrics=none-reported log_sha256=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
cargo-clippy command=cargo clippy --workspace --all-targets -- -D warnings status=0 started=2026-08-21T19:28:42Z ended=2026-08-21T19:29:29Z nonzero_metrics=none-reported log_sha256=fecc12686f6f9e9ab1f8de4649e3219a1678ebe450c60cd22324429da5015671
daemon-acceptance-clippy command=cargo clippy -p bagentd --features stage7a-acceptance\,stage8-acceptance --all-targets -- -D warnings status=0 started=2026-08-21T19:29:29Z ended=2026-08-21T19:29:42Z nonzero_metrics=none-reported log_sha256=e507d798451eb0bfa4ef7822fa3c6ca1a545a145c162d3e02050590f9c9f9cca
cargo-test command=cargo test --workspace --no-fail-fast status=0 started=2026-08-21T19:29:42Z ended=2026-08-21T19:31:23Z nonzero_metrics=21,39,17,19,10,239,9,5,8,4,1,12,2,14,27,30,15,13 log_sha256=1241d9053274f6968948650e84db559714221ea11ae279e1b3a93118586b2e55
daemon-acceptance-tests command=cargo test -p bagentd --features stage7a-acceptance\,stage8-acceptance --bin bagentd --no-fail-fast status=0 started=2026-08-21T19:31:23Z ended=2026-08-21T19:31:50Z nonzero_metrics=241 log_sha256=8a11aa92546895ad2653e766f4f350196bdf8408c16d03825ca6337c39d405e1
swift-build command=swift build --package-path apps/macos status=0 started=2026-08-21T19:31:50Z ended=2026-08-21T19:32:23Z nonzero_metrics=none-reported log_sha256=0f5da3ba589c0af5041945877d805b02f1fa78510c3bd009fab8fcbee62528b6
swift-test command=swift test --package-path apps/macos status=0 started=2026-08-21T19:32:23Z ended=2026-08-21T19:32:40Z nonzero_metrics=3,1,4,2,6,11,7,9,12,5,20,149 log_sha256=682845a0fc94ccfeae8c7cb078ea850ac41a2173c0b1172422c92c56611f297e
git-diff-check command=git -C /var/folders/wc/d2tp_dbj12lbq389yxv649lh0000gn/T//bagent-stage8-reproducibility.5ThLcW/checkout diff --check status=0 started=2026-08-21T19:32:40Z ended=2026-08-21T19:32:40Z nonzero_metrics=none-reported log_sha256=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
documentation-links command=scripts/acceptance/documentation-links.sh status=0 started=2026-08-21T19:32:40Z ended=2026-08-21T19:32:41Z nonzero_metrics=none-reported log_sha256=2e3d5a72062f7b45b1b95a09618a9a564d67503dd5b4eae64a44f159a838b062
authority-inventory command=scripts/acceptance/final-authority-inventory.sh status=0 started=2026-08-21T19:32:41Z ended=2026-08-21T19:32:44Z nonzero_metrics=none-reported log_sha256=f57bf6b969b6ca14bb9b9c00c194ad3da2cd05a577ef9564f7182a59d237efab
work-authority command=scripts/acceptance/work-authority.sh status=0 started=2026-08-21T19:32:44Z ended=2026-08-21T19:32:46Z nonzero_metrics=none-reported log_sha256=d92801aeff81ddd675ebebf6528fd4c0d501b4fb3ce80b523368bc6b85be6e7c
model-runtime-authority command=scripts/acceptance/model-runtime-authority.sh status=0 started=2026-08-21T19:32:46Z ended=2026-08-21T19:32:47Z nonzero_metrics=none-reported log_sha256=8974b912a8f7922d81ed12ac55ee5bc6c7b9984f940dde0c1e9278413ec21795
current-chat-authority command=scripts/acceptance/current-chat-authority.sh status=0 started=2026-08-21T19:32:47Z ended=2026-08-21T19:32:49Z nonzero_metrics=none-reported log_sha256=ec5b6d64d96e2c554ee76bca25066dff77e9b84b8ce39594cc142b9090cd6e41
settings-authority command=scripts/acceptance/settings-authority.sh status=0 started=2026-08-21T19:32:49Z ended=2026-08-21T19:32:50Z nonzero_metrics=5,12 log_sha256=a530fa60f6c4e036d112bf25d2d908312c4205233cbf3b1d8a7ce499f89db98e
notch-mode-authority command=scripts/acceptance/notch-mode-authority.sh status=0 started=2026-08-21T19:32:50Z ended=2026-08-21T19:32:50Z nonzero_metrics=none-reported log_sha256=6c0949a956ce2c7ad48f631d94af6f2200c122441d708c2a439ff21012096de5
work-cutover-rollback command=scripts/acceptance/work-cutover-rollback.sh status=0 started=2026-08-21T19:32:50Z ended=2026-08-21T19:32:51Z nonzero_metrics=1 log_sha256=d84d5c916aedea3e5e8e953c1b0b622f2239f484509caea1294f98a6f236fd8a
accessibility-audit command=scripts/acceptance/accessibility-audit.sh status=0 started=2026-08-21T19:32:51Z ended=2026-08-21T19:35:39Z nonzero_metrics=1,3,5,9 log_sha256=52ae7ace238b888ab82609934de2e0a6d760ed6cf6103b6ea15ef7297e59c66f
settings-localization command=scripts/acceptance/settings-localization.sh status=0 started=2026-08-21T19:35:40Z ended=2026-08-21T19:35:40Z nonzero_metrics=none-reported log_sha256=dea208fe82ff90ea375f25ef434338dd98a6dc1dd696e4a4b6c41939592ccb2b
automation-sessions-regression command=cargo test -p bagentd --test automation_sessions --no-fail-fast status=0 started=2026-08-21T19:35:40Z ended=2026-08-21T19:35:41Z nonzero_metrics=9 log_sha256=6a4b24c6e17d4b7b526cfc0079ef8993046c42c21ab88c2be65d15742ab29c00
current-chat-regression command=cargo test -p bagentd --test current_chat --no-fail-fast status=0 started=2026-08-21T19:35:41Z ended=2026-08-21T19:35:48Z nonzero_metrics=5 log_sha256=3ef52a43d33fba311829b916f95c52d77fd17fb819e303f5aa93b49bc7c379b6
work-coordinator-regression command=cargo test -p bagentd --test work_coordinator --no-fail-fast status=0 started=2026-08-21T19:35:48Z ended=2026-08-21T19:35:49Z nonzero_metrics=12,1 log_sha256=0e71303d9ad1de8ec9c2fe03fe19d021d7aba760bebf7faf1d8f5a94c34d2657
work-failure-regression command=cargo test -p bagentd --test work_failure_injection --no-fail-fast status=0 started=2026-08-21T19:35:50Z ended=2026-08-21T19:35:50Z nonzero_metrics=1 log_sha256=cb5c679a3179e32fb4d2aa14185d5f3ada6147d0da8cca751aa022092bfe8aa7
model-runtime-regression command=cargo test -p bagentd --test model_runtime --no-fail-fast status=0 started=2026-08-21T19:35:50Z ended=2026-08-21T19:35:51Z nonzero_metrics=8 log_sha256=547fb8537b3d7bd8dec555b266de6c1f0e9ba6f32c58b9f83e4ef77c4546b036
migration-clean-v14 command=cargo test -p bagentd --test persistence_migration clean_and_v14 -- --exact status=0 started=2026-08-21T19:35:52Z ended=2026-08-21T19:35:52Z nonzero_metrics=1 log_sha256=85bd6dd12baba1cbfad7c3b72cbe0b270be1911ef20276e117b719abb80cbbad
migration-interruption command=cargo test -p bagentd --test persistence_migration interrupted_migration -- --exact status=0 started=2026-08-21T19:35:52Z ended=2026-08-21T19:35:53Z nonzero_metrics=1 log_sha256=98586e388163d945fd9d68cff648ded7982cd8ee6e41c339663c479c84428143
work-crash-recovery command=cargo test -p bagentd --test work_concurrency crash_recovery -- --exact status=0 started=2026-08-21T19:35:53Z ended=2026-08-21T19:35:53Z nonzero_metrics=1 log_sha256=415075ddfe994d55356b6e58f0a85fa7c12ac6d9cc1052f4348432968a4392a6
work-fairness command=cargo test -p bagentd --test work_concurrency fairness_foreground -- --exact status=0 started=2026-08-21T19:35:53Z ended=2026-08-21T19:35:54Z nonzero_metrics=1 log_sha256=dd3b7ad3a909870e6dc253791d89bf0767e0a3c7189c83cdbc064da48b701b0e
model-poison command=cargo test -p bagentd --test model_runtime poison_changed_pid -- --exact status=0 started=2026-08-21T19:35:54Z ended=2026-08-21T19:35:54Z nonzero_metrics=1 log_sha256=082d3efd565787ea16581a93a09e6b373653763c5b721fc4759e4f0e08100ed5
signed-bundle-make command=make -C apps/macos bundle status=0 started=2026-08-21T19:35:54Z ended=2026-08-21T19:35:58Z nonzero_metrics=none-reported log_sha256=8bfb77de3ecb6a0407a23c15ee2a1559de3f108184f7089c1fa671a5fa0dbd54
signed-bundle-verification command=scripts/acceptance/signed-bundle-verification.sh apps/macos/bagent.app status=0 started=2026-08-21T19:35:58Z ended=2026-08-21T19:36:01Z nonzero_metrics=4 log_sha256=bfa66f9eb6601707d6015778d28c6c064550de8b51c995e0d672ce02f27aefa3
signed-bundle-codesign command=codesign --verify --deep --strict apps/macos/bagent.app status=0 started=2026-08-21T19:36:01Z ended=2026-08-21T19:36:01Z nonzero_metrics=none-reported log_sha256=e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
signed-bundle-designated-requirement command=codesign -dr - apps/macos/bagent.app status=0 started=2026-08-21T19:36:01Z ended=2026-08-21T19:36:01Z nonzero_metrics=none-reported log_sha256=c33fa27cb1d66cd6a691c2ec814d06bf9a50c9dc66c0f1a56888ebe36b2d38b9
privacy-scan command=scripts/acceptance/stage8-privacy-scan.sh apps/macos/bagent.app status=0 started=2026-08-21T19:36:01Z ended=2026-08-21T19:36:49Z nonzero_metrics=1,4,5 log_sha256=fc17ba09dcb2c46221edc497d007b7ec0a6fa1fcfae5763be3b599a748cc20e4
notch-state-capture command=scripts/acceptance/capture-notch-states.sh apps/macos/bagent.app status=0 started=2026-08-21T19:36:49Z ended=2026-08-21T19:36:57Z nonzero_metrics=1 log_sha256=039de609072de43e9c70a9ee6ff60f1776bd6454e83042fc7757ffaa11197d06
settings-catalog command=scripts/acceptance/settings-catalog.sh apps/macos/bagent.app status=0 started=2026-08-21T19:36:57Z ended=2026-08-21T19:37:30Z nonzero_metrics=11,5,12,57 log_sha256=60ff75ff00ce402aeeae0f1501356c5ddc727a758ab0d89f65b1392ff34ad91b
signed-ui-relaunch command=scripts/acceptance/ui-relaunch-handoff.sh apps/macos/bagent.app status=0 started=2026-08-21T19:37:30Z ended=2026-08-21T19:37:54Z nonzero_metrics=5,10 log_sha256=6206cb733a6d31b1c12401fe279a3f69a977a8b39d649e4e023670234ebfe76b
stage8-rollback command=scripts/acceptance/stage8-rollback-qualification.sh apps/macos/bagent.app status=0 started=2026-08-21T19:37:54Z ended=2026-08-21T19:39:45Z nonzero_metrics=1 log_sha256=a27cbca9c54f43ec44e56092cac969ff5048142c5b5800abc8cf32acfc4ff92e
stage8-visual command=scripts/acceptance/stage8-visual-qualification.sh apps/macos/bagent.app status=0 started=2026-08-21T19:39:45Z ended=2026-08-21T19:40:52Z nonzero_metrics=5,3,4,11,1,12,57 log_sha256=cb29b2d11415d8fbe06b2ab116d58db69065c16aa2ea7b7956b0aabbe2b83cb4
stage8-accessibility command=scripts/acceptance/stage8-accessibility-qualification.sh apps/macos/bagent.app status=0 started=2026-08-21T19:40:52Z ended=2026-08-21T19:41:34Z nonzero_metrics=11,1 log_sha256=55ffb1ad043551e89970c17662e499bb1e56b35e77a81decf89437067713300a
stage8-active-load-relaunch command=scripts/acceptance/stage8-active-load-relaunch.sh apps/macos/bagent.app status=0 started=2026-08-21T19:41:34Z ended=2026-08-21T19:41:52Z nonzero_metrics=1 log_sha256=5d6d54975a660fd495ecb87f235d7d9798d1a99c9076708c0b7996bd080aa556
stage8-live-smoke command=scripts/acceptance/stage8-live-smoke.sh apps/macos/bagent.app status=0 started=2026-08-21T19:41:52Z ended=2026-08-21T19:42:21Z nonzero_metrics=none-reported log_sha256=f30d6440a2c25aac1cb90dec2ff6335f074b6c1986d3d14b76903296d35b65cb
signed-stage8-e2e command=scripts/acceptance/stage8-signed-e2e.sh apps/macos/bagent.app status=0 started=2026-08-21T19:42:21Z ended=2026-08-21T19:42:29Z nonzero_metrics=21 log_sha256=ce1805f434eed183fcd29d0311323474497ad1f041d5d438be31c0f71f9bc405

protected_8080_after=
protected_8082_after=62597,
final_worktree=clean
cleanup=all gate-owned fixtures and processes cleaned; detached checkout and record directory removed by EXIT trap
production_database=not used; production application and port-8080 owner untouched
```

The superseded record has SHA-256
`02b3ce188e9d2b57e5fa3622d0a2a256e0154b48587571e02af66bc4863307c2`.
Its zero statuses are retained for traceability, but several silent commands
reported no metric and the old runner allowed skip observations alongside a
PASS verdict. Those are known defects in the superseded evidence. The final
A60 record requires one executed-command count for every gate, rejects any
skipped, blocked, or conditional observation, and records additional nonzero
metrics when a command emits them. The environment-dependent hosted XCTest
Accessibility probe is not a qualification input; the signed candidate AX
checks are the executed evidence.

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
- A53 process recovery: the external restart script failed red because no
  process-kill marker existed, then passed all four `SIGKILL` cases after the
  acceptance-only startup seams were added.
- A60 skip accounting: the first frozen-candidate run stopped on the hosted
  Accessibility test's skipped result. That environment-dependent runner test
  was removed from the deterministic XCTest suite; signed live AX remains in
  A57, and A60's classifier now catches uppercase gate states, positive skip
  counts, and XCTest's `Test skipped` output.
- Earlier review passes identified and were followed by fixes for the counted
  A51 migration allowlist, shared A55 canary scanner, signed A56 transition
  evidence, signed A57 accessibility evidence, signed A59 observation order,
  and final-candidate A60 reproducibility.
- The first fixed-base final reviews rejected synthetic A55 files, unsigned
  A56 captures, asserted A57 booleans, separated A58 load checks, unit-only A59
  retirement, shipped Stage 8 mutation controls, weak PID cleanup, and missing
  A60 metrics. The corrected candidate uses live signed captures, measured AX
  evidence, a combined relaunch load, compile-time Swift acceptance isolation,
  executable-bound cleanup, live retirement/reload, and named nonzero gate
  checks. A fresh two-axis review is still required after the final A60 run.
- The two final independent review reports and their zero-finding results are
  attached to the Stage 8 ticket resolution comment after the final candidate
  is frozen.
