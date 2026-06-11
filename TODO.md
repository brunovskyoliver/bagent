# TODO

Prioritized task list. Mirrors [`docs/ROADMAP.md`](docs/ROADMAP.md) phases.
Check off items as they are completed.

---

## Phase 0 — Research Spikes

- [x] Measure notch height and safe inset per device:
  - [x] MacBook Pro M5 (Mac17,2) — notch=221pt, menubar=39pt, auxiliaryTopLeft w=791 — see `docs/spikes/notch_geometry.md`
  - [ ] MacBook Air M2 (no notch) — menu-bar fallback path (test needed)
- [x] Prototype `NSPanel` anchored to notch:
  - [x] Verify z-order above menu bar items — `BagentPanel: NSPanel` with `canBecomeKey = true`
  - [ ] Test with Mission Control active
  - [ ] Test with full-screen apps
- [ ] Benchmark ScreenCaptureKit:
  - [ ] CPU/memory at 1 fps frame capture
  - [ ] CPU/memory at 5 fps
  - [ ] Confirm black-frame handling for DRM content
- [x] Benchmark Ollama (Slovak) — see `docs/spikes/ollama.md`:
  - [x] Latency on M5: TTFT ~269ms warm, ~26 tok/s (llama3.1:8b)
  - [x] Roundtrip test: **llama3.1 FAILS** (62% diacritics, mixes Czech); **qwen2.5:7b PASSES** (16/16)
  - [x] Invoice fixture: all 3 SK fixtures pass with qwen2.5:7b — DPH/faktúra/splatnosť preserved, zero Czech
  - [x] `ollama pull qwen2.5:7b` — done; confirmed default model
  - [ ] `ollama pull bge-m3` — needed for Phase 3 embeddings
  - [ ] Benchmark qwen2.5:7b cold start time
- [x] Snapshot Apple Mail SQLite schema — see `docs/spikes/apple_mail.md`:
  - [x] `Envelope Index` confirmed at `~/Library/Mail/V10/MailData/Envelope Index`
  - [x] Unread messages query confirmed (joins messages + subjects + addresses)
  - [x] emlx format documented (int header + RFC 2822 + quoted-printable)
  - [x] **emlx path = `{mbox}/Data/{(ROWID/1000)%10}/{(ROWID/10000)%10}/Messages/{ROWID}.emlx`** — confirmed
  - [x] IMAP partial-download limitation documented: ~768 local emlx out of 84,273 DB rows; AppleScript fallback needed
  - [ ] Write and test emlx MIME parser (QP decode + MIME multipart + HTML strip)
  - [ ] Implement AppleScript body fallback for IMAP-only messages
- [x] Snapshot Apple Notes SQLite schema — see `docs/spikes/apple_notes.md`:
  - [x] `NoteStore.sqlite` confirmed at `~/Library/Group Containers/group.com.apple.notes/`
  - [x] Schema documented (Core Data polymorphic ZICCLOUDSYNCINGOBJECT)
  - [x] Note body: protobuf (ZMERGEABLEDATA) — use AppleScript/JXA, not direct decode
  - [ ] Identify ICNote Z_ENT number on this macOS version
  - [ ] Test JXA body retrieval for 5 sample notes
- [ ] Odoo XML-RPC handshake:
  - [ ] Authenticate against sandbox Odoo instance (needs credentials)
  - [ ] Read one `account.move` invoice record
  - [ ] Document version detection via `/web/webclient/version_info`
  - [ ] Document in `docs/spikes/odoo.md`
- [x] Create SK QA fixtures (`fixtures/sk/`) — faktura-upomienka, stretnutie, staznost
- [x] Write spike docs under `docs/spikes/`

---

## Phase 1 — Notch UI Shell ✅ COMPLETE
<!-- non-notch Mac test is a hardware gate — marked pending until device available -->

