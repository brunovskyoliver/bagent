# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with this repository.

## Commands

### Rust daemon

```bash
# Build debug
cargo build --workspace

# Build release
cargo build --release --workspace

# Run all tests
cargo test --workspace

# Run a single crate's tests
cargo test -p bagent-agent
cargo test -p basert-connector
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

### Required model

```bash
basert pull basecompute/Qwen3-4B-Instruct-2507
```

The app manages a dedicated BaseRT LaunchAgent on `127.0.0.1:8082`. Port 8080
belongs to the user's independent BaseRT/Claude-local setup and must not be changed.

## Architecture

```
SwiftUI/AppKit (notch panel)   apps/macos/Sources/bagent/
        ↕  HTTP + SSE  127.0.0.1:<dynamic port>
Rust daemon (axum)             crates/daemon/src/main.rs
        ↕
BaseRT  ·  Connectors  ·  SQLite/FTS5 (refinery migrations)
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
| `crates/memory` | `MemoryStore` (SQLite+FTS5), `selector`, `markdown_mirror` |
| `crates/rules` | YAML rules engine (`auto` / `ask` / `forbidden`); hot-reloads every 5 s |
| `crates/skills` | `SKILL.md` loader + selector; scanned at startup from `skills/` |
| `crates/attachments` | Content extraction pipeline (text/PDF/image) |
| `crates/connectors/basert` | `BaseRtClient` — OpenAI chat/SSE, tool calls, summarize, JSON generation |
| `crates/connectors/apple_mail` | Envelope Index SQLite reader, emlx parser, AppleScript fallback, `MailSearchFilter` |
| `crates/connectors/apple_notes` | NoteStore SQLite + JXA body retrieval |
| `crates/connectors/odoo` | MCP client via `rmcp 1.8` — spawns `uvx mcp-server-odoo` as child process |
| `crates/connectors/filesystem` | `PathPolicy`-gated file search/read/open |
| `crates/connectors/whatsapp` | Node.js bridge subprocess (whatsapp-web.js + QR auth) |
| `crates/connectors/codex` | Subprocess wrapper for Codex CLI (sandboxed, approval-gated) |

### Scheduled automations

`/automations` in the notch manages persisted cron-like agent tasks. The
scheduler is daemon-owned (survives app exit via the launchd agent); schedules
are typed (once / every-N-hours / daily / weekdays / selected / weekly) with
IANA-zone DST-correct recurrence, a 24h single-catch-up policy, atomic overlap
claims, 2-run concurrency, and unattended safety (side-effecting tools always
require fresh approval; approvals carry automation provenance). Key code:
`crates/automations` (semantics), `crates/daemon/src/{automations_api,scheduler,agent_exec}.rs`,
`apps/macos/Sources/bagent/Automation*.swift`. Full spec: `docs/AUTOMATIONS.md`.

### Swift app structure

**The notch is the default UI** — one `NSPanel`, no chat window, no settings
window, and no menu-bar item. The only approved exception is one chromeless,
floating Browser Panel per live bagent Browser Session. Read `docs/UI_DESIGN.md`
and the browser ADRs before any UI change.

| File | Role |
|---|---|
| `AppDelegate.swift` | App lifecycle; `⌥Space` toggles the notch input |
| `NotchWindowController.swift` | The single `BagentPanel`: geometry, monitors, present/collapse, paste wheel |
| `ChatView.swift` | `NotchWrapView` + `InlineNotchContent` — every notch state renders here |
| `NotchSettingsContent.swift` | Settings pages (general / permissions / model / connectors / setup) |
| `ChatViewModel.swift` | `@MainActor ObservableObject`; all daemon calls; session/attachment/screen state; `notchInteractionMode` |
| `DaemonClient.swift` | HTTP + SSE client; all REST/SSE types |
| `ScreenContextProvider.swift` | ScreenCaptureKit capture → local Vision OCR; only text reaches `/chat` |

### bagent Browser

bagent Browser is an optional, explicit setting backed by WebKit and a dedicated
persistent `WKWebsiteDataStore`. The Swift app owns every `WKWebView`, Browser
Session, Browser Cue, and Browser Panel on the main actor. The bundled
`bagent-browser-mcp` executable speaks MCP over stdio and forwards framed RPC
over the user-only `~/Library/Application Support/bagent/browser.sock` socket;
it owns no browser state.

