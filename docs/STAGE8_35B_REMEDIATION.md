# Stage 8 35B model-admission remediation

Date: 2026-07-29

Base commit: `0131bca7889e5b3c27ef313ea57747b22d9badc6`

Recommendation: **admit Qwen3.6-35B-A3B conditionally** on this 32 GiB M5.

Stage 9 remains unauthorized. `BAGENT_EVIDENCE_ORCHESTRATOR` remains disabled
unless explicitly enabled. Prompts, evidence, citations, and model text were
held only in memory; this record contains structural measurements only.

## Finding

The Stage 8 result combined one real Metal failure with 27 rapid requests made
after that failure. Those later requests were not independent model-admission
failures. BaseRT's HTTP response did not expose the Metal failure: it returned
HTTP 200 with a one-character completion.

The earliest retained 35B failure was:

- BaseRT process configuration: max context 16,384, KV4, max batch size 1,
  output cap 512.
- First-failure request index: 1 in that 35B residency.
- Prompt: 261 provider-reported tokens; private content was not retained.
- Exact Metal category: `metal_oom`.
- Exact Metal error:
  `Insufficient Memory
  (00000008:kIOGPUCommandBufferCallbackErrorOutOfMemory) (code 8)`.
- BaseRT phase: decode at position 261.
- HTTP result: 200, one completion character, 878 ms, `stop`.
- The immediately following request failed during prefill at chunk 1/1 and
  returned zero characters. That request is evidence of poisoned-process
  behavior, not an independent trial.
- Exact system pressure, swap, and BaseRT RSS were not sampled at the request
  boundary by the original harness. The campaign baseline was approximately
  9,636 MiB swap used; the omission is retained rather than reconstructed.

In the remediation campaign, `/v1/models` reported zero loaded weights and
BaseRT RSS was approximately 5 MiB, but system headroom was only 15%. Stopping
that BaseRT process raised headroom to 77% without changing the workload. This
proves that RSS and the model list do not account for retained Metal/wired
allocations.

## Controlled clean-process matrix

Every row below used a dedicated BaseRT process, max batch size 1, one frozen
Mail synthesis request, and no overlapping daemon, 4B lease, or legacy
completion. A trial stopped at the first Metal signature.

| Max context | KV bits | Cap | Start/lifecycle | Pre-request pressure | Swap used MiB | BaseRT RSS KiB | Latency ms | Result |
| ---: | ---: | ---: | --- | ---: | ---: | ---: | ---: | --- |
| 4,096 | 4 | 256 | Direct with 35B | 78% | 9,410 | 864,096 | 8,503 | Grounded; no Metal |
| 8,192 | 4 | 256 | Direct with 35B | 82% | 10,317 | 2,037,440 | 8,299 | Grounded; no Metal |
| 16,384 | 4 | 256 | Direct with 35B | 75% | 10,227 | 1,947,456 | 6,816 | Grounded; no Metal |
| 4,096 | 8 | 256 | Direct with 35B | 81% | 9,899 | 1,734,752 | 6,853 | Grounded; no Metal |
| 4,096 | 16 | 256 | Direct with 35B | 79% | 9,883 | 1,735,664 | 6,172 | Grounded; no Metal |
| 4,096 | 4 | 512 | Direct with 35B | 79% | 9,883 | 1,734,992 | 6,548 | Grounded; no Metal |

RSS was sampled at different points in BaseRT's paged-weight residency and is
not a reliable admission signal. Context 4,096 / KV4 / cap 256 is selected
because it has the smallest up-front cache and generation allocation; the
single-request latency differences are not treated as statistically
significant.

### Lifecycle comparison

| Lifecycle | Result |
| --- | --- |
| Start directly with 35B | Grounded in 8,503 ms |
| Start empty, load 35B through API | Grounded in 6,502 ms |
| Load 4B, unload, load 35B | Grounded in 7,595 ms; no overlapping model |
| Unload and reload 35B in one process | Residency 1 grounded in 7,607 ms. API unload left only 16% headroom; residency 2 hit `metal_oom` during decode at position 400 and returned one character in 1,184 ms. It is an unsafe lifecycle result, not an independent admission failure. |
| Restart between 35B residencies | Three independent 30-request repetitions passed without Metal or model error |

Stable: clean process, context 4,096, KV4, cap 256, batch size 1, and a process
restart between residencies.

Unstable: reloading 35B after API unload in the same process when the previous
residency left unsafe headroom. API unload and low RSS do not establish a clean
Metal state.

## Independent reliability

Each repetition used a new BaseRT process, direct 35B startup, context 4,096,
KV4, cap 256, batch size 1, and the repeating sequence Mail, direct web,
corroborated web.

| Repetition | Requests | Metal/model errors | Initial grounded | Grounded after one repair | Initial pressure | Final pressure | Initial/final swap MiB |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 30 | 0 | 26 | 27 | 78% | 17% | 8,489 / 9,764 |
| 2 | 30 | 0 | 27 | 29 | 80% | 14% | 9,756 / 9,432 |
| 3 | 30 | 0 | 22 | 24 | 78% | 13% | 9,424 / 9,280 |
| **Total** | **90** | **0** | **75** | **80** | — | — | — |