- [x] Swift Package (`apps/macos/Package.swift`) — macOS 14+, Swift 6, Carbon linked
- [x] `NSStatusItem` fallback (non-notch Macs) — `StatusBarController.swift`
- [x] `NSPanel` anchored to notch region — `NotchWindowController.swift`
  - [x] Position via `auxiliaryTopLeftArea` / `auxiliaryTopRightArea`
  - [x] `styleMask: [.borderless, .nonactivatingPanel]` (collapsed) / `[.borderless]` (expanded)
  - [x] `level: .statusBar`
  - [x] `BagentPanel` subclass: `canBecomeKey = true` (required for text input in borderless panel)
- [x] Global hotkey `⌥Space` via Carbon `RegisterEventHotKey` — `GlobalHotkey.swift`
- [x] Chat UI — `ChatView.swift`
  - [x] Multi-line `TextField` input
  - [x] Message bubble list with stub response
  - [x] Send button + `⌘↩` shortcut
  - [x] Suggestion chips (3 SK/EN prompts)
  - [x] Thinking indicator (animated dots)
  - [x] Clear conversation button (trash icon)
- [x] Animate expand/collapse (150 ms ease-out via `NSAnimationContext`)
- [x] Dark mode — `.regularMaterial` background auto-adapts
- [x] `Escape` key collapses panel — `NSEvent.addLocalMonitorForEvents` (replaces broken `onKeyPress`)
- [x] Notch Mac: dark pill hangs below physical notch; external display: transparent wide pill inside menu bar *(pill-below-notch approach superseded by Phase 1A)*
- [x] `NSStatusItem` hidden on notch Mac (pill is the indicator); shown on non-notch as right-side fallback
- [x] `make bundle` produces ad-hoc signed `bagent.app`, `make run` builds daemon first
- [x] `swift build` passes with zero errors (Swift 6 strict concurrency)
- [ ] Test on non-notch Mac (fallback geometry path)

---

## Phase 1A — Notch-Wrapping UI (NotchNook / Alcove style)

Built-in display only (notch present). External / non-notch path unchanged. See `docs/UI_DESIGN.md` for anatomy + animation vocabulary.

- [ ] `NotchWrapShape.swift` — SwiftUI `Shape` with animatable `wingWidth` + `bridgeHeight`
- [ ] Geometry: compute left/right wing rects from `auxiliaryTopLeft/RightArea`; `pillFrame` becomes union rect spanning both wings + notch gap + bridge room
- [ ] Replace `PillView` notch branch (`ChatView.swift`) with `NotchWrapView` — sparkles icon left, chevron icon right, no title text
- [ ] Hover state: wings expand 32 pt → 96 pt, bridge fades in, subtle white stroke on shape
- [ ] `hoverChanged(isHovered:)` callback from SwiftUI → `NotchWindowController` to drive `setFrame` in sync with SwiftUI layout
- [ ] Click / `⌥Space`: redesigned 3-phase expand animation (Phase A wings spread → Phase B bridge drops → Phase C content fades in)
- [ ] Collapse: reverse animation, anchored at notch top-center
- [ ] Hit-test via `.contentShape(NotchWrapShape(...))` — notch cutout stays click-through
- [ ] Visual QA on M1/M2/M3/M4/M5 notch geometries (inner corner radius match)
- [ ] Update `docs/spikes/notch_geometry.md` with per-model notch corner radii
- [ ] `docs/UI_DESIGN.md` — notch wrap anatomy, animation language, iconography slots, reduced-motion fallback

---

## Phase 2 — Rust Backend + IPC ✅ COMPLETE

- [x] Cargo workspace at repo root (`Cargo.toml`)
- [x] `crates/daemon/` (`bagentd`): axum 0.7 server on `127.0.0.1:0`
  - [x] Write port to `~/Library/Application Support/bagent/daemon.port`
  - [x] Generate bearer token on first run; written to `daemon.token` (Keychain: Phase 10)
  - [x] `GET /health` endpoint (checks Ollama up/down, returns model)
  - [x] `POST /chat` — SSE streaming to Ollama with ndjson → `data:` translation
  - [x] Bearer token auth middleware
- [x] SQLite with refinery migrations (`migrations/V1__initial.sql`, `V2__full_schema.sql`)
  - [x] Schema: `audit_entries`, `approvals`, `messages`, `sessions`, `connectors`
