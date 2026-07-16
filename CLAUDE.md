# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with this repository.

## Commands

### Rust daemon

```bash
# Build debug
cargo build --workspace

# Build release
cargo build --release --workspace

# Run all tests (excluding live-Ollama tests)
cargo test --workspace

# Run live-Ollama tests (requires Ollama running with qwen3:8b + qwen3:0.6b + bge-m3)
cargo test --workspace -- --include-ignored

# Run a single crate's tests
cargo test -p bagent-agent
cargo test -p ollama-connector

# Slovak diacritics regression tests (live Ollama required)
cargo test -p ollama-connector -- --include-ignored
```

### Swift macOS app (from `apps/macos/`)

```bash
# Dev run: builds daemon debug binary first, then swift app (mic/TCC works)
make run

# Release .app bundle (codesigned ad-hoc)
make bundle && open bagent.app

# Build Swift only
swift build

# Clean everything including Rust target/
make clean
```

**Note:** `swift run` lacks Info.plist → screen-recording permissions are denied. Screen-context features **must** be tested via `make bundle && open bagent.app`.

### WhatsApp bridge (one-time setup)

```bash
make whatsapp-bridge-install   # from apps/macos/ — installs Node deps + Chromium
```

### Required models (pull once)

```bash
ollama pull qwen3:8b        # default chat model (passes SK diacritics)
ollama pull qwen3:0.6b      # screen-intent / memory-extractor classifier (fast, think disabled)
ollama pull bge-m3          # embeddings (Phase 3+, evidence rerank)
ollama pull qwen2.5vl:7b   # vision / screen-context (Phase 7+, ~6 GB)
```

## Architecture

```
SwiftUI/AppKit (notch panel)   apps/macos/Sources/bagent/
        ↕  HTTP + SSE  127.0.0.1:<dynamic port>
Rust daemon (axum)             crates/daemon/src/main.rs
        ↕
Ollama  ·  Connectors  ·  SQLite (refinery migrations)
```

### IPC contract

- Daemon writes its port to `~/Library/Application Support/bagent/daemon.port` and bearer token to `daemon.token`.
- Swift `DaemonLauncher` starts `bagentd`, polls port file (40 × 100 ms), then `DaemonClient` attaches.
- Chat uses `POST /chat` → SSE stream of typed JSON events (`delta`, `done`, `mail_found`, `file_found`, `task_rating`, `debug_trace`, `memory_saved`, …).
- Every write action goes through `GET /approvals/pending` + `POST /approvals/:id/decide` before execution.

### Rust crates

| Crate | Purpose |
|---|---|
| `crates/daemon` | axum server, route handlers, `AppState`, SQLite migrations, agentic tool loop + gated tool dispatch |
| `crates/agent` | `PromptBuilder`, `ScreenIntentClassifier`, `TaskRater`, `MemoryExtractor`, correction/feedback classifiers |
| `crates/memory` | `MemoryStore` (SQLite+FTS5+cosine), `selector`, `markdown_mirror` |
| `crates/rules` | YAML rules engine (`auto` / `ask` / `forbidden`); hot-reloads every 5 s |
| `crates/skills` | `SKILL.md` loader + selector; scanned at startup from `skills/` |
| `crates/attachments` | Content extraction pipeline (text/PDF/image) |
| `crates/connectors/ollama` | `OllamaClient` — chat_stream, chat_stream_with_tools, embed, summarize, generate_json |
| `crates/connectors/apple_mail` | Envelope Index SQLite reader, emlx parser, AppleScript fallback, `MailSearchFilter` |
| `crates/connectors/apple_notes` | NoteStore SQLite + JXA body retrieval |
| `crates/connectors/odoo` | MCP client via `rmcp 1.8` — spawns `uvx mcp-server-odoo` as child process |
| `crates/connectors/filesystem` | `PathPolicy`-gated file search/read/open |
| `crates/connectors/whatsapp` | Node.js bridge subprocess (whatsapp-web.js + QR auth) |
| `crates/connectors/codex` | Subprocess wrapper for Codex CLI (sandboxed, approval-gated) |

### Swift app structure

