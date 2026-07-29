# Stage 8 final grounding requalification

Date: 2026-07-29

Base commit: `4fe10b64fb3fcf8830d49ab71aaa4a40bc1d038e`

Overall verdict: **FAIL**

Stage 9 authorization: **NOT AUTHORIZED**

`BAGENT_EVIDENCE_ORCHESTRATOR` remains disabled by default and was removed from
the live daemon environment after acceptance. No prompt, Mail content, fetched
passage, raw connector identifier, model answer, or credential was retained in
this report or in an artifact. Model output was held only in process memory and
reduced to structural counts.

## Remediation

The synthesis contracts now:

- require strictly extractive Mail and web answers;
- prohibit introductions, explanations, inferences, transitions, conclusions,
  and background claims unless directly supported by evidence;
- require exactly one Mail entry per supplied message, in order, containing
  only Sender, Subject, Date, and a contiguous body excerpt as Summary;
- require every factual web sentence to end with an allowlisted,
  claim-adjacent citation; a citation-only following sentence is rejected;
- identify the exact failing Mail entry or web sentence in repair feedback;
- include the eligible fetched final URL when an uncited sentence overlaps a
  supporting passage;
- tell repair to remove unsupported claims or rewrite them using only
  overlapping evidence terms.

Numeric, coverage, grounding, citation, allowlist, conflict, shortfall, and
injection validation were not relaxed relative to the base commit. Citation
position validation was tightened to implement the requested sentence-ending
contract. An uncited sentence with no overlapping eligible passage is
classified as unsupported rather than being given an unrelated citation.

Deterministic regressions cover the observed residual shapes: Mail preambles,
header-only summaries, paraphrased/inferred body claims, extra continuation
claims, exact failing-entry feedback, uncited supported web sentences with an
eligible final URL, unsupported cited sentences, exact failing-sentence
feedback, citation-only following sentences, unallowlisted citations, internal
metadata, corroborated two-source repair, repair-still-invalid fallback, and
held invalid output.

## Grounding failure analysis

The earlier Stage 8 campaign recorded 75 of 90 initially valid and 80 of 90
valid after repair. Its only retained structural aggregate was three missing
citations and twelve initial unsupported claims, with seven unsupported claims
still invalid after repair. The per-request rows needed to assign those exact
22 observations to Mail, direct web, or corroborated web were not retained.
Model output was intentionally not retained, so that exact historical workload
breakdown cannot be reconstructed and remains **unverified**.

Because the earlier structural rows were not retained, a new controlled
pre-change rebaseline ran the untouched base binary in three clean processes.
It reproduced the same failure classes and supplied the required workload
breakdown:

| Workload | Requests | Initially valid | Repaired valid | Final grounded | Missing citation initial/final invalid | Unsupported claim initial/final invalid |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Mail | 30 | 29 | 1 | 30 | 0 / 0 | 1 / 0 |
| Direct web | 30 | 28 | 0 | 28 | 0 / 0 | 2 / 2 |
| Corroborated web | 30 | 9 | 7 | 16 | 3 / 2 | 18 / 12 |
| **Total** | **90** | **66** | **8** | **74** | **3 / 2** | **21 / 14** |

The controlled rebaseline was stochastic and therefore does not replace the
earlier 75/80 admission record. It shows that missing citations were confined
to corroborated web and that unsupported claims were overwhelmingly a
corroborated-web repair problem.

Final strict remediation matrix:

| Workload | Requests | Initially valid | Repaired valid | Final grounded | Deterministic rendering | Missing citation initial/final invalid | Unsupported claim initial/final invalid |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Mail | 30 | 23 | 0 | 23 | 7 | 0 / 0 | 7 / 6 |
| Direct web | 30 | 7 | 3 | 10 | 20 | 23 / 19 | 0 / 1 |
| Corroborated web | 30 | 9 | 0 | 9 | 21 | 19 / 14 | 2 / 7 |
| **Total** | **90** | **39** | **3** | **42** | **48** | **42 / 33** | **9 / 14** |

The strict sentence-ending citation rule exposed that many previously accepted
web answers put the citation in a later citation-only sentence. Under the final
required contract, grounded model synthesis is 42/90, below both the 74/90
controlled base rebaseline and the earlier 80/90 campaign. Three of 51 repairs
became valid. This is not materially improved grounded synthesis and is a
decisive acceptance failure.

The 48 remaining deterministic results are not counted as grounded model
output:

- seven Mail repairs remained invalid: six unsupported claims and one coverage
  failure used the bounded deterministic body-excerpt renderer;
- 20 direct-web repairs and 21 corroborated-web repairs remained invalid and
  used deterministic fetched-passage rendering with allowlisted final URLs;
- every case has the structural reason
  `validation_rejected_after_one_bounded_repair`.

