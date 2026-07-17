# bagent

A private, local-first macOS assistant that lives near the MacBook notch. Built for Slovak and English business users.

---

## What it does

bagent is a smart assistant you invoke with `⌥Space`. It runs entirely on your Mac:

- **Summarizes email** — reads Apple Mail, produces concise summaries in Slovak or English.
- **Drafts replies** — formal Slovak business tone; legal terms never auto-translated.
- **Answers questions** about your mail, notes, and Odoo data (read-only).
- **Routes privately** — all inference runs via local Ollama; cloud models are opt-in only.
- **Asks before acting** — no email is sent, no Odoo record modified, no shell command run without your explicit approval.

---

## Slovak / English Support

bagent is built for bilingual Slovak–English business users:

- Correctly understands Slovak emails, invoices, and contracts.
- Preserves diacritics: `á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž`.
- Keeps business terms verbatim: `DPH`, `faktúra`, `splatnosť`, `IČO`, `DIČ`.
- Drafts in formal Slovak (`Dobrý deň, … S pozdravom`).
- Detects language per-message; replies in source language.

---

## Architecture

```
SwiftUI/AppKit (notch panel, modals, settings)
        ↕ HTTP / SSE (127.0.0.1)
Rust daemon (agent runtime, model router, rules, SQLite)
        ↕
Ollama (local LLM)  ·  Codex CLI  ·  Connectors (Mail, Notes, Odoo, Shell)
```

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for full design.

---

## Repo Layout (Planned)

```
bagent/
├── apps/
│   └── macos/              SwiftUI + AppKit frontend (Xcode project)
├── crates/
│   ├── daemon/             axum HTTP server, startup
│   ├── agent/              agent runtime, turn loop
│   ├── router/             model router, privacy filter
│   ├── rules/              YAML rules engine
│   ├── memory/             SQLite, FTS5, embeddings
│   ├── audit/              append-only audit log
│   ├── connectors/         connector trait + impls
│   └── tools/              tool registry
├── docs/                   architecture and planning docs
├── fixtures/
│   └── sk/                 Slovak QA test fixtures
└── README.md
```

---

## Prerequisites

| Tool | Version | Purpose |
|---|---|---|
| Xcode | 16+ | Swift / macOS app build |
| Rust | 1.80+ | Daemon binary |
| Ollama | latest | Local LLM inference |
| Codex CLI | optional | Coding tasks |
| macOS | 14.0+ | ScreenCaptureKit, modern SwiftUI |

---

## Status

Working. The UI is the notch surface only — see [`docs/UI_DESIGN.md`](docs/UI_DESIGN.md)
before touching anything visual. Chat runs through an agentic tool-calling loop
(no routing pipeline).

Build: `cargo build --workspace`, then `make bundle && open bagent.app` from `apps/macos/`.

---

## Docs

- [`docs/AUTOMATIONS.md`](docs/AUTOMATIONS.md) — scheduled automations: `/automations` flow, recurrence + DST semantics, missed-run/overlap/restart policies, unattended approval safety, API, SSE, retention

| Doc | Contents |
|---|---|
| [`ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Full system design, IPC, tool layer, packaging |
| [`UI_DESIGN.md`](docs/UI_DESIGN.md) | **The notch surface** — states, metrics, animation, what not to add |
| [`ROADMAP.md`](docs/ROADMAP.md) | Phased plan (historical; phases 0–10) |
| [`MVP_SPEC.md`](docs/MVP_SPEC.md) | First release scope (historical) |
| [`RULES.md`](docs/RULES.md) | What the agent may do auto, ask, or is forbidden |
| [`CONNECTORS.md`](docs/CONNECTORS.md) | Per-connector specs (Mail, Notes, Odoo, Ollama, …) |
| [`DATA_MODEL.md`](docs/DATA_MODEL.md) | SQLite schema (DDL), FTS5, embeddings, audit |
| [`SECURITY.md`](docs/SECURITY.md) | Threat model, mitigations, OWASP LLM Top 10 |
| [`TODO.md`](TODO.md) | Prioritized task list |

---

## Security & Privacy

- All data stays on-device by default.
- Cloud models: opt-in only; PII redacted before any cloud call.
- Keychain for all secrets; no plaintext API keys.
- Full audit log of every model decision and tool call.
- Human approval required for every write action (email send, Odoo write, shell command).
