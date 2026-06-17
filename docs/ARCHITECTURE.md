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
│  Ollama     Codex      Connectors    SQLite DB              │
│  :11434      CLI      (Mail/Notes    (bagent.db +            │
│  (local)  subprocess   Odoo/Shell    embeddings)            │
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

- `SwiftUI.TextEditor` for multi-line input, `ScrollView` with streaming token append for output.
- SSE stream from daemon: each `data:` chunk appended to `@Published var tokens: [String]`.

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
| Network | Odoo API, Ollama, optional cloud | Phase 3 |
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

### Core Crates

```
crates/
  daemon/        — axum server, startup, port/token mgmt, route handlers
  agent/         — PromptBuilder, ContextPlanner, MemoryExtractor, MailIntent, WindowIntent
  rules/         — rules engine (YAML loader + matcher, hot-reload)
  memory/        — SQLite read/write, FTS5, embeddings, MemorySelector
  skills/        — SKILL.md loader + selector (repo skills/ dir + app data override)
  attachments/   — file extraction pipeline (text/PDF/image)
  connectors/
    ollama/      — chat stream, embeddings, generate_json (format:json mode)
    apple_mail/  — Envelope Index SQLite + emlx parser + AppleScript fallback
    apple_notes/ — NoteStore SQLite + JXA body retrieval
```

---

## IPC Design

### MVP — Local HTTP + SSE

```
POST /chat           { messages, context, rules_override? }
                     → 200 SSE stream: data: {"token":"…"}\n\n
                     → 200 SSE stream: data: {"done":true,"tool_calls":[…]}\n\n

POST /approve        { approval_id, decision: "allow"|"deny", reason? }
                     → 200 { ok: true }

GET  /approvals/pending   → 200 [ApprovalRequest]
GET  /audit          { since?, limit? } → 200 [AuditEntry]
GET  /connectors     → 200 [ConnectorStatus]
POST /connectors/{id}/sync  → 200 { queued: true }
GET  /health         → 200 { status:"ok", version, ollama_up }
```

Auth: `Authorization: Bearer <token>` on all requests.

### v2 — Unix Domain Socket

Replace HTTP with a UDS at `~/Library/Application Support/bagent/daemon.sock`. Same JSON framing. Eliminates port conflicts, slightly lower latency.

### v3 — gRPC (Optional)

`tonic` + protobuf. Consider if multiple frontends (iOS companion, CLI) need the daemon. Not planned before Phase 10.

---

## Model Router

See [`MODEL_ROUTER.md`](MODEL_ROUTER.md) for full routing table and prompt templates.

Summary:
- Local Ollama → classification, summarization, embeddings, Slovak text, single-source tasks.
- Codex CLI → advanced cross-source business/admin reasoning (Phase 8, approval-gated).
- Cloud LLM → complex reasoning, user opt-in, privacy-filtered.

---

## Ollama Integration

- Base URL: `http://localhost:11434` (configurable).
- Endpoints used: `POST /api/chat` (streaming), `POST /api/embeddings`, `GET /api/tags`.
- Preflight on daemon start: `GET /api/tags` with 2 s timeout; set `ollama_up` flag; expose via `/health`.
- Streaming: parse `ndjson` lines, emit as SSE tokens upstream.
- Model selection: stored in `connectors` table config; default `qwen2.5:7b`.
- Context window management: sliding window of last N tokens; summarize older turns with local model.

---

## Codex CLI Integration (Phase 8 — Advanced Task Harness)

Codex is an **external reasoning harness** for complex cross-source business/admin workflows,
not a coding tool. It is invoked only when the deterministic `TaskRater` returns
`CodexRecommended` or `CodexRequired` (score ≥ 60), and only after explicit user approval.

### Task Rating

`crates/agent/src/task_rater.rs` — deterministic keyword-gate rater (no LLM fallback):

| Level | Score | Meaning | Example |
|---|---|---|---|
| `LocalOnly` | 0–9 | Ollama handles it | "zhrň mi tento email" |
| `LocalPreferred` | 10–29 | Ollama preferred | "navrhni krátky email" |
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

  - id: allow-ollama-read
    match:
      connector: ollama
    action: allow

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
ContextPlanner::plan()          — deterministic rules + Ollama JSON-mode fallback
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
- `sqlite-vec` extension for cosine similarity search on embeddings.
- Embedding model: `bge-m3` via Ollama (multilingual SK/EN).
- Per-source namespaces prevent cross-connector bleed.
- `language` column on every text-storing table (`sk`, `en`, `und`).
- Retrieval: hybrid BM25×0.35 + cosine×0.45 + importance×0.10 + recency×0.10; per-namespace cap 3; MMR near-dup filter.
- **Memory ledger fields** (V11): `confidence`, `importance`, `status` (`active`/`superseded`/`deleted`), `source` (`passive`/`explicit`/`user_edit`/`import`), `sensitivity` (`normal`/`sensitive`), `subject`, `supersedes_id`.
- Hard retrieval filter: `status='active'` + `sensitivity='normal'`; explicit/user_edit insertion supersedes conflicting passive items.
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