All 48 invalid model answers remained held and were never emitted. The
deterministic web renderer is safe but less expressive and may select one
extractive claim after the two-source acquisition requirement was satisfied;
that is why it is classified separately rather than credited as model
grounding.

## Independent 90-request runtime result

Each repetition used a new BaseRT process, Qwen3.6-35B-A3B loaded directly,
4,096 maximum context, KV4, output cap 256, maximum batch size 1, and the
repeating Mail/direct-web/corroborated-web sequence.

| Process | Requests | Metal/model errors | Initially valid | Repaired valid | Final grounded | Safe terminal | Initial/final free pressure | Initial/final swap MiB |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 30 | 0 | 9 | 0 | 9 | 30 | 79% / 13% | 4,459.31 / 4,733.06 |
| 2 | 30 | 0 | 9 | 2 | 11 | 30 | 77% / 11% | 4,701.06 / 4,620.12 |
| 3 | 30 | 0 | 21 | 1 | 22 | 30 | 75% / 19% | 4,752.25 / 4,656.25 |
| **Total** | **90** | **0** | **39** | **3** | **42** | **90** | — | — |

Results:

- 90/90 free of Metal, timeout, transport, and model errors;
- 90/90 safe terminal results: 42 validated model answers and 48 separately
  classified deterministic renderings;
- zero invalid model answers emitted;
- zero Metal signatures in the three runtime logs;
- exactly one system message, one user message, and zero tools in every initial
  and repair synthesis request.

## Latency and memory

Initial 35B completion latency over 90 requests:

| n | Minimum | p50 | p90 | p95 | Maximum |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 90 | 2,426 ms | 2,609 ms | 4,989 ms | 5,146 ms | 10,081 ms |

Repair latency:

| n | Minimum | p50 | p90 | p95 | Maximum |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 51 | 2,511 ms | 2,579 ms | 4,815 ms | 4,841 ms | 5,182 ms |

Sampled BaseRT RSS ranged from 801,456 to 1,489,648 KiB. Per-process free
pressure fell to 11–19% while the 35B model was resident. RSS is recorded
diagnostically, not used as an admission or cleanup proof; final zero-weight
state is verified separately after shutdown.

## Signed-app live acceptance

The Apple Development-signed `apps/macos/bagent.app` used its existing Mail and
Automation permissions. No Full Disk Access, Automation, TCC, rules, or
approval setting was changed. These live cases were completed before the final
independent-review correction that made sentence-ending citation placement
strict. They remain exact observations of that signed remediation build, but
were not repeated on the final source revision; therefore they do not qualify
the final strict binary where behavior could be affected.

| Case | Exact live result | Verdict |
| --- | --- | --- |
| `can you read me the 3 latest emails?` | Signed UI trace executed one `mail_list_inbox` plus three distinct `mail_read` operations, zero denials. A separate signed-daemon turn returned exactly three Sender, Subject, Date, and Summary fields, nonempty output, no internal metadata, one Evidence Outcome, and one done event. Because operation trace and answer coverage were not captured from the same turn, the combined end-to-end criterion remains unverified. | Unverified |
| Header only | One listing, zero body reads, verified nonempty response, one Evidence Outcome, one done event, no internal metadata. | Pass |
| Naturally uncached body | The three latest messages naturally required Automation hydration; all three became readable. Existing permissions were used unchanged. | Pass |
| Authoritative current fact | The first World Bank query safely shortfalled. A Federal Reserve first-party query then acquired and cited exactly 1 of 1 authoritative source with one terminal outcome. | Pass |
| Corroborated current fact | Current federal-funds query acquired exactly 2 of 2 independent sources and returned two citations with one terminal outcome. | Pass |
| Safe redirect/final URL | IANA direct page followed the safe redirect, acquired 1 of 1, and cited `https://www.iana.org/help/example-domains`. | Pass |
| Verification Shortfall | `.invalid` direct page acquired 0 of 1 and returned explicit Verification Shortfall text with one terminal outcome. | Pass |
| Terminal outcome cardinality | Every evidence-routed live turn emitted exactly one Evidence Outcome. | Pass |

The first authoritative query and the first `example.com` shortfall target were
not counted as passes for the intended fact/shortfall cases; successful,
structurally matching retries are listed explicitly above.

## Rollback and no-mutation comparison

- The daemon and BaseRT launch plists were captured before testing and restored
  byte-for-byte afterward.
- The test-only flag was present only in the test daemon process. The final
  plist, shell environment, and runtime contain no
  `BAGENT_EVIDENCE_ORCHESTRATOR`.
- A post-disable live request produced the legacy activity/tool/token/done
  stream and zero Evidence Outcomes, proving legacy routing was restored.
- Approval rows remained 0; pending-approval rows remained 20; automation rows
  remained 1. No approval decision or automation execution occurred.