- [x] Swift `DaemonClient` — `DaemonClient.swift`
  - [x] Read port + token from files on app launch (40 × 100 ms retry)
  - [x] SSE streaming client via `URLSession.bytes(for:)`
  - [x] `healthStatus()` → `DaemonHealth` (daemon up, Ollama up, model)
- [x] `DaemonLauncher.swift` — auto-restarts on crash, max 3/min rolling window
- [x] Audit entry on every chat request (SQLite `audit_entries`)
- [x] Settings tab: daemon + Ollama status indicator with live indicator dots

---

## Phase 3 — Ollama Integration ✅ COMPLETE

- [x] `crates/connectors/ollama/` — standalone library crate (`OllamaClient`)
  - [x] `models()` → sorted list from `/api/tags`
  - [x] `chat_stream()` → `impl Stream<Item = Result<String>>` via async-stream
  - [x] `embed()` → `Vec<f32>` from `/api/embeddings`
  - [x] `summarize()` → single-shot summarisation call
  - [x] `is_up()` → 2 s health ping
- [x] Daemon uses `OllamaClient` for all Ollama I/O
- [x] `POST /embeddings` endpoint in daemon (proxies to Ollama, uses `bge-m3` by default)
- [x] System prompt — Slovak business assistant: formal tone, diacritics enforced, legal terms never translated
- [x] Context window management:
  - [x] Sliding hard truncation (last 40 messages) for moderate histories
  - [x] Automatic summarisation when history > 60 messages (old turns → single summary system message)
- [x] Model router: all requests → Ollama; client-supplied `model` field overrides default
- [x] Model picker in `SettingsView.swift` — fetches live from `/models`, persists to UserDefaults
- [x] Default model: `qwen2.5:7b`
- [x] Ollama up/down in `GET /health`
- [x] Slovak diacritics regression tests — `crates/connectors/ollama/tests/diacritics.rs` (`#[ignore]`, run with `cargo test -p ollama-connector -- --include-ignored`)
- [x] Streaming tokens appear in UI (TTFT < 1 s on warm Ollama)
- [ ] `ollama pull bge-m3` — user must run once before embeddings work

---

## Phase 4 — Read-Only Apple Mail + Notes ✅ COMPLETE

- [x] `crates/connectors/apple_mail/`:
  - [x] Read `Envelope Index` SQLite (readonly WAL, busy_timeout 2s)
  - [x] Parse `.emlx` body (mailparse crate — QP decode + MIME multipart + HTML strip)
  - [x] Tool: `GET /mail/inbox?limit=N&unread=true`
  - [x] Tool: `GET /mail/message/:rowid` (includes body from emlx)
  - [x] Language detection per message (whatlang — sk/en/cs/de)
  - [x] Incremental sync: `POST /mail/sync` → upserts into `mail_cache` (V3 migration), updates `connectors.last_sync_at`; `fetch_tool_context` reads cache first, falls back to live Envelope Index
  - [x] AppleScript body fallback for non-cached IMAP messages (`body_via_applescript` via osascript; requires Automation → Mail)
- [x] `crates/connectors/apple_notes/`:
  - [x] SQLite read path for metadata (title, snippet, folder, dates)
  - [x] JXA body retrieval via `osascript -l JavaScript`
  - [x] Tool: `GET /notes/list?limit=N`
  - [x] Tool: `GET /notes/search?q=...&limit=N`
  - [x] Tool: `GET /notes/:pk` (includes body via JXA)
  - [x] Language detection per note body
  - [x] Locked notes skipped with `is_locked` flag
- [x] `PermissionsManager.swift` — FDA probe, deep-link to System Settings
- [x] Settings → Oprávnenia section: FDA dot + "Udeliť" button
- [x] Settings → Konektory section: Mail + Notes status dots from `/health`
- [x] Privacy gate: `pii: true` field on body responses; system prompt instructs LLM to summarize, not quote raw email
- [x] Daemon `/health` now includes `connectors: { mail, notes }` status
- [x] `ollama pull bge-m3` — pulled by user
- [x] Background sync with progress indicator — "Sync" button in Settings → Konektory; shows spinner + result count
- [x] Slovak email summarization regression test — `sk_email_body_summarization` in `crates/connectors/ollama/tests/diacritics.rs`
- [x] emlx path resolution unit tests — `emlx_shard_calc_*` in `crates/connectors/apple_mail/src/lib.rs` (6 tests, all pass)

