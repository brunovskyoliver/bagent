# Stage 6 synthesis acceptance record

Date: 2026-07-29

Target: signed `apps/macos/bagent.app` on Apple M5, 32 GiB unified memory

Runtime: BaseRT 0.1.7

No prompts, Mail content, fetched passages, model output, connector arguments, or
raw connector identifiers were retained in this record.

## Automated verification

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --no-fail-fast`: passed.
  - `bagentd`: 158 passed, 2 explicitly ignored live tests.
  - BaseRT protocol: 8 passed.
  - Connector suites: Apple Mail 19 passed/1 ignored; Codex 26/1;
    filesystem 30/0; Odoo 12/3; WhatsApp 13/0.
- `swift test` in `apps/macos`: 34 passed.
- `cargo clippy --workspace --all-targets`: passed with pre-existing warnings.
- `git diff --check`: passed.
- `codesign --verify --deep --strict apps/macos/bagent.app`: passed.

## Runtime compatibility and residency

The resident BaseRT service registered the exact model IDs
`basecompute/Qwen3.6-35B-A3B` and
`basecompute/Qwen3-4B-Instruct-2507`. It started with zero loaded model
weights. No interactive request selected Qwen3 8B.

The final clean state had the BaseRT service and daemon running, the evidence
feature flag unset, no pressure-simulation environment variable, and zero
loaded weights.

## Signed-app live matrix

Durations below are backend phase durations. Model output and evidence content
were intentionally not captured.

| Check | Result |
| --- | --- |
| Cold 35B Mail | Cold load completed in 640 ms, but synthesis hit a real Metal memory failure after 237 ms. Exactly one 4B fallback completed in 20,842 ms and failed strict output validation, so deterministic Mail rendering was used. Wall time was 27,376 ms. |
| Warm 35B Mail | Eight consecutive requests reached warm 35B before memory pressure forced fallback. Primary synthesis durations were 6,190, 6,077, 2,860, 1,344, 1,296, 6,203, 6,534, and 1,563 ms. A final post-review smoke synthesized in 1,528 ms; strict coverage validation rejected it, its one repair failed transport in 14 ms, and deterministic rendering completed the request. |
| Direct-page web | The final post-review build began with externally loaded 4B, unloaded it, cold-loaded 35B in 473 ms, synthesized in 3,550 ms, passed validation, and left only 35B resident. An earlier run synthesized in 988 ms but safely rendered deterministically after citation validation and repair transport failure. |
| Corroborated web | Warm 35B synthesized in 3,438 ms and passed grounding validation. Wall time, including acquisition, was 5,634 ms. |
| Forced 35B failure | An intentionally missing model path failed in 8 ms with normalized reason `model_unavailable`. Exactly one 4B fallback completed in 20,691 ms. Its invalid output was rejected and deterministic Mail rendering was used. Wall time was 26,798 ms. |
| Simulated memory pressure | A direct-page request loaded 35B in 498 ms. The pressure signal then unloaded it in 11,876 ms, within the 30-second maintenance interval. |

The eight-sample warm distribution has nearest-rank p50 2,860 ms and p95
6,534 ms. This sample is too small, and the acceptance rate too unstable, to
establish the 8-second/15-second targets as an SLA. Only one of the eight warm
Mail outputs passed strict validation directly; the others safely reached
repair or deterministic rendering.

The exact final-build cold retry sampled a BaseRT peak RSS of 1,801,216 KiB.
The highest sampled BaseRT RSS during the signed Stage 6 sequence was
2,354,512 KiB. Sustained trials also increased swap use and eventually caused
real Metal out-of-memory failures, so memory admission remains a rollout
constraint.

## Stage 7 blockers and follow-up

- The macOS client does not yet decode and render the new evidence phase
  events. Stage 7 owns that UI work.
- Web generation frequently omits the required adjacent Markdown citation,
  and Mail generation frequently emits unsupported identifiers or claims.
  Validation and deterministic rendering prevent those outputs from reaching
  the user, but prompt/model acceptance needs improvement.
- Repair requests sometimes fail in BaseRT under sustained 35B memory load.
- The warm latency sample is directional only. A larger acceptance-weighted
  distribution is required before enabling the redesigned route by default.
