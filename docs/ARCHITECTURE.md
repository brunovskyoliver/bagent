# Architecture

## System Overview

```
┌─────────────────────────────────────────────────────────────┐
│  macOS Menu Bar / Notch                                      │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  SwiftUI / AppKit Frontend  (apps/macos/)            │   │
│  │  NSStatusItem · NSPanel · Global Hotkey              │   │
│  │  Approval Modals · Settings · Audit Viewer           │   │
│  └───────────────────┬──────────────────────────────────┘   │
│                      │ HTTP / SSE (127.0.0.1:<port>)        │
│  ┌───────────────────▼──────────────────────────────────┐   │
│  │  Rust Daemon  (crates/daemon/)                       │   │
│  │  axum · tokio · Agent Runtime · Model Router         │   │
│  │  Rules Engine · Memory · Audit Log · SQLite          │   │
│  └──┬──────────┬──────────┬──────────────┬─────────────┘   │
│     │          │          │              │                   │
│  BaseRT     Codex      Connectors    SQLite DB              │
│  :8082       CLI      (Mail/Notes    (bagent.db + FTS5)      │
│  (local)  subprocess   Odoo/Shell                            │
│                        Screen)                               │
└─────────────────────────────────────────────────────────────┘
```

---

## Frontend — SwiftUI + AppKit

### Notch Window

- `NSStatusItem` in `.statusBar(length: NSStatusItem.squareLength)` for menu-bar icon fallback on non-notch Macs.
- Primary notch UI: `NSPanel` with `styleMask: [.borderless, .nonactivatingPanel]`, `level: .mainMenu + 1`, positioned to cover the notch area (`NSScreen.main?.auxiliaryTopLeftArea` / `auxiliaryTopRightArea` or hardcoded safe insets per device model).
- Panel autohides unless pinned; triggered by global hotkey or status item click.
- Animation: slide-down expand from notch pill (150ms ease-out).

### Global Hotkey

- `⌥Space` default; configurable. Registered via `CGEventTap` or `HotKey` library wrapping Carbon `RegisterEventHotKey`.
- Sandboxed entitlement `com.apple.security.temporary-exception.mach-lookup.global-name` may be needed; test early.

### Chat View

- The daemon emits semantic SSE events for answer deltas, activity lifecycle,
  retained sources, approvals, errors, and completion.
- `AdaptiveStreamPresenter` separates BaseRT transport chunks from the displayed
  prefix, revealing complete words at an adaptive cadence with bounded catch-up.
- Each assistant message owns its canonical text, displayed prefix, activity
  transcript, and validated HTTP(S) sources for the current in-memory session.

### Approval Modals

- `NSAlert` subclass or SwiftUI `Sheet` presented modally over all spaces.
- Fields: `action_description`, `tool`, `args_preview`, `dry_run_diff` (optional), `[Allow] [Deny] [Edit]`.
- Response POSTed back to daemon `/approvals/{id}`.
- Timeout: 60 s default (configurable); auto-deny on timeout.

### macOS Permissions Required

| Permission | Purpose | Trigger |
|---|---|---|
| Accessibility | Selected text, active app, UI inspection | Phase 7 |
| Automation (Mail, Notes) | AppleScript/JXA bridge | Phase 4 |
| Screen Recording | ScreenCaptureKit frames | Phase 7 |
| Full Disk Access | Mail `.emlx` + `Envelope Index`, Notes SQLite | Phase 4 |
| Network | Odoo API, BaseRT, optional cloud | Phase 3 |
| Keychain | API keys, daemon bearer token | Phase 2 |

> Request permissions lazily at first use; explain reason in native `NSAlert` before system prompt appears.

### AppleScript / JXA Bridge

- Invoke via `Process` → `osascript -l JavaScript` or `NSAppleScript.executeAndReturnError`.
- JXA preferred for structured output (JSON-serializable).
- All script text is hardcoded in the app bundle, never generated at runtime from LLM output.

---

## Rust Backend Daemon

### Stack

