# Stage 8 canonical final acceptance

Date: 2026-07-29

Revision under test: `877da8341d3de3c8fb01a472784e9fcb0793a3f4`

Overall verdict: **CONDITIONAL PASS**

Current Stage 7C/8 qualification boundary: signed/live qualification covers
macOS 26 only. macOS 14 and 15 remain compile targets when configuration
permits them, without runtime, System Settings, TCC, visual, or accessibility
qualification claims. Live TCC grant, denial, revocation, and
drag-to-System-Settings mutation are outside the campaign. This historical
Stage 8 record does not turn omitted macOS 14/15 or live-TCC checks into PASS.
Deterministic permission adapters, signed-bundle and drag-payload validation,
privacy tests, and daemon-preserving relaunch remain required; release
evidence must carry this limitation.

Stage 9 default enablement is **not authorized**.

The deterministic canonical path passed the 90-request reliability campaign,
runtime containment, signed Mail turn, rollback, privacy, and build gates. The
required final-revision live web gate did not pass: direct redirect and total
failure behaved correctly, but live search did not acquire an authoritative
source for the simple fact or two independent sources for corroborated and
conflicting-source cases. The safe terminal result was an explicit shortfall,
not an unsupported answer. No safety rule was weakened to turn those shortfalls
into passes.

`BAGENT_EVIDENCE_ORCHESTRATOR` was enabled only in an isolated signed-app daemon
process. It is absent from the final plist and environment. This report retains
only structural counts and public web URLs; it contains no Mail body, private
prompt, raw connector identifier, model answer, credential, or fetched passage.

## Canonical reliability campaign

Three independent clean BaseRT processes each ran 30 alternating frozen
requests: ten Mail, ten direct web, and ten corroborated web. Every process used
Qwen3.6-35B-A3B, context 4,096, KV4, output cap 256, and batch size 1. Each
process was retired before the next repetition.

| Workload | Requests | Canonical grounded | Safe terminal | Polish accepted | Polish rejected | Accepted after repair | Poisoned |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Mail | 30 | 30 | 30 | 22 | 8 | 1 | 0 |
| Direct web | 30 | 30 | 30 | 3 | 27 | 2 | 0 |
| Corroborated web | 30 | 30 | 30 | 2 | 28 | 1 | 0 |
| **Total** | **90** | **90** | **90** | **27** | **63** | **4** | **0** |

Canonical gates:

- 90/90 deterministic canonical answers were complete before optional model
  admission and remained the fallback for every rejected polish;
- 90/90 safe terminal results; zero Metal, model, transport, or timeout failure;
- zero unsupported canonical claims, missing canonical Mail entries, unsafe or
  missing canonical citations, raw connector IDs, internal metadata,
  boilerplate, or invalid model output reaching the terminal result;
- exactly one terminal Evidence Outcome is enforced by the event sink and was
  observed in every signed evidence turn;
- every initial and repair model request had one system message, one user
  message, and zero tools.

The campaign harness labels accepted optional model output as `grounded`; it
does not re-label rejected polish as a canonical failure. Canonical output is
constructed first and returned when polish is invalid.

## Optional 35B polish

| Status | Count | Scope |
| --- | ---: | --- |
| Attempted | 90 | Clean-process campaign |
| Accepted | 27 | Includes four accepted after one repair |
| Rejected | 63 | Canonical bytes retained |
| Repaired | 4 | Accepted subset, not extra attempts |
| Timed out | 0 | Campaign |
| Memory-ineligible | 6 | Signed live evidence turns |
| Skipped | 0 | Campaign |
| Poisoned | 0 | Campaign |

Accepted polish passed the invariant validator for facts, numbers, dates,
coverage, conflicts, shortfalls, and citation targets. Rejected polish was held
and the precomputed canonical output was returned byte-for-byte. Deterministic
regressions separately exercised timeout, memory-ineligible admission,
rejection, Metal poisoning, restart-before-fallback, failed restart, and clean
retirement. Polish acceptance rate is informational.

## Signed-app Mail acceptance

The release bundle was rebuilt from the tested revision and passed strict deep
verification. Bundle identifier is `sk.bagent.app`, signing authority is Apple
Development, and team identifier is `QUB47S3XTF`. No Full Disk Access,
Automation, or TCC permission was changed.

