# Stage 8 final acceptance against 5aadc86

Date: 2026-07-29

Revision under test: `5aadc8627e8110a63520fad45e5e1afd182cce20`

Overall verdict: **FAIL**

Stage 9 default enablement is **not authorized**. The final acceptance run found
one reproducible deterministic relevance defect on the exact revision and did
not obtain the mandatory positive corroborated or renderable-conflict live
results. Safe shortfalls were preserved. The feature flag remained opt-in and
was removed after the run.

This report retains only public URLs and structural acceptance metadata. It
contains no Mail content, private prompt, raw connector identifier, credential,
or fetched passage.

## Criterion verdicts

| # | Criterion | Verdict | Acceptance evidence |
| ---: | --- | --- | --- |
| 1 | Exact Mail content | **PASS** | Signed turn `04c34547-e1b9-4109-abf9-7c1b04cc9dfa`: one list, three distinct reads, three complete Sender/Subject/Date/Summary entries, one done, one `Read 3 of 3 emails` outcome. All three reads reported `body_origin=mail_automation`; no forbidden identifier marker was present. Validation was `bundle_complete`; optional polish was rejected and canonical output survived. |
| 2 | Simple authoritative current fact | **FAIL** | Exact-revision turn `f31c7c18-fe37-4dd9-bb72-dbea1ee6f46f` fetched one page typed first-party and emitted `Web verified · 1 sources`, but its canonical claim concerned U.S. presidential elections rather than the President of Slovakia. This was a reproducible answer-quality relevance defect. The deterministic correction was added and verified; corrected signed turn `b110b62a-bd7f-4cdc-b128-92b440fbb8f8` safely shortfalled instead of exposing the unrelated claim, but still did not provide the required useful positive answer. |
| 3 | Corroborated fact | **FAIL** | Turn `42bd0ff8-4622-4c47-aae1-a0bedcac221f` fetched `wikipedia.org` and `planetrulers.com`, but fewer than two answer-quality claims remained. It emitted one `verification_shortfall` outcome, not the required two cited claims and verified terminal result. Tavily and DuckDuckGo both returned typed successful searches. |
| 4 | Renderable conflict | **FAIL** | Turns `3d16dc76-456e-4236-976f-49b3e327775b` and `42ef7e7a-1ff4-44b5-a0e8-042a7456cc35` acquired two typed source identities but did not retain two explicit figure/date-or-definition claims. Both safely emitted one shortfall. The required separate bullets, adjacent citations, typed conflict, and `Web verified · 2 sources · conflict` were not reached live. Deterministic conflict rendering and terminal-label tests pass. |
| 5 | Unstructured Bratislava conflict | **PASS** | Exact prompt turn `39dd7276-dfcd-4d8b-be2c-ea1e218727da` fetched independent publishers but rejected unreliable figure/date-or-definition associations and emitted one `Couldn't verify sources` outcome. No flattened row reached the answer. |
| 6 | Provider failures | **FAIL** | Focused provider suites distinguish missing credential, 429/quota, timeout, malformed response, challenge, empty, and normalized failure. Tavily 429 consumes one call with no retry; DuckDuckGo retains the bounded fallback slot. Live successful searches recorded provider, status, candidate count, and bounded search/fetch consumption, but this run did not inject all four provider failures through the signed app. Automated proof is not a substitute for the required signed-app failure cases. |
| 7 | Rollback | **PASS** | After removing `BAGENT_EVIDENCE_ORCHESTRATOR` and restoring the registered 4B model, the signed legacy request emitted three legacy tool calls, one done, zero Evidence Outcomes, and zero Tavily events. Automated routing tests also prove the disabled flag executes no typed operations. |
| 8 | Privacy and credentials | **PASS** | The Tavily entry exists only as a Keychain generic password for service `sk.bagent.app` and account `bagent.tavily.apikey`. The signed client sends it only to the authenticated loopback `/web/tavily/config` endpoint. Posting `null` cleared daemon memory. Secret-pattern scans found no live key in logs, diagnostics, prompt traces, application data, repository worktree, commit history, or LaunchAgent plists; the daemon plist contains no Tavily field. |
| 9 | Runtime | **PASS** | Signed Mail turn `04c34547-e1b9-4109-abf9-7c1b04cc9dfa` recorded rejected optional 35B polish and returned the canonical result unchanged. Other signed turns recorded rejected, unavailable, or not-attempted polish without losing canonical output. Deterministic lifecycle suites cover bounded admission, timeout, Metal poisoning, restart-before-fallback, failed restart, and zero-evidence model avoidance. The final registered 4B legacy model was explicitly unloaded; app, daemon, and BaseRT were then stopped, leaving no loaded process or model. |
| 10 | Complete validation and baseline comparison | **PASS** | Rust workspace, Swift suite, focused evidence/rendering/events/diagnostics/provider/lifecycle tests, formatting, clippy, diff, privacy, bundle, and signing checks passed. UI shell results matched the established baseline exactly: three passes and four known failures. |

