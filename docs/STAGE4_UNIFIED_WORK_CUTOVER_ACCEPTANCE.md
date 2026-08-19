# Stage 4 unified Work cutover acceptance

## Scope and starting state

Stage 4 atomically moves foreground Conversation Turns and admitted Automation
Runs behind the daemon-owned Work Coordinator. Stage 5 is not part of this
change.

- Required base and initial `HEAD`: `20997ac43ff729b5b5dc874ebd64898aceb5867c`.
- Branch: `t3code/basert-notch-automation-ux`.
- Initial local `HEAD` and upstream were equal and the worktree was clean.
- Stage 4 issue: [#30](https://github.com/brunovskyoliver/bagent/issues/30).
- Native prerequisite: closed Stage 3 [#29](https://github.com/brunovskyoliver/bagent/issues/29).
- No Stage 5 implementation was started.

The implementation follows the accepted command, snapshot, event, projection,
migration, privacy, Model Runtime, and source-authority seams in
`UNIFIED_BACKGROUND_WORK_STATE_MACHINE_DECISION.md` and
`IMPLEMENTATION_SEQUENCE_ACCEPTANCE_GATES_DECISION.md`.

## Protected runtime baseline and restoration

The final pre-review read-only baseline was captured at
`2026-08-18T20:14:14Z`:

| Protected surface | State |
|---|---|
| Installed app daemon | `/Applications/bagent.app/Contents/MacOS/bagentd`, PID 758 |
| Managed BaseRT | `/Users/oliver/.basert/basert-serve`, PID 792 |
| IPv4 port 8080 | Docker listener PID 74402 |
| IPv4 port 8082 | BaseRT listener PID 792 |

All migration, restart, failure, and port tests use disposable databases and
child-process fixtures. They do not inspect or mutate the live user database,
installed app, protected daemon or BaseRT process, TCC state, credentials,
caches, launch agents, or protected listeners. A final read-only topology
comparison is recorded after review and verification.

## Unified authority and persistence boundary

`UnifiedWorkAuthority` is the only production admission and execution-capacity
owner for foreground and Automation Work. It submits canonical Work, selects
queued work using bounded foreground priority, limits distinct Automation Run
execution to two, releases capacity while approval is pending, resolves durable
approval decisions with revision checks, and makes cancellation idempotent.
Model demand is constructed only inside the Model Runtime boundary and always
carries the canonical Work Identity.

The accepted deterministic scheduling constants are an Automation capacity of
two, a foreground burst limit of three, and an Automation aging boundary of 30
fake-clock ticks. Production callers cannot construct their own model demand,
write canonical Work tables, hold a competing semaphore, own an approval map,
deny approvals on startup, emit ad-hoc lifecycle events, or directly stop chat.

Every Work command, revision, transition, and ordered outbox event commits
through `WorkCoordinator`. Swift remains a compatibility projection consumer;
legacy Automation Run rows and pending-approval rows cannot originate Work
lifecycle state. The Stage 3 `TypedOrigin`/`TypedModelRuntime` adapter and
synthetic `legacy:{session_id}` identities are removed. Canonical Work events
are projected from the durable cursor-ordered outbox.

## V16 cutover, legacy records, and rollback

Startup creates and verifies an explicit-path SQLite backup before applying the
V16 cutover. After Model Runtime establishes its safe changed-PID boundary, the
cutover transaction records schema generation 16 and converts eligible V14
terminal Automation Runs into bounded, viewed, no-attention Legacy Run Records.
Only a strict, privacy-safe summary shape is retained. Active run and approval
rows are abandoned only after that safe boundary. Conversion is idempotent and
retains at most 50 records per Automation definition.

The first committed post-cutover Work writes the forward-only marker in the
same transaction as Work creation. Before that marker, a checksum-verified
backup restores for the old-reader fixture. After it, downgrade is rejected;
the only supported path archives the post-cutover database, discloses that
post-cutover Work is unavailable to the older schema, then restores the
verified backup. Interrupted migration and repeated conversion fixtures leave
an integral, deterministic database.

## Post-commit correction: dead admission queue and a competing semaphore

An independent re-inspection after the candidate above found that
`UnifiedWorkAuthority::dispatch_next` — the only code implementing A18's
bounded foreground priority and Automation aging — had **no production
caller**. `crates/daemon/src/main.rs` and `automations_api.rs` admitted Work
by calling `submit_conversation`/`submit_automation` and then transitioning
straight to `WaitingForModel`→`Running`, bypassing the fairness queue
entirely. Foreground had no capacity gate at all; Automation admission held a
second, independent `tokio::sync::Semaphore` (`automation_slots`) that
happened to cap concurrency at two but never consulted `dispatch_next`'s
aging logic. This is exactly the "competing semaphore" this document's own
"Unified authority" section claims does not exist — it did, inside
`unified_work.rs` itself. The A26 static detector did not catch it because
its production-file allowlist never scanned `unified_work.rs`, and its
semaphore pattern only matched the old `run_slots`/`MAX_CONCURRENT_RUNS`
names from the pre-cutover code, not any `Semaphore::new(`.

A18's PASS in the table below was therefore evidence against
`dispatch_next` called directly by the test, not against the real admission
path — a class of gap the acceptance harness could not see because nothing
exercised production wiring end-to-end.

Fix, in `crates/daemon/src/unified_work.rs`:

- Removed `automation_slots`/`held_automation_slots`; capacity is now solely
  the scheduler's `foreground_running`/`automation_running` counters,
  attributed per Work via a `running_origin` map so `release_slot` and
  `cancel` take a `WorkIdentity` (not a caller-asserted origin) and can never
  double-account or mismatch.
- Added `admit(work)` (async, notify-driven: blocks until `work` leaves the
  queue) and `run_dispatcher(clock)` (background loop: drains
  `dispatch_next` on every queue change, plus a 1s fallback tick so the
  30-tick Automation aging boundary fires without new arrivals). Spawned once
  in `main.rs` after `UnifiedWorkAuthority::new`.
- Added `resume(work, origin)` for the approval-resume path: the coordinator
  already moves `WaitingForApproval` → `Running` unconditionally inside
  `resolve_approval` (both allow and deny), so resume only needs to wait for
  a free capacity slot, not re-enter the Queued/WaitingForModel transition.
- `request_approval`/`resolve_approval` no longer take an `origin`/`now`
  parameter; callers already have `execution_origin` in scope and now call
  `resume` explicitly after a successful resolve.
- `cancel` now releases a dispatched Work's capacity slot (previously it only
  pruned queue membership — a live leak for any Work cancelled after being
  granted, masked because nothing dispatched Work in production to leak).

Call sites (`main.rs` chat handler, `screen_intent_handler`,
`automations_api.rs::execute_automation_run`, both approval-resolution paths
in `request_approval_core` and `approval_decide`) were updated to
`submit_*` → `admit().await` → transition to `Running`, and to call
`release_slot(&work_identity)` at every terminal exit (Completed/Failed for
chat and screen-intent; the existing terminal transition for automation).
`enqueued_at`/dispatcher clock inputs were switched from
`timestamp_millis()` to `timestamp()` (seconds) to match the
`AUTOMATION_AGE_BOUNDARY = 30` unit; this was previously a dead mismatch
since `dispatch_next` was never called with production timestamps.

`scripts/acceptance/work-authority.sh` was corrected to scan every file
under `crates/daemon/src` (not a fixed five-file allowlist that excluded
`unified_work.rs`, `work_coordinator.rs`, and every connector), and to match
any `Semaphore::new(` rather than only the old `run_slots`/
`MAX_CONCURRENT_RUNS` names.

Three new integration tests in `crates/daemon/tests/work_concurrency.rs`
drive `submit_*` → `admit()` through a spawned `run_dispatcher`, the real
production call sequence, instead of calling `dispatch_next` directly:
`admission_dispatcher_grants_through_the_real_async_path` (regression guard:
`admit` used to hang forever — nothing ever popped the queue),
`admission_dispatcher_serializes_foreground_independent_of_automation`, and
`admission_dispatcher_enforces_automation_capacity_of_two`.

### Second correction pass: independent review, then a real slot-leak

Independent Standards and Spec sub-agent reviews were run against this same
fixed point after the first correction above was committed (`97e6539`).
Standards reported no hard violations (some judgement-call duplication
smells, noted and left as-is: minimal-diff terminal-transition boilerplate
repeated across three handlers, not required by any gate). Spec reported two
real findings, both repaired here:

1. **A genuine slot leak on five call paths.** `admit()` grants a capacity
   slot before the caller has confirmed the subsequent `current()`
   lookup/`transition(..., Running)` actually succeeds. The chat handler,
   `screen_intent_handler`, and `execute_automation_run` each had an
   early-return branch on that lookup/transition failing which skipped
   `release_slot`. Because `admit()` has no timeout and `dispatch_next`'s
   foreground rule requires `foreground_running == 0`, a single leaked
   foreground slot would permanently hang every later chat turn and
   screen-intent call in `admit().await` — a daemon-wide deadlock, not a
   capacity regression. Fixed by adding `release_slot` to all five early-return
   branches (`main.rs` chat handler ×2, `screen_intent_handler` ×1 covering
   both its Ok/Err arms, `automations_api.rs` ×2). The full span from the
   successful `Running` transition through to the terminal transition in both
   the chat handler and `execute_automation_run` was re-audited line by line
   for any further `return`/`?` in between; none exists — `prompt_builder.build`'s
   only fallible step falls through to `Vec::new()` rather than returning.
   A structural (`Drop`-based `AdmissionGuard`) fix was considered and is
   left as a deliberate non-requirement — see the code review discussion; the
   explicit calls now cover every real path and are cheaper to verify.
2. **A22's `ExecutionBoundaryAdapter` has no production caller** — disclosed
   in the A22 row below rather than silently overclaimed.

`scripts/acceptance/work-authority.sh` gained a static pairing check (own
seeded red case): any function under `crates/daemon/src` calling
`work_authority.admit(` must also call `.release_slot(` somewhere in its own
body. This is a co-presence heuristic, not branch coverage or
cancellation-safety proof — it would not, by itself, have caught the two
defects below, only a function that calls `admit` and never mentions
`release_slot` anywhere at all. It still raised the detector from 10 to 11
forbidden/required categories, all zero findings against production, and is
worth keeping as a coarse tripwire.

### Third correction pass: a second independent re-review found a real gap

A second round of Standards/Spec sub-agent review was run against the fixed
point after the slot-leak fix above (`de1b7fd`). Standards confirmed no new
hard violations. Spec found the fix above was **incomplete**: `admit()`'s
five newly-patched early-return branches were correctly fixed, but
`screen_intent_handler` was — unlike the `chat` handler and
`execute_automation_run` — a bare axum route handler, not run inside a
`tokio::spawn`. Axum drops a handler's future mid-flight on client
disconnect; a disconnect while `classifier.classify(...).await` was pending
(after `admit()` had already granted the foreground slot) would skip both
its Ok/Err arms entirely, leaking the slot — the exact deadlock class this
correction pass set out to close, on a path neither the fix nor its own
line-by-line re-audit had examined (the audit only searched for
`return`/`?`, and future-cancellation is neither).

Fixed by wrapping `screen_intent_handler`'s post-admission body in
`tokio::spawn`, matching the pattern `chat` and `execute_automation_run`
already use: a spawned task is detached from the connection's future, so
the client disconnecting cannot cancel it mid-flight; the handler awaits the
`JoinHandle` and falls back to the existing graceful-degrade response only
if the task panics (never from the caller side being dropped). `chat`
(`tokio::spawn` at `main.rs:2161`, wrapping every `admit`/`release_slot`
call) and `execute_automation_run`'s two callers (`automations_api.rs:953`,
already inside `tokio::spawn`; `scheduler.rs:247`, itself inside a spawned
task) were re-confirmed already immune to this vector — `screen_intent_handler`
was the only one of the three admission call sites that was not.