One correlated turn (`39fe8f87-3ba2-40fb-81b0-2b441a2aff28`) proved:

- one completed `mail.list` Logical Activity and three distinct completed
  `mail.read` Logical Activities;
- one three-entry canonical answer from that same turn, with Sender, Subject,
  Date, and bounded body-supported Summary for every entry;
- zero `rowid`, `connector_id`, or internal-metadata markers in the SSE;
- exactly one `Read 3 of 3 emails` terminal outcome and one done event;
- diagnostic export contained 21 structural events, one outcome, four started
  and four completed activities, and no prompt, response, Mail body, raw ID,
  credential, or passage marker.

Additional Mail results:

- header-only: one list, zero reads, one outcome, one done;
- 20 requested: one list, ten reads, a `Read 10 of 20 emails · partial`
  outcome, and explicit text that ten messages were omitted because the batch
  limit is ten;
- partial, empty, unavailable, and safely simulated denied states remain
  distinct in deterministic event/presentation suites;
- the correlated live bodies were readable through the existing signed-app
  path, but the live structural trace does not expose whether each body was
  initially uncached. Therefore the narrower claim that this exact revision
  naturally hydrated an initially uncached body remains **unverified**;
- executed evidence operations were read-only list/read/fetch operations. No
  Mail write-capable operation or approval decision was invoked.

## Signed-app web acceptance

| Case | Exact final-revision result | Verdict |
| --- | --- | --- |
| Direct page and safe redirect | Acquired one source and cited the allowlisted final URL `https://www.iana.org/help/example-domains`; no passage dump. | Pass |
| Simple authoritative fact | Search/fetch exhausted safely without an eligible authoritative source; explicit Verification Shortfall. | Fail |
| Corroborated current fact | Acquired one independent source, not two; terminal partial outcome. A targeted retry had the same result. | Fail |
| Conflicting-source live-safe case | Acquired one independent source, so no two-source conflict comparison was possible; terminal partial outcome. | Fail |
| All fetches fail | Acquired zero of one and returned explicit Verification Shortfall. | Pass |

The failed cases were caused by the live provider returning candidates that
collapsed to one registrable source or failed authority/relevance validation.
This is an acquisition shortfall, not evidence that an unsupported canonical
claim escaped. The direct-page answer used its validated final URL, and every
factual canonical web statement remained claim-adjacently cited from the
allowlist.

## Runtime, rollback, and mutation boundary

- Memory-eligible 35B residency: three clean-process campaign runs completed
  without Metal/model failure.
- Memory-ineligible skip: six signed live turns returned canonical output
  without loading 35B.
- Rejected polish: 63 campaign cases retained canonical output.
- Poisoned runtime: deterministic Metal/timeout simulations prove canonical
  output is unaffected, further requests to the poisoned process are
  suppressed, and restart precedes any later fallback.
- Clean-process retirement: every campaign process unloaded before shutdown;
  final runtime inspection after shutdown is recorded below.
- Rollback: after removing the flag and restarting, the next Mail request used
  the legacy route, emitted three legacy tool calls and one done event, and
  emitted zero Evidence Outcomes.

Independent before/after hashes:

| Protected surface | Before | After | Result |
| --- | --- | --- | --- |
| `rules.yaml` | `6b8db65652bb2e5349349d17ce3942f34e714fbd0a0eb0d70b1779400afb0bc0` | same | Unchanged |
| Approval policy/rows | `0c50a158a44379ecdff8a177b1581837237cf688c5c84d54f0ee10b302a0bdd8` | same | Unchanged |
| Automation definition/state row | `2d5ef053c31ef93fc810927351c880a041be772ce6b14194aaea064c6fa455a0` | same | Unchanged |

Expected writes were audit entries, evidence diagnostic files, session rows,
and Mail cache synchronization. `chat_turns` remained zero. Prohibited source,
rule, approval, automation, permission, and Mail-write mutations were not
observed. User-owned untracked files were untouched.

## UI, diagnostics, and regression checks

Read-only inspection of the signed app showed the collapsed `Couldn't verify
sources` label. Expanding it showed the `web.fetch` Logical Activity with its
typed empty/failure detail. Swift accessibility/label tests also cover verified,
partial, empty, unavailable, and denied outcomes.