## Structural live record

| Case | Turn | Exact provider statuses | Fetched domains and claim decision | Validation decision | Polish result | Canonical result and terminal |
| --- | --- | --- | --- | --- | --- | --- |
| Mail 3 latest | `04c34547-e1b9-4109-abf9-7c1b04cc9dfa` | Mail list/read completed | Private domains omitted | `bundle_complete` | rejected | `Read 3 of 3 emails` |
| Simple official, exact revision | `f31c7c18-fe37-4dd9-bb72-dbea1ee6f46f` | Tavily `succeeded { result_count: 6 }`; DuckDuckGo `challenged` | fetched `usembassy.gov`, `robert-schuman.eu`; one unrelated claim incorrectly accepted | `bundle_complete`, eligible | `rejected` | unrelated canonical claim; `Web verified · 1 sources` |
| Simple official, corrected | `b110b62a-bd7f-4cdc-b128-92b440fbb8f8` | Wikipedia `succeeded { result_count: 2 }` twice; DuckDuckGo `succeeded { result_count: 6 }`, then `challenged` | fetched `wikipedia.org`, `prezident.sk`, `saia.sk`; zero claims accepted | `recovery`, ineligible | not attempted | canonical Verification Shortfall; `Couldn't verify sources` |
| Corroborated | `42bd0ff8-4622-4c47-aae1-a0bedcac221f` | Tavily `succeeded { result_count: 6 }`; DuckDuckGo `succeeded { result_count: 6 }` | fetched `wikipedia.org`, `planetrulers.com`; fewer than two claims accepted | `bundle_complete`, eligible, one typed evidence conflict | `unavailable` | canonical Verification Shortfall; `Couldn't verify sources` |
| Conflict attempt | `3d16dc76-456e-4236-976f-49b3e327775b` | Tavily `succeeded { result_count: 6 }`; DuckDuckGo `succeeded { result_count: 6 }` | fetched `nationalgeographic.com`, `eathealthy365.com`; fewer than two associated claims accepted | `bundle_complete`, eligible, one typed evidence conflict | `unavailable` | canonical Verification Shortfall; `Couldn't verify sources` |
| Targeted conflict retry | `42ef7e7a-1ff4-44b5-a0e8-042a7456cc35` | Tavily `succeeded { result_count: 6 }`; DuckDuckGo `succeeded { result_count: 6 }` | fetched `nationalgeographic.com`, `nationalgeographic.org`; Britannica extraction `empty`; fewer than two associated claims accepted | `bundle_complete`, eligible, one typed evidence conflict | `unavailable` | canonical Verification Shortfall; `Couldn't verify sources` |
| Bratislava | `39dd7276-dfcd-4d8b-be2c-ea1e218727da` | Tavily `succeeded { result_count: 6 }`; DuckDuckGo `succeeded { result_count: 6 }` | fetched `citypopulationdata.com`, `worldpopulationreview.com`; fewer than two reliable associations accepted | `bundle_complete`, eligible, one typed evidence conflict | `rejected` | canonical Verification Shortfall; `Couldn't verify sources` |

## Reproducible correction

The exact revision accepted a prose fact when only one generic query term
overlapped the passage. The correction requires two salient query terms when a
multi-term query supplies them. A deterministic regression proves that an
American presidential-election passage is ineligible for a President of
Slovakia query while a passage naming the President of Slovakia remains
eligible. No routing, provider budget, privacy, model, or Stage 9 default was
changed.

## Validation

- `cargo test --workspace --no-fail-fast`: pass; `bagentd` 204 passed, three
  ignored after the correction; BaseRT protocol/lifecycle 13 passed; all other
  workspace suites passed with only their documented live-test ignores.
- `swift test`: 39 passed.
- Focused web provider suite: 28 passed.
- Focused deterministic web fallback suite: three passed.
- New relevance regression: passed.
- `cargo clippy --workspace --all-targets`: pass with established warnings.
- `cargo fmt --all -- --check`: pass.
- `git diff --check`: pass.
- `make bundle`: pass.
- `codesign --verify --deep --strict`: pass; identifier `sk.bagent.app`, team
  `QUB47S3XTF`.
- UI shell baseline: pass for inline focus, no-input overflow, and thinking/output
  dot layer; established fail for fullscreen notch, non-notch inline pill,
  Mail-output regression shell, and output-scroll stability.

## Final decision

Stage 9 remains disabled. The narrow remaining blockers are:

1. a signed simple-authoritative turn must fetch a relevant first-party page and
   produce a useful canonical answer after the relevance correction; and
2. a signed two-publisher turn must retain two explicit, associated
   figure/date-or-definition claims and render the required conflict outcome;
   and
3. the Tavily 429, timeout, malformed-response, and unavailable-credential
   shapes must be exercised through the signed app, including observed bounded
   DuckDuckGo fallback and zero Tavily retry.

The exact-revision false-positive relevance defect is corrected by the only
implementation change included with this report. No legacy seam was retired and
typed evidence routing was not enabled by default.