- `tokio` async runtime (multi-thread scheduler).
- `axum` HTTP server bound to `127.0.0.1:0` (OS-assigned port); port written to `~/Library/Application Support/bagent/daemon.port`.
- Bearer token at `~/Library/Application Support/bagent/token` (32-byte hex, generated on first run, stored in Keychain under `bagent.daemon.token`).
- `rusqlite` + `r2d2` for SQLite connection pool; migrations via `refinery`.
- `serde` / `serde_json` for all wire formats.
- `tracing` + `tracing-subscriber` for structured logs.

### Daemon residency (launchd)

The daemon is a per-user launchd agent, not a child of the notch app, so
scheduled automations keep running after the app exits.

- On every app launch, `DaemonLauncher` writes
  `~/Library/LaunchAgents/com.bagent.daemon.plist` (current binary path +
  model env from UserDefaults) and does `launchctl bootout` + `bootstrap` —
  a deterministic restart into the current binary. This covers app relaunch,
  packaging, and upgrades.
- App exit does **not** stop the daemon.
- Crashes are restarted by launchd (`KeepAlive.SuccessfulExit=false`); a
  clean exit or explicit `launchctl bootout gui/$UID/com.bagent.daemon`
  (`DaemonLauncher.shutdownDaemon()`) stays down.
- Discovery contract unchanged: the daemon writes `daemon.port`,
  `daemon.token`, and `daemon.pid` in Application Support regardless of who
  started it. Daemon logs go to `~/Library/Logs/bagent/daemon.log`.
- Migration: a pre-launchd daemon recorded in `daemon.pid` that launchd does
  not own is SIGTERMed once at app launch.
- The app also owns `com.bagent.basert`, serving Qwen3-4B on loopback port
  8082. It survives normal UI exit and is stopped by explicit shutdown. The
  independent BaseRT service on port 8080 is never touched.

### Core Crates

```
crates/
  daemon/        — axum server, startup, port/token mgmt, route handlers
  agent/         — PromptBuilder, ContextPlanner, MemoryExtractor, MailIntent, WindowIntent
  rules/         — rules engine (YAML loader + matcher, hot-reload)
  memory/        — SQLite read/write, FTS5, MemorySelector
  skills/        — SKILL.md loader + selector (repo skills/ dir + app data override)
  attachments/   — file extraction pipeline (text/PDF/image)
  connectors/
    basert/      — OpenAI-compatible chat SSE, tools, JSON generation
    apple_mail/  — Envelope Index SQLite + emlx parser + AppleScript fallback
    apple_notes/ — NoteStore SQLite + JXA body retrieval
```

---

## IPC Design

### MVP — Local HTTP + SSE

```
POST /chat           { message, model, history, context }
                     → 200 SSE: token/activity_started/activity_completed
                     → 200 SSE: source_discovered/approval_requested/error/done

POST /approve        { approval_id, decision: "allow"|"deny", reason? }
                     → 200 { ok: true }

GET  /approvals/pending   → 200 [ApprovalRequest]
GET  /audit          { since?, limit? } → 200 [AuditEntry]
GET  /connectors     → 200 [ConnectorStatus]
POST /connectors/{id}/sync  → 200 { queued: true }
GET  /health         → 200 { status:"ok", model, basert }
```

Auth: `Authorization: Bearer <token>` on all requests.

### v2 — Unix Domain Socket

Replace HTTP with a UDS at `~/Library/Application Support/bagent/daemon.sock`. Same JSON framing. Eliminates port conflicts, slightly lower latency.

### v3 — gRPC (Optional)

`tonic` + protobuf. Consider if multiple frontends (iOS companion, CLI) need the daemon. Not planned before Phase 10.

---

## Model selection

There is no routing pipeline — it was replaced by an agentic tool-calling loop
(the model sees native tool definitions and decides what to call). `MODEL_ROUTER.md`
described the old design and has been removed.

The typed Mail/web evidence-answer path is the production default for deterministically
classified latest-Mail Header Listings, latest-Mail Content Readings, direct pages,
single-authority web facts, corroborated web facts, and their supported quoted-evidence
wrappers. Its classifier is intentionally narrow. Targeted Mail, ambiguous Mail,
mixed Mail/web, multi-page ambiguity, unrelated requests, unsupported classifications,
and ordinary agentic tool use remain on the agentic loop.