The model-admission threshold passes: every clean process completed 30
consecutive mixed requests without Metal or model error, in three independent
repetitions.

Across 105 BaseRT completions including bounded repairs, protocol latency was:
n=105, minimum 2,627 ms, p50 3,079 ms, p90 6,072 ms, p95 6,156 ms, maximum
7,128 ms. Client-observed initial-request latency ranged from 2,628 to
7,497 ms. The 20-second synthesis and 45-second cold-ready deadlines remain in
place.

Initial BaseRT RSS across the three repetitions was 1,031,888–1,523,680 KiB
(median 1,036,432 KiB). Post-unload RSS was 163,952–331,584 KiB (median
167,312 KiB), while final system headroom was still only 13–17%. This
distribution is why retirement now restarts the process.

## Grounding breakdown

The 90 clean-process 35B requests produced:

| Category | Initial failures | Still invalid after repair |
| --- | ---: | ---: |
| Empty | 0 | 0 |
| Truncated | 0 | 0 |
| Malformed | 0 | 0 |
| Missing coverage | 0 | 0 |
| Missing citation | 3 | 3 |
| Unsupported claim | 12 | 7 |
| Internal metadata | 0 | 0 |
| Transport/model error | 0 | 0 |

The same frozen bundles against clean-process 4B produced 0/3 grounded
answers after repair: one Mail `missing_coverage` and two web
`unsupported_claim` failures. No output was truncated at cap 256.

The validators are compatible with the prompt shape: clean 35B passed 75/90
initially and accepted five corrected repairs without a parser or policy
change. The remaining failures contain unsupported claims or omit required
citations. They are model-output failures, not safe formatting-only
mismatches. Grounding, coverage, conflict, shortfall, and citation requirements
were not weakened. A structured answer envelope was not adopted: BaseRT's
generic JSON mode does not by itself prove evidence coverage or claim
grounding, while the constrained text contract is already machine validated.

## Memory admission

Before loading 35B, bagent runs macOS `memory_pressure -Q` and fails closed if
the report cannot be parsed. Admission requires both:

1. system-wide free-memory headroom at least 25%; and
2. estimated available headroom at least 8 GiB, computed from physical memory
   and the reported free percentage.

On this 32 GiB machine both conditions are required; 25% corresponds to 8 GiB.
At 14–16% the preferred model is not loaded. The flow goes directly to the
single bounded 4B fallback, whose output is validated, or to deterministic
rendering. Swap and pressure remain diagnostic fields, but RSS and
`/v1/models` are not admission signals.

This threshold is a load-admission check, not a residency-eviction threshold.
Once a cleanly admitted 35B residency is serving warm requests, the model's own
allocation is expected to reduce free headroom; reevaluating the load threshold
at that point would incorrectly retire a healthy process. Normal idle expiry
still unloads and restarts the runtime.

## Poisoned-runtime recovery

The BaseRT connector checkpoints operational-log file identity and length
around both model loads and synthesis requests, including HTTP errors. It scans
the newest 256 KiB of appended data, starts from the new file after rotation,
and gives error/short-output/timeout paths a bounded 250 ms flush window. It
normalizes:

- Metal out-of-memory to `metal_oom`;
- Metal device loss/removal to `metal_device`;
- other Metal command-buffer failure to `metal_command_buffer`.

After any of these:

1. the shared runtime is marked poisoned;
2. no repair, preferred, unload/reload, or fallback request is sent to that
   process;
3. bagent restarts the managed `com.bagent.basert` LaunchAgent;
4. a changed launchd PID, health, and zero loaded weights are confirmed;
5. 4B is loaded for one bounded fallback;
6. if restart fails, fallback is suppressed and deterministic rendering is
   used.

A load or completion timeout is also an unhealthy process boundary even when no
Metal line arrives within the flush window: cancelling the HTTP future does not
prove the server-side operation ended. Bagent therefore restarts before fallback
instead of risking a 35B/4B overlap. A Metal failure or timeout during the
bounded fallback marks the process unhealthy for the next request and renders
deterministically.

Normal 35B retirement also unloads and restarts BaseRT. This makes a clean
process—not API unload—the residency boundary. Deterministic tests cover
classification, typed fault propagation, poisoned-state request suppression,
load-time and delayed timeout poisoning, restart-before-fallback,
restart-failure suppression, repair-time poisoning, concurrent leases, idle
retirement, load-admission rejection, and continued warm service after
admission.

## Decision and boundaries

35B is conditionally admitted only with the 4,096/KV4/batch-1 configuration,
the 256-token synthesis cap, the preflight rule, held-output validation, one
bounded repair, poisoned-process restart, and deterministic rendering. It is
not admitted when memory preflight fails. 4B remains a bounded fallback rather
than a grounding-quality substitute.

This remediation does not authorize Stage 9 or default evidence routing.