Seeded privacy, retention, per-turn rotation, export, redaction, and prohibited
field tests passed. The correlated export was independently scanned and had
zero forbidden-content hits.

The shell scripts were invoked with `bash`. Results match the recorded base:

- pass: inline-input focus retention, no-input-overflow promotion, and
  thinking/output-dot layer;
- fail: fullscreen-notch visibility, non-notch inline pill, Mail-output
  regressions, and output-scroll stability.

The same four failures are pre-existing non-regressions. No behavior relevant
to the evidence outcome/activity UI regressed. They remain separate follow-up
work and do not block canonical evidence correctness, but no new external
ticket identifiers were created during this repository-only acceptance run.

## Fifteen specification criteria

| # | Criterion | Verdict | Evidence |
| ---: | --- | --- | --- |
| 1 | One list, three distinct reads, same-turn grounded answer | Pass | Correlated signed turn. |
| 2 | Header-only performs no reads | Pass | Signed turn: one list, zero reads. |
| 3 | Partial/empty/unavailable/denied remain distinct | Pass | Rust and Swift suites. |
| 4 | Final-URL citations; no snippet-only answer | Pass | Signed IANA redirect plus safety suites. |
| 5 | Two independent sources or explicit shortfall | Conditional pass | Safety contract passed; required live 2-of-2 case did not. |
| 6 | Mandatory acquisition is model-independent | Pass | Canonical-first path and tool-free polish. |
| 7 | No invented ID, URL, claim, or citation reaches user | Pass | 63 rejected polish results held. |
| 8 | One system, one user, zero tools | Pass | 90-request campaign assertions. |
| 9 | Bounded fallback; zero evidence invokes no model | Pass | Runtime suites. |
| 10 | Operation/retry/concurrency/load limits | Pass | Full orchestrator/lifecycle suites. |
| 11 | One outcome; retry/duplicates do not inflate progress | Pass | Signed turns and event suites. |
| 12 | Useful privacy-safe diagnostics | Pass | Export scan and retention suites. |
| 13 | Same contracts for chat and automation | Pass | Origin-independent suites. |
| 14 | Unrelated rules/approvals/automations unchanged | Pass | Independent equal hashes. |
| 15 | Stable 35B optional polish boundary | Pass | 90 safe turns, zero poisoned, clean-process isolation. |

## Complete verification

- `cargo test --workspace --no-fail-fast`: pass; `bagentd` 192 passed, three
  ignored; BaseRT protocol/lifecycle 13 passed; Apple Mail 19 passed, one live
  test ignored; Codex 26 passed, one ignored; filesystem 30; Odoo 12, three
  ignored; WhatsApp 13.
- `swift test`: 39 passed.
- `cargo clippy --workspace --all-targets`: pass with existing warnings.
- `cargo fmt --all -- --check`: pass.
- `git diff --check`: pass.
- `make bundle`: pass.
- `codesign --verify --deep --strict`: pass.
- UI shell comparison and signed visual smoke: completed as described above.
- Privacy/retention/export scans: pass.

Final state after acceptance: routing flag absent; daemon plist restored to its
original hash `df0ff88f04f56929d44ba9a3e8ad4a1b42faa01ea051460dfe8fd4c88319ec6d`;
app, daemon, and BaseRT stopped; port 8082 closed; zero loaded weights; no
campaign harness process running.

## Final decision and blockers

Stage 8 canonical evidence is safe and correct under the deterministic
canonical contract, including 90/90 clean-process canonical answers. The final
decision remains **CONDITIONAL PASS**, not full pass, because the task requires
all live criteria on this exact revision.

Precise blockers:

1. obtain and retain a signed final-revision simple/current authoritative live
   answer rather than a safe shortfall;
2. obtain and retain a signed final-revision corroborated answer with exactly
   two independent sources and 2-of-2 progress;
3. exercise a signed final-revision two-source conflict case;
4. prove from a correlated structural signal that a naturally uncached Mail
   body was hydrated through the existing Automation permission;
5. create/link separate tickets for the four baseline-identical UI script
   failures if ticket tracking is required as an acceptance artifact.

Because those blockers remain, the required authorization sentence is not
issued.
