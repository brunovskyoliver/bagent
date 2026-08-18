# BaseRT residency transitions under chat and automation load

Research ticket: [Measure BaseRT residency transitions under chat and automation load](https://github.com/brunovskyoliver/bagent/issues/17)
Map: [Rebuild the bagent notch UX around model residency and automation sessions](https://github.com/brunovskyoliver/bagent/issues/15)
Observed: 2026-07-30 on Apple M5, 32 GiB unified memory, macOS 26.5.2
Code snapshot: `05c5b8ff5901db0139c8776ef5371d4aae59871b`

## Decision

Adopt one daemon-owned **Model Residency coordinator** as the only authority
allowed to load, lease, retire, or restart bagent's BaseRT process. Treat a
changed-PID clean process, not API unload or low RSS, as the boundary before
every new 35B residency and after every Metal/device/command-buffer fault or
indeterminate lifecycle/completion timeout.

The coordinator should enforce:

1. 4B and 35B are mutually exclusive on port 8082. Model discovery is not
   residency, and `loaded=true` is transport readiness rather than proof of a
   usable completion.
2. Every Conversation Turn and Automation Run owns a non-preemptible completion
   lease. No load, unload, or restart may reach BaseRT while any lease is active.
3. At most one lifecycle transition is in flight. Same-model demand joins it;
   different-model demand queues until it settles and all leases are idle.
4. Queue priority is foreground demand, then Automation Runs FIFO, then
   speculative input preload. Work already executing is never preempted.
5. Input preload is a cancellable, lowest-priority 4B demand, not a lease. If a
   higher-priority different-model request arrives, do not cancel an in-flight
   BaseRT HTTP load: let it settle and switch while idle, or poison and restart
   on its deadline.
6. Admit a cold 35B load only with both at least 25% free memory and at least
   8 GiB estimated available. Do not reapply that load gate to an already
   healthy warm residency. Critical pressure requests retirement after active
   leases drain.
7. Normal 35B retirement is unload followed by changed-PID restart, health, and
   zero-loaded-weight proof. A 4B API unload may be used for ordinary retirement
   only when measured headroom returns; restart before a later 35B admission.
8. A client cancellation or timeout is indeterminate server state. Stop or
   poison that process immediately; only a healthy changed PID with zero loaded
   weights may receive one bounded next request.
9. BaseRT remains a long-lived service while weights are retireable. When the
   initial topology is stopped, bounded work must restore the app, daemon,
   BaseRT job, listeners, and weights to stopped.

This policy makes task correctness independent of residency. Canonical
deterministic rendering remains available when 35B is inadmissible or unusable;
4B must not silently become a grounding-quality substitute.

## Current code facts

### Service and discovery

- The Swift launcher reserves the independent service on port 8080 and owns
  only port 8082. Its generated job is fixed at context 4,096, KV4,
  `--max-batch-size 1`, 300 s server request timeout, and 20-minute BaseRT idle
  timeout. It registers the cached 4B and 35B packages without initially loading
  either. See [DaemonLauncher.swift](../apps/macos/Sources/bagent/DaemonLauncher.swift).
- Opening input calls only `presentInputOnly()`; there is no model-preload call
  in the current Swift path. `loadModels()` lists models for settings and does
  not load weights. See
  [NotchWindowController.swift](../apps/macos/Sources/bagent/NotchWindowController.swift)
  and [ChatViewModel.swift](../apps/macos/Sources/bagent/ChatViewModel.swift).
- The daemon's `/models` route lists BaseRT models. The UI-facing response drops
  BaseRT's `loaded` flag, so Swift cannot currently distinguish discovery from
  residency. See [main.rs](../crates/daemon/src/main.rs) and
  [DaemonClient.swift](../apps/macos/Sources/bagent/DaemonClient.swift).

### Lifecycle ownership

- `BaseRtClient` clones share one `RwLock`: completions take a read guard and
  lifecycle calls take a write guard. This blocks load/unload during an active
  completion only inside that shared client instance; it is not a
  cross-process or system-wide coordinator. See
  [the BaseRT connector](../crates/connectors/basert/src/lib.rs).
- `ModelRuntimeManager` separately owns 35B synthesis leases, load
  single-flight, memory admission, idle retirement, poisoning, and restart. It
  does not own ordinary 4B agent-loop requests. See
  [synthesis.rs](../crates/daemon/src/evidence/synthesis.rs).
- Restart recovery requires the managed endpoint, a changed launchd PID,
  healthy service, and zero loaded models. See
  [the research-snapshot runtime control](https://github.com/brunovskyoliver/bagent/blob/dc37a0b07a12384bb78d066769da361c7c140713/crates/daemon/src/evidence/runtime_control.rs).
- `ensure_fallback()` contains 35B-to-4B lease-safe switching but has no
  production caller; only a test invokes it. Current synthesis failure returns
  the canonical answer without loading 4B. The glossary's “one bounded backup
  model” wording and the executable path therefore need an explicit decision,
  not an inferred promise.

### Chat and Automation Runs

- Chat and Automation Runs use the same `run_agent_loop` and shared
  `BaseRtClient`. Two Automation Runs may execute concurrently because
  `MAX_CONCURRENT_RUNS` is 2. Foreground chat does not acquire those permits, so
  chat plus two runs can submit three concurrent completions to a batch-1
  runtime. See [automations policy](../crates/automations/src/policy.rs),
  [automation execution](../crates/daemon/src/automations_api.rs), and
  [chat execution](../crates/daemon/src/main.rs).
- The semaphore queues excess Automation Runs before model work; it is not a
  model request scheduler. There is no foreground-vs-automation priority,
  queue position, cancellation contract, or residency event schema.
- One active run per automation is atomically claimed, while different
  automations may overlap. Scheduler restart marks in-flight runs abandoned.
  There is no user cancellation endpoint for a running Automation Run. See
  [scheduler.rs](../crates/daemon/src/scheduler.rs).

## Deterministic automated evidence

Current-tree connector tests passed **13/13**, including typed discovery/load/
readiness/unload, active-completion exclusion for both load and unload,
bounded-completion empty/truncation handling, Metal log classification, and
suppression of a second request after a poisoned-runtime signature:

```text
cargo test -p basert-connector --test protocol
```

The focused daemon lifecycle and automation suites could not run on this
snapshot because the existing `bagentd` test target does not compile:
`main.rs` has four unresolved references to `TavilyConfiguration` and
`TavilyConfigurationStatus`. No product-code repair was made in this research
ticket.

The retained deterministic lifecycle tests specify:

- warm 35B reuse performs one load;
- fallback switching waits for every concurrent preferred lease;
- 20-minute 35B idle expiry unloads and restarts;
- memory pressure rejects a cold 35B load but does not evict a healthy warm
  residency;
- load/completion timeouts preserve canonical output;
- Metal OOM, device, and command-buffer failures poison the runtime;
- restart occurs before any admitted fallback, and restart failure suppresses
  fallback.

See [synthesis lifecycle tests](../crates/daemon/src/evidence/synthesis.rs) and
[Stage 8 35B remediation](STAGE8_35B_REMEDIATION.md). Those retained tests are
strong design evidence, but they are not a current-tree pass while the daemon
test target is uncompilable.

## Controlled live measurements

The initial and final topology was app stopped, daemon stopped, managed BaseRT
stopped, port 8082 closed, and no loaded weights. Port 8080 remained untouched.
No production automation request or database write was made.

### Transition timings

| Transition | Result |
|---|---:|
| Clean BaseRT service start to health | 81 ms |
| 4B cold API load/readiness | 1,673 ms |
| 4B same-PID completions | 552 ms, then 383 ms |
| 4B API unload | 68 ms |
| Clean changed-PID restart after 4B | PID `82154` to `86158` |
| 35B cold API load/readiness | 586 ms |
| First 35B completion request | 5,476 ms, HTTP 200 |
| 35B API unload | 80 ms |
| Clean changed-PID restart after 35B | PID `86158` to `93381` |
| Second clean 4B load | 2,422 ms |

The 35B `loaded=true` transition was lazy: major wired allocation appeared
during completion, not the API load. A 256-token bounded probe took 4.18 s,
used the cap for 778 reasoning characters, and returned zero user-visible
content. The runtime was transport-ready, but this trial did not establish a
usable 35B completion or warm user-visible latency.

### Memory release

| State | BaseRT RSS | Free memory | Wired pages | Loaded registry |
|---|---:|---:|---:|---|
| Clean service | 6.6 MiB | 86% | about 227k | none |
| 4B after completion | 2.72 GiB | 72% after load | about 517k after load | 4B |
| 4B unload, immediate | 87 MiB | 86% | about 227k | none |
| 35B after completion | 785 MiB | 21% | about 1.51M | 35B |
| 35B unload, +2 s | 384 MiB | 25% | about 1.46M | none |
| Changed-PID restart, +1 s | 6.5 MiB | 85% | about 226k | none |

The 4B sample released RSS, wired allocation, and headroom promptly through API
unload. The 35B sample did not: `/v1/models` reported zero loaded weights and
RSS fell, while wired allocation and pressure remained. Only the changed-PID
restart restored the clean baseline. Swap remained 2,037.5 MiB throughout this
bounded campaign.

### Queueing and cancellation

Three simultaneous 4B callers against configured batch size 1 completed at
1.635 s, 3.113 s, and 4.570 s. The near-linear spacing is consistent with
serialized BaseRT queueing; it is not evidence of origin-aware fairness.

A separate client cancellation ended locally after 106 ms with no HTTP
response. Server completion state was indeterminate, so the process was
immediately booted out without a same-PID follow-up. Port 8082 closed and the
old PID exited. This confirms why client cancellation cannot release a lease or
authorize a model switch until the coordinator observes completion or crosses a
changed-PID boundary.

No Metal/device fault was induced. Prior controlled live evidence found that
API-unload/reload of 35B in one PID can produce Metal OOM while three
changed-process 30-request campaigns completed without Metal/model errors. See
[Stage 8 35B remediation](STAGE8_35B_REMEDIATION.md).

## Inferred conclusions

- BaseRT's model registry, RSS, and service health are necessary observations
  but are insufficient residency truth. The coordinator must retain its own
  target, phase, PID generation, leases, transition deadline, and poison state.
- Batch size 1 turns chat plus two Automation Runs into an implicit server
  queue. UI state cannot be derived from `isThinking`; each caller needs a
  stable queued/running/cancelling/terminal session state.
- Load cancellation is a lifecycle fault boundary, not a harmless optimization.
  Speculative input preload must never directly own a BaseRT request.
- Active-completion safety currently depends on all calls sharing one Rust
  client. Moving lifecycle control into Swift, a second daemon component, or a
  test harness would bypass that guarantee unless every call goes through the
  coordinator.
- The 35B memory gate is admission-only. A healthy residency naturally pushes
  free memory below 25%; treating that as immediate eviction would thrash.

## Unverified or blocked criteria

- A current-tree isolated end-to-end overlap of foreground chat plus two
  Automation Runs is blocked by the daemon test-target compile failure. The
  production automation database was not used.
- Origin-aware fairness, queue cancellation, and Automation Run cancellation
  are not implemented, so they cannot be measured as product contracts.
- Active completion versus live lifecycle mutation is covered by deterministic
  connector tests, not by a live daemon overlap.
- No live Metal/device/command-buffer fault was intentionally induced. Recovery
  is supported by prior controlled evidence and deterministic tests.
- No live 35B completion in this bounded campaign produced user-visible content;
  35B transport readiness and memory behavior were measured, not synthesis
  quality.
- BaseRT's own 20-minute idle timeout was not waited out live. Daemon
  20-minute retirement is deterministic code/test evidence.
- Different-model contention between input preload and automation is
  hypothetical because input preload does not yet exist.

## Questions for the unified state-machine ticket

[Design the unified background-work state machine and event contract](https://github.com/brunovskyoliver/bagent/issues/23)
must settle these exact questions:

1. What is the single enum/state graph for service generation, desired model,
   load/readiness, queued demand, active leases, retirement, poison, restart,
   and terminal failure?
2. Which daemon API replaces Swift's discovery-only `/models` view, and which
   privacy-safe events carry PID generation, model class, phase, queue position,
   lease count, deadline, and normalized failure?
3. Does canonical synthesis have any production 4B fallback, or is the
   currently unused `ensure_fallback()` path removed and the glossary narrowed?
4. What bounded priority/fairness rule governs foreground chat and two
   Automation Runs on batch size 1, including starvation limits and deadline
   accounting while queued?
5. How are client disconnect, explicit Automation Run cancellation, daemon
   shutdown, and speculative-preload cancellation distinguished, and which
   require a changed-PID boundary?
6. When input preload and an Automation Run request different models, does a
   completed but unused preload retire immediately or remain as a short
   opportunistic 4B residency?
7. Which pressure signal requests graceful retirement of a warm model, distinct
   from the existing cold 35B admission gate?
8. How are restored runtime topology and protected-state invariants made
   acceptance-testable without the production database?

These questions belong to the existing unified state-machine ticket; this
research does not justify another implementation ticket or dependency change.

## Restoration and privacy

Final inspection matched the initial state: app, daemon, BaseRT, port 8082,
loaded weights, and both bagent launchd jobs were stopped. Port 8080's unrelated
listener was unchanged. All protected table row counts and hashes, `rules.yaml`,
attachments, daemon-token metadata, and Keychain metadata matched the pre-live
snapshot. Memory returned to 85% free and about 226k wired pages.

Only model classes, timings, PIDs, memory figures, structural response lengths,
and normalized failures were retained. No prompt, model output, credential,
connector identifier, Mail content, evidence passage, or raw provider error is
present in this asset.