`BAGENT_EVIDENCE_ORCHESTRATOR` controls routing only. Absent or `1` selects the typed
route; `0` immediately restores the previous agentic loop after daemon restart. Any
other value uses the production default and emits a normalized warning that does not
include the supplied value. Typed routing is decided before legacy Mail prefetch,
guidance, prose-result heuristics, or agentic tools are prepared, preventing duplicate
Mail/web operations. See [`STAGE9_ROUTING_ROLLBACK.md`](STAGE9_ROUTING_ROLLBACK.md).

The bounded typed route uses Tavily Basic plus DuckDuckGo when the signed app supplies a
Tavily key ephemerally from Keychain. Without a key it retains Wikipedia plus DuckDuckGo;
the legacy tool stays on that keyless provider set. Tavily is discovery-only and is never
retried; fetched pages still pass the complete DNS, redirect, SSRF, extraction, relevance,
authority, and independent-source pipeline. See
[`TAVILY_WEB_DISCOVERY.md`](TAVILY_WEB_DISCOVERY.md) for free-tier setup and budgets.

For every validated Evidence Bundle, the daemon first builds a complete
`CanonicalGroundedAnswer`. This deterministic record owns coverage, citations,
conflicts, shortfalls, source identities, and evidence outcome. Qwen3.6-35B-A3B may
optionally polish wording with context 4,096, KV4, 256 output tokens, batch size 1,
and the 25%/8 GiB admission gate. Polished text replaces the canonical text only after
bundle and canonical-invariant validation. Rejection, timeout, unavailability, memory
ineligibility, or runtime poisoning preserves the canonical bytes. The 4B model is not
a grounding-quality fallback. Structured synthesis remains a disabled experiment.

- Local BaseRT (`basecompute/Qwen3-4B-Instruct-2507`) → chat, tool calls, and classifiers.
- Retrieval uses SQLite FTS5 only. Screen frames are processed by Apple Vision
  OCR locally and only extracted text reaches the model; image attachments are rejected.
- Codex CLI → advanced cross-source tasks, approval-gated.
- Cloud LLM → opt-in only.

See the "Agentic tool loop" section of `CLAUDE.md` for the retained rollback and
unsupported-intent loop.

---

## BaseRT Integration

- Base URL: `http://127.0.0.1:8082/v1`; API key: `basert-local`.
- Endpoints used: `POST /v1/chat/completions`, `GET /v1/models`, `GET /health`,
  and `POST /v1/models/unload`.
- Streaming follows OpenAI SSE (`data: {...}` ending in `data: [DONE]`);
  fragmented UTF-8 and tool-call names/arguments are reassembled before semantic
  transcript events cross the daemon-to-app seam.
- The configured model and classifier are both
  `basecompute/Qwen3-4B-Instruct-2507`.
- `/embeddings` remains as a compatibility route returning
  `501 embeddings_disabled`.

---

## Codex CLI Integration (Phase 8 — Advanced Task Harness)

Codex is an **external reasoning harness** for complex cross-source business/admin workflows,
not a coding tool. It is invoked only when the deterministic `TaskRater` returns
`CodexRecommended` or `CodexRequired` (score ≥ 60), and only after explicit user approval.

### Task Rating

`crates/agent/src/task_rater.rs` — deterministic keyword-gate rater (no LLM fallback):

| Level | Score | Meaning | Example |
|---|---|---|---|
| `LocalOnly` | 0–9 | BaseRT handles it | "zhrň mi tento email" |
| `LocalPreferred` | 10–29 | BaseRT preferred | "navrhni krátky email" |
| `CodexCandidate` | 30–59 | May benefit from Codex | "porovnaj dve zmluvy" |
| `CodexRecommended` | 60–84 | Codex recommended | "priprav brief pre klienta z mailov a Odoo" |
| `CodexRequired` | 85+ | Codex required | "hromadné odpovede na faktúry po splatnosti" |

