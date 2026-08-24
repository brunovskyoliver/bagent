# Stage 3 Model Runtime cutover acceptance

## Scope and starting state

Stage 3 cuts bagent's port-8082 BaseRT authority over atomically to one
daemon-owned Model Runtime. Stage 4 is not part of this change.

- Required base and initial `HEAD`: `dc37a0b07a12384bb78d066769da361c7c140713`.
- Branch: `t3code/basert-notch-automation-ux`.
- Initial local `HEAD`, upstream, and remote branch were equal; `git status
  --short` was empty.
- Stage 3 issue: [#29](https://github.com/brunovskyoliver/bagent/issues/29),
  assigned to `brunovskyoliver` with `ready-for-agent` and `wayfinder:task`.
- Wayfinder map: [#15](https://github.com/brunovskyoliver/bagent/issues/15).
- Native prerequisite: closed Stage 2 [#28](https://github.com/brunovskyoliver/bagent/issues/28).
- GitHub reported `blocked_by=0`, `total_blocked_by=1` before implementation.
- No duplicate open Stage 3 or Model Runtime authority ticket existed.

The implementation was based on complete reads of
`IMPLEMENTATION_SEQUENCE_ACCEPTANCE_GATES_DECISION.md`,
`UNIFIED_BACKGROUND_WORK_STATE_MACHINE_DECISION.md`,
`BASERT_RESIDENCY_TRANSITIONS_RESEARCH.md`,
`STAGE2_WORK_COORDINATOR_FOUNDATIONS_ACCEPTANCE.md`, `CONTEXT.md`, and the
app-facing boundary in `UI_DESIGN.md`.

## Protected runtime baseline and restoration

The read-only baseline was captured at `2026-08-18T16:32:27Z` before any
subprocess acceptance test:

| Protected surface | Initial state |
|---|---|
| Installed app daemon | `/Applications/bagent.app/Contents/MacOS/bagentd`, launchd `com.bagent.daemon`, PID 758 |
| Managed BaseRT | `/Users/oliver/.basert/basert-serve`, launchd `com.bagent.basert`, PID 792 |
| IPv4 port 8080 | Docker listener PID 74402 |
| IPv4 port 8082 | BaseRT listener PID 792 |
| Loaded weights | Not safely observable without using the protected credential; deliberately not claimed |

A15 and A16 used disposable subprocesses bound only to IPv6 loopback
`[::1]:8082` and `[::1]:8080`; the protected services own IPv4
`127.0.0.1`. Fixtures were child processes of the integration-test binary,
were reaped by `Drop`, and were serialized by an async exact-port lock in the
complete test target.

The final read-only inspection at `2026-08-18T17:43:27Z` through
`2026-08-18T17:43:29Z` exactly matched the
initial topology: daemon PID 758, BaseRT PID 792, Docker PID 74402 on 8080, and
BaseRT PID 792 on 8082. No installed application, launch agent, model cache,
credential, TCC state, user database, protected listener, or protected process
was changed. The candidate configuration disables BaseRT's autonomous idle
unload (`--idle-timeout 0`, confirmed by the installed binary's read-only
`serve --help`) so the daemon-owned 20-minute policy is the sole authority when
the candidate is later deployed.

## Source graph before and after

At the fixed base, the production graph contained all of the following:

- Swift `ensureBaseRTRunning`, direct health/model probes, launchd BaseRT
  bootout/bootstrap, and the 35B-to-4B fallback path variable;
- direct `BaseRtClient` ownership in the daemon entry point and agent
  classifiers;
- direct completion helpers in `agent_exec.rs`;
- synthesis-only `ModelRuntimeManager` and `SynthesisModelClient`;
- a separate `runtime_control::restart_managed_basert` path;
- acceptance-feature lifecycle delegation; and
- a caller-owned `RwLock<()>` lifecycle guard inside `BaseRtClient`.

After cutover, `crates/daemon/src/model_runtime.rs` is the only production file
that contains `BaseRtClient` lifecycle or completion operations. Agent
classifiers use `AgentInference`; foreground and automation paths enter through
`TypedModelRuntime` with typed origin and Work Identity. Swift supplies static
configuration only. The synthesis manager, runtime-control module, fallback
model path, connector lifecycle lock, shutdown unload path, and all other
production callers are removed.

`scripts/acceptance/model-runtime-authority.sh` scans every Rust production
source below `crates` and all Swift app sources, strips only `#[cfg(test)]` Rust
items, and rejects lifecycle/completion/readiness/poison calls, service mutation,
fallback managers, and duplicate guards. Its seeded red check detects 13
independently forbidden categories. The first production scan failed with 85
forbidden matches and four missing adapter operations; the final scan reports
zero forbidden production matches and no missing operation.

## Model Runtime boundary

The closed runtime state set is `unavailable`, `unloaded`, `loading(model)`,
`loaded_not_ready(model)`, `ready(model)`, `retiring(model)`,
`poisoned(model)`, and `restarting`. A single async transition lock serializes
all lifecycle mutation. Only `ready(model)` grants a lease.

Each non-preemptible lease carries Work Identity, model class, and Model Runtime
Generation. Same-model demand joins ready residency; different-model demand
waits for every lease to drain. Ordering is foreground first, Automation Run
FIFO second, speculative preload last. Speculative preload has no Work, slot,
or lease and starts only a discardable idle timer.

The production adapter alone owns service provisioning, model registration,
load/readiness/unload, completion dispatch, launchd changed-PID restart, health,
zero-loaded-weight proof, and measured memory recovery. Startup establishes a
clean changed-PID generation before admission. Loading either model consumes
that clean boundary, so every later new 35B residency requires another healthy
changed PID with zero weights. Cold 35B admission requires both 25% free memory
and 8 GiB estimated available; a healthy warm 35B residency is not regated.
35B retirement is unload, changed-PID restart, health, and zero-weight proof.
Verified 4B retirement additionally waits for measured headroom to return.

Metal/device/command-buffer failures, completion cancellation, a stream ending
without `[DONE]`, and indeterminate timeouts retain the lease and poison the
generation. No work is admitted until recovery advances the generation across
a healthy changed-PID, zero-weight boundary. Lifecycle failures also leave the
generation poisoned (or unavailable during initial service setup), never
half-ready.

Every lifecycle await is protected by an armed cancellation guard: dropping a
load/readiness, retirement, or restart/proof future fails the generation closed.
Demand cancellation removes its queued identity. Graceful shutdown closes
admission, waits behind the lifecycle serializer, drains active leases or
recovers indeterminate leases across a changed-PID zero-weight boundary, and
retires whichever model is actually resident before returning success. The
production changed-PID proof correlates the launchd PID with the exact IPv4
8082 listener and polls through delayed bind until that same PID is healthy.

## Sequential red-green gates

The slices were implemented in A11-A17 order. Red capability was observed
before production implementation: A11 first failed on the absent module; A12
on absent lease/clock types; A13 on absent maintenance actions; A14 on absent
35B retirement/proofs; A15 on absent poison/recovery and then on a deliberately
invalid IPv4 fixture bind; A16 on the absent preload interface; and A17 on the
85-call legacy production graph. The A15 fixture was corrected to genuinely
isolated IPv6 loopback rather than weakening the assertion.

Final timestamped run: `2026-08-18T17:42:06Z` through
`2026-08-18T17:42:08Z`.

| Gate | Exact command | Exact result |
|---|---|---|
| A11 | `cargo test -p bagentd --test model_runtime speculative_preload -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 7 filtered |
| A12 | `cargo test -p bagentd --test model_runtime lease_residency -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 7 filtered |
| A13 | `cargo test -p bagentd --test model_runtime idle_retirement -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 7 filtered |
| A14 | `cargo test -p bagentd --test model_runtime retirement_35b -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 7 filtered; 35B special retirement, both-class shutdown, active/indeterminate lease shutdown, and blocked-transition serialization |
| A15 | `cargo test -p bagentd --test model_runtime poison_changed_pid -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 7 filtered; controlled device failure, cancellation-safe load/retirement/recovery, retained lease, changed PID, health, zero weights, generation advance |
| A16 | `cargo test -p bagentd --test model_runtime port_isolation -- --exact` | PASS: 1 passed, 0 failed, 0 ignored, 7 filtered; disposable 8080 PID/request count/state hash and protected IPv4 listener PIDs unchanged |
| A17 | `scripts/acceptance/model-runtime-authority.sh` | PASS: 13 seeded forbidden matches; zero forbidden production matches; sole adapter |

The complete Model Runtime target passes 6 executable tests, 0 failures, and 2
ignored subprocess entrypoints. Both ignored entrypoints are explicitly and
nonzero invoked by A15/A16; they are not unexecuted behavior claims.

## Full verification

| Command | Result |
|---|---|
| `cargo test -p basert-connector --test protocol` | 14 passed, 0 failed, 0 ignored |
| `cargo test -p bagentd --bin bagentd` | 226 passed, 0 failed, 2 ignored live-only tests |
| `cargo test -p bagentd --test work_coordinator` | 6 outer tests passed, 0 failed, 1 subprocess entrypoint ignored; its recovery scenario invokes the child 1/1 |
| `cargo test --workspace` | 453 top-level tests discovered; 441 passed, 0 failed, 12 ignored; one nested Work Coordinator child result is separately 1/1 and is not double-counted |
| `swift test --package-path apps/macos list` | 52 tests discovered |
| `swift test --package-path apps/macos` | 52 passed, 0 failed |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0 under strict repository lint policy |
| `cargo build --workspace --all-targets` | exit 0 |
| `cargo check -p bagentd --all-targets --features stage8-acceptance` | exit 0 |
| `swift build --package-path apps/macos` | exit 0 |
| `git diff --check` | exit 0 |
| `scripts/acceptance/model-runtime-authority.sh` | exit 0; seeded red detector and production graph both PASS |

Focused Rust and full-workspace verification ran from `2026-08-18T17:42:17Z`
through `2026-08-18T17:42:47Z`. Format, strict Clippy, builds, Swift, whitespace,
script syntax, and link verification ran from `2026-08-18T17:43:03Z` through
`2026-08-18T17:43:12Z`.

The 12 top-level Rust ignores and reasons are:

- two Model Runtime subprocess entrypoints, explicitly executed by A15/A16;
- one Work Coordinator restart subprocess entrypoint, explicitly executed by
  its parent test;
- two daemon live synthesis/web smokes requiring the app-managed BaseRT and, for
  one, public web access;
- two BaseRT live tests requiring the protected port-8082 service;
- one Apple Mail test requiring Full Disk Access, Mail Automation, and an
  uncached message;
- one Codex connector test requiring an authenticated CLI; and
- three Odoo connector tests requiring live Odoo and `mcp-server-odoo`.

None was executed against protected runtime, represented as a pass, or needed
to establish an A11-A17 verdict. Zero-test crate and doc-test targets are not
used as validation claims.

## Artifact integrity, links, and repository state

Final SHA-256 hashes after the zero-finding review pass and before commit:

| Artifact | SHA-256 |
|---|---|
| `crates/daemon/src/model_runtime.rs` | `54332e50005cd07f8f4415709c227a4fb6de009695e0a8c3ed8df09c0d04a783` |
| `crates/daemon/tests/model_runtime.rs` | `c243afcb198501f6d0a683be8d6d9f28c584cbcc1ac47858bdc9bddc6f98158b` |
| `scripts/acceptance/model-runtime-authority.sh` | `70dcbc5ad88e22d92251352535987ddaa1a66913b239391abd782ee67afe6839` |
| `crates/connectors/basert/src/lib.rs` | `4438d1724b295abd3a542ba187882d68945b648d21fd045a5307f3156e7ff8f0` |
| `crates/daemon/src/evidence/synthesis.rs` | `fe7c4963419ef38248c5b4a6c1a035dfcf641bdd04b561fb0d7121baaf54c59d` |
| `apps/macos/Sources/bagent/DaemonLauncher.swift` | `56abf654ab13556d80e7fc1e8a30ab25915b847de1ed76a5cbc16a8c8211c291` |

The local Markdown checker inspected 114 links with zero broken. Authenticated
GitHub reads verified #15 open, #28 closed, and #29 open before resolution.
Script syntax and `git diff --check` both passed. No dependency was added or
upgraded.

The validated pre-commit candidate contains only authorized Stage 3 source,
tests, acceptance script, and this evidence document. Commit, push equality,
clean-worktree proof, resolution comment, and closure are recorded on #29 after
the immutable commit exists. Stage 4 is eligible after closure but remains
untouched.

## Two-axis review

The mandated review fixed point is
`dc37a0b07a12384bb78d066769da361c7c140713`. Independent Standards and Spec
reviewers repeatedly inspected the full fixed-point diff. Findings covering
registry safety, queue and transition cancellation, priority rechecks, plist
replacement, exact listener/PID proof, source-graph coverage, synthesis
fallback, requested/current retirement, and shutdown races were repaired and
re-reviewed. Final verdicts: Standards — zero unresolved findings; Spec — zero
unresolved findings.
