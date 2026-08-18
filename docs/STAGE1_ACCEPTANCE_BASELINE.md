# Stage 1 acceptance baseline

Date: 2026-08-17
Ticket: [Stage 1: Establish an honest acceptance baseline](https://github.com/brunovskyoliver/bagent/issues/25)
Decision: [Implementation sequence and acceptance gates](IMPLEMENTATION_SEQUENCE_ACCEPTANCE_GATES_DECISION.md)
Related completed planning map: [Wayfinder issue 15](https://github.com/brunovskyoliver/bagent/issues/15)

## Current status

Stage 1 is complete. The final post-#27 verification below supersedes the two
historical failed verification runs while retaining them as evidence. A01, A02,
A03, and A04 are all **PASS**, so issue 25 may close and Stage 2 becomes eligible.
No Stage 2 work is included or begun here.

## Scope and starting state

- Pre-ticket revision: `53d5f937f0bd73bc63486b9c20f107b0262f62ba`.
- Branch: `t3code/basert-notch-automation-ux`.
- Before work, `HEAD` and `origin/t3code/basert-notch-automation-ux` were equal (`0` ahead, `0` behind), and the worktree had no staged, modified, or untracked files.
- Confirmed public seams: Rust workspace discovery/execution; SwiftPM discovery/execution; independent static-script status/count output; repository formatting, lint, and build commands.
- No production behavior, dependency, application, daemon, BaseRT, TCC, port 8080, or port 8082 state was changed.

Environment: macOS 26.5.2 (25F84), arm64; `rustc 1.91.1`; `cargo 1.91.1`; Apple Swift 6.3.1. The Stage 1 run began with issue creation at 2026-08-17T17:26:04+02:00 and ended at 2026-08-17T17:40:07+02:00. Per-command timestamps were not captured before execution, so criterion-level start/end timestamps remain UNVERIFIED rather than reconstructed.

## Before-change baseline

| Criterion | Command | Baseline result |
|---|---|---|
| A01 | `cargo test --workspace -- --list` | Exit 101: four unresolved `TavilyConfiguration*` references in the daemon test module prevented listing. |
| A01 | `cargo test --workspace` | Exit 101 on the same four compilation errors; no workspace PASS claimed. |
| A02 | `swift test --package-path apps/macos list` | Exit 1: stale `TavilyConfigurationSynchronizer`, status, and health-fixture references prevented listing. |
| A03 | Seven static scripts run separately | Three PASS; fullscreen, non-notch, scroll, and combined output/Mail scripts FAIL. No script reported an assertion count, and output/Mail findings shared one verdict. |
| A04 | `cargo fmt --all -- --check` | Exit 1 on stale Rust test formatting. |
| A04 | `cargo clippy --workspace --all-targets -- -D warnings` | Exit 101; first failure class was pre-existing memory lint debt. |
| A04 | `swift build --package-path apps/macos` | Exit 0. |
| A04 | `git diff --check` | Exit 0. |

## Corrections and red capability

- The obsolete Rust and Swift Tavily tests were restored unchanged after review found that removing their privacy/readiness/retry/reconnect assertions weakened meaningful coverage. Retargeting those behavioral guarantees requires a new injectable production seam and is outside Stage 1.
- Split the combined notch-output/Mail static check into independently executable notch-layout and Mail-rendering scripts.
- Added `scripts/run-static-acceptance.sh`, which emits one normalized verdict, positive assertion count, and exit status per invocation.
- `scripts/test-static-acceptance-red-capability.sh` first proved that a zero-assertion script is BLOCKED, then ran a marked synthetic exit-7 assertion. The runner exited nonzero and emitted `verdict=FAIL assertions=1 exit_status=7`; it did not promote the failure.
- Added crate-specific lint waivers only for the enumerated pre-existing lint classes. Every waiver is owned by issue 25, explains the Stage 1 scope boundary, and expires on 2026-09-17.

## Historical verification evidence — superseded

### A01 — FAIL

- Final `cargo test --workspace -- --list`: exit 101 on four unresolved `TavilyConfiguration*` references restored to preserve the meaningful privacy test.
- Final `cargo test --workspace`: exit 101 on the same compile blocker; no final test execution is claimed.
- During the reviewed but rejected test-weakening attempt, listing reached 467 cases and execution exposed a second deterministic production blocker: `distinct_irrelevant_fetches_are_not_reported_as_duplicate_activities` expected two completions but observed four. A focused exact rerun failed. This diagnostic is not promoted to final A01 execution proof.
- Both blockers require production-interface or evidence-orchestrator changes prohibited in Stage 1.

### A02 — FAIL

- Final `swift test --package-path apps/macos list`: exit 1 on the restored stale synchronizer/status/health fixture references.
- Final `swift test --package-path apps/macos`: exit 1 on the same compilation blocker; no test execution is claimed.
- A reviewed but rejected compile-only replacement listed and executed 49 XCTest cases. That result is not accepted because it removed meaningful readiness, retry, PID, reconnect, and absent-credential behavior coverage.

### A03 — PASS

The changed acceptance infrastructure is independently attributable and red-capable. The selected surfaces were run separately through `scripts/check-stage1-static-acceptance.sh`:

| Surface | Assertions | Verdict | Exit |
|---|---:|---|---:|
| Inline input focus | 6 | PASS | 0 |
| Input overflow non-promotion | 10 | PASS | 0 |
| Thinking/output dot layer | 22 | PASS | 0 |
| Fullscreen notch visibility | 6 | FAIL | 1 |
| Non-notch inline pill | 6 | FAIL | 1 |
| Notch output scroll stability | 17 | FAIL | 1 |
| Notch output layout | 19 | PASS | 0 |
| Mail rendering | 7 | FAIL | 1 |

Total: 93 source-measured assertions. The runner rejects zero or mismatched counts before execution. The four failing surfaces are pre-existing product findings, remain independently visible, and do not change the A03 infrastructure verdict.

### A04 — FAIL

- Final `cargo fmt --all -- --check`: exit 1 on the restored stale Rust test formatting.
- Final `cargo clippy --workspace --all-targets -- -D warnings`: exit 101 because the restored stale Rust test does not compile. The command did exit 0 with the specific temporary waivers while the weakening attempt was present; that result is not the final repository state.
- `swift build --package-path apps/macos`: exit 0.
- `git diff --check`: exit 0.

Temporary waivers, all owner issue 25, reason pre-existing production lint debt outside Stage 1, expiry 2026-09-17:

- `bagent-memory`: Rust `dead_code`; Clippy `unnecessary_map_or`, `too_many_arguments`, `await_holding_lock`.
- `filesystem-connector`: Rust `unused_variables`, `dead_code`; Clippy `collapsible_if`.
- `apple-mail-connector`: Clippy `single_match`, `manual_strip`.
- `bagent-rules`: Clippy `derivable_impls`.
- `codex-connector`: Rust `unused_variables`; Clippy `ptr_arg`.
- `whatsapp-connector`: Clippy `ptr_arg`, `derivable_impls`, `redundant_closure`, `unnecessary_lazy_evaluations`.
- `bagent-agent`: Clippy `empty_line_after_doc_comments`, `too_many_arguments`, `bool_assert_comparison`.
- `bagentd`: Clippy `nonminimal_bool`, `collapsible_if`, `single_match`, `needless_question_mark`, `items_after_test_module`, `too_many_arguments`.

## Cleanup and final state

- Static-runner temporary output files are removed by each runner invocation.
- No generated report was retained. Compiler build artifacts remain in the ordinary ignored build directories; no destructive cleanup was performed.
- Test processes exited. The installed application, daemon, BaseRT, TCC state, port 8080, and port 8082 were not started, stopped, reloaded, reconfigured, or probed.
- Link validation: a local Markdown-target check found one local link and zero missing files; authenticated GitHub fetches resolved issues 15 and 25. Unauthenticated `curl` returned 404 for the private repository and is not treated as a broken-link verdict.
- Acceptance artifact SHA-256: runner `2e6ad44e0e44172170f7a88531a9209d0720a881947be5b76a4bd357dff6483e`; red-capability test `3afc62a99e8035def230193ee600d9749ed00fbfa1ddd5f69fc418abdf9a11c6`; notch split `b4811fbce4c3873f2046189ba4b131cc5bb5c0c9b3ba13f0f69c9a6bb129c032`; Mail split `316909f8ccc876f2ee95c850799bd76dbc9c9abdbb64557711de456c6e188999`. Fixture identity is the synthetic marked exit-7 script generated by `test-static-acceptance-red-capability.sh`; no private content is used.
- Stage 1 is not complete because A01, A02, and A04 are FAIL. Criterion-level timing metadata is UNVERIFIED. Issue 25 must remain open, and Stage 2 is not eligible.

## Post-prerequisite resumed verification — 2026-08-18

This section preserves the 2026-08-17 record above and reports a fresh run after
[issue 26](https://github.com/brunovskyoliver/bagent/issues/26) was resolved.

### Starting state and environment

- Base commit, initial `HEAD`, and initial upstream:
  `aa986da0661865330414ed1b86236551b5bafc8f`.
- Branch: `t3code/basert-notch-automation-ux`.
- Run start: `2026-08-18T16:12:01+02:00`; run end:
  `2026-08-18T16:18:25+02:00` (Europe/Bratislava).
- Environment: macOS 26.5.2 (25F84), arm64; `rustc 1.91.1`;
  `cargo 1.91.1`; Apple Swift 6.3.1.
- The initial dirty paths exactly matched the Stage 1 paths recorded in issue 26's
  resolution. No Tavily production path or unrelated path was dirty. No reset,
  restore, stash, broad staging, broad formatting, dependency change, or runtime
  operation was performed.

### Fresh commands and results

| Criterion | Start | End | Exact command | Exit | Nonzero result |
|---|---|---|---|---:|---|
| A01 | 16:12:23 | 16:12:24 | `cargo test --workspace -- --list` | 0 | 468 tests discovered |
| A01 | 16:12:40 | 16:12:41 | `cargo test -p bagentd evidence::orchestrator::tests::distinct_irrelevant_fetches_are_not_reported_as_duplicate_activities -- --exact` | 101 | 1 executed; 0 passed, 1 failed |
| A01 | 16:12:49 | 16:12:57 | `cargo test --workspace` | 101 | Cargo reached 366 cases: 362 passed, 1 failed, 3 ignored; stopped at the daemon failure |
| A02 | 16:13:07 | 16:13:11 | `swift test --package-path apps/macos list` | 0 | 53 tests discovered, including all 5 `TavilyConfigurationSyncTests` cases |
| A02 | 16:13:19 | 16:13:22 | `swift test --package-path apps/macos` | 0 | 53 passed, 0 failed; Tavily 5/5 passed |
| A03 | 16:13:35 | 16:13:35 | `scripts/run-static-acceptance.sh inline-input-focus 6 scripts/check-inline-input-focus-retention.sh` | 0 | PASS, 6 assertions |
| A03 | 16:13:35 | 16:13:35 | `scripts/run-static-acceptance.sh input-overflow-non-promotion 10 scripts/check-no-input-overflow-promotion.sh` | 0 | PASS, 10 assertions |
| A03 | 16:13:35 | 16:13:36 | `scripts/run-static-acceptance.sh thinking-output-dot-layer 22 scripts/check-notch-thinking-output-dot-layer.sh` | 0 | PASS, 22 assertions |
| A03 | 16:13:36 | 16:13:36 | `scripts/run-static-acceptance.sh fullscreen-notch-visibility 6 scripts/check-fullscreen-notch-visibility.sh` | 1 | FAIL, 6 assertions |
| A03 | 16:13:36 | 16:13:36 | `scripts/run-static-acceptance.sh non-notch-inline-pill 6 scripts/check-non-notch-inline-pill.sh` | 1 | FAIL, 6 assertions |
| A03 | 16:13:36 | 16:13:36 | `scripts/run-static-acceptance.sh notch-output-scroll-stability 17 scripts/check-notch-output-scroll-stability.sh` | 1 | FAIL, 17 assertions |
| A03 | 16:13:36 | 16:13:36 | `scripts/run-static-acceptance.sh notch-output-layout 19 scripts/check-notch-output-regressions.sh` | 0 | PASS, 19 assertions |
| A03 | 16:13:36 | 16:13:36 | `scripts/run-static-acceptance.sh mail-rendering 7 scripts/check-mail-rendering-regressions.sh` | 1 | FAIL, 7 assertions |
| A03 | 16:13:49 | 16:13:49 | `scripts/check-stage1-static-acceptance.sh` | 1 | Eight independent verdicts, 93 assertions; aggregate preserves four product-surface failures |
| A03 | 16:13:50 | 16:13:50 | `scripts/test-static-acceptance-red-capability.sh` | 0 | Zero count was BLOCKED; the marked 1-assertion synthetic fixture exited 7 and remained FAIL |
| A04 | 16:13:57 | 16:13:58 | `cargo fmt --all -- --check` | 0 | PASS |
| A04 | 16:14:03 | 16:14:09 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | PASS under the documented scoped waiver policy |
| A04 | 16:14:18 | 16:14:18 | `swift build --package-path apps/macos` | 0 | PASS |
| A04 | 16:14:18 | 16:14:19 | `git diff --check` | 0 | PASS |

All timestamps above are on 2026-08-18 with offset `+02:00`.

### Focused orchestrator diagnosis

The exact test first reproduced the recorded `left: 4`, `right: 2` completion
count. Its scripted adapter returns the same two candidates for both the initial
and diversified searches. The second search therefore produces two legitimate
suppressed-operation completion updates in addition to the first two fetch
completions.

That stale fixture symptom is not the release blocker. The two initial fetches
have distinct candidate IDs, final URLs, and source identities, but relevance
selection removes all passages from both. Production duplicate classification
then compares their empty content fingerprints and classifies the second source
as `Duplicate`, rather than preserving both as `Irrelevant`. A diagnostic-only
assertion reorder ran the same exact 1-test command at
`2026-08-18T16:16:36+02:00`–`16:16:39+02:00`; it exited 101 directly on
`contribution == "irrelevant"`. The original assertion order was immediately
restored, and `git diff -- crates/daemon/src/evidence/orchestrator.rs` was empty.

This proves the test is red for its intended public activity-projection
regression. Making it green requires a production evidence-orchestrator change,
such as changing how empty relevance fingerprints participate in duplicate
classification. Stage 1 does not authorize that change. The expected value was
not changed, and the test was not deleted, skipped, ignored, loosened, or
replaced.

### Lint waiver audit

Every waiver still names a specific lint class, is confined to one affected
crate, remains owned by issue 25, states that the debt is pre-existing and
outside Stage 1, and expires on 2026-09-17. No `warnings = "allow"` or global
suppression exists. Each individual class was forced back to deny in its owning
crate; every audit exited 101 on the named pre-existing diagnostic, so no waiver
became removable after the Tavily prerequisite.

- `bagent-memory`: Rust `dead_code`; Clippy `unnecessary_map_or`,
  `too_many_arguments`, `await_holding_lock`.
- `filesystem-connector`: Rust `unused_variables`, `dead_code`; Clippy
  `collapsible_if`.
- `apple-mail-connector`: Clippy `single_match`, `manual_strip`.
- `bagent-rules`: Clippy `derivable_impls`.
- `codex-connector`: Rust `unused_variables`; Clippy `ptr_arg`.
- `whatsapp-connector`: Clippy `ptr_arg`, `derivable_impls`,
  `redundant_closure`, `unnecessary_lazy_evaluations`.
- `bagent-agent`: Clippy `empty_line_after_doc_comments`,
  `too_many_arguments`, `bool_assert_comparison`.
- `bagentd`: Clippy `nonminimal_bool`, `collapsible_if`, `single_match`,
  `needless_question_mark`, `items_after_test_module`, `too_many_arguments`.

### Historical resumed criterion verdicts — superseded

| Criterion | Verdict | Reason |
|---|---|---|
| A01 | **FAIL** | Discovery found 468 tests, but the nonzero exact and workspace executions retain one deterministic production orchestrator failure. |
| A02 | **PASS** | 53 tests were discovered and 53/53 passed, including all five preserved Tavily synchronization behaviors. |
| A03 | **PASS** | Eight surfaces independently emitted source-measured verdicts, counts, and exits for 93 assertions; zero/mismatch is BLOCKED and synthetic exit 7 remains FAIL. The four known product findings remain visible. |
| A04 | **PASS** | Formatting, strict workspace/all-target Clippy under the scoped policy, Swift build, and diff check all exited zero. |

### Artifacts, cleanup, and final protected state

Retained acceptance source SHA-256 hashes:

- `scripts/run-static-acceptance.sh`: `2e6ad44e0e44172170f7a88531a9209d0720a881947be5b76a4bd357dff6483e`
- `scripts/test-static-acceptance-red-capability.sh`: `3afc62a99e8035def230193ee600d9749ed00fbfa1ddd5f69fc418abdf9a11c6`
- `scripts/check-stage1-static-acceptance.sh`: `bf9e8c657daad502ffa6cefe38a007d94e940baa7d9982b044fcb030d743c4a8`
- `scripts/check-notch-output-scroll-stability.sh`: `0589b88b3c2eeff08ccf73d8382e53a75f1d4fb84d044295eaa75f6b00d6f18a`
- `scripts/check-notch-output-regressions.sh`: `b4811fbce4c3873f2046189ba4b131cc5bb5c0c9b3ba13f0f69c9a6bb129c032`
- `scripts/check-mail-rendering-regressions.sh`: `316909f8ccc876f2ee95c850799bd76dbc9c9abdbb64557711de456c6e188999`

Static-runner temporary fixtures were removed by their traps. Diagnostic command
logs were kept only under `/tmp/bagent-stage1-resume.RJIB0L` during this run and
contain no credentials, raw tool arguments, hidden reasoning, private identities,
or Evidence Content. Ordinary ignored compiler build products remain; no
destructive cleanup was performed.

Final `HEAD` and upstream remain equal at
`aa986da0661865330414ed1b86236551b5bafc8f`. The preserved Stage 1 changes remain
uncommitted and unstaged. No application, daemon, BaseRT, TCC state, port 8080,
or port 8082 was started, stopped, reloaded, configured, or probed. Because A01
is FAIL, issue 25 remains open, no commit or push is permitted, and Stage 2 is not
eligible.

## Final post-#27 verification — 2026-08-18

This is the current Stage 1 verification record. It was captured after
[issue 27](https://github.com/brunovskyoliver/bagent/issues/27) corrected the
empty-relevance duplicate classification, at base commit
`d69cc30791145f820db3bf6cd26f628ef349b9e0`.

### Starting state and environment

- Branch: `t3code/basert-notch-automation-ux`.
- Initial local `HEAD` and `origin/t3code/basert-notch-automation-ux` both:
  `d69cc30791145f820db3bf6cd26f628ef349b9e0`.
- Run start: `2026-08-18T17:24:54+02:00`; verification and waiver-audit end:
  `2026-08-18T17:28:22+02:00` (Europe/Bratislava).
- Environment: macOS 26.5.2 (25F84), arm64; `rustc 1.91.1`;
  `cargo 1.91.1`; Apple Swift 6.3.1.
- The initial dirty inventory exactly matched the sixteen recorded Stage 1
  paths. The index was empty. The #26 Tavily production files, the #27
  `crates/daemon/src/evidence/orchestrator.rs`, and every other production file
  had no new uncommitted change.
- No reset, restore, stash, broad formatting, broad staging, dependency change,
  product behavior change, or protected runtime operation was performed.

### Fresh commands and results

| Criterion | Start | End | Exact command | Exit | Fresh result |
|---|---|---|---|---:|---|
| A01 | 17:25:02 | 17:25:03 | `cargo test --workspace -- --list` | 0 | 471 tests discovered across the workspace |
| A01 | 17:25:24 | 17:25:25 | `cargo test -p bagentd evidence::orchestrator::tests::distinct_irrelevant_fetches_are_not_reported_as_duplicate_activities -- --exact` | 0 | 1 passed, 0 failed, 0 ignored |
| A01 | 17:25:25 | 17:25:25 | `cargo test -p bagentd evidence::orchestrator::tests::identical_nonempty_claim_content_remains_a_duplicate -- --exact` | 0 | 1 passed, 0 failed, 0 ignored |
| A01 | 17:25:25 | 17:25:25 | `cargo test -p bagentd evidence::orchestrator::tests::same_final_url_remains_a_duplicate_when_selected_content_is_empty -- --exact` | 0 | 1 passed, 0 failed, 0 ignored |
| A01 | 17:25:25 | 17:25:25 | `cargo test -p bagentd evidence::orchestrator::tests::same_source_identity_remains_a_duplicate_when_selected_content_is_empty -- --exact` | 0 | 1 passed, 0 failed, 0 ignored |
| A01 | 17:25:25 | 17:25:25 | `cargo test -p bagentd evidence::orchestrator::tests::canonical_duplicate_web_operations_are_suppressed_before_gate_and_fetch -- --exact` | 0 | 1 passed, 0 failed, 0 ignored |
| A01 | 17:25:34 | 17:25:34 | `cargo test -p bagentd evidence::orchestrator::tests` | 0 | 37 passed, 0 failed, 0 ignored |
| A01 | 17:25:41 | 17:25:49 | `cargo test -p bagentd` | 0 | 261 passed, 0 failed, 3 ignored |
| A01 | 17:26:02 | 17:26:14 | `cargo test --workspace` | 0 | 460 passed, 0 failed, 11 ignored; 471 total |
| A02 | 17:26:32 | 17:26:36 | `swift test --package-path apps/macos list` | 0 | 53 tests discovered, including five Tavily synchronization tests |
| A02 | 17:26:36 | 17:26:36 | `swift test --package-path apps/macos --filter TavilyConfigurationSyncTests` | 0 | 5 passed, 0 failed |
| A02 | 17:26:36 | 17:26:39 | `swift test --package-path apps/macos` | 0 | 53 passed, 0 failed |
| A03 | 17:27:00 | 17:27:01 | Eight separate `scripts/run-static-acceptance.sh NAME COUNT SCRIPT` invocations listed below | mixed by product surface | 8 independent records; 93 source-measured assertions |
| A03 | 17:27:01 | 17:27:02 | `scripts/check-stage1-static-acceptance.sh` | 1 | Preserved four independently reported product-surface failures |
| A03 | 17:27:02 | 17:27:02 | `scripts/test-static-acceptance-red-capability.sh` | 0 | Harness detected zero-count BLOCKED and marked one-assertion exit-7 FAIL |
| A03 | 17:27:14 | 17:27:14 | `scripts/run-static-acceptance.sh synthetic-mismatch 5 scripts/check-inline-input-focus-retention.sh` | 2 | BLOCKED with 6 measured versus 5 expected assertions |
| A04 | 17:27:31 | 17:27:31 | `cargo fmt --all -- --check` | 0 | PASS |
| A04 | 17:27:31 | 17:27:32 | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | PASS under the audited scoped waiver policy |
| A04 | 17:27:32 | 17:27:32 | `swift build --package-path apps/macos` | 0 | PASS |
| A04 | 17:27:32 | 17:27:32 | `git diff --check` | 0 | PASS |

All command timestamps in the table are on 2026-08-18 with offset `+02:00`.
The five focused duplicate-classification tests were individually filtered by
their full names, each executed exactly one non-ignored case, and all passed.
No test source changed in the Stage 1 diff, so no meaningful Rust, Tavily
readiness, bounded-retry, PID/reconnect, absent-credential, or privacy test was
removed, ignored, skipped, filtered out of its required full run, or weakened.

### A03 independent surface inventory

| Surface | Exact runner arguments | Assertions | Verdict | Exit |
|---|---|---:|---|---:|
| Inline input focus | `inline-input-focus 6 scripts/check-inline-input-focus-retention.sh` | 6 | PASS | 0 |
| Input overflow non-promotion | `input-overflow-non-promotion 10 scripts/check-no-input-overflow-promotion.sh` | 10 | PASS | 0 |
| Thinking/output dot layer | `thinking-output-dot-layer 22 scripts/check-notch-thinking-output-dot-layer.sh` | 22 | PASS | 0 |
| Fullscreen notch visibility | `fullscreen-notch-visibility 6 scripts/check-fullscreen-notch-visibility.sh` | 6 | FAIL | 1 |
| Non-notch inline pill | `non-notch-inline-pill 6 scripts/check-non-notch-inline-pill.sh` | 6 | FAIL | 1 |
| Notch output scroll stability | `notch-output-scroll-stability 17 scripts/check-notch-output-scroll-stability.sh` | 17 | FAIL | 1 |
| Notch output layout | `notch-output-layout 19 scripts/check-notch-output-regressions.sh` | 19 | PASS | 0 |
| Mail rendering | `mail-rendering 7 scripts/check-mail-rendering-regressions.sh` | 7 | FAIL | 1 |

The selected inventory is exactly eight surfaces and 93 assertions. Each
surface emitted its own source-measured count, verdict, and underlying exit
status. The four known product findings remain visible and were not promoted.
Notch layout and Mail rendering remain separate scripts and verdicts; the
obsolete combined `scripts/check-notch-output-mail-regressions.sh` is
intentionally deleted. Invalid zero counts and nonzero count mismatches are
BLOCKED. The marked synthetic one-assertion fixture exits 7 and remains FAIL;
the red-capability harness passes only because it detects those non-PASS states.

### Ignored Rust test inventory

The eleven ignored workspace cases are unchanged, were listed by the full run,
and retain these existing environment-dependent reasons:

- `tests::real_mail_uncached_message_becomes_readable` — requires Full Disk
  Access, Mail.app Automation, and a currently uncached message.
- `memory_extractor::tests::passive_extraction_returns_empty_for_one_off` —
  requires BaseRT and the classifier model.
- `agent_exec::tests::live_content_synthesis_smoke_uses_configured_4b_model` —
  requires the app-managed BaseRT service and configured 4B model.
- `agent_exec::tests::live_web_direct_page_smoke_fetches_before_bounded_synthesis`
  — requires public web access and the app-managed BaseRT 4B model.
- `agent_exec::tests::stage8_live_frozen_bundle_matrix_and_performance` —
  requires all three installed BaseRT models and explicit acceptance runtime.
- `native_tool_call_round_trip_is_openai_compatible` and
  `slovak_diacritics_are_preserved` — require bagent BaseRT on port 8082.
- `exec::tests::real_codex_exec_returns_output` — requires an installed and
  authenticated Codex CLI.
- `tests::live_connect`, `tests::live_my_tickets`, and
  `tests::live_search_partners` — require live Odoo and an installed
  `uvx mcp-server-odoo`.

The protected runtimes and external services required by these cases were not
started, stopped, configured, or probed. They remain intentionally ignored
rather than silently skipped or represented as executed.

### Lint waiver audit

All waivers remain limited to one named crate, explicitly named pre-existing
lint classes, owner issue 25, debt outside Stage 1, and expiry 2026-09-17. There
is no blanket or global warning suppression. The final strict workspace command
compiled all targets and passed; no waiver masks compilation errors, failing
tests, or new Stage 1 shell/documentation code.

From `2026-08-18T17:27:55+02:00` through
`2026-08-18T17:28:22+02:00`, each of the following 25 classes was separately
forced back to deny with `cargo clippy -p CRATE --all-targets -- -D warnings -D
LINT`. Every command exited 101 on its named diagnostic, proving that no waiver
is presently removable:

- `bagent-memory`: Rust `dead_code`; Clippy `unnecessary_map_or`,
  `too_many_arguments`, `await_holding_lock`.
- `filesystem-connector`: Rust `unused_variables`, `dead_code`; Clippy
  `collapsible_if`.
- `apple-mail-connector`: Clippy `single_match`, `manual_strip`.
- `bagent-rules`: Clippy `derivable_impls`.
- `codex-connector`: Rust `unused_variables`; Clippy `ptr_arg`.
- `whatsapp-connector`: Clippy `ptr_arg`, `derivable_impls`,
  `redundant_closure`, `unnecessary_lazy_evaluations`.
- `bagent-agent`: Clippy `empty_line_after_doc_comments`,
  `too_many_arguments`, `bool_assert_comparison`.
- `bagentd`: Clippy `nonminimal_bool`, `collapsible_if`, `single_match`,
  `needless_question_mark`, `items_after_test_module`, `too_many_arguments`.

### Final criterion verdicts

| Criterion | Final verdict | Reason |
|---|---|---|
| A01 | **PASS** | 471 discovered; the formerly failing regression, five focused duplicate cases, 37-test orchestrator module, 261-test daemon suite, and full workspace all executed nonzero with zero failures; workspace result was 460 passed and 11 explicitly environment-dependent ignored. |
| A02 | **PASS** | 53 discovered and 53/53 passed; all five Tavily synchronization behaviors executed and passed. |
| A03 | **PASS** | Exactly eight independent surfaces emitted source-measured verdict/count/exit records totaling 93 assertions; zero/mismatch is BLOCKED and the expected exit-7 synthetic failure remains FAIL. |
| A04 | **PASS** | Formatting, strict workspace/all-target Clippy, Swift build, and diff check all exited zero; all 25 narrow waivers remain demonstrably required and expire 2026-09-17. |

### Artifacts, links, cleanup, and protected state

Retained Stage 1 source SHA-256 hashes:

- `scripts/run-static-acceptance.sh`: `2e6ad44e0e44172170f7a88531a9209d0720a881947be5b76a4bd357dff6483e`
- `scripts/test-static-acceptance-red-capability.sh`: `3afc62a99e8035def230193ee600d9749ed00fbfa1ddd5f69fc418abdf9a11c6`
- `scripts/check-stage1-static-acceptance.sh`: `bf9e8c657daad502ffa6cefe38a007d94e940baa7d9982b044fcb030d743c4a8`
- `scripts/check-notch-output-scroll-stability.sh`: `0589b88b3c2eeff08ccf73d8382e53a75f1d4fb84d044295eaa75f6b00d6f18a`
- `scripts/check-notch-output-regressions.sh`: `b4811fbce4c3873f2046189ba4b131cc5bb5c0c9b3ba13f0f69c9a6bb129c032`
- `scripts/check-mail-rendering-regressions.sh`: `316909f8ccc876f2ee95c850799bd76dbc9c9abdbb64557711de456c6e188999`

The evidence document's local decision link resolved to an existing file.
Authenticated GitHub validation resolved issues 15, 25, 26, and 27; issues 26
and 27 were closed prerequisites and issue 25 remained open during verification.
The retained hashes above were recalculated after all verification commands.

The runner and red-capability traps removed their synthetic/output temporary
files. Privacy-safe command logs were retained only under
`/tmp/bagent-stage1-final.1OH4ER` during evidence assembly; ordinary ignored
compiler products remain. No destructive cleanup was performed and no private
content, credentials, raw tool arguments, hidden reasoning, or private
identities were captured.

At the verification boundary, local `HEAD` and upstream remained equal at the
base commit, the index was empty, and only the expected Stage 1 paths were dirty.
No installed application, daemon, BaseRT, TCC state, port 8080, or port 8082 was
started, stopped, reloaded, configured, or probed. Stage 2 became eligible only
through these four PASS verdicts; it was not created, claimed, or begun.
