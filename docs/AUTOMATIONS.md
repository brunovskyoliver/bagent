# Scheduled Automations

Local, cron-like agent automations: a saved natural-language task runs through
the existing agentic tool loop at a scheduled time, entirely on-device. The
scheduler lives in the Rust daemon (a per-user launchd agent), so automations
run with the notch app closed.

## User flow

1. `⌥Space` opens the notch input.
2. Type `/automations` (suggested after typing `/`).
3. The list shows upcoming automations (name, next run, status glyph). `+`
   starts the editor.
4. Editor steps (each fits the notch; Escape steps back):
   **Úloha** (name + natural-language task) → **Kedy** (Dnes / Zajtra / Dátum,
   ±15 min time stepper, time zone shown) → **Opakovanie** (Raz, Každé N h,
   Denne, Po–Pia, Vybrané dni, Týždenne) → **Zhrnutie** → save.
   Success is claimed only after the daemon confirms persistence.
5. Selecting a list row opens the detail: schedule, next run, last status +
   concise result (tap the result to view the full text in the normal output
   surface), up to 3 recent runs, and Zapnúť/Vypnúť · Spustiť · Upraviť ·
   Vymazať (inline confirmation).

## Recurrence

Structured schedules only — no cron syntax anywhere:

| Kind | Wire (`schedule`) | Semantics |
|---|---|---|
| Run once | `{"kind":"once","at":"<UTC RFC3339>"}` | one-shot; `next_run_at` becomes NULL afterwards |
| Every N hours | `{"kind":"recurring","rule":{"type":"every_n_hours","hours":N}}` | fixed UTC duration from the last dispatch; 1 ≤ N ≤ 8784 |
| Daily | `…{"type":"daily","time":"HH:MM:SS"}` | local wall-clock in the automation's zone |
| Weekdays | `…{"type":"weekdays","time":…}` | Mon–Fri local |
| Selected weekdays | `…{"type":"selected_weekdays","days":["mon","fri"],"time":…}` | non-empty lowercase weekday set |
| Weekly | `…{"type":"weekly","day":"fri","time":…}` | one weekday per week |

Validation (backend-authoritative, `bagent-automations` crate): non-empty
name/prompt (80/4000 char caps), valid IANA zone, interval ≥ 1 hour,
non-empty/valid weekdays, a computable next occurrence.

## Time zones and DST

- Instants persist as UTC RFC3339; the user-selected IANA zone (e.g.
  `Europe/Bratislava`) persists alongside and recurrence is calculated in it —
  daily is **not** UTC+24 h.
- Ambiguous local times (fall-back): the **earlier** instant runs.
- Nonexistent local times (spring-forward): the **next valid local time**
  (e.g. 02:30 → 03:00 CEST).
- System zone changes don't affect stored automations (they keep their zone);
  wall-clock changes are picked up within one scheduler wake-up.

## Scheduler

Daemon-owned Tokio task (`crates/daemon/src/scheduler.rs`):

- Sleeps until `MIN(next_run_at)` over enabled automations; woken immediately
  via `AppState.automations_changed` on any create/edit/enable/disable/delete
  and on run completion.
- Sleep is chunked at 60 s because macOS monotonic timers do not advance
  during system sleep; each chunk re-reads the wall clock, so sleep/wake and
  clock jumps recalculate within a minute. Idle sleep is 1 h when nothing is
  scheduled. No short-interval polling.
- Claims are atomic: `INSERT … WHERE NOT EXISTS (running row)` in a single
  statement; the schedule advances at claim time, and no DB lock is held while
  the agent executes.
- Bounded concurrency: at most **2** automations execute at once (semaphore
  shared with run-now).

### Missed runs / catch-up

- At most **one** catch-up per automation, and only when the missed occurrence
  is ≤ 24 h old; the run is flagged `is_catch_up` when > 5 min late.
- Older occurrences are recorded as `skipped_stale` (never executed) and the
  schedule advances to the next future occurrence — no replay storms.

### Overlap

A due occurrence while the same automation is running records a
`skipped_overlap` run and advances. Run-now respects the same claim.

### Restart / shutdown

On daemon startup, `running` rows become terminal `abandoned` (mirrored onto
the automation, audited) and stale undecided approvals are **denied** —
approvals never revive across restarts. A run in flight at shutdown is
recovered this way on the next start.

