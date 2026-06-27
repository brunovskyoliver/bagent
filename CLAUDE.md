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

# Run live-Ollama tests (requires Ollama running with qwen2.5:7b + bge-m3)
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

**Note:** `swift run` lacks Info.plist → microphone and screen-recording permissions are denied. Voice and screen-context features **must** be tested via `make bundle && open bagent.app`.

### WhatsApp bridge (one-time setup)

```bash
make whatsapp-bridge-install   # from apps/macos/ — installs Node deps + Chromium
```

### Required models (pull once)

```bash
ollama pull qwen2.5:7b      # default chat model (passes SK diacritics)
ollama pull bge-m3          # embeddings (Phase 3+)
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
| `crates/daemon` | axum server, route handlers, `AppState`, SQLite migrations, `fetch_tool_context` dispatcher |
| `crates/agent` | `PromptBuilder` (9-layer), intent classifiers (mail/odoo/window/file/screen/whatsapp), `ContextPlanner`, `ReferenceResolver`, `TaskRater`, `MemoryExtractor` |
| `crates/memory` | `MemoryStore` (SQLite+FTS5+cosine), `selector`, `markdown_mirror` |
| `crates/rules` | YAML rules engine (`auto` / `ask` / `forbidden`); hot-reloads every 5 s |
| `crates/skills` | `SKILL.md` loader + selector; scanned at startup from `skills/` |
| `crates/attachments` | Content extraction pipeline (text/PDF/image) |
| `crates/connectors/ollama` | `OllamaClient` — chat_stream, embed, summarize, generate_json |
| `crates/connectors/apple_mail` | Envelope Index SQLite reader, emlx parser, AppleScript fallback, `MailSearchFilter` |
| `crates/connectors/apple_notes` | NoteStore SQLite + JXA body retrieval |
| `crates/connectors/odoo` | MCP client via `rmcp 1.8` — spawns `uvx mcp-server-odoo` as child process |
| `crates/connectors/filesystem` | `PathPolicy`-gated file search/read/open |
| `crates/connectors/whatsapp` | Node.js bridge subprocess (whatsapp-web.js + QR auth) |
| `crates/connectors/codex` | Subprocess wrapper for Codex CLI (sandboxed, approval-gated) |

### Swift app structure

| File | Role |
|---|---|
| `AppDelegate.swift` | App lifecycle; `⌥Space` hotkey double-press logic |
| `NotchWindowController.swift` | `NSPanel` geometry (notch-wrap + non-notch), 3-phase expand animation |
| `ChatViewModel.swift` | `@MainActor ObservableObject`; all daemon calls; session/attachment/voice/screen state |
| `DaemonClient.swift` | HTTP + SSE client; all REST/SSE types |
| `SpeechController.swift` | WhisperKit `AudioStreamTranscriber`; state machine `idle→loadingModel→listening→finalizing→done` |
| `ScreenContextProvider.swift` | ScreenCaptureKit capture → Vision OCR → base64 for `/chat` |
| `SettingsView.swift` | All settings: model picker, connectors, permissions, rules, memory, skills, debug |

### Planning / context layer

Every `/chat` request runs through a planning layer before prompt assembly:

1. `ContextPlanner` — deterministic keyword gates (mail / file / odoo / screen / window / whatsapp) with Ollama JSON-mode fallback → `ContextPlan { task_type, needs_*, language_hint }`
2. `SkillSelector` — picks up to 3 matching `SKILL.md` files from `skills/`
3. `MemorySelector` + corrections + cross-session recall run in `tokio::join!`
4. `fetch_tool_context` dispatches the appropriate connector
5. `PromptBuilder::build` assembles 9 layers (identity, style, glossary, correction, memory, tool-data, attachment, session summary, recent turns)

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
