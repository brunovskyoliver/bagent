# Stage 8 evidence acceptance record

Date: 2026-07-29

Base commit: `65198c85bead95085c7a5cbe6ecaef84828fe5ff`

Overall verdict: **FAIL**

Stage 9/default enablement is **not authorized**. Cold and sustained 35B
admission remained unreliable, the warm run entered a Metal failure state, and
live uncached Mail hydration could not be verified without changing the
machine's existing Full Disk Access state.

No Mail content, private prompt, fetched passage, raw connector identifier,
model output, or credential was retained in this report or in the measurement
output. Model text was validated in memory and discarded.

## Environment

- Apple M5, 32 GiB unified memory, arm64.
- macOS 26.5.2 (25F84).
- Rust/Cargo 1.91.1; Swift 6.3.1.
- App-managed BaseRT on `127.0.0.1:8082`, max context 16,384, KV cache 4-bit,
  max batch size 1, request timeout 300 seconds.
- Installed models:
  `basecompute/Qwen3-4B-Instruct-2507`, `basecompute/Qwen3-8B`, and
  `basecompute/Qwen3.6-35B-A3B`.
- Evidence routing feature flag unset by default and in the final runtime
  state.

## Procedure and safety controls

1. Captured the tracked and runtime baseline at the exact Stage 7 commit.
2. Ran the deterministic Mail/web, policy-gate, diagnostic, event, synthesis,
   routing, automation, and Swift presentation suites.
3. Simulated Mail list and individual-read denial only through the in-memory
   `ScriptedGate`; the user's normal rules database was not edited.
4. Ran the exact prompt against frozen synthetic Mail evidence. Fixture values
   contain no real identity or Mail content.
5. Ran live web direct-page acquisition against
   `https://iana.org/help/example-domains`; the typed fetch followed the safe
   redirect and synthesis cited the final `www.iana.org` URL.
6. Ran the installed-model matrix with the same three frozen contracts for
   every model. Every request was asserted to contain exactly one system
   message, one user message, and no tools.
7. Unloaded every model between matrix/cold transitions, sampled BaseRT RSS,
   then ran 30 consecutive warm 35B attempts, ten per workload.
8. Attempted the ignored uncached-Mail hydration smoke without changing
   permissions. The test binary could not open the Mail envelope database, so
   the case is unverified.
9. Re-ran the complete build/test/lint/UI/signature checks.
10. Unloaded all model weights and confirmed no measurement process, policy
    override, feature-flag override, or temporary runtime hook remained.

The reproducible installed-model procedure is retained as the ignored
`stage8_live_frozen_bundle_matrix_and_performance` test. It must be explicitly
selected and is never part of normal or default-enabled runtime behavior.

## Mail acceptance

| Case | Evidence | Verdict |
| --- | --- | --- |
| Exact `can you read me the 3 latest emails?` | Fixture route executed one list plus three distinct reads and produced a complete three-body bundle. Live Mail was not exercised. | Unverified live; fixture pass |
| Header-only request | Model-free header fixture performed one list and zero body reads. | Pass |
| Three-message complete | Complete bundle, three bodies, grounded held-output validation. | Pass |
| One unavailable body | All three reads attempted; two bodies acquired; one typed body shortfall. | Pass |
| Empty inbox | Successful empty listing, no body reads, distinct empty outcome. | Pass |
| Connector unavailable | Typed unavailable recovery, distinct from empty and denied. | Pass |
| List/read denial | Isolated in-memory gate; list denial executed no connector call, read denial continued other reads. | Pass |
| Ten-body limit/continuation | Eleven operations maximum (one list plus ten reads), 10-of-20 partial result, ten-message continuation shortfall. | Pass |
| Uncached Mail.app hydration | Test binary received SQLite error 14 opening the Mail envelope database. Permissions were not changed. | Unverified |
| Duplicate subject | Sender/date disambiguation regression selected the correct fixture message. | Pass |
| Instruction-like body | Excluded before synthesis unless explicitly requested as quoted analysis. | Pass |
| Collapsed/expanded UI | Verified/partial/empty/unavailable/denied labels and Logical Activity retry/duplicate details passed Swift tests. | Pass |

## Web acceptance

