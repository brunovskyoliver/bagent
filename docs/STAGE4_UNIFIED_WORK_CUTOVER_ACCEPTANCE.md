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

Final exact run: `2026-08-18T20:12:46Z` through
`2026-08-18T20:12:56Z`.

| Gate | Exact command | Exact result |
|---|---|---|
| A18 | `cargo test -p bagentd --test work_concurrency fairness_foreground -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 3 filtered; bounded foreground priority and aged Automation progress |
| A19 | `cargo test -p bagentd --test work_concurrency automation_capacity_two -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 3 filtered; exactly two distinct Automation executions and zero slot leak |
| A20 | `cargo test -p bagentd --test work_concurrency approval_restart_capacity -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 3 filtered; same approval identity survives restart, capacity is free, one valid decision resumes, stale decision conflicts |
| A21 | `cargo test -p bagentd --test work_concurrency cancellation_races -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 3 filtered; queued/loading/executing/approval/completion races, one terminal outcome, zero leases/slots |
| A22 | `cargo test -p bagentd --test work_failure_injection` | PASS: 1 passed, 0 failed; admission, persistence, outbox, runtime, tool, approval, and completion failpoints close deterministically |
| A23 | `cargo test -p bagentd --test persistence_migration legacy_run_records -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 1 filtered; bounded/idempotent privacy-safe records and safe active abandonment |
| A24 | `cargo test -p bagentd --test privacy_contract work_surfaces -- --exact` | PASS: 1 passed, 0 failed; canaries absent from events, snapshots, projections, logs, and diagnostics; unknown fields rejected |
| A25 | `cargo test -p bagentd --test persistence_migration cutover_boundary -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 1 filtered |
| A25 rollback | `scripts/acceptance/work-cutover-rollback.sh` | PASS: checksum backup/restore, old-reader fixture, interruption boundaries, first-Work barrier, archive disclosure |
| A26 | `scripts/acceptance/work-authority.sh` | PASS: seeded detector found 10 forbidden categories; zero forbidden production authorities; canonical command/event path present |

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
duplicate Work Identity, direct canonical Work SQL writers, and caller-owned
model demand. Final production findings are zero. The only canonical Work SQL
writers are `work_coordinator.rs` and the bounded cutover module.

## Full verification

| Command | Result |
|---|---|
| A11-A16 exact Model Runtime commands | 6 independently exact tests passed, 0 failed |
| `scripts/acceptance/model-runtime-authority.sh` | PASS: 13 seeded forbidden matches and zero forbidden production matches |
| `cargo test -p basert-connector --test protocol` | 14 passed, 0 failed, 0 ignored |
| `cargo test -p bagentd` | 248 top-level tests passed, 0 failed, 5 ignored; nested restart child passed 1/1 |
| `cargo test --workspace` | 460 top-level tests discovered; 448 passed, 0 failed, 12 ignored; nested restart child passed 1/1 and is not double-counted |
| `swift test --package-path apps/macos` | 52 passed, 0 failed |
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