---

## Phase 4B — Conversation Persistence + Memory Items ✅ COMPLETE

- [x] Migration `crates/daemon/migrations/V4__sessions_messages_memory.sql`:
  - [x] `sessions` table (id, started_at, ended_at, language, summary, metadata_json)
  - [x] `chat_turns` table (turn_id, session_id, role, content, language, model, created_at, parent_turn_id)
  - [x] `memory_items` table + `memory_fts` FTS5 virtual table (per DATA_MODEL.md spec)
- [x] Daemon `/chat` accepts optional `session_id`; creates one if absent; persists user turn + assistant final reply
- [x] Endpoints: `POST /sessions`, `GET /sessions`, `GET /sessions/{id}/turns`, `DELETE /sessions/{id}`
- [x] Endpoints: `POST /memory` (kind, namespace, text, source_ref?) and `GET /memory?namespace=&q=` (FTS)
- [x] Keep `history` field in `/chat` as fallback for one release; prefer server-loaded history by session_id
- [x] `crates/agent/` new crate — `PromptBuilder` struct with 9-layer assembly; replaces inline prompt code in daemon
- [x] Daemon `chat` handler calls `PromptBuilder::build(session_id, user_turn, lang)` → `Vec<Message>`
- [x] Audit entry on every memory write (`action='memory_save'`) and forget (`action='memory_forget'`)
- [x] Swift `DaemonClient` updated: store/restore session_id; call Sessions + Memory API
- [x] Settings → Pamäť tab: list memory items (grouped by kind), delete

---

## Phase 4C — Vector Memory + Hybrid Retrieval ✅ COMPLETE

- [ ] Load `sqlite-vec` extension at daemon startup (rusqlite `load_extension`; bundle `.dylib` in app resources)
- [x] Migration `crates/daemon/migrations/V5__embeddings.sql`: `embeddings` table
- [x] `crates/memory/` new crate:
  - [x] `embed_and_store(item_id, namespace, text)` — calls `OllamaClient::embed` with `bge-m3`, writes float32 blob
  - [x] `retrieve(query, namespace, k)` → `Vec<MemoryHit>` — BM25 + cosine merged with `0.4*bm25 + 0.6*cos`, recency-decayed
  - [ ] Backfill job: embed existing `memory_items` + `messages` + `notes` on startup if embedding missing
- [x] `PromptBuilder` layer 5: calls `memory::retrieve(user_turn, ["global","user_pref","sk_glossary"], 8)` → `Message::system`
- [x] Per-namespace cap: max 3 retrieved items per namespace to bound prompt size
- [x] `GET /memory/search?q=&namespace=` endpoint for Settings debug view
- [ ] `ollama pull bge-m3` — user must run once before embeddings work

---

## Phase 4D — Self-Improvement / Feedback Loop ✅ COMPLETE

- [x] Explicit capture: scan user turn for trigger phrases (SK: "pamätaj si", "od teraz", "už nikdy", "vždy"; EN: "remember", "from now on", "never", "always") → extract directive via Ollama call → insert `memory_items` kind=`preference` namespace=`user_pref`; ACK in stream: `{"type":"memory_saved","id":...}`
- [x] Implicit capture (background post-turn): spawn task after `done` event; Ollama call classifies `{prev_assistant, user_turn}` → `{is_correction, what_was_wrong, correct_behavior, confidence}`. If `confidence > 0.7`: insert `kind='correction'`
- [x] Slovak glossary corrections: `kind='sk_glossary'` namespace — injected as layer 4 prompt
- [x] Style profile: `kind='style_profile'` row — injected as layer 3 prompt
- [x] `DELETE /memory/{id}` forget endpoint; audit logs `action='memory_forget'`
- [x] Settings → "Naučené preferencie" (Pamäť section): list grouped by kind, delete
- [ ] Settings toggle: disable implicit correction capture
- [x] Dedup guard: reject new item if cosine similarity > 0.92 against existing namespace items
- [x] Weekly auto-prune: remove items not retrieved in 60 days (`MemoryStore::prune()`)
- [x] Test fixtures: `fixtures/sk/style_corrections.yaml`, `fixtures/sk/memory_recall.yaml`