Final pre-close rerun (supersedes all earlier timestamps in this document):
`2026-08-19T17:01:30Z` through `2026-08-19T17:03:22Z`. `cargo test -p
bagentd` (all targets): 251 top-level passed, 0 failed, 5 ignored (unchanged
environment/fixture boundaries), nested restart child 1/1 not
double-counted. `cargo test --workspace`: 451 distinct top-level tests passed,
0 failed, 12 ignored; Cargo also reported the nested restart child as a
separate 1/1 result line, for 452 reported passes in the raw output. `swift
test --package-path apps/macos`: 52 passed, 0 failed, 0 ignored. `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets --all-features -- -D warnings`, `cargo build --workspace
--all-targets`, `cargo check -p bagentd --all-targets --features
stage8-acceptance`, `swift build --package-path apps/macos`, `git diff
--check`, `bash -n scripts/acceptance/work-authority.sh
scripts/acceptance/work-cutover-rollback.sh`: all exit 0.
`bash scripts/acceptance/work-authority.sh`: PASS, 11 forbidden/required
categories seeded, zero forbidden matches against the full
`crates/daemon/src` tree. `bash scripts/acceptance/work-cutover-rollback.sh`:
PASS, unchanged.

Final artifact hashes:

| Artifact | SHA-256 |
|---|---|
| `crates/daemon/src/unified_work.rs` | `8416257d0c3f88c9c12aa6ecda6b10735fee60411db4dd07bfa11a45a7baf120` |
| `crates/daemon/src/main.rs` | `80c8f8035131e6d1c1a350d0e1c880fad34677822dbee8bf2b4fc2c1269cc3b0` |
| `crates/daemon/src/automations_api.rs` | `2d95b13fec2d82177a62281a796ed21416779d9a1b7c68c11948f351c938473b` |
| `scripts/acceptance/work-authority.sh` | `2c9c3153b53c971650cb4ef8154759529a2210cff3d1d9f36354f571c7eee255` |

