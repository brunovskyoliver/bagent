# Stage 7C Permission Grant Assist and UI Relaunch Acceptance

Status: implementation evidence separated by type; Stage 7C and Stage 8
signed/live qualification is limited to macOS 26.

## Scope and provenance

- Fixed comparison commit: `6a93757e107f4077b62629884eabd09415b93d4a`
- Implementation ticket: [#36](https://github.com/brunovskyoliver/bagent/issues/36)
- Wayfinder map: [#15](https://github.com/brunovskyoliver/bagent/issues/15)
- Required bundle identifier: `sk.bagent.app`
- Required Team ID: `QUB47S3XTF`

macOS 14 and 15 remain compile targets when existing configuration permits
them. This campaign does not claim runtime, System Settings, TCC, visual, or
accessibility qualification on those versions. Live grant, denial, revocation,
and drag-to-System-Settings mutation are outside this campaign. No omitted
check below is PASS.

The historical permission research document is unchanged. It is not evidence
of macOS 14/15 qualification, live TCC mutation, or drag mutation; its dated
macOS 26 route observation is cited only as read-only observational evidence.

## PASS evidence

The implementation now provides:

- a daemon-owned Full Disk Access probe that opens the exact Mail and Notes
  resources, returns separate normalized outcomes, reads no content, and does
  not expose paths or raw operating-system errors;
- injectable deterministic FDA, Screen Recording, and Accessibility adapters;
- process-scoped relaunch-required detection derived from authoritative probe
  transitions, with a fresh post-takeover probe required before completion;
- a one-use, fenced UI transfer protocol with hidden replacement launch,
  authoritative refetch, reserved successor authority, old-consumer fencing,
  successor activation, acknowledgement, timeout rollback, and late-process
  exit; and
- a UI-only replacement path that never calls `DaemonLauncher.launch()` and
  never mutates daemon or BaseRT ownership.

Focused deterministic evidence:

```text
swift test --package-path apps/macos --filter UIRelaunchHandoffTests       # 5 passed, 0 failed
swift test --package-path apps/macos --filter UIRelaunchTransferTests     # 5 passed, 0 failed
swift test --package-path apps/macos --filter PermissionRecheckTests      # 12 passed, 0 failed
swift test --package-path apps/macos --filter PermissionRoutingTests      # 5 passed, 0 failed
swift test --package-path apps/macos --filter ApplicationDragTests       # 4 passed, 0 failed
swift test --package-path apps/macos --filter EventConsumerRecoveryTests # 7 passed, 0 failed
swift test --package-path apps/macos --filter AppLaunchModeTests          # 1 passed, 0 failed
cargo test -p bagentd ui_relaunch                                           # 11 passed, 0 failed
cargo test -p bagentd permission_probe                                       # 3 passed, 0 failed
```

The seven focused Swift suites executed 39 tests with zero failures. The two
focused daemon suites executed 14 filtered tests with zero failures. The
combined focused count is 53 passed and 0 failed. One unrelated existing
Accessibility-permission skip remains outside these counts.

## A46 signed bundle and drag-payload evidence

Required command:

```text
make -C apps/macos bundle
scripts/acceptance/signed-bundle-verification.sh apps/macos/bagent.app
```

The candidate must pass strict deep verification under `sk.bagent.app`, Team
ID `QUB47S3XTF`, with separately valid nested UI and daemon code, the packaged
icon and declared `CFBundleIconFile`, and the deterministic `public.file-url`
round trip to `Bundle.main.bundleURL`. Tests reject images, executables, alias
items, source directories, file promises, missing bundles, wrong identifiers,
wrong Team IDs, invalid signatures, and ad-hoc signatures.

No actual System Settings drop was tested or claimed.

Final A46 result: the actual signed candidate at `apps/macos/bagent.app`
passed strict deep verification, identifier `sk.bagent.app`, Team ID
`QUB47S3XTF`, nested UI and daemon signatures, declared and packaged icons,
the signed candidate `public.file-url` round trip, and all rejection fixtures.

## A47 route evidence

```text
swift test --package-path apps/macos --filter PermissionRoutingTests
```

The five-test route suite passed with zero failures. It covers every accepted category anchor, supported
Screen Recording title, root fallback, localization, stale confirmation, and
the invariant that opening a route never grants permission. The accepted
read-only observation was recorded on macOS 26.5.2 (build 25F84) on Apple
silicon: LaunchServices accepted all four URLs and the System Settings
accessibility hierarchy exposed Privacy & Security, Full Disk Access, Screen
& System Audio Recording, and Accessibility. No switch was touched and no
prompt was invoked. That observation is documented in
`docs/MACOS_PERMISSION_GRANT_ASSIST_RESEARCH.md`. macOS 14/15 route behavior
is unqualified.

No fresh System Settings pane opening or System Settings mutation was
performed in this campaign. The read-only observation does not qualify
macOS 14/15.

## A48 deterministic permission evidence

```text
swift test --package-path apps/macos --filter PermissionRecheckTests
```

The twelve-test suite passed with zero failures. It covers all twelve phases, separate daemon Mail/Notes results,
Screen Recording and Accessibility adapters, activation debounce and
coalescing, stale-generation suppression, delayed propagation, revocation
mapping, observer cleanup, process-scoped relaunch-required mapping, and no
optimistic success. Production relaunch-required detection does not use a
fixture callback. Post-takeover convergence requires the authoritative
recheck and terminal granted/denied state. Real TCC grant/revoke timing is
unqualified and live TCC mutation is omitted.

## A49 relaunch evidence

```text
scripts/acceptance/ui-relaunch-handoff.sh <signed-disposable-app>
```

The acceptance harness requires a signed candidate, a controlled disposable
daemon and BaseRT, a real Work and model lease, unchanged daemon/BaseRT PIDs,
unchanged Work identities and revisions, one UI consumer, restored Current
Chat and Compass Rail state, timeout rollback, and an untouched port 8080.
The focused transfer tests additionally cover stale consumers, duplicate
replacement, token replay, replacement and old-UI crashes, failed readiness,
failed acknowledgement, daemon unavailable before and after takeover, exactly
one visible interactive UI, rollback, and a late hidden replacement.

Final A49 result: `scripts/acceptance/ui-relaunch-handoff.sh
apps/macos/bagent.app` passed through the real `AppDelegate`, opaque handoff,
daemon transfer endpoints, fence reservation, hidden replacement,
old-UI fencing and exit after acknowledgement, visible/interactive
presentation, authoritative post-takeover permission probe, rollback, and
stale-fence checks. The flow kept disposable daemon and BaseRT PIDs,
Work/model lease, Work identities and revisions, Current Chat identity,
Compass Rail state, one active UI consumer, and protected port snapshots
unchanged. It did not intentionally restart the daemon during the takeover
proof.

## A50 privacy evidence

```text
scripts/acceptance/stage7c-privacy-scan.sh <capture-directory>
```

The production fixture emitted nine nonempty sanitized capture files. The
scanner checked 9 surfaces, 9 files, 2,756 bytes, 9 surface assertions, and
234 structured synthetic canaries; it detected all 234 synthetic matches and
zero forbidden matches in the production capture. Capture manifest SHA-256:
`5c9292ad901029b6ab4e4da3013b915ed99d7daf95f4d2ad255c858c8191013f`.
The capture directory was deleted after the hash and counts were recorded.
Live TCC and macOS 14/15 evidence remain omitted; neither omission is PASS.

## macOS 26 observational evidence

Only macOS 26 may contribute signed/live observations to this campaign. Any
such observation must identify the candidate bundle, build, route, and
read-only result. Opening System Settings or launching a replacement is not
itself permission or relaunch proof.

## Explicitly omitted evidence

- disposable macOS 14 and 15 runtime, System Settings, TCC, visual, and
  accessibility qualification;
- live TCC grant, denial, revocation, and timing propagation;
- actual drag-to-System-Settings mutation;
- fresh System Settings pane opening or mutation during A47;
- any claim that an omitted environment or check passed.

## Required remaining support limitation

macOS 14/15 may continue to compile if the existing deployment configuration
allows it, but support qualification for those versions remains absent. The
release evidence must carry this limitation forward into Stage 8. Deterministic
permission adapters, signed-bundle validation, drag-payload validation,
privacy tests, and daemon-preserving relaunch remain required.

The Stage 8 document edits are limited to this approved macOS 26-only
qualification boundary and its explicit omissions. No Stage 8 implementation
is included.

## Campaign validation and review record

- Full Swift suite: 149 passed, 1 pre-existing Accessibility-permission skip,
  0 failures.
- Full Rust workspace: passed; strict Clippy with `-D warnings` passed.
- Formatting, `git diff --check`, release build, localization, settings
  authority/catalog, Current Chat/A42, signed Stage 7A/A41 relaunch, Work/A26,
  and Model Runtime/A17 checks passed.
- Two independent reviews against the fixed comparison commit reported zero
  unresolved Standards findings and zero unresolved Revised Stage 7C Spec
  findings.