---

## Phase 5 — Rules Engine + Approval Framework ✅ COMPLETE

- [x] `crates/rules/`:
  - [x] YAML loader (`serde_yaml`)
  - [x] Hot-reload via 5 s mtime poll (background tokio task)
  - [x] Matcher — tool name exact match + optional args regex; first-match-wins
- [x] Default `rules.yaml` written to `~/Library/Application Support/bagent/` on first run
- [x] Integrate rules into tool dispatcher — chat handler checks `RuleEngine::check()` before `fetch_tool_context`
- [x] Approval modal (SwiftUI overlay inside ExpandedChatView):
  - [x] Action description + tool name
  - [x] `[Schváliť]` / `[Zamietnuť]` buttons (⌘↩ / Esc shortcuts)
  - [x] 60 s countdown → auto-deny on timeout
- [x] `GET /approvals/pending` — real DB query (pending_approvals table, V7 migration)
- [x] `POST /approvals/:id/decide` — resolves oneshot channel, persists decision
- [x] `/approvals/pending` polling in Swift (1 s interval via `startApprovalPolling`)
- [x] Badge on status item for pending approvals (orange shield badge)
- [x] Orange shield badge in chat header when approvals pending
- [x] Audit entries for every approval decision (`approval_decide`, `approval_timeout`)
- [x] `GET /rules` + `POST /rules` — load/save/hot-validate YAML
- [x] Settings → Pravidlá section: TextEditor + Save button with validation

---

## Phase 6 — Odoo Connector

- [ ] `crates/connectors/odoo/`:
  - [ ] XML-RPC client (`/xmlrpc/2/common`, `/xmlrpc/2/object`)
  - [ ] Version detection
  - [ ] Tool: `odoo_search_contacts`
  - [ ] Tool: `odoo_get_invoice`
  - [ ] Tool: `odoo_list_opportunities`
  - [ ] Tool: `odoo_get_task`
  - [ ] All write tools registered as `Forbidden`
- [ ] Keychain storage for Odoo credentials
- [ ] Settings tab: connection config + test button
- [ ] Privacy filter: `pii_present = true` for Odoo connector
- [ ] Slovak field values preserved verbatim

---

## Phase 7 — Screen Context

- [ ] ScreenCaptureKit frame capture (one-shot, on-demand)
- [ ] Active app via `NSWorkspace`
- [ ] Selected text via Accessibility `AXSelectedText`
- [ ] Vision OCR via `VNRecognizeTextRequest` (on-device)
- [ ] Screen context attached to chat turn
- [ ] Permission prompt: Screen Recording at first use
- [ ] Ephemeral frame policy: no persistence by default
- [ ] `Tool: screen_get_active_app` (auto)
- [ ] `Tool: screen_get_selected_text` (auto after Accessibility)
- [ ] `Tool: screen_capture_frame` (ask per session)
- [ ] Password field exclusion (`AXIsPasswordField`)

---

## Phase 8 — Codex Connector

- [ ] `crates/connectors/codex/`:
  - [ ] Subprocess wrapper for `codex` binary
  - [ ] Sandboxed temp working directory
  - [ ] JSON I/O protocol
  - [ ] 120 s timeout (SIGTERM + SIGKILL)
- [ ] Tool: `codex_run_task` (Ask every time)
- [ ] Diff preview in approval modal
- [ ] Codex binary path configurable in Settings
- [ ] Graceful "not found" message
- [ ] Audit: task description + exit code + diff hash

---

## Phase 9 — Slovak / English Polish

- [ ] Language detector integrated in agent runtime
- [x] Formal Slovak tone prompt template — system prompt in daemon enforces diacritics + formal tone
- [ ] Glossary lock post-processing pass
- [ ] Diacritics regression test suite (50+ sentences, all pass)
- [ ] `Localizable.strings` with Slovak locale
- [ ] Date/number formatting: `sk_SK` locale in summaries
- [ ] Formal greeting/closing enforced in email drafts