`crates/daemon/migrations/V16__unified_work_cutover.sql` and
`scripts/acceptance/work-cutover-rollback.sh` are unchanged from the
original candidate hashes above.

## Sequential red-green gates

Red capability was established before production implementation. A18 first
failed because the `work_concurrency` target did not exist. Subsequent slices
exposed the absent coordinator-owned capacity, restart-stable approval,
cancellation, failure adapters, migration, privacy projection, rollback, and
source-graph boundaries. A20 then exposed non-idempotent V16 reopening and A23
exposed a 51-record retention error; both assertions stayed red until the
production transaction boundaries were corrected. The first daemon regression
run found seven legacy tests still treating scheduler claims as execution
authority; those fixtures were retargeted to canonical Work and the final
daemon run is green.

Final exact run: `2026-08-19T17:01:30Z` through `2026-08-19T17:01:34Z`.
This supersedes the original `2026-08-18T20:12:46Z` run and every correction
rerun above. The table reflects the corrected admission path at `2d91930`.

| Gate | Exact command | Exact result |
|---|---|---|
| A18 | `cargo test -p bagentd --test work_concurrency fairness_foreground -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 6 filtered; bounded foreground priority and aged Automation progress |
| A19 | `cargo test -p bagentd --test work_concurrency automation_capacity_two -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 6 filtered; exactly two distinct Automation executions and zero slot leak |
| A20 | `cargo test -p bagentd --test work_concurrency approval_restart_capacity -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 6 filtered; same approval identity survives restart, capacity is free, one valid decision resumes, stale decision conflicts |
| A21 | `cargo test -p bagentd --test work_concurrency cancellation_races -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 6 filtered; queued/loading/executing/approval/completion races, one terminal outcome, zero leases/slots |
| A22 | `cargo test -p bagentd --test work_failure_injection` | PASS: 1 passed, 0 failed; admission, persistence, outbox, runtime, tool, approval, and completion failpoints close deterministically. `ExecutionBoundaryAdapter`/`execute_with_adapter` (`unified_work.rs`) is a test-only harness seam with no production caller — it proves `UnifiedWorkAuthority`'s own recovery/invariant behavior at each named boundary, not that production's chat/automation handlers route execution through it (they call `transition`/`admit`/`release_slot` directly; see the correction section above). |
| A23 | `cargo test -p bagentd --test persistence_migration legacy_run_records -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 1 filtered; bounded/idempotent privacy-safe records and safe active abandonment |
| A24 | `cargo test -p bagentd --test privacy_contract work_surfaces -- --exact` | PASS: 1 passed, 0 failed; canaries absent from events, snapshots, projections, logs, and diagnostics; unknown fields rejected |
| A25 | `cargo test -p bagentd --test persistence_migration cutover_boundary -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 1 filtered |
| A25 rollback | `scripts/acceptance/work-cutover-rollback.sh` | PASS: checksum backup/restore, old-reader fixture, interruption boundaries, first-Work barrier, archive disclosure |
| A26 | `scripts/acceptance/work-authority.sh` | PASS: seeded detector found 11 forbidden/required categories (adds a static admit/release-slot pairing check per function); zero forbidden production authorities; canonical command/event path present |

