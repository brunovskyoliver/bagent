# Stage 8 structured synthesis experiment

Date: 2026-07-29

Baseline: `c88b84e`

Decision: **do not adopt**

Stage 9 remains unauthorized. `BAGENT_EVIDENCE_ORCHESTRATOR` and
`BAGENT_STRUCTURED_SYNTHESIS_EXPERIMENT` remain disabled by default and were
unset for the final state.

## Prototype

The experiment adds strict JSON Mail and web envelopes behind
`BAGENT_STRUCTURED_SYNTHESIS_EXPERIMENT=1`. The model receives opaque evidence
IDs and untrusted evidence content, but never receives or emits final citation
formatting. Bagent:

- rejects invalid JSON and unknown fields;
- validates every ID, Mail ordering and exact coverage, duplicate IDs,
  shortfall/conflict acknowledgements, claim numbers, claim grounding, and
  corroborated-source independence;
- permits exactly the existing single repair, now supplied with field paths;
- renders trusted Mail sender, subject, and date fields itself;
- appends only allowlisted final URLs beside validated web claims; and
- uses the existing deterministic renderer after an invalid repair.

The shared synthesis boundary gained `render_validated`, separating internal
model output from UI text. The legacy free-form contracts keep their identity
renderer. No evidence acquisition, grounding, conflict, shortfall,
independence, or allowlist rule was relaxed.

## Frozen-bundle comparison

The control is the final strict free-form campaign recorded in
`STAGE8_FINAL_REQUALIFICATION.md`: 42/90 grounded after repair and 48/90
deterministic terminal results.

The structured campaign used the identical frozen sequence of Mail, direct
web, and corroborated web bundles. Each repetition used a newly started BaseRT
process, Qwen3.6-35B-A3B loaded directly, context 4,096, KV4, max batch size 1,
and a 256-token cap. Each process handled 30 alternating requests and was then
stopped before the next repetition.

For each fresh process, the harness command was:

```sh
BAGENT_STRUCTURED_SYNTHESIS_EXPERIMENT=1 \
BAGENT_STAGE8_TRIAL_ONLY=1 \
BAGENT_STAGE8_SKIP_LOAD=1 \
BAGENT_STAGE8_REQUEST_COUNT=30 \
BAGENT_STAGE8_OUTPUT_CAP=256 \
cargo test -p bagentd stage8_live_frozen_bundle_matrix_and_performance \
  -- --ignored --nocapture
```

BaseRT was started directly with the cached 35B model and the configuration
above before each command. `STAGE8_METRIC` emitted one structural JSON record
per request without prompt, evidence, or model-response content.

| Process | Requests | Valid JSON/envelope initially | Valid IDs and coverage initially | Grounded initially | Successful repair | Grounded after repair | Deterministic fallback | Safe terminal | Metal/model errors |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 30 | 16 | 15 | 8 | 3 | 11 | 19 | 30 | 0 |
| 2 | 30 | 13 | 11 | 6 | 5 | 11 | 19 | 30 | 0 |
| 3 | 30 | 10 | 9 | 5 | 5 | 10 | 20 | 30 | 0 |
| **Total** | **90** | **39** | **35** | **19** | **13** | **32** | **58** | **90** | **0** |

“Valid JSON/envelope” means the strict typed shape parsed with no syntax,
unknown-field, or shape error. “Valid IDs and coverage” additionally excludes
missing coverage, invented/duplicate IDs, ordering defects, and insufficient
independent sources. “Grounded” additionally passes claim/body grounding,
number, conflict, and shortfall validation. Invalid initial or repair output
was held and never shown.

Initial completion latency was approximately 2,406–6,339 ms, with the typical
request near 2,500 ms. Repair latency was approximately 2,454–4,730 ms. These
are client-observed protocol times from the clean-process campaign, not an
interactive end-to-end SLA.

The frozen prompts are deterministic, but BaseRT sampling can vary. The three
independent process results above are the admission evidence; focused tests
cover deterministic validator and rendering behavior.

## Threshold decision

| Requirement | Result |
| --- | --- |
| 90/90 safe terminal results | Pass: 90/90 |
| Zero invalid output shown | Pass: all invalid output was held; 58 deterministic fallbacks |
| No Metal/model errors | Pass: 0/90 |
| Materially better than strict free-form 42/90 | Fail: 32/90 |
| At least 85/90 grounded after repair | Fail: 32/90 |

The adoption threshold fails. The structured envelope is therefore retained
only as a disabled experiment. The better-performing strict free-form contract
is retained solely for optional 35B wording polish. Deterministic rendering is
the canonical grounded answer, not merely a safety terminal; no model acceptance
rate is a correctness dependency.

## Verification and final state

Focused structured regressions cover valid rendering plus invented IDs,
duplicates/order, unsupported claims and numbers, Markdown/URL output, unknown
fields, acknowledgement requirements, citation allowlisting, and corroborated
source independence.

Because adoption failed, the conditional post-adoption release/signing matrix
was not triggered. Final live requalification is **not warranted** for this
candidate: it did not meet the synthesis-quality gate. A future prototype
should first improve JSON adherence and corroborated-web claim behavior on
frozen bundles, then repeat the same clean-process gate.

Final cleanup on 2026-07-29 showed port 8082 closed, the
`com.bagent.basert` LaunchAgent absent, no BaseRT or bagent daemon/app process,
and neither feature flag present in the environment. With no BaseRT process,
zero model weights were loaded.