| Case | Evidence | Verdict |
| --- | --- | --- |
| Direct page/final URL | Live IANA safe redirect passed three consecutive runs and cited `https://www.iana.org/help/example-domains`. | Pass |
| Safe and unsafe redirect | Live safe redirect plus private/local redirect and peer-pinning fixtures. | Pass |
| Simple authoritative fact | First-party fetched-evidence contract passed; no current live-fact run was retained. | Unverified live; fixture pass |
| Corroborated current fact | Two independent source identities and exact 2-of-2 progress passed fixtures; no current live-fact run was retained. | Unverified live; fixture pass |
| Conflicting sources | Typed conflict remained explicit with claim-adjacent citations. | Pass |
| Provider challenge | Challenged provider did not block the successful provider. | Pass |
| Every fetch fails | Produced Verification Shortfall and no model-eligible evidence. | Pass |
| Navigation/boilerplate removal | Wikipedia navigation, dense menus, and duplicated title text were excluded. | Pass |
| Evidence beyond initial region | Relevant late-page passages were retained and selected. | Pass |
| Duplicate candidate/final URLs | Canonical duplicates were suppressed and did not increment progress. | Pass |
| Retry budgets/grouping | Only retryable failures retried once; retry consumed budget and stayed in one Logical Activity. | Pass |
| Fetch overlap | Instrumented fixture observed a maximum of two concurrent fetches. | Pass |
| Citation policy | Only fetched final URLs were allowlisted; every factual segment required adjacent grounding. | Pass |

The original `example.com` smoke target produced a Verification Shortfall after
its page content became too minimal for the quality threshold. The live smoke
was changed to IANA's substantive example-domain help page; no extraction or
safety threshold was weakened.

## Installed-model compatibility

All matrix requests used frozen bundles, exactly one initial system message,
exactly one user message, and zero tools. Evidence acquisition was not rerun by
or delegated to a model. Production policy tests confirm 8B is never selected.

| Model | Cold load | Mail | Direct web | Corroborated web | Grounded |
| --- | ---: | --- | --- | --- | ---: |
| Qwen3 4B | 5,216 ms | Invalid; repair 8,767 ms invalid | Invalid; repair 4,516 ms invalid | Invalid; repair 4,607 ms invalid | 0/3 |
| Qwen3 8B | 26,029 ms | Timed out at 20,011 ms | Valid in 7,988 ms | Invalid; repair 6,431 ms invalid | 1/3 |
| Qwen3.6 35B-A3B | 323 ms | Model error in 247 ms | Invalid one-character output in 825 ms; repair failed in 15 ms | Model error in 7 ms | 0/3 |

The bounded production fallback behavior remains deterministic in automated
tests: preferred timeout/unavailability causes one 4B attempt, invalid fallback
output is rejected, and zero usable evidence invokes neither model. The live 4B
probe demonstrated readiness but not grounding: load 5,216 ms; initial
completion latencies 4,241/4,465/4,901 ms; 0/3 valid after repair. Deterministic
rendering therefore remains necessary.

## Performance

Thirty consecutive warm 35B attempts were collected: ten Mail, ten direct web,
and ten corroborated web. Production synthesis is non-streamed and buffers the
complete answer for validation, so TTFT is not applicable/observable.

The sanitized measurement command was:

```text
cargo test -p bagentd stage8_live_frozen_bundle_matrix_and_performance -- --ignored --nocapture
```

Raw prompt sizes were 1,452 characters for Mail, 1,133 for direct web, and
1,263 for corroborated web. These are raw measured character counts, not
provider-reported token estimates. Completion sizes across all 30 attempts were
0 to 68 characters because 27 attempts returned model errors.

| Population | n | p50 | p90 | p95 | Maximum |
| --- | ---: | ---: | ---: | ---: | ---: |
| All warm attempts, including immediate model errors | 30 | 6 ms | 93 ms | 931 ms | 3,254 ms |
| Attempts that returned any completion | 3 | 931 ms | 3,254 ms | 3,254 ms | 3,254 ms |

Only three warm attempts returned text: one invalid Mail output (931 ms), one
valid direct-web output (3,254 ms), and one invalid corroborated output (350
ms). Initial validation was 1/30 (3.3%). Repair was attempted for two returned
invalid outputs, succeeded 0/2, and grounding was 1/30 (3.3%). The other 27
attempts were model errors.

Raw warm rows:

| Sample | Workload | Latency ms | Completion chars | Outcome | Repair ms |
| ---: | --- | ---: | ---: | --- | ---: |
| 1 | Mail | 931 | 1 | Invalid | 11 |
| 2 | Direct web | 5 | 0 | Model error | - |
| 3 | Corroborated web | 5 | 0 | Model error | - |
| 4 | Mail | 14 | 0 | Model error | - |
| 5 | Direct web | 5 | 0 | Model error | - |
| 6 | Corroborated web | 6 | 0 | Model error | - |
| 7 | Mail | 11 | 0 | Model error | - |
| 8 | Direct web | 5 | 0 | Model error | - |
| 9 | Corroborated web | 13 | 0 | Model error | - |
| 10 | Mail | 4 | 0 | Model error | - |
| 11 | Direct web | 5 | 0 | Model error | - |
| 12 | Corroborated web | 16 | 0 | Model error | - |
| 13 | Mail | 5 | 0 | Model error | - |
| 14 | Direct web | 13 | 0 | Model error | - |
| 15 | Corroborated web | 13 | 0 | Model error | - |
| 16 | Mail | 93 | 0 | Model error | - |
| 17 | Direct web | 3,254 | 68 | Valid | - |
| 18 | Corroborated web | 350 | 3 | Invalid | 16 |
| 19 | Mail | 12 | 0 | Model error | - |
| 20 | Direct web | 15 | 0 | Model error | - |
| 21 | Corroborated web | 5 | 0 | Model error | - |
| 22 | Mail | 9 | 0 | Model error | - |
| 23 | Direct web | 4 | 0 | Model error | - |
| 24 | Corroborated web | 3 | 0 | Model error | - |
| 25 | Mail | 8 | 0 | Model error | - |
| 26 | Direct web | 3 | 0 | Model error | - |
| 27 | Corroborated web | 4 | 0 | Model error | - |
| 28 | Mail | 6 | 0 | Model error | - |
| 29 | Direct web | 8 | 0 | Model error | - |
| 30 | Corroborated web | 4 | 0 | Model error | - |

Three additional cold 35B cycles measured loads of 220, 225, and 333 ms
(p50 225 ms; maximum 333 ms). Their Mail completions were 941 ms invalid,
5,999 ms valid, and 973 ms invalid. Cold admission therefore succeeded
semantically only 1/3.

The nominal 8-second p50 and 15-second p95 latency numbers are not accepted:
immediate Metal/model errors make the all-attempt percentiles meaningless, and
there is only one grounded warm completion. No SLA is claimed from this sample.

## Runtime and memory

| State | BaseRT RSS |
| --- | ---: |
| Clean baseline | 8,256 KiB |
| 4B loaded | 663,504 KiB |
| 35B loaded, separate residency sample | 1,733,760 KiB |
| Peak during matrix/sustained run | 2,722,688 KiB |
| Immediate post-unload | 130,896 KiB |
| Recovered clean process | 8,416 KiB |

Swap used increased from 9,636.38 MiB to 11,368.06 MiB during the campaign
(+1,731.68 MiB), and the BaseRT log recorded Metal
`kIOGPUCommandBufferCallbackErrorOutOfMemory` failures during 35B prefill/decode.
The previous cold/sustained 35B Metal failure is reproduced, not ruled out.

Fixture cycles passed 4B -> 35B -> pressure/idle unload -> 4B fallback,
single-flight loading, fallback-before-preferred unloading, and shutdown
cleanup. Two Stage 8 lifecycle defects were fixed:

1. Legacy streaming 4B requests could overlap explicit 35B load/unload HTTP
   calls. A clone-shared BaseRT read/write lifecycle guard now holds model
   changes until the active request ends.
2. A 4B fallback could begin while another synthesis request still held a 35B
   lease, potentially leaving both models resident. Fallback now waits for all
   preferred leases to become idle before unloading 35B and loading 4B.

The regressions prove the lifecycle call cannot reach BaseRT during an active
legacy stream and that concurrent synthesis fallback leaves only the intended
4B model resident. Model selection is unchanged.

The 30-attempt mixed run did not show a steadily growing BaseRT RSS after
unload, but it did reproduce Metal failure and material swap growth. This is a
failure of admission/reliability, not evidence of a leak-free sustained
runtime. Final model inspection reported zero loaded weights. Repeated
load/unload exclusivity and concurrent synthesis are proven by deterministic
regressions; live concurrent synthesis and per-transition live exclusivity were
not run and remain unverified.

## Diagnostics and UI

- Exactly one terminal Evidence Outcome per turn: pass.
- Retry/duplicate grouping and non-inflating progress: pass.
- Seeded forbidden marker absent from retained trace and export: pass.
- Retention: seven-day age, 1,000-turn count, 256 KiB per turn, 512 events per
  turn, and 8 MiB global bounds all passed.
- Legacy non-evidence streaming/presentation tests: pass.
- Swift event decoding, collapsed outcomes, expanded activity detail, and
  accessibility labels: pass.

The shell regressions were captured before changes at `65198c8` and after all
Stage 8 changes. The same four scripts failed both times:

1. `check-fullscreen-notch-visibility.sh`
2. `check-non-notch-inline-pill.sh`
3. `check-notch-output-mail-regressions.sh`
4. `check-notch-output-scroll-stability.sh`

The inline-input focus, no-overflow, and thinking/output-dot scripts passed in
both runs. The four failures are baseline-identical and are not attributed to
Stage 8.

## Rollback and final state

- Feature flag disabled returns recognized Mail/web requests to legacy routing:
  passed by routing and integration regressions.
- The flag is read once at daemon startup; switching it requires only a daemon
  restart. Route selection itself has no Mail/rule/session write path.
- A live mutation comparison against an isolated FDA-capable daemon was not
  available, so the no-user-data-mutation aspect is unverified live.
- Release `bagent.app` rebuilt successfully and passed
  `codesign --verify --deep --strict` with an Apple Development signature.
- Final state: feature flag unset, zero BaseRT models loaded, normal app,
  daemon, BaseRT service, and WhatsApp bridge only; no acceptance harness
  process or temporary policy/configuration override remained.

## Complete verification

- `cargo fmt --all -- --check`: pass.
- `cargo test --workspace --no-fail-fast`: pass.
  - `bagentd`: 175 passed, 3 explicitly ignored live/acceptance tests.
  - BaseRT protocol: 9 passed, including the new lifecycle regression.
  - Apple Mail: 19 passed, 1 ignored.
  - Codex: 26 passed, 1 ignored.
  - Filesystem: 30 passed.
  - Odoo: 12 passed, 3 ignored.
  - WhatsApp: 13 passed.
- `cargo clippy --workspace --all-targets`: pass with pre-existing warnings.
- `swift test`: 39 passed.
- Live 4B Mail synthesis fixture: pass.
- Live IANA direct-web smoke: three consecutive passes after target repair.
- Signed release bundle build and strict verification: pass.
- `git diff --check`: pass.

## Section 14 acceptance map

| # | Criterion | Verdict | Evidence/qualification |
| ---: | --- | --- | --- |
| 1 | Exact prompt: one list, three reads, grounded answer | Unverified | Complete fixture pass; no live Mail proof. |
| 2 | Header-only never reads bodies | Pass | One list, zero reads. |
| 3 | Partial/empty/unavailable/denied are distinct | Pass | Orchestrator, validator, event, and Swift tests. |
| 4 | No snippet-only answer; final-URL citations | Pass | Validator tests plus live IANA final URL. |
| 5 | Two independent sources or explicit partial | Pass | Independence, duplicate, progress, and shortfall tests. |
| 6 | Mandatory operations model-independent | Pass | Typed orchestration plus zero-tool matrix. |
| 7 | No invented ID/URL/claim/citation reaches execution/user | Pass | Validated-ID, allowlist, grounding, and held-output tests. |
| 8 | One system, one user, no tools/mid-system | Pass | Asserted for every matrix and repair request. |
| 9 | One bounded 4B fallback; zero evidence calls neither | Pass | Runtime fallback/zero-evidence regressions. |
| 10 | Operation/retry/concurrency/round/load/synthesis limits | Pass | Budget, max-two fetch, single-flight, timeout, repair tests; live concurrent synthesis remains unverified. |
| 11 | Outcome UI; retries/duplicates cannot inflate progress | Pass | Rust event and Swift presentation tests. |
| 12 | Useful diagnostics with prohibited content absent | Pass | Seeded privacy and all retention-bound tests. |
| 13 | Same contracts for chat and automation | Pass | Origin-independent route/event regressions. |
| 14 | Unrelated connectors/rules/approvals unchanged | Conditional pass | Full workspace passes; four UI shell failures are identical to base. |
| 15 | Warm 35B admission target | Fail | 1/30 grounded, 27 model errors, reproduced Metal OOM. |

## Remaining risks and decision

1. Cold and sustained 35B Metal admission is unreliable on the target 32 GiB
   machine.
2. Grounded live synthesis is unreliable for 35B and 4B; deterministic
   rendering protects correctness but does not satisfy model admission.
3. Live uncached Mail.app hydration and the exact prompt on real Mail remain
   unverified because existing permissions were not changed.
4. Live simple/current and corroborated web-fact acquisition were not retained;
   their deterministic production-adapter fixtures passed.
5. A no-mutation rollback comparison with an isolated FDA-capable daemon
   remains unverified.

Because criteria 1 and live rollback/Mail proof are unverified and criterion 15
fails, Stage 8 does not accept the redesign for default enablement.