No zero-test result is used as evidence for A18-A26.

## Privacy and authority graph

The compact event, snapshot, projection, log, and diagnostic contracts are
closed structural allowlists. Test canaries cover hidden reasoning, raw tool
arguments, credentials, evidence content, private identities, and unknown
fields. Unknown fields fail deserialization rather than passing through.

`scripts/acceptance/work-authority.sh` scans all daemon production Rust sources
after stripping only `#[cfg(test)]` items. Its seeded red capability covers the
typed-origin adapter, synthetic Work IDs, semaphore authority, in-memory
approval authority, startup denial, ad-hoc event senders, direct chat stop,
duplicate Work Identity, direct canonical Work SQL writers, caller-owned
model demand, and an admit-without-release capacity-slot leak. Final
production findings are zero. The only canonical Work SQL writers are
`work_coordinator.rs` and the bounded cutover module.

## Full verification

| Command | Result |
|---|---|
| A11-A16 exact Model Runtime commands | 6 independently exact tests passed, 0 failed |
| `scripts/acceptance/model-runtime-authority.sh` | PASS: 13 seeded forbidden matches and zero forbidden production matches |
| `cargo test -p basert-connector --test protocol` | 14 passed, 0 failed, 0 ignored |
| `cargo test -p bagentd` | 251 top-level tests passed, 0 failed, 5 ignored; nested restart child passed 1/1 (superseded original candidate figures — see correction section above) |
| `cargo test --workspace` | 451 distinct top-level tests passed, 0 failed, 12 ignored; the nested restart child also passed 1/1, producing 452 reported passes in the raw Cargo result lines |
| `swift test --package-path apps/macos` | 52 passed, 0 failed, 0 ignored; the separate Swift Testing runner contained 0 tests and is not evidence |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| `cargo build --workspace --all-targets` | exit 0 |
| `cargo check -p bagentd --all-targets --features stage8-acceptance` | exit 0 |
| `swift build --package-path apps/macos` | exit 0 |
| `bash -n scripts/acceptance/work-authority.sh scripts/acceptance/work-cutover-rollback.sh` | exit 0 |
| `git diff --check` | exit 0 |