**The notch is the only UI** — one `NSPanel`, no chat window, no settings window,
no menu-bar item. Read `docs/UI_DESIGN.md` before any UI change.

| File | Role |
|---|---|
| `AppDelegate.swift` | App lifecycle; `⌥Space` toggles the notch input |
| `NotchWindowController.swift` | The single `BagentPanel`: geometry, monitors, present/collapse, paste wheel |
| `ChatView.swift` | `NotchWrapView` + `InlineNotchContent` — every notch state renders here |
| `NotchSettingsContent.swift` | Settings pages (general / permissions / model / connectors / setup) |
| `ChatViewModel.swift` | `@MainActor ObservableObject`; all daemon calls; session/attachment/screen state; `notchInteractionMode` |
| `DaemonClient.swift` | HTTP + SSE client; all REST/SSE types |
| `ScreenContextProvider.swift` | ScreenCaptureKit capture → Vision OCR → base64 for `/chat` |

### Agentic tool loop

Every `/chat` request runs an agentic tool-calling loop — qwen3:8b sees native
tool definitions and decides what to call; there is no keyword routing or
intent classifier:

1. `PromptBuilder::build` assembles the system prompt (identity, style, glossary, skills, attachment/screen context, recent tool refs for follow-ups like "open it")
2. Tool registry is built per turn from available connectors: `mail_search` / `mail_list_inbox` / `mail_read` / `mail_open`, `filesystem_*`, `notes_*`, `whatsapp_*`, `odoo_*`, `macos_*`, plus always-on `web_search` / `web_fetch` (DuckDuckGo lite + Wikipedia REST, no API key; rules actions `web.search` / `web.fetch` default `auto`, private hosts blocked) (daemon `chat` handler)
3. Loop (max 5 rounds, 8 tool calls/turn): `chat_stream_with_tools` streams deltas + tool calls → each call is gated in the dispatcher (rules engine `auto`/`ask`/`forbidden`, PathPolicy inside the fs connector, approval modal for writes, `audit_entries` row per call) → results fed back as `role:"tool"` messages → repeat until a round emits no tool calls (that round's stream is the final answer)
4. Each tool call emits a `tool_call` SSE event (UI shows "🔎 Hľadám v pošte…"); `mail_found`/`file_opened`/`odoo_found` events keep the notch chips working
5. `mail_search` normalizes sender args internally: an empty result with a `sender` retries with the sender tokenized into AND keywords (bridges "Tomas Juricek" ↔ `tomas.juricek@novem.sk`)
6. Vision turns (image attachment / screen capture) skip tools — the vision model answers directly from injected context
7. Model output is never trusted for authorization — the dispatcher enforces every gate; unknown tools and exhausted budgets return corrective tool results
8. Follow-ups: the Swift client sends a sliding window of prior turns in `ChatRequest.history` (last 10 user/assistant turns, clamped server-side to 8k chars) which the handler splices before the current user turn; `PromptBuilder` itself stays stateless

Tool dispatch helpers live at the bottom of `crates/daemon/src/main.rs` (`tool_mail_search`, `tool_odoo`, …); live regression tests in `crates/connectors/ollama/tests/tool_loop.rs`.

### Slovak / English bilingual rules

- System prompt enforces diacritics and formal Slovak tone; legal/business terms (`DPH`, `faktúra`, `IČO`, `DIČ`, `splatnosť`) are never auto-translated
- `whatlang` detects per-message language; classifier prompts include coreference context (last 4 turns, 200 chars/turn)

### Security model

- All data stays on-device by default; cloud models are opt-in
- Odoo API key flows via child process env only — never written to disk or CLI args
- Every write action (email send, Odoo write, Codex run, WhatsApp send) requires explicit user approval via the approval modal (60 s timeout → auto-deny)
- Keychain for all secrets (`KeychainStore.swift`)
- Full audit log in SQLite `audit_entries` for every model decision and tool call
- `PathPolicy` blocks `.ssh`, Keychains, password managers, system dirs, and dangerous extensions from file connector

### Live-Ollama tests

Tests marked `#[ignore]` require Ollama running with the correct models. Run with `--include-ignored`. Do not remove the `#[ignore]` attribute — these are intentionally excluded from CI.