### Context Packet Privacy Model

Codex receives only a daemon-built `CodexContextPacket` (JSON via stdin). It **never** gets:
- Raw email/WhatsApp/Gmail bodies (unless explicitly approved)
- Memory DB, conversation history, session tokens
- Odoo credentials or API tokens
- Keychain, `.ssh`, `.gnupg`, browser stores, password managers
- `~/Library/Application Support/bagent/` contents
- Unrelated private files or screenshots

The user must approve the context packet before dispatch. The packet is shown in the
approval modal including: task description, complexity rating, privacy risk, list of context
items (summaries + record refs).

### Codex Binary Configuration

- Binary path: user-configurable in Settings → Codex (`UserDefaults` key `bagent.codex_path`).
- Default: auto-discover from `$PATH`.
- The connector resolves the actual binary path; never uses shell aliases.
- Invoked as: `codex exec --sandbox read-only -` (prompt via stdin, never `--dangerously-bypass-*`).
- Timeout: 120 s; SIGTERM then SIGKILL on timeout.
- Graceful degradation: `{ran:false, error:"codex_not_found"}` if binary absent.

### Output Contract

Codex returns structured JSON:
```json
{
  "summary": "...",
  "findings": [...],
  "conflicts": [...],
  "proposed_actions": [...],
  "drafts": [...],
  "questions_for_user": [...]
}
```
**Proposed actions are never auto-executed.** They flow back as proposals into bagent's
normal approval/tools framework.

### Rules Engine

`codex.run_task` is set to `Ask` level in `rules.yaml` — approval is always required,
regardless of session context. This cannot be downgraded to `Auto`.

### API Routes

| Method | Path | Description |
|---|---|---|
| `GET` | `/codex/status` | Binary availability + version |
| `POST` | `/codex/rate-task` | Rate task complexity (no Codex invoked) |
| `POST` | `/codex/run-task` | Rate → approve → run → structured result |

### Audit

Every `/codex/run-task` attempt is logged to the audit table with:
`{description, level, privacy_risk, context_sources, approval_id, exit_code, timed_out, output_hash}`
Raw private context bodies are never audited.

---

## MCP-Style Tool Layer

### Tool Trait

```rust
pub trait Tool: Send + Sync {
    fn manifest(&self) -> ToolManifest;
    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<Value>;
}

pub struct ToolManifest {
    pub name: String,
    pub description: String,
    pub input_schema: Value,        // JSON Schema
    pub side_effect_class: SideEffectClass,
    pub approval_level: ApprovalLevel,
}

pub enum SideEffectClass { ReadOnly, LocalWrite, ExternalRead, ExternalWrite, CodeWrite, Shell }
pub enum ApprovalLevel  { Auto, Ask, Forbidden }
```

### Registry

- Loaded at daemon start; tools registered from each connector + built-ins.
- Dispatcher checks rules engine before execution; upgrades `Auto` → `Ask` if matching deny/ask rule.
- Tool call recorded in `tool_calls` table before + after execution.

---

## Rules Engine

- Rules defined in `~/.config/bagent/rules.yaml` (user-editable; hot-reloaded).
- Schema:

```yaml
rules:
  - id: no-send-without-approval
    match:
      side_effect_class: ExternalWrite
      connector: mail
    action: ask

  - id: block-root-shell
    match:
      tool: shell_exec
      args_contains: "sudo"
    action: deny
```

- Evaluation order: first match wins; default = `ask` for writes, `allow` for reads.
- Rules immutable to LLM — only the user (via Settings) can modify them.

---

## Planning Layer (Phase 4J+)

Before prompt assembly, every chat turn runs through three sequential stages:

```
user_turn
  │
  ▼
ContextPlanner::plan()          — deterministic rules + BaseRT JSON-mode fallback
  │  produces ContextPlan { task_type, response_language_hint, needs_memory,
  │                         memory_namespaces, memory_kinds, needs_conversation_recall,
  │                         candidate_skill_names, confidence }
  ▼
skill_selector::select()        — picks ≤ 3 SKILL.md files matching candidate_skill_names
  │                               or keyword-matched from user_turn
  ▼
tokio::join!
  memory_selector::select()     — RetrieveQuery filtered by namespaces+kinds; max 6 items, budget 4800 chars
  corrections retrieve          — namespace="corrections", kind filter
  recall_candidates retrieve    — injected only when needs_conversation_recall=true
  │
  ▼
PromptBuilder::build()          — injects selected skills + memory into prompt layers
```

**Rule:** `confidence < 0.6` on the deterministic plan triggers the LLM JSON-mode fallback (`generate_json`). Parse failure → fail closed (fewer injections, `needs_memory=false`).

---

## Memory and Indexing

- SQLite with FTS5 virtual tables for full-text search over `messages`, `notes`, `memory_items`.
- Historical embedding rows remain in SQLite but are not consulted.
- Per-source namespaces prevent cross-connector bleed.
- `language` column on every text-storing table (`sk`, `en`, `und`).
- Retrieval: BM25×0.70 + importance×0.15 + recency×0.15; per-namespace cap 3; near-duplicate text filter.
- **Memory ledger fields** (V11): `confidence`, `importance`, `status` (`active`/`superseded`/`deleted`), `source` (`passive`/`explicit`/`user_edit`/`import`), `sensitivity` (`normal`/`sensitive`), `subject`, `supersedes_id`.
- Hard retrieval filter: `status='active'` + `sensitivity='normal'`; insertion uses normalized exact-text deduplication.
- Passive extraction gates: `confidence ≥ 0.75`, `importance ≥ 0.60`, no sensitive-text indicators, no one-off content patterns.

## Skills

- Local `SKILL.md` files with YAML frontmatter (`name`, `description`, `version`, `risk`, `allowed_tools`, `tags`) + Markdown body.
- Scanned from `skills/` (repo root, dev) and `~/Library/Application Support/bagent/skills/` (user override); later dirs win by name.
- Selected at runtime by `ContextPlanner` candidate names + keyword matching; max 3 per turn; body truncated to 1 500 chars.
- `allowed_tools` in frontmatter is **descriptive only** — rules engine remains the authority for actual permission grants.
- Default skills shipped: `sk-business-email`, `mail-search`, `invoice-analysis`, `odoo-readonly`, `aerospace-window-control`.

---

## Audit Log

- Append-only table `audit_entries`; no UPDATE/DELETE ever issued against it.
- Each row: `id`, `seq` (monotonic), `prev_hash` (SHA-256 of previous row JSON), `actor`, `action`, `payload_json`, `model`, `language`, `created_at`.
- JSONL mirror at `~/Library/Application Support/bagent/audit.jsonl` — rotated at 10 MB.
- Viewer: `/audit` endpoint + SwiftUI list in Settings tab.

---

## Connector Design

```rust
pub trait Connector: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> ConnectorCapabilities;
    async fn read(&self, query: ReadQuery) -> Result<Vec<ConnectorRecord>>;
    async fn write_with_approval(&self, op: WriteOp, approval: ApprovalToken) -> Result<WriteResult>;
}
```

- Each connector declares capabilities: `{ can_read, can_write, requires_approval_for_write, pii_present }`.
- `pii_present = true` → privacy filter engaged before any cloud model call.
- Connectors are isolated; a crash in one does not take down the daemon.

---

## Packaging and Signing

- Xcode project target: `bagent.app` (macOS 14.0+ minimum for ScreenCaptureKit improvements).
- Embedded daemon binary: `bagent.app/Contents/MacOS/bagentd` (Rust universal binary).
- Hardened Runtime: required for notarization. Entitlements file must list `com.apple.security.automation.apple-events` per-app.
- Notarization: `xcrun notarytool submit` in CI; staple to `.dmg`.
- Distribution: direct `.dmg` download + optional Homebrew Cask.
- Auto-update: Sparkle 2.x framework; delta updates; signature verification.
- Universal binary: `lipo` arm64 + x86_64 Rust targets; Swift via Xcode "Any Mac" destination.