The 12 top-level Rust ignores are unchanged environment/fixture boundaries:

- two Model Runtime subprocess entrypoints, explicitly invoked by A15/A16;
- one Work Coordinator restart subprocess entrypoint, explicitly invoked by
  its passing parent test;
- two daemon live synthesis/web smokes requiring protected BaseRT and public
  web access;
- two BaseRT live tests requiring the protected port-8082 service;
- one Apple Mail test requiring Full Disk Access, Mail Automation, and an
  uncached message;
- one Codex connector test requiring an authenticated CLI; and
- three Odoo connector tests requiring live Odoo and `mcp-server-odoo`.

None is represented as a pass or required to establish A18-A26. Zero-test
crate and doc-test targets are not counted as validation surfaces.

## Artifact integrity and repository state

Candidate SHA-256 hashes before independent review:

| Artifact | SHA-256 |
|---|---|
| `crates/daemon/migrations/V16__unified_work_cutover.sql` | `792c6710f25c3e254c34c98c1be85e8b6c4450f414e9df366053fb9d50865364` |
| `scripts/acceptance/work-authority.sh` | `050b6ad4f644622ba2c7243785e8f052525bee831a2101a80488dc6f7c9333c3` |
| `scripts/acceptance/work-cutover-rollback.sh` | `2bc4f1d1e814f64aab8c181ddadd42f8272692da85d4bec5861d02b6426273d7` |

Final post-review hashes, Markdown/link integrity, protected topology,
fixed-point review results, commit, push equality, and clean-worktree proof are
recorded below or on issue #30 once the immutable commit exists.

## Two-axis review

The mandated fixed point is
`20997ac43ff729b5b5dc874ebd64898aceb5867c`. Independent Standards and Spec
reviewers inspect the complete fixed-point diff after the green candidate is
committed. Every finding is repaired and both axes rerun until zero unresolved
findings remain.

Three rounds ran against this fixed point, each against the state left by the
previous round's fix commit:

1. Against `77167d3`+`97e6539`: Standards found no hard violations (judgement-
   call smells only, left as-is). Spec found the dead `dispatch_next`
   admission path — fixed by `97e6539` itself (committed before this round
   ran) plus the competing-semaphore removal, and the A22 harness-caller
   disclosure — see the first correction section above.
2. Against `77167d3`+`97e6539`+`de1b7fd`: Standards found no hard violations
   and confirmed the `release_slot` additions and the new static checker are
   internally consistent (one cosmetic latent bug noted in the checker's
   error-message line lookup — does not affect pass/fail). Spec found the
   slot-leak fix in `de1b7fd` was itself incomplete: `screen_intent_handler`
   was not spawn-detached, so a client-disconnect mid-classification could
   still leak the slot — fixed by `b067184`; see the third correction
   section above.
