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

### Core Crates (planned)

```
crates/
  daemon/        — axum server, startup, port/token mgmt
  agent/         — agent runtime, turn loop, tool dispatcher
  router/        — model router, privacy filter
  rules/         — rules engine (YAML loader + matcher)
  memory/        — SQLite read/write, FTS5, embeddings
  audit/         — append-only audit log, hash chain
  connectors/    — connector trait + impls
  tools/         — MCP-style tool registry
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
- Local Ollama → classification, summarization, embeddings, Slovak text.
- Codex CLI → coding tasks only.
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

## Codex CLI Integration

- Invoked via `tokio::process::Command::new("codex")` with `["--json", "--no-confirm"]` (all confirmations handled by daemon approval layer, not Codex's own prompts).
- Working directory: sandboxed temp dir per session, cleaned after.
- Stdin: JSON task description (`{"task": "…", "context": "…"}`).
- Stdout: JSON stream `{"type":"patch"|"message"|"done", …}`.
- Approval required for every invocation (side-effect class: `CODE_WRITE`).
- Timeout: 120 s default; SIGTERM on timeout.
- Codex binary path configurable; graceful degradation if not found.

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

## Memory and Indexing

- SQLite with FTS5 virtual tables for full-text search over `messages`, `notes`, `memory_items`.
- `sqlite-vss` (or `sqlite-vec`) extension for cosine similarity search on embeddings.
- Embedding model: `nomic-embed-text` or `bge-m3` via Ollama (multilingual SK/EN).
- Per-source namespaces prevent cross-connector bleed.
- `language` column on every text-storing table (`sk`, `en`, `und`).
- Retrieval: hybrid BM25 + cosine; top-K re-ranked by recency.

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
