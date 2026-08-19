# Stage 7A Current Chat and Slash Command acceptance

## Scope

- Fixed starting commit: `7b88b4b018fc4e0d9742b1288fb51ea747e1c778`
- Implementation ticket: [#33 — Stage 7A: Make Current Chat durable and fix slash-command execution](https://github.com/brunovskyoliver/bagent/issues/33)
- Dependency: closed Stage 6 issue [#32](https://github.com/brunovskyoliver/bagent/issues/32)
- Wayfinder map: [#15](https://github.com/brunovskyoliver/bagent/issues/15)
- Design decision: [#22](https://github.com/brunovskyoliver/bagent/issues/22)

This change is limited to Stage 7A. It does not include the Stage 7B settings redesign or any Stage 7C permission workflow.

## Authority and files

The daemon authority is `crates/daemon/src/current_chat.rs`, reached through the Current Chat routes and Conversation Work admission in `crates/daemon/src/main.rs`. `crates/daemon/src/work_coordinator.rs` owns canonical Work mutations and the atomic user-turn/Work commit. `crates/daemon/src/automation_sessions.rs` preserves the Stage 6 continuation contract while replacing the Current Chat through the daemon authority. The durable schema is `crates/daemon/migrations/V22__durable_current_chat.sql`.

The Swift transport and restoration seam is `apps/macos/Sources/bagent/DaemonClient.swift` plus `apps/macos/Sources/bagent/ChatViewModel.swift`. `apps/macos/Sources/bagent/SlashCommandRegistry.swift` is the only command registry. `apps/macos/Sources/bagent/NotchWindowController.swift` owns keyboard precedence and marked-text handling; `apps/macos/Sources/bagent/ChatView.swift` owns completion, confirmation, error, focus, and accessibility presentation. New text is localized in `apps/macos/Sources/bagent/Resources/Localizable.xcstrings` with English and Japanese values for all 18 Stage 7A keys.

Other production integration changes are in:

- `apps/macos/Package.swift`
- `apps/macos/Sources/bagent/AutomationSplitView.swift`
- `apps/macos/Sources/bagent/Stage7AAcceptanceCLI.swift`
- `apps/macos/Sources/bagent/Stage8AcceptanceCLI.swift`
- `apps/macos/Sources/bagent/main.swift`
- `crates/daemon/Cargo.toml`
- `crates/daemon/src/agent_exec.rs`
- `crates/daemon/src/automations_api.rs`
- `crates/daemon/src/lib.rs`
- `crates/daemon/src/model_runtime.rs`
- `crates/daemon/src/unified_work.rs`

Tests and acceptance surfaces changed or added in:

- `apps/macos/Tests/bagentTests/AutomationsSurfaceTests.swift`
- `apps/macos/Tests/bagentTests/CurrentChatRestorationTests.swift`
- `apps/macos/Tests/bagentTests/SlashCommandTests.swift`
- `crates/daemon/tests/automation_sessions.rs`
- `crates/daemon/tests/current_chat.rs`
- `scripts/acceptance/current-chat-authority.sh`
- `scripts/acceptance/current-chat-ui-relaunch.sh`

## Schema and migration

V22 treats all V19 Current Chat targets as unowned because that schema had no singleton authority and Swift-local clear did not remove the rows. It removes their continuation provenance, drops the writable legacy `automation_current_chats` table, and lets the daemon issue a fresh authoritative identity. The regression covers a continuation followed by legacy local clear/replacement so obsolete seed text cannot be resurrected. The cutover leaves source-session viewed state unchanged. Foreign-key cascades scope turns, drafts, submitted attachments, sources, Connector References, and completed approval presentation to that identity. Clear commands and lifecycle audit rows preserve idempotent retry evidence without retaining deleted chat content.

The schema enforces a maximum of 500 completed turns, 16 MiB retained encoded content, one active turn, and a 16 KiB UTF-8 draft. The daemon rejects additions at a bound; it does not truncate, summarize, or evict Current Chat turns. Draft and pending attachment references expire after seven days. Startup expires only those transient records and converts an active turn into a normalized `daemon_restart` interruption while abandoning pending runtime and approval ownership. The normalized terminal approval outcome is copied into the bounded, privacy-safe Current Chat presentation after Work recovery commits. Current Chat itself does not expire.

The V19 bootstrap schema can still be invoked idempotently by the Work coordinator, so the coordinator drops `automation_current_chats` again after that bootstrap. No writable legacy identity path is recreated.

## Acceptance gates

| Gate | Exact command | Nonzero result |
| --- | --- | --- |
| A39 | `swift test --package-path apps/macos --filter SlashCommandRegistryTests` | 20 passed |
| A40 | `cargo test -p bagentd --test current_chat clear_atomicity -- --exact` | 1 passed |
| A41 | `cargo test -p bagentd --test current_chat restoration -- --exact` | 1 passed |
| A42 | `scripts/acceptance/current-chat-authority.sh` | 5 Swift surfaces, 4 Rust surfaces, 3 registry entries, 8 independently asserted seeded failures |

A39 covers character-by-character entry, exact canonical and alias execution, ordinary partial submission, Tab/click completion without execution, paste, paths, URLs, whitespace, Unicode casing without diacritic folding, marked text, modifier keys, failure preservation, focus, and keyboard precedence.

A40 covers atomic content replacement, draft, continuation seed and provenance, Automation Sessions, saved memory, attachments, Connector References, approval presentation, response loss, same-key retry, stale revisions, concurrency, and injected rollback at every clear transaction boundary.

A41 covers exact identity/content restoration, 500-turn and 16 MiB content bounds, the 16 KiB draft bound and seven-day draft expiry, restart interruption, and retained submitted user content.

A42 proves its red capability against eight independent fixtures covering Swift-created identity, UserDefaults identity authority, local-only clear, trimmed execution, diacritic-folded execution, suggestion-triggered execution, and duplicate registries. The production scan found none.

## Signed UI-only relaunch evidence

`scripts/acceptance/current-chat-ui-relaunch.sh` built a disposable Apple Development-signed fixture with identifier `sk.bagent.stage7a.fixture`. Its designated requirement named `Apple Development: obrunovsky7@gmail.com (D63PW2838J)`.

- UI PID changed from `13654` to `13879`; the first process terminated and was reaped before the replacement launched.
- Daemon PID remained `13605` during UI replacement.
- An idle daemon restart changed its PID from `13562` to `13605` while preserving the exact Current Chat snapshot.
- disposable BaseRT PID remained `13558` across daemon restart and UI replacement.
- active model lease count remained `1`.
- daemon-owned Current Chat identity remained `ab49660f-2bc9-4690-8d28-3c6007a6ea04`, with a canonical full-snapshot hash match before and after replacement. The hash covers interruption reason, turns, draft, continuation, submitted attachments and availability, Validated Sources, Connector References, and completed approval presentations.
- the restored 21-byte draft had its UTF-16 caret at 21 and selection length zero.
- both signed UI processes restored one unavailable submitted attachment, one Validated Source, one Connector Reference, and one completed approval presentation through the production Swift projection.
- a real daemon crash during an active Conversation Turn retained the user message, discarded incomplete assistant output, recorded `daemon_restart`, abandoned Work and the model lease, and resumed no request or side effect.
- protected port 8080 remained unused.
- protected port 8082 remained owned only by the user's BaseRT PID `792`.

The CLI replacement path runs before `AppDelegate` and does not call `DaemonLauncher.launch()`. The fixture used an isolated database, random daemon and BaseRT ports, a copied model registry, and a disposable signed bundle. Its trap removed the bundle, database, logs, and fixture processes. A post-run listener check found no process on 8080 and only PID 792 on 8082. The user's database, daemon, BaseRT, TCC state, and protected ports were not mutated.

## Privacy, bounds, and failure evidence

- Snapshots expose only the retained bounded fields. `content_bytes` is the exact compact JSON encoding of the retained authority, including framing and escaping; escape-heavy boundary tests fail closed. Completed approval presentation stores category and normalized outcome, not approval payloads.
- Submitted attachment metadata is durable, retains its owning Conversation Turn and availability, and renders unavailable content without a synthetic local path. Pending attachment references remain draft-scoped, are refetched into visible/sendable metadata on reopen, remain visibly removable when content is missing, and expire with the draft.
- Chat-scoped validated sources and Connector References are deleted only with the replaced Current Chat. A Connector Reference persistence or bound failure interrupts the turn and never installs a memory-only fallback.
- Conversation admission validates attachments before Work admission and commits the submitted user turn and Work atomically.
- Conversation completion or interruption, approval presentation capture, Work terminalization, and its ordered outbox event commit in one immediate transaction. Failure injection before the transaction, after chat mutation, after the outbox insert, and at commit proves that both Current Chat and Work roll back together. Volatile capacity is released only after the durable commit. Restart abandons pending approvals and Work, records only the normalized approval outcome for presentation, and never resumes tools, side effects, or model requests.
- A rejected admission refetches the authority revision and then restores the exact raw submitted draft and all pending references. Stale revision, content-bound, and missing-attachment tests prove that whitespace and slash text remain unchanged; missing content stays visible as unavailable and removable.
- Clear rejects active turns and approvals, requires confirmation only for non-empty retained chat content, and uses a client-generated idempotency key. Injected failures roll back the entire transaction. A lost response is resolved by refetch and same-key retry.
- Clearing does not mutate Automation Sessions, Automation Runs or Definitions, Saved Long-Term Memory, source-session viewed state, detached history, or external side effects.

## Regression results

- Full Swift suite: 103 passed, 0 failed, 1 skipped (104 executed).
- Full Rust workspace: 480 passed, 0 failed, 12 ignored environment-dependent or fixture/helper entrypoints.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: passed with only existing approved waivers.
- `swift build --package-path apps/macos`: passed.
- `git diff --check`: passed.
- `scripts/acceptance/model-runtime-authority.sh`: 13 seeded forbidden matches detected; 0 production matches.
- `scripts/acceptance/notch-mode-authority.sh`: passed.
- `scripts/acceptance/work-authority.sh`: 11 seeded forbidden categories detected; 0 production authorities or missing edges.
- `scripts/acceptance/work-cutover-rollback.sh`: passed, including its one exact migration test.
- `scripts/acceptance/accessibility-audit.sh`: 8 passed, 0 failed, 1 environment-dependent skip; signed bundle verification and static accessibility checks passed.
- `scripts/acceptance/capture-notch-states.sh`: rendered and verified all 11 deterministic states.
- Localization catalog JSON and English/Japanese coverage audit: 18 of 18 keys passed.

The only environment-dependent skip is `testHostedNotchPreservesRailAndPillButtonsInAccessibilityTree`: the test runner does not have Accessibility API permission. The signed accessibility fixture, source audit, stable labels/values, keyboard bindings, semantic text, contrast checks, and all other accessibility tests passed. No A39–A42 gate was skipped.

## Review

Two independent agents reviewed the complete diff from `7b88b4b018fc4e0d9742b1288fb51ea747e1c778`. The final Standards review reported zero unresolved findings. The final Spec review against ticket #33 and the accepted Stage 7A decisions reported zero unresolved findings. Earlier findings covering atomic turn/Work terminalization, ambiguous V19 cutover, rejected-admission draft preservation, retained availability projection, signed process replacement, and production caret restoration were fixed and re-reviewed before this clean pass.