---

## Phase 10 — Packaging, Security Hardening, Beta

- [ ] Hardened Runtime enabled
- [ ] Entitlements plist reviewed and minimized
- [ ] `bagentd` universal binary (arm64 + x86_64)
- [ ] Notarization pipeline in CI
- [ ] Sparkle 2.x integration with Ed25519 signature
- [ ] SQLCipher encryption on `bagent.db`
- [ ] Audit log hash-chain verification (`bagentd --verify-audit`)
- [ ] Crash reporter (opt-in)
- [ ] Onboarding flow (permissions, Ollama guide, language pref)
- [ ] Staged rollout config (10% → 50% → 100%)
- [ ] OWASP LLM Top 10 checklist completed (see `SECURITY.md`)
- [ ] Beta `.dmg` distributed to initial test users
- [ ] All `SECURITY.md` Phase 10 checklist items ticked
- [ ] Bundle `sqlite-vec.dylib` universal binary (arm64 + x86_64) in app resources
- [ ] Notarization entitlement review for `load_extension` (rusqlite)

---

## Phase 11 — WhatsApp Connector

- [ ] Spike: compare `whatsapp-web.js` bridge (QR-pair, individual) vs Meta Cloud API (Business). Document in `docs/spikes/whatsapp.md`
- [ ] `crates/connectors/whatsapp/`: read chats, list contacts, fetch message history, draft send (approval-gated)
- [ ] Schema reuses `messages` table (`source='whatsapp'`)
- [ ] Tool `whatsapp_send_message` — `ApprovalLevel::Ask` always
- [ ] Slovak diacritics preserved through bridge encoding (UTF-8 contract test)
- [ ] Settings → Konektory → WhatsApp: QR-pair flow, connected status indicator
- [ ] Memory integration: contacts + conversations vectorized for semantic queries ("kde mi písal Peter o faktúre")
- [ ] Onboarding warning: unofficial bridge risks (account ban, session expiry)

---

## Phase 12 — Claude Code Connector

- [ ] `crates/connectors/claude_code/`: subprocess wrapper for `claude` binary
- [ ] Tool `claude_code_run_task` — `ApprovalLevel::Ask`, side_effect `CodeWrite`
- [ ] Sandboxed temp working directory per invocation; user provides repo path explicitly
- [ ] Diff preview reuses Codex approval modal (Phase 8)
- [ ] Settings: Claude Code binary path + model preference
- [ ] Anthropic API key stored in Keychain under `bagent.claude_code.apikey`; never logged; privacy filter applied
- [ ] Audit: task description, args, diff hash, exit code
- [ ] Model router: long-context refactor tasks → route to Claude Code over Codex when available

---

## Phase 13 — Universal Computer Access

- [ ] `crates/connectors/macos_control/`:
  - [ ] `ui_inspect` (`Auto`): read AX tree of frontmost app
  - [ ] `ui_click(element_id)` / `ui_type(element_id, text)` (`Ask` per session)
  - [ ] `applescript_run(script)` (`Ask` every time, no LLM-generated scripts)
  - [ ] `shell_exec(cmd)` (`Ask` every time; `sudo` always `Forbidden`)
  - [ ] `file_open_with(path, app)` (`Auto`)
- [ ] Permissions: Accessibility, per-app Automation, additional FDA prompts
- [ ] Approval modal: app name, target element, action description, dry-run where possible
- [ ] Session-scoped allow-lists stored in `approvals` table (`expires_at = session end`)
- [ ] Memory integration: store learned app workflows as replayable macros (approval required on replay)
- [ ] Audit: target element id + AX path + screenshot hash (no raw screenshot stored)
- [ ] Kill switch: menu bar item immediately revokes all active session-scoped permits
- [ ] Forbidden list: `sudo`, `rm -rf`, password fields (`AXIsPasswordField`), Keychain paths, system files
- [ ] Hard per-minute action budget (default 20 actions/min); configurable in Settings