```bash
# Phase 0 signed WebKit proof harness
scripts/run-agent-browser-phase0.sh

# Release app, daemon, and bundled MCP proxy
make bundle
codesign --verify --deep --strict --verbose=2 bagent.app
```

The proxy launches the packaged app when needed. Enable bagent Browser in the
notch settings before connecting Codex or Claude. Sessions are implicit per
stdio connection, capped at four, hidden by default, and share only the
bagent-owned Browser Profile. Read `docs/AGENT_BROWSER_ROADMAP.md` for the
complete policy and acceptance matrix.

Claude Code uses the same MCP initialization policy as Codex. For UI layout,
visual debugging, screenshots, rendered-page validation, interaction bugs,
forms, dialogs, menus, loading/responsive state, or local web-app debugging,
route to `bagent-browser`, call `browser_open` first when the URL is known, and
inspect with hidden `get_page_content` plus `screenshot`. Do not open a browser
for non-UI coding. Keep the popup hidden unless the user asks to see it or
direct human input is required. If the setting is off, expect structured
`browser_disabled` and tell the user to enable bagent Browser in Settings.

Development client registration uses the packaged executable directly:

```bash
codex mcp add bagent-browser -- "$PWD/apps/macos/bagent.app/Contents/MacOS/bagent-browser-mcp"
claude mcp add bagent-browser -- "$PWD/apps/macos/bagent.app/Contents/MacOS/bagent-browser-mcp"
```

### Agentic tool loop

Every `/chat` request runs an agentic tool-calling loop — Qwen3-4B sees native
tool definitions and decides what to call; there is no keyword routing or
intent classifier:

1. `PromptBuilder::build` assembles the system prompt (identity, style, glossary, skills, attachment/screen context, recent tool refs for follow-ups like "open it")
2. Tool registry is built per turn from available connectors: `mail_search` / `mail_list_inbox` / `mail_read` / `mail_open`, `filesystem_*`, `notes_*`, `whatsapp_*`, `odoo_*`, `macos_*`, plus always-on `web_search` / `web_fetch` (DuckDuckGo lite + Wikipedia REST, no API key; rules actions `web.search` / `web.fetch` default `auto`, private hosts blocked) (daemon `chat` handler)
3. Loop (max 5 rounds, 8 tool calls/turn): `chat_stream_with_tools` streams deltas + tool calls → each call is gated in the dispatcher (rules engine `auto`/`ask`/`forbidden`, PathPolicy inside the fs connector, approval modal for writes, `audit_entries` row per call) → results fed back as `role:"tool"` messages → repeat until a round emits no tool calls (that round's stream is the final answer)
4. Each tool call emits a `tool_call` SSE event (UI shows "🔎 Hľadám v pošte…"); `mail_found`/`file_opened`/`odoo_found` events keep the notch chips working
5. `mail_search` normalizes sender args internally: an empty result with a `sender` retries with the sender tokenized into AND keywords (bridges "Tomas Juricek" ↔ `tomas.juricek@novem.sk`)
6. Screen turns run Apple Vision OCR locally and inject text only; image attachments are rejected
7. Model output is never trusted for authorization — the dispatcher enforces every gate; unknown tools and exhausted budgets return corrective tool results
8. Follow-ups: the Swift client sends a sliding window of prior turns in `ChatRequest.history` (last 10 user/assistant turns, clamped server-side to 8k chars) which the handler splices before the current user turn; `PromptBuilder` itself stays stateless

Tool dispatch helpers live at the bottom of `crates/daemon/src/main.rs` (`tool_mail_search`, `tool_odoo`, …); protocol regressions live in `crates/connectors/basert/tests/protocol.rs`.

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

### Live BaseRT verification

Run the app-managed service on port 8082 and verify authenticated `/health`,
`/v1/models`, Slovak output, streaming, and tool-call round trips. Never reuse
or stop the unrelated service on port 8080.

## Agent skills

### Issue tracker

Issues are tracked as local Markdown under `.scratch/`; external PRs are not a
request surface. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the canonical labels `needs-triage`, `needs-info`, `ready-for-agent`,
`ready-for-human`, and `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

This is a single-context repository using root `CONTEXT.md` and system ADRs at
`docs/ADR-*.md`. See `docs/agents/domain.md`.
