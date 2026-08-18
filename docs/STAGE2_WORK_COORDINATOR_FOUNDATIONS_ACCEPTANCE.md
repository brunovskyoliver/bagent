# Stage 2 Work Coordinator foundations acceptance

Status: **PASS**

Ticket: [Stage 2: Build additive Work Coordinator foundations](https://github.com/brunovskyoliver/bagent/issues/28)

Base commit: `a32eca2a64d2b925c311a17d22e533697ff85f88`

Normative contracts:

- [Implementation sequence and acceptance gates](IMPLEMENTATION_SEQUENCE_ACCEPTANCE_GATES_DECISION.md)
- [Unified background-work state machine and event contract](UNIFIED_BACKGROUND_WORK_STATE_MACHINE_DECISION.md)

## Scope and module seam

Stage 2 adds migration `V15` and an otherwise unused Rust `WorkCoordinator`
module. Production chat, scheduler, approval, automation, event, model-runtime,
Swift, and UI paths do not call it. The legacy lifecycle remains the sole
production authority; there are no shadow or dual lifecycle writes.

The external test and caller interface is intentionally small:

```text
submit(command) -> Command Acknowledgement
snapshot() -> Work Snapshot
events(optional after Event Cursor, expected Daemon Generation) -> events or gap
```

The implementation owns SQLite transactions, Work identity generation,
revision compare-and-set, command-result idempotency, outbox cursor allocation,
retention, transactionally consistent snapshot/event reads, and restart
recovery. Production and
deterministic identity/clock adapters are internal seams. Tests use only an
isolated temporary SQLite path, fixed structural timestamps, and deterministic
opaque identities.

## Environment and starting state

- Verification date: 2026-08-18, Europe/Bratislava (`+02:00`).
- macOS 26.5.2 (25F84), arm64.
- `rustc 1.91.1`; `cargo 1.91.1`; Apple Swift 6.3.1.
- Initial local `HEAD` and upstream were equal at
  `a32eca2a64d2b925c311a17d22e533697ff85f88` (0 ahead, 0 behind).
- Initial worktree was clean.
- Stage 1 issue 25 was closed with A01-A04 PASS before issue 28 was created,
  linked to map 15, dependency-linked to issue 25, and claimed.

## Red-capable TDD evidence

The first exact A05 run used:

```text
cargo test -p bagentd --test work_coordinator persistence_atomicity -- --exact
```

It exited 101 because the `bagentd::work_coordinator` module did not yet exist.
After the minimal module and transaction were added, the same exact command
executed one test and passed.

A07 was later strengthened from a same-generation reopen to a real changed
Daemon Generation. The exact command exited 101 with the deterministic typed
failure `StaleDaemonGeneration`; moving durable command replay ahead of the
generation fence made lost-response replay succeed after restart without a
duplicate state transition or event.

A10's first execution exited 101 on an independent literal count error in the
fixture (the then-current worked sequence had 16 commits, not 15). Correcting
only the expected literal produced the required five recovery commits. Final
review then added one active Automation Run to make its recovery assertion
dynamic; the final sequence has 17 pre-restart commits and six recovery commits
at cursors 18-23. No product behavior was changed to accommodate the mistaken
initial expected value.

Focused strict Clippy also produced one deterministic in-scope failure,
`clippy::match_like_matches_macro`, in the legal-transition predicate. The
predicate was expressed with `matches!`; the focused and workspace strict
Clippy gates then passed.

## Final A05-A10 executions

Each exact command below ran between `2026-08-18T18:21:41+02:00` and
`2026-08-18T18:21:43+02:00`. Every filter executed exactly one non-ignored test.

| Criterion | Exact command | Result | Verdict |
|---|---|---:|---|
| A05 | `cargo test -p bagentd --test work_coordinator persistence_atomicity -- --exact` | 1 passed, 0 failed, 0 ignored | **PASS** |
| A06 | `cargo test -p bagentd --test work_coordinator revision_conflicts -- --exact` | 1 passed, 0 failed, 0 ignored | **PASS** |
| A07 | `cargo test -p bagentd --test work_coordinator command_idempotency_restart -- --exact` | 1 passed, 0 failed, 0 ignored | **PASS** |
| A08 | `cargo test -p bagentd --test work_coordinator event_ordering_cursor_gap -- --exact` | 1 passed, 0 failed, 0 ignored | **PASS** |
| A09 | `cargo test -p bagentd --test work_coordinator snapshot_reconnect -- --exact` | 1 passed, 0 failed, 0 ignored | **PASS** |
| A10 | `cargo test -p bagentd --test work_coordinator daemon_restart_recovery -- --exact` | 1 passed, 0 failed, 0 ignored | **PASS** |

### A05 — persistence atomicity

Failure injection before the transaction, after the Work mutation, after the
outbox insert, and immediately before commit leaves the original Work at
revision 1/cursor 1 with no command result or orphan event. Retrying commits
revision 2/cursor 2 once. `PRAGMA integrity_check` returns `ok` after every
injection and retry.

The same transaction creates the required origin projection records. A second
active Automation Run for the same historical definition fails the partial
unique constraint and rolls back without creating Work, session, projection,
command-result, or cursor residue.

### A06 — revision conflicts

Two independent coordinator handles synchronize on the same prior revision.
Exactly one writer commits revision 2; the other returns typed
`Conflict { current_revision: Some(2) }`. The next legal mutation commits
revision 3/cursor 3, proving the losing compare-and-set creates no revision or
cursor gap.

### A07 — idempotency through restart

A transition commits and injects response loss after commit. Reopening with a
new Daemon Generation performs recovery, while replaying the byte-identical
old command returns the original `Committed` acknowledgement and identical
receipt at revision 2/cursor 2. The Work has one recovery transition at
revision 3/cursor 3; the original transition/event is not duplicated. Reusing
the command identity with different content returns `CommandIdentityConflict`.

### A08 — ordering and explicit cursor gaps

Concurrent conversation and automation origins receive unique cursor 1 and 2
in database commit order. After a two-event retention window advances to
cursors 3 and 4, resume from cursor 2 returns ordered events 3 and 4. Resume
from cursor 1 returns an explicit `Gap` containing the authoritative snapshot;
it is never treated as continuous delivery. A missing cursor (`None`) also
returns an explicit snapshot-required gap. Cursor bounds, retained-event
selection, and the gap snapshot are read in one SQLite read transaction.

### A09 — snapshot/reconnect convergence

A projection starting from cursor 2 converges exactly to a fresh authoritative
snapshot after replaying retained cursors 3 and 4. Re-delivering the same
events does not duplicate activity or regress a Work Revision. After cursor 5
expires, reconnect returns a snapshot-required gap whose snapshot equals a
fresh authoritative read. Both the fresh snapshot and reconnect response are
transactionally consistent across their component queries.

### A10 — daemon restart recovery

The parent acceptance test launches the isolated test executable as a real
subprocess against the persisted temporary SQLite database. The fixture holds
Conversation Work in queued, waiting-for-model, running,
waiting-for-approval, cancelling, and completed states, plus a queued active
Automation Run, a pending approval, and prior Model Runtime trust. Opening
under a new Daemon Generation atomically abandons all six nonterminal Works
and the pending approval, deactivates the Automation Run, records five
Conversation interruption markers, clears prior Model Runtime trust, preserves
completed Work unchanged, and appends ordered cursors 18-23. Assertions inspect
the Automation Run as active before restart and inactive afterward. The
constructor returns only after recovery is complete. The old generation
receives a snapshot-required gap and new commands fenced to it return typed
`StaleDaemonGeneration`.

## Focused, relevant, and full regressions

| Gate | Result |
|---|---|
| `cargo test -p bagentd --test work_coordinator` | 6 acceptance tests passed, 0 failed; 1 subprocess entrypoint ignored by the outer runner and executed 1/1 by A10 |
| `cargo test -p bagentd` | 267 outer tests passed (261 existing + 6 Stage 2), 0 failed; 3 existing environment-dependent ignores plus the test-only subprocess entrypoint; A10 child 1/1 passed |
| `cargo test --workspace` | 466 outer tests passed, 0 failed; 11 existing environment-dependent ignores plus the test-only subprocess entrypoint; A10 child 1/1 passed |
| `swift test --package-path apps/macos list` | 53 tests discovered |
| `swift test --package-path apps/macos` | 53 passed, 0 failed |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 under the unchanged Stage 1 scoped waiver policy |
| `cargo build --workspace` | exit 0 |
| `swift build --package-path apps/macos` | exit 0 |
| `git diff --check` | exit 0 |
| `git diff --no-index --check /dev/null PATH` for each new file | expected diff exit 1, no whitespace-error exit 2 |

The 11 pre-existing Rust ignores are the unchanged Stage 1 inventory requiring
Mail/TCC, BaseRT on port 8082, Codex CLI, or live Odoo. None was represented as
executed or promoted to PASS. The additional ignored function is only the
subprocess entrypoint; A10 explicitly invokes it and proves its nonzero 1/1
execution.

## Two-axis code review

The first independent Standards review reported two judgment-call findings:
interchangeable primitive identifiers and repeated lifecycle vocabulary. The
first independent Spec review reported five findings: nontransactional
multi-query reads, no missing-cursor representation, a replay acknowledgement
that differed from the original, incomplete in-process A10 recovery evidence,
and missing additive structural tables/constraints.

All seven findings were fixed in scope. The public seam now uses focused
newtypes for Command, Work, Current Chat, Conversation Turn, Automation Run,
Automation Session, Automation Definition, Approval, Daemon Generation, Model
Runtime Generation, Work Revision, Automation Definition Revision, and Event
Cursor; terminal-state selection uses the Rust lifecycle authority;
snapshot/events use SQLite read transactions; `events(None, ...)` is an
explicit gap; exact replay returns the original acknowledgement; A10 crosses a
subprocess boundary and covers approval, interruption, Automation Run, and
Model Runtime trust recovery; and V15 includes the accepted
origin/session/approval/projection/continuation/recovery structures and
one-active-run/one-pending-approval constraints.

The second review pass found two residual Standards judgment calls (remaining
opaque identity strings and a misleading `monotonic_identity` macro name) and
one Spec evidence overclaim (Automation Run deactivation existed but was not
exercised by A10). These were fixed by the complete focused-newtype seam, the
`monotonic_value` name, and a dynamically asserted active Automation Run in the
subprocess recovery fixture.

The final independent rereview of commit-tree `a613a60` reported **zero
unresolved Standards findings** and **zero unresolved Spec findings**. It
confirmed all earlier findings resolved, all hashes matched, and no production
caller, lifecycle cutover, Swift/UI surface, protected runtime, port, or Stage 3
behavior entered the diff.

## Static authority, links, and artifact integrity

`rg -n "work_coordinator|WorkCoordinator" crates/daemon/src crates/daemon/tests`
found references only in the new library export, new module, and new integration
test. It found no production route, scheduler, approval, automation, event, or
model-runtime caller.

Authenticated link checks resolved map 15 (open), decisions 23 and 24 (closed),
Stage 1 issue 25 (closed), and Stage 2 issue 28 (open during implementation).
Both local normative documents exist.

SHA-256 after review repairs and before the final commit:

- migration: `69694e2116283b0aa0e0f5ec2b9f65356f21c1b622568e08e68429f8505002e8`
- library export: `e74121412a5174a6997a978683fae1bf43dbc765e6d458aa38f2c26b87c58d78`
- Work Coordinator module: `b62c17444cc8978fcd5ab1db44cfa30b204db442f9d9c594c0a10054e30ca815`
- A05-A10 integration target: `8a3106748ff58530df6ddd7030ae9e7df8daa599d136edbc3232e92de69ba5b1`

## Cleanup and protected state

Every test used `tempfile` directories and isolated SQLite files; their guards
removed them. No production database path was opened by the Stage 2 target.
No application, daemon, BaseRT process, TCC state, protected data, port 8080,
or port 8082 was started, stopped, configured, read, or probed. No dependency
was added or upgraded. No Swift/UI or Stage 3 Model Runtime file was changed.

Stage 2 acceptance is PASS after the final gates and zero-finding two-axis
review. Push equality, clean-worktree state, and ticket closure are recorded on
issue 28 because they occur after this artifact is committed. Stage 3 becomes
eligible on closure but remains untouched.