3. Against all four commits through `b067184`: independently re-verified the
   spawn-wrap genuinely closes that gap (traced the full handler body, cross-
   checked `req` ownership, re-confirmed all three `admit()` call sites — chat,
   screen-intent, automation — are each spawn-detached), hunted for a fourth
   defect in the same family across `model_runtime.rs` (its lease is already
   `Drop`-based RAII, unaffected) and the evidence orchestrator (transitively
   protected — only reachable from the already-spawned tool loop), and found
   none. Rebuilt, reran `work_concurrency` and `work-authority.sh`: all green.
   One disclosed, accepted behavior change (not a defect): a disconnected
   client's screen-intent/chat/automation request now always runs to
   completion in the background rather than being cancelled early, consistent
   across all three admission paths — the necessary tradeoff for closing the
   slot leak.

Zero unresolved findings remain as of `b067184`.

## Final closure verification

The target-locked closeout reran every command required by issue #30 against
`2d9193016a2d5a8c1744cc1b84939be743246170`. A18-A26 and the full regression
set passed. Every claimed focused test executed one case. No production defect
was found, so the closeout changes only this acceptance record and does not
require a fourth code-review round.

The final Stage 4 implementation and review history is exactly these five
commits, in order:

1. `77167d34ec234f5862a0b2e309457df79224b83d`: cut over unified Work authority.
2. `97e6539f7868be34f9e6f399f3c520cb69010c7e`: wire production admission through the fairness queue.
3. `de1b7fd49d1a2fa0bcd5282dce4486459072d54e`: close early-return capacity-slot leaks.
4. `b067184ec997c0b0ab58f66328baf937e071ef00`: protect screen-intent admission from client-disconnect cancellation.
5. `2d9193016a2d5a8c1744cc1b84939be743246170`: record the third zero-unresolved-finding review pass.

Final artifact hashes at that reviewed implementation commit:

| Artifact | SHA-256 |
|---|---|
| `crates/daemon/src/unified_work.rs` | `8416257d0c3f88c9c12aa6ecda6b10735fee60411db4dd07bfa11a45a7baf120` |
| `crates/daemon/src/main.rs` | `80c8f8035131e6d1c1a350d0e1c880fad34677822dbee8bf2b4fc2c1269cc3b0` |
| `crates/daemon/src/automations_api.rs` | `2d95b13fec2d82177a62281a796ed21416779d9a1b7c68c11948f351c938473b` |
| `crates/daemon/migrations/V16__unified_work_cutover.sql` | `792c6710f25c3e254c34c98c1be85e8b6c4450f414e9df366053fb9d50865364` |
| `scripts/acceptance/work-authority.sh` | `2c9c3153b53c971650cb4ef8154759529a2210cff3d1d9f36354f571c7eee255` |
| `scripts/acceptance/work-cutover-rollback.sh` | `2bc4f1d1e814f64aab8c181ddadd42f8272692da85d4bec5861d02b6426273d7` |

The acceptance record contains two Markdown links and no local file-link
targets. Its Markdown structure and link syntax passed the local checker;
authenticated GitHub reads resolved #29 as closed and #30 as open before
resolution. Shell syntax and `git diff --check` also passed.

The read-only protected-runtime state was identical before and after the final
verification. The installed-app daemon PID 758 recorded during the earlier
review no longer existed before this closeout and remained absent. Port 8080
had no listener before or after. Port 8082 remained owned by the same BaseRT
process, PID 792, started `2026-08-16 10:42:21 +0200`. No command in this
closeout bound, stopped, restarted, or otherwise touched either port, the live
database, installed app, launch agents, credentials, caches, or TCC state.

Standards verdict: zero hard violations and zero unresolved findings. The
earlier judgement-call duplication notes remain accepted and do not violate a
repository rule. Spec verdict: zero unresolved findings after all three review
rounds. Stage 5 is eligible after issue #30 closes, but no Stage 5 work was
created or started.

The five reviewed commits were pushed without rewriting history. Immediately
after that push, local `HEAD`, upstream, and the remote branch all resolved to
`2d9193016a2d5a8c1744cc1b84939be743246170`, and the worktree was clean. The
documentation-only closeout commit containing this section is pushed next;
the issue #30 resolution records its exact SHA and the final local/upstream/
remote equality and clean-worktree proof, since a commit cannot contain its
own SHA.
