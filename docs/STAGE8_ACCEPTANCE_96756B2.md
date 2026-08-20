# Stage 8 definitive acceptance — `96756b2`

Date: 2026-07-30
Candidate: `96756b27c1d5798887e3baa379d5b0ab449bbc09`
Baseline product revision: `8536f01`
Acceptance interpretation: [ADR-0002](ADR-0002-REPRODUCIBLE-STAGE8-RELEASE-GATE.md)

## Verdict

Stage 8 passes. Stage 9 is authorized to begin, but is not enabled or
implemented by this work.

The mandatory reproducible gate passed every deterministic signed fixture.
The mandatory live smoke produced only safe observational outcomes: Mail was
partial and public-web verification shortfalled. Neither live result contained
an unsupported answer, fabricated claim, incorrect citation, or unsafe output.

## Mandatory deterministic signed E2E

The exact clean candidate was built with the `stage8-acceptance` Cargo feature,
strictly signed, and launched with both explicit runtime flags. The harness ran
all 21 cases twice through the signed Swift executable, authenticated fixture
control, `DaemonClient.chatStream`, SSE decoding, and the same
`EvidencePresentation.apply` reducer used by `ChatViewModel`.

- Campaigns: 2
- Cases per campaign: 21
- Structural equality: identical
- Canonical validation: reviewed token SHA-256, citation-set SHA-256, and byte
  count matched for every case
- Terminal contract: exactly one outcome and one `done` in every run
- Fixture diagnostics written: 0

### Mail

| Case | Result |
|---|---|
| Complete | verified, 3/3 |
| One transient body retry | verified, 3/3; first read exactly two attempts |
| Partial | partial, 1/3 |
| Unavailable | unavailable, 0/3 |
| Denied | denied, 0/3; no reads |
| Empty inbox | empty, 0/3; no reads |

Every applicable Mail case performed exactly one `mail.list` followed by three
distinct, opaque-hash `mail.read` activities. No raw Mail rowid crossed the
public event or report boundary.

### Web

| Case | Result |
|---|---|
| Authoritative first party | verified, 1/1 |
| Two independent corroborating publishers | verified, 2/2 |
| Renderable conflict | conflict, 2/2 |
| Ambiguous Bratislava-style table | verification shortfall, 0/2 |
| Irrelevant entity | verification shortfall, 0/2 |
| Redirect/final URL | verified, 1/1; final citation hash matched |
| Tavily missing credential | bounded verification shortfall |
| Tavily 429 | bounded verification shortfall |
| Tavily timeout | bounded verification shortfall |
| Tavily malformed response | bounded verification shortfall |
| DuckDuckGo fallback | Tavily rate-limited, DDG succeeded, verified 1/1 |
| All fetches fail | bounded verification shortfall |

Every fetch remained downstream of typed discovery. Provider status, search
count, fetch count, retry bounds, independence, authority, conflict, relevance,
and final-URL behavior were asserted structurally in addition to exact answer
and citation hashes.

### Synthesis and runtime

- Accepted polish: accepted; canonical bytes matched.
- Rejected polish: rejected; canonical bytes preserved.
- Unavailable polish: unavailable; canonical bytes preserved.
- The three canonical Mail outputs were byte-identical.
- The signed Swift reducer retained every terminal outcome and activity state.

## Acceptance route boundary

| Runtime | Unauthenticated | Authenticated |
|---|---:|---:|
| Acceptance feature and exact runtime flag enabled | 401 | 200 |
| Ordinary feature-absent signed bundle | 404 | 404 |

The fixture selector is process-local, authenticated, unavailable to prompt
routing, and absent from ordinary builds. Production adapters and routing remain
the fallback whenever the acceptance selector is inactive.

## Observational live smoke

The fixture selection was cleared before both live runs.

| System | Observation | Release classification |
|---|---|---|
| Mail.app | partial, 1/3 bodies available | Safe observational partial; acceptable |
| Tavily/public pages | Tavily discovery succeeded; DDG was challenged; fetched pages did not yield two usable claims; verification shortfall 0/2 | Safe observational shortfall; acceptable |

The web shortfall cited three fetched pages and all three citation hosts matched
completed fetch domains. No unsupported citation, factual answer, or unsafe
output was emitted.

## Verification

- `cargo test -p bagentd --features stage8-acceptance`: 253 passed, 3 ignored.
- `cargo test --workspace --no-fail-fast`: passed.
- Focused evidence tests: 169 passed.
- Focused web/provider tests: 55 passed.
- Focused synthesis/lifecycle tests: 33 passed.
- `swift test`: 39 passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --features bagentd/stage8-acceptance`:
  passed with only pre-existing repository warnings and no acceptance warning.
- `cargo build --workspace --release`: passed.
- Acceptance and ordinary bundles: strict deep code-sign verification passed.
- Signature: `sk.bagent.app`, Team ID `QUB47S3XTF`, Apple Development identity.
- Independent standards review of `0ef2886c...96756b2`: PASS.
- Independent specification review of `0ef2886c...96756b2`: PASS.
- `git diff --check`: passed.

## Privacy, rollback, and cleanup

- Deterministic and live reports contain zero raw Mail IDs, connector IDs,
  prompts, fixture selections, credentials, tokens, or secret patterns.
- Fixture-active turns use an in-memory event sink and wrote zero diagnostic
  files or prompt-trace markers.
- Seventy-three diagnostic files created by superseded pre-isolation campaigns
  were identified by fixture-only markers and removed; zero remain.
- The final bundle is the ordinary feature-absent signed build.
- Final runtime state: zero bagent app processes, zero daemon processes, zero
  BaseRT processes/loaded model weights, and zero bagent launch agents.
- Unrelated untracked `.scratch/`, `CONTEXT.md`, and
  `docs/KHOJ_BASERT_TAILSCALE_RESEARCH.md` were preserved unchanged.

## Authorization decision

All deterministic signed mandatory fixtures pass; live smoke contains no unsafe
or unsupported answer; default-off rollback, privacy, signing, and runtime
cleanup pass. Under ADR-0002, Stage 9 is therefore **authorized to begin**.

This authorization changes neither production grounding rules nor authority,
independence, citation, validation, or safety requirements. Stage 9 remains
disabled until separate Stage 9 work explicitly enables it.