- The combined rules/approval/automation/connector hash changed because the
  connector `last_sync_at` state changed during normal startup. Protected
  rules/approval/automation definitions were not independently hashed before
  that full live sequence, so exact byte-level immutability of those three
  subsets remains **unverified**, although counts and executed operation types
  show no mutation path.
- The derived Mail cache gained one header during normal background sync
  (1,501 to 1,502). This is an expected local cache write, not a source Mail
  mutation. Direct raw hashing of `~/Library/Mail` from the shell was denied by
  macOS TCC (`Operation not permitted`), so source-store byte comparison remains
  **unverified**. Live connector operations were read-only and no read-state or
  Mail write operation was invoked.
- Expected acceptance writes were 61 audit rows and 12 evidence-turn audit
  rows. Chat remained stateless (`chat_turns` stayed 0).
- Final state: app and daemon stopped, BaseRT stopped, port 8082 closed, zero
  resident model process/weights, and no acceptance harness process.

Because exact protected-subset and raw Mail-store before/after hashes were not
both available, rollback receives a conditional rather than unconditional
pass. This alone forbids Stage 9 authorization under the task rules.

## Fifteen acceptance criteria

| # | Criterion | Verdict | Evidence/qualification |
| ---: | --- | --- | --- |
| 1 | Exact prompt: one list, three distinct reads, grounded answer | Unverified | The operation trace and three-entry answer were captured in separate signed-daemon turns, not one end-to-end turn. |
| 2 | Header-only request never reads bodies | Pass | Live one list, zero reads. |
| 3 | Partial/empty/unavailable/denied remain distinct | Pass | Full deterministic evidence and presentation suites. |
| 4 | No snippet-only answer; final-URL citations | Unverified on final revision | Live IANA redirect passed on the prior signed remediation build; final strict citation placement was not re-exercised live. |
| 5 | Two independent sources or explicit shortfall | Unverified on final revision | Live exactly 2 of 2 and explicit shortfall passed on the prior signed remediation build only. |
| 6 | Mandatory evidence operations are model-independent | Pass | Typed signed-app trace and zero-tool synthesis matrix. |
| 7 | No invented ID, URL, claim, or citation reaches execution/user | Pass | Held-output validation, allowlists, exact repair tests, and 48 safely rejected matrix answers. |
| 8 | One system, one user, no tools or mid-system messages | Pass | Asserted across all 90 initial/repair requests. |
| 9 | One bounded 4B fallback; zero evidence calls neither model | Pass | Runtime lifecycle and zero-evidence suites. |
| 10 | Operation/retry/concurrency/round/load/synthesis limits | Pass | Full evidence and BaseRT lifecycle suites. |
| 11 | Outcome UI; retry/duplicate progress cannot inflate | Pass | Rust/Swift suites and one live terminal outcome per routed turn. |
| 12 | Useful diagnostics with prohibited content absent | Pass | Privacy/retention suites and structural-only live capture. |
| 13 | Same contracts for chat and automation | Pass | Origin-independent Rust suites. |
| 14 | Unrelated connectors/rules/approvals/automations unchanged | Conditional pass | Counts and operation paths unchanged; connector sync and missing independent pre-hashes prevent an exact protected-subset proof. |
| 15 | Warm 35B admission and grounding target | Fail | Runtime 90/90, but only 42/90 grounded model synthesis after repair. |

## Complete verification

- `cargo fmt --all -- --check`: pass.
- `cargo test --workspace --no-fail-fast`: pass.
  - `bagentd`: 187 passed, 3 explicitly ignored live/acceptance tests.
  - BaseRT protocol/lifecycle: 13 passed.
  - Apple Mail: 19 passed, 1 explicitly ignored live test; the equivalent
    naturally uncached signed-app path passed live.
  - Codex: 26 passed, 1 ignored; filesystem 30 passed; Odoo 12 passed,
    3 ignored; WhatsApp 13 passed.
- `swift test`: 39 passed.
- `cargo clippy --workspace --all-targets`: pass with pre-existing warnings.
- UI regression comparison: inline-input focus, no-overflow promotion, and
  thinking/output-dot scripts passed. The same four scripts recorded as failing
  at the base commit still fail: fullscreen notch visibility, non-notch inline
  pill, Mail-output regressions, and output-scroll stability. No UI file changed.
- Release bundle rebuilt after the final source change and passed
  `codesign --verify --deep --strict` with identifier `sk.bagent.app`, Apple
  Development signing, and team `QUB47S3XTF`.
- `git diff --check` and privacy scans: pass.

## Final decision

Stage 8 final requalification is **FAIL**. Runtime recovery, safety containment,
and diagnostics privacy passed. Grounded synthesis did not improve under the
required strict citation contract: 48 of 90 clean-process requests required
deterministic rendering. The exact Mail end-to-end turn, final-revision signed
web behavior, and parts of rollback immutability remain unverified. Stage 9 is
therefore **not authorized**.