### Failures

Failed runs persist a short redacted reason and the automation simply waits
for its next occurrence — no immediate retry, no automatic disabling.
Lifecycle during an active run is finish-then-apply: edits apply from the next
occurrence, disable prevents the next claim, delete returns 409 until the run
finishes.

## Execution safety

Every scheduled/run-now execution goes through the shared execution service
(`crates/daemon/src/agent_exec.rs`):

- Trusted `AutomationExecutionContext` (ids, run id, schedule/start instants,
  catch-up flag, unattended, zone) is rendered as a **system layer** — never
  only in the user prompt.
- Read-only tools follow the existing rules engine (with the real serialized
  arguments). Side-effecting tools escalate `auto → ask` when unattended:
  a fresh pending approval per action, 60 s auto-deny, one-shot permission.
- Unknown/unmapped tools fail closed in unattended runs.
- Approvals carry provenance (`pending_approvals.origin_json`); the notch
  modal opens automatically, preempts every surface, and shows
  "Automatizácia · <name>".
- Mail/web/tool results are declared untrusted (prompt-injection defense) and
  the stored prompt is explicitly not a policy override.
- Denied/timed-out approvals mark the run `partial`; the agent continues with
  remaining read-only work where possible.

## Persistence & retention

`V13__automations.sql`: `automations` (typed schedule JSON, UTC instants,
IANA zone, mirror of last run) with a due index, and `automation_runs`
(status, concise ≤ 2000-char redacted summary, catch-up/manual flags).
The newest **50** runs per automation are retained; each cleanup writes an
`automation_retention_cleanup` audit row. `audit_entries` is append-only and
never pruned. Audit/event payloads never contain prompts, connector payloads,
stack traces, or model internals (pinned by test).

## API

Bearer-authenticated local routes:

```
GET    /automations                     list (due-ordered)
POST   /automations                     create  → 201 | 400 validation
GET    /automations/{id}                → 404 when missing
PATCH  /automations/{id}                partial update, revalidates
DELETE /automations/{id}                → 409 while a run is active
POST   /automations/{id}/enable|disable recomputes next occurrence on enable
POST   /automations/{id}/run-now        → 202 + claimed run | 409 disabled/active
GET    /automations/{id}/runs?limit=N   recent runs (≤ 50)
```

## SSE events

`GET /events` streams concise typed envelopes (ids/status only — clients
refetch authoritative records): `automation_created/updated/deleted/enabled/
disabled`, `automation_run_started/finished`, `automation_next_run_changed`,
and `approval_requested` (with origin). The Swift app keeps one reconnecting
subscription and refetches `/approvals/pending` at start and on reconnect.

## Audit events

`automation_create/update/enable/disable/delete`, `automation_run_manual`,
`automation_run_start/finish`, `automation_run_catch_up`,
`automation_run_skipped_stale`, `automation_run_overlap_skip`,
`automation_runs_abandoned`, `approvals_denied_on_restart`,
`automation_retention_cleanup`, plus the existing per-tool-call and approval
rows.

## Testing

- `cargo test -p bagent-automations` — recurrence, DST transitions,
  validation, catch-up window (deterministic, injected instants).
- `cargo test -p bagentd` — repository, claims, scheduler passes with an
  injected clock (due/exhaust, catch-up, stale skip, overlap, disabled,
  restart recovery, DST advancement, lifecycle-during-run, redaction).
- `apps/macos: swift test` — wire models, editor flow, slash commands,
  selection/back navigation, geometry ceilings.

Live end-to-end runs require Ollama and are exercised manually (see
Limitations).

## Known limitations

- Manual scenarios not yet exercised end to end: real Mac sleep/wake across a
  due instant, wall-clock/timezone changes on a live system, and live
  approval paths for email/Odoo/shell/file writes from an unattended run
  (the gating logic is unit-tested; the connectors' write paths are unchanged).
- Every-N-hours anchors to dispatch time, not a fixed phase (an interval
  drifts by dispatch latency).
- The scheduler wakes at most 60 s late after system sleep; sub-minute
  scheduling precision is not a goal.
- Automation runs use the default chat model; per-automation model selection
  is not implemented.
