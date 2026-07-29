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
- [ ] Odoo JSON-RPC handshake:
  - [x] Authenticate via `common.authenticate` → `uid` (Phase 6 connector built + unit-tested)
  - [x] Read `account.move` invoices + `helpdesk.ticket` + `res.partner` (live `#[ignore]` tests)
  - [x] Version detection via `common.version` service call
  - [ ] Document in `docs/spikes/odoo.md` (deferred)
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

- [x] `NotchWrapShape.swift` — SwiftUI `Shape` with animatable `wingWidth` + `bridgeHeight`
- [x] Geometry: compute left/right wing rects from `auxiliaryTopLeft/RightArea`; `pillFrame` becomes union rect spanning both wings + notch gap + bridge room
- [x] Replace `PillView` notch branch (`ChatView.swift`) with `NotchWrapView` — sparkles icon left, chevron icon right, no title text
- [x] Hover state: wings expand 32 pt → 96 pt, bridge fades in, subtle white stroke on shape
- [x] `hoverChanged(isHovered:)` callback from SwiftUI → `NotchWindowController` to drive `setFrame` in sync with SwiftUI layout
- [x] Click / `⌥Space`: redesigned 3-phase expand animation (Phase A wings spread → Phase B bridge drops → Phase C content fades in)
- [x] Collapse: reverse animation, anchored at notch top-center
- [x] Hit-test via `.contentShape(NotchWrapShape(...))` — notch cutout stays click-through
- [x] Visual QA on M1/M2/M3/M4/M5 notch geometries (inner corner radius match)
- [x] Update `docs/spikes/notch_geometry.md` with per-model notch corner radii
- [x] `docs/UI_DESIGN.md` — notch wrap anatomy, animation language, iconography slots, reduced-motion fallback

---

## Phase 1B — Spotlight Input + Source Modes ✅ IMPLEMENTED

- [x] Input-only chat surface opens from notch/status click when no assistant output is generating
- [x] Voice shortcut behavior: voice enabled keeps single `⌥Space` for voice; double `⌥Space` opens input; voice disabled opens input directly
- [x] Send collapses input back to notch and keeps blue thinking status until first assistant token
- [x] First assistant token opens full chat panel automatically; thinking-stage notch click can open chat manually
- [x] Source bubbles: Mail, Files, WhatsApp, Odoo; local use-count ordering; hover placeholder updates; `⌘1`-`⌘4` selection
- [x] `source_mode` request hint reaches daemon planning before tool approval/routing
- [x] Liquid-glass-style fallback material for current macOS SDK; native Liquid Glass remains gated for future SDK adoption
- [ ] Manual QA on notch and non-notch displays: idle open, send collapse, first-token expansion, Cmd source reveal, reduced motion

---

## Phase 2 — Rust Backend + IPC ✅ COMPLETE

- [x] Cargo workspace at repo root (`Cargo.toml`)
- [x] `crates/daemon/` (`bagentd`): axum 0.7 server on `127.0.0.1:0`
  - [x] Write port to `~/Library/Application Support/bagent/daemon.port`
  - [x] Generate bearer token on first run; written to `daemon.token` (Keychain: Phase 10)
  - [x] `GET /health` endpoint (checks BaseRT up/down, returns model)
  - [x] `POST /chat` — typed SSE events backed by BaseRT OpenAI-compatible streaming
  - [x] Bearer token auth middleware
- [x] SQLite with refinery migrations (`migrations/V1__initial.sql`, `V2__full_schema.sql`)
  - [x] Schema: `audit_entries`, `approvals`, `messages`, `sessions`, `connectors`
- [x] Swift `DaemonClient` — `DaemonClient.swift`
  - [x] Read port + token from files on app launch (40 × 100 ms retry)
  - [x] SSE streaming client via `URLSession.bytes(for:)`
  - [x] `healthStatus()` → `DaemonHealth` (daemon up, BaseRT up, model)
- [x] `DaemonLauncher.swift` — auto-restarts on crash, max 3/min rolling window
- [x] Audit entry on every chat request (SQLite `audit_entries`)
- [x] Settings tab: daemon + BaseRT status indicator with live indicator dots

---

## Phase 3 — BaseRT Integration ✅ COMPLETE

- [x] `crates/connectors/basert/` — standalone library crate (`BaseRTClient`)
  - [x] OpenAI-compatible `/v1/models` and `/v1/chat/completions`
  - [x] Bearer-authenticated SSE content and fragmented tool-call parsing
  - [x] 2 s health check and explicit model unload
- [x] Daemon uses `BaseRTClient` for all BaseRT I/O
- [x] Embeddings disabled until a dedicated BaseRT embedding model is configured
- [x] System prompt — Slovak business assistant: formal tone, diacritics enforced, legal terms never translated
- [x] Context window management:
  - [x] Sliding hard truncation (last 40 messages) for moderate histories
  - [x] Automatic summarisation when history > 60 messages (old turns → single summary system message)
- [x] Model router: all requests → BaseRT; client-supplied `model` field overrides default
- [x] Model picker in `SettingsView.swift` — fetches live from `/models`, persists to UserDefaults
- [x] Default model: `basecompute/Qwen3-4B-Instruct-2507`
- [x] BaseRT up/down in `GET /health`
- [x] BaseRT protocol and ignored live regressions in `crates/connectors/basert/tests/`
- [x] Streaming tokens appear in UI (TTFT < 1 s on warm BaseRT)

---

## Phase 4 — Read-Only Apple Mail + Notes ✅ COMPLETE

- [x] `crates/connectors/apple_mail/`:
  - [x] Read `Envelope Index` SQLite (readonly WAL, busy_timeout 2s)
  - [x] Parse `.emlx` body (mailparse crate — QP decode + MIME multipart + HTML strip)
  - [x] Tool: `GET /mail/inbox?limit=N&unread=true`
  - [x] Tool: `GET /mail/message/:rowid` (includes body from emlx)
  - [x] Language detection per message (whatlang — sk/en/cs/de)
  - [x] Incremental sync: `POST /mail/sync` → upserts into `mail_cache` (V3 migration), updates `connectors.last_sync_at`; `fetch_tool_context` reads cache first, falls back to live Envelope Index
  - [x] AppleScript body fallback for non-cached IMAP messages (`MailConnector::hydrate_message` via osascript; requires Automation → Mail)
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
- [x] `basert pull bge-m3` — pulled by user
- [x] Background sync with progress indicator — "Sync" button in Settings → Konektory; shows spinner + result count
- [x] Slovak email summarization regression test — `sk_email_body_summarization` in `crates/connectors/basert/tests/diacritics.rs`
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
  - [x] `embed_and_store(item_id, namespace, text)` — calls `BaseRTClient::embed` with `bge-m3`, writes float32 blob
  - [x] `retrieve(query, namespace, k)` → `Vec<MemoryHit>` — BM25 + cosine merged with `0.4*bm25 + 0.6*cos`, recency-decayed
  - [ ] Backfill job: embed existing `memory_items` + `messages` + `notes` on startup if embedding missing
- [x] `PromptBuilder` layer 5: calls `memory::retrieve(user_turn, ["global","user_pref","sk_glossary"], 8)` → `Message::system`
- [x] Per-namespace cap: max 3 retrieved items per namespace to bound prompt size
- [x] `GET /memory/search?q=&namespace=` endpoint for Settings debug view
- [ ] `basert pull bge-m3` — user must run once before embeddings work

---

## Phase 4D — Self-Improvement / Feedback Loop ✅ COMPLETE

- [x] Explicit capture: scan user turn for trigger phrases (SK: "pamätaj si", "od teraz", "už nikdy", "vždy"; EN: "remember", "from now on", "never", "always") → extract directive via BaseRT call → insert `memory_items` kind=`preference` namespace=`user_pref`; ACK in stream: `{"type":"memory_saved","id":...}`
- [x] Implicit capture (background post-turn): spawn task after `done` event; BaseRT call classifies `{prev_assistant, user_turn}` → `{is_correction, what_was_wrong, correct_behavior, confidence}`. If `confidence > 0.7`: insert `kind='correction'`
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

## Phase 5B — Chat Attachments ✅ COMPLETE

- [x] `ChatView.swift` input bar: `plus.circle` button opens `NSOpenPanel` (images, PDF, text/source)
- [x] `ChatViewModel.swift`:
  - [x] `ChatAttachmentKind` enum (`image`, `pdf`, `text`, `other`)
  - [x] `ChatAttachment` struct: `id, filename, mime, kind, localURL, sizeBytes, thumbnail?`
  - [x] `@Published var pendingAttachments: [ChatAttachment] = []`
  - [x] Extend `ChatMessage` with `attachments: [ChatAttachment] = []`
  - [x] Upload flow: `POST /attachments` (multipart), store returned id
  - [x] Pass `attachment_ids: [String]` in `/chat` request
- [x] Render attachment chips above input bar (filename + remove ×); limit 5 per turn
- [x] Render image thumbnails + paperclip chips in message bubbles
- [x] Drag-and-drop files into open conversation (`onDrop` on `ExpandedChatView`)
- [x] Drag-and-drop onto collapsed notch pill → expand + attach
- [x] Daemon — `POST /attachments` (multipart, axum):
  - [x] Content-addressed storage `~/Library/Application Support/bagent/attachments/{sha256}.{ext}`
  - [x] Dedup by sha256; returns `{attachment_id, mime, size, sha256, kind}`
  - [x] `ChatRequest` extended with `attachment_ids: Vec<String>`
- [x] `crates/attachments/`: extraction pipeline
  - [x] `text/*`, `*.md`, source files → UTF-8 read (truncated to 8 000 chars)
  - [x] `application/pdf` → `pdftotext` / `textutil` fallback
  - [x] `image/*` → store path, flag `requires_vision: true`
- [x] `PromptBuilder::build` gains `attachments_ctx: Option<String>` — Layer 6.5 between tool data and session summary
- [x] BaseRT `Message` extended with `images: Vec<String>` (base64, skip_serializing_if empty)
- [x] Auto-route to `qwen2.5vl:7b` when any attachment `kind=image` and no explicit model override; audit `model_swap`
- [x] Migration V8: `attachments` + `chat_turn_attachments` link table
- [x] Settings → BaseRT: vision model status indicator + pull hint
- [x] Privacy: `pii: true` on attachment-derived context; max 20 MB per file
- [x] Onboarding: first image attachment triggers one-time alert if vision model not installed
- [x] Resize glitch fixed: removed `Task { @MainActor }` hop in `NotchWindowController.swift`; `.regularMaterial` swapped for solid color during active drag; `layerContentsRedrawPolicy = .onSetNeedsDisplay` on chat hosting view
- [ ] `basert pull qwen2.5vl:7b` — in progress (large model ~6GB)

---

## Phase 5C — Apple Mail Attachments + Vision Routing ✅ COMPLETE

- [x] `crates/connectors/apple_mail/src/lib.rs`:
  - [x] `MailAttachment { filename, mimetype, size, part_index, content_id }`
  - [x] `extract_attachments_from_parsed()` — walks MIME tree, detects non-body parts
  - [x] `MailMessage` extended with `attachments: Vec<MailAttachment>`
  - [x] `get_message` populates attachments alongside body
  - [x] `get_message_attachment(rowid, idx)` → raw bytes; `_base64` variant for JSON
- [x] New daemon routes:
  - [x] `GET /mail/message/:rowid/attachments` → list metadata
  - [x] `GET /mail/message/:rowid/attachments/:idx` → base64-encoded bytes + metadata
- [x] Migration V9: `mail_attachments(message_rowid, idx, filename, mime, size)`
- [x] `mail_message` response includes `attachments` field in JSON
- [x] Vision route: image mail attachment → auto-route to vision model (same logic as 5B)
- [x] Mail attachment chips rendered identically to chat attachments (`AttachmentStrip`)
- [x] Test fixtures:
  - [x] `fixtures/sk/mail_with_pdf_invoice.eml` — Slovak invoice PDF; test asserts DPH/IBAN in body
  - [x] `fixtures/sk/mail_with_image_receipt.eml` — JPEG receipt; test asserts vision-route triggers
  - [x] 4 new unit tests in `crates/connectors/apple_mail/src/lib.rs` (all pass)

---

## Phase 5D — LLM-Driven Mail Search (supersedes 5C heuristics)

- [x] `crates/agent/src/mail_intent.rs` — `MailIntent` struct + `MailIntentClassifier`
  - [x] `action`: "list_recent" | "search" | "read_attachment" | "none"
  - [x] Structured fields: `sender`, `subject`, `date` (ISO), `keywords`, `wants_attachment`
  - [x] LLM prompt includes today's date; normalizes Slovak "DD.MM.YYYY" → ISO
  - [x] `unwrap_or_default()` fallback to `action:"none"` on parse failure
- [x] `crates/connectors/apple_mail/src/lib.rs` — `MailSearchFilter` + `search_messages()`
  - [x] Dynamic SQL WHERE over Envelope Index (sender LIKE, subject LIKE, date range)
  - [x] `ORDER BY date_received DESC LIMIT n`
- [x] `crates/daemon/src/main.rs` — rewrite mail branch of `fetch_tool_context`
  - [x] `parse_date_to_range("YYYY-MM-DD")` → day-boundary unix epoch `(start, end)`
  - [x] Classifier-driven dispatch: none / list_recent / search / read_attachment
  - [x] `search`: `MailSearchFilter` from intent; best-effort keyword filter on cached bodies
  - [x] `read_attachment`: search → `get_message_attachment` → PDF text extraction
  - [x] Removed `extract_subject_hint` heuristic
  - [x] Injected context header tells LLM to state plainly when mail not found
- [x] `crates/agent/src/prompt.rs` — persona reinforced: never invent mail contents
- [ ] Unit tests:
  - [ ] `parse_date_to_range("2026-06-10")` → correct `[start, end)` bounds
  - [ ] `MailIntent` deserializes documented JSON shapes incl. `action:"none"` and `action:"open"`
  - [ ] `search_messages` filter combos (sender-only, subject+date, empty)
  - [ ] Classifier round-trip (`#[ignore]`, needs live BaseRT)
  - [ ] `MailMessage.message_id` extracted from fixture emlx file

---

## Phase 5E — Mail-Open + AeroSpace Window Control

### Mail identity + open
- [x] `MailMessage.message_id: Option<String>` — extract from emlx top-level headers in `parse_emlx_body_and_attachments` (`crates/connectors/apple_mail/src/lib.rs`)
- [x] `apple_mail_connector::open_message(message_id, subject, sender)` — AppleScript; primary path `whose message id is`, fallback subject+sender search across all mailboxes
- [x] `MailIntent` gains `action="open"` — LLM classifier prompt updated (`crates/agent/src/mail_intent.rs`)
- [x] `MailRef { rowid, message_id, subject, sender }` struct in daemon; `fetch_tool_context` returns `(ctx, pdf_paths, Option<MailRef>)` (`crates/daemon/src/main.rs`)
- [x] `"open"` dispatch in `fetch_tool_context`: search → enrich → call `open_message()` in background task; set `found_mail_ref`
- [x] SSE event `{"type":"mail_found", rowid, message_id, subject, sender}` — emitted early (before tokens) when a mail was found (`crates/daemon/src/main.rs`)
- [x] `POST /mail/open` endpoint — resolves message_id from rowid via emlx if needed, then calls `open_message()` (`crates/daemon/src/main.rs`)
- [x] `DaemonClient.MailRef`, `ChatEvent.mailFound`, SSE decode, `openMail()` function (`apps/macos/Sources/bagent/DaemonClient.swift`)
- [x] `ChatMessage.mailRef: MailRef?` — set on `.mailFound` event (`apps/macos/Sources/bagent/ChatViewModel.swift`)
- [x] `ChatViewModel.openMail(_ ref:)` — calls `DaemonClient.openMail`
- [x] `MailOpenButton` — 28 pt circle → hover spring-morphs to 150 pt rounded rect; envelope icon slides left; "Otvoriť mail" text fades+slides in (`apps/macos/Sources/bagent/ChatView.swift`)
- [x] `MailOpenButton` shown above `MessageContentView` in `MessageBubble` when `message.mailRef != nil`
- [ ] Test: ask "nájdi email od X a otvor ho" → Mail.app opens the message; button appears above answer

### AeroSpace window management
- [x] `WindowIntent { action, workspace, app }` + `WindowIntentClassifier` — new file `crates/agent/src/window_intent.rs`; exported from `crates/agent/src/lib.rs`
- [x] `find_aerospace_binary()` — resolves via `which` then `/Applications/AeroSpace.app` fallback
- [x] `run_aerospace(args)` — `tokio::process::Command`, silent degrade on error
- [x] `run_aerospace_intent(intent)` — maps actions: `focus_workspace`, `open_app` (open + poll + move), `move_app`, `focus_app`; `app_to_bundle_id()` helper
- [x] Keyword gate in `fetch_tool_context` ("plochu", "prepni", "presuň", "zameraj"); runs `WindowIntentClassifier` → `run_aerospace_intent`; appends SK confirmation to context parts
- [ ] Test: "prepni na plochu 3" → workspace focuses; "otvor mail na ploche 1" → Mail opens on ws 1; silent degrade when AeroSpace not running

---

## Phase 5F — Conversational Entity & Coreference Resolution ✅ COMPLETE

Classifiers previously saw only the current user turn. Pronoun references across turns (SK "od nej" → "Katarína Horváthová" from a prior turn) were silently lost, causing searches to return wrong or empty results.

- [x] `format_history_snippet(history, max_turns)` — last 4 turns, 200 chars/turn cap; `[User]`/`[Assistant]` labels (`crates/daemon/src/main.rs`)
- [x] `fetch_tool_context` receives `history: &[Message]`; builds snippet before classifiers run
- [x] `MailIntentClassifier::classify(user_turn, context)` — prepends context block + coreference instruction to LLM prompt (`crates/agent/src/mail_intent.rs`)
- [x] `WindowIntentClassifier::classify(user_turn, context)` — same treatment (`crates/agent/src/window_intent.rs`)
- [ ] Unit tests: SK pronoun-resolution fixtures ("od nej" → resolved sender from prior turn)

---

## Phase 5G — Voice Input (removed)

Shipped, then removed in the notch-only refactor — the WhisperKit dependency,
`SpeechController`, voice overlay and inline mic no longer exist. `⌥Space` opens
the notch text input.

---

## Phase 6 — Odoo Connector (JSON-RPC) ✅ SUPERSEDED BY PHASE 6B

Original JSON-RPC implementation complete and unit-tested (see git history).
Superseded by Phase 6B — MCP transport. All items below were checked before the switch.

- [x] `crates/connectors/odoo/` — JSON-RPC transport (Odoo 18, now replaced)
- [x] `crates/agent/src/odoo_intent.rs` — `OdooAction` enum + `OdooIntentClassifier` (unchanged, reused in 6B)
- [x] Daemon integration — routes, SSE, coreference (updated in 6B)
- [x] `apps/macos/Sources/bagent/KeychainStore.swift` — Keychain helpers (unchanged)
- [x] Slovak field values preserved verbatim (IČO, DIČ, DPH, faktúra, tiket)
- [x] `skills/odoo-readonly/SKILL.md` — read-only; Forbidden write rules kept in 6B
- [ ] `docs/spikes/odoo.md` — MCP topology docs (moved to Phase 6B)

---

## Phase 6B — Odoo via MCP server (rmcp client) ✅ IMPLEMENTED

Rewired the Odoo connector to use the installed `mcp_server` Odoo module instead of direct JSON-RPC.
Daemon spawns `uvx mcp-server-odoo` as a child process and speaks MCP over stdio via `rmcp 1.8`.

### Connector (`crates/connectors/odoo/`)
- [x] `rmcp = "1.8"` dependency added (`features = ["client", "transport-child-process"]`)
- [x] `src/mcp.rs` (new): `find_uvx()` (PATH + common macOS locations), `spawn_mcp()`, `extract_text()`, `extract_first_id()`, `extract_first_name()` helpers; 6 unit tests
- [x] `src/lib.rs` rewritten: `OdooConnector` owns `McpClient` (running subprocess); `connect_with_uvx()` — find uvx → spawn (90 s timeout) → verify creds via `search_records(res.users)` → uid; `search_partners`, `my_invoices`, `my_helpdesk_tickets`, `get_record` all route through `call_mcp()` → return `OdooMcpResult { text, model, first_id, first_name }`
- [x] `src/types.rs`: added `OdooError::McpUnavailable` (distinct from `Auth` — shows "install uv/uvx" hint); added `OdooMcpResult`; kept `Partner`/`Invoice`/`HelpdeskTicket`/`M2O`/`false_or_*` for deserialization tests
- [x] `src/json_rpc.rs` deleted (JSON-RPC envelope no longer needed)
- [x] API key flows exclusively via child env — **never** written to disk or command line
- [x] uid is resolved via MCP (`search_records res.users`) — used in `my_invoices` + `my_helpdesk_tickets` domains
- [x] 11 unit tests pass (6 mcp.rs + 5 lib.rs)

### Daemon (`crates/daemon/src/main.rs`)
- [x] `OdooConfigReq` (new) — accepts optional `uvx_path` field alongside credentials
- [x] `odoo_config_handler`: uses `connect_with_uvx()`; returns `mcp_available` + `tool_count`; `McpUnavailable` → HTTP 503 (not 401)
- [x] `odoo_status_handler`: returns `mcp_available: bool` + `tool_count`
- [x] `fetch_tool_context` match arms simplified: inject MCP text directly, extract ref from `first_id`/`first_name`
- [x] Known v1 gap documented: if stdio child dies, calls fail until reconfigure (no auto-respawn)

### Swift (`apps/macos/Sources/bagent/`)
- [x] `DaemonClient.swift`: `OdooConfigResult` + `OdooStatusResult` extended with `mcp_available` + `tool_count`; `odooConfigure()` accepts optional `uvxPath`
- [x] `ChatViewModel.swift`: `odooUvxPath` @Published property (UserDefaults); `odooMcpAvailable` + `odooToolCount` state; `configureOdoo()` passes uvxPath + shows MCP-specific error messages; **`restoreOdooFromKeychain()` wired** at daemon-ready in `refreshHealth()` (was dead code — now called on every launch)
- [x] `SettingsView.swift`: MCP status row (server.rack icon + tool count); uvx path TextField; updated test result message format; "first run may take a minute" hint

### Pending / Verification
- [ ] **Step 1 (do first):** run `uvx mcp-server-odoo` against the live MCP-enabled Odoo instance, inspect `CallToolResult` shape (text vs structured), verify uid extraction, confirm field richness — decides whether text injection is enough or typed parsing needed
- [ ] End-to-end chat test (Slovak): "nájdi kontakt", "ukáž moje faktúry", "moje tikety" → live data + "Otvoriť v Safari" button
- [ ] Restart test: creds restored from Keychain without re-entering (via `restoreOdooFromKeychain` wiring)
- [ ] API key not on disk: `grep -r api_key ~/Library/Application\ Support/bagent/` → empty
- [ ] `docs/spikes/odoo.md` — MCP topology, env vars, tools, uvx-PATH gotcha

---

## Phase 7 — Screen Context ✅ COMPLETE

- [x] `ScreenContextProvider.swift` — `@MainActor` class; ScreenCaptureKit one-shot capture → in-memory PNG base64; downscale ≤1568px; Vision OCR (`sk-SK`+`en-US`); active app via `NSWorkspace`; AX selected text (`kAXSecureTextFieldSubrole` password exclusion)
- [x] `crates/agent/src/screen_intent.rs` — `ScreenIntent` + `ScreenIntentClassifier`; `ScreenAction { None, View, Analyze, Read, Find }`; `wants_screen/wants_ocr/wants_selection` fields; uses `qwen2.5:0.5b` via `generate_json`; 4 unit tests
- [x] `crates/agent/src/context_planner.rs` — `is_screen_context(low)` gate (SK+EN keywords) placed before `is_file_search`; `task_type:"screen_context"` returned; `"screen_context"` added to LLM fallback task-type enum; `screen-context` skill candidate
- [x] `crates/daemon/src/main.rs`:
  - [x] `ChatRequest` extended with `screen_image_b64`, `screen_ocr_text`, `active_app`, `selected_text` (`#[serde(default)]`)
  - [x] In-memory injection block: pushes `screen_image_b64` into `images_b64`, injects app/selection/OCR as `ctx_parts`, triggers `model_swap` audit (reason: `screen_context`) → routes to `qwen2.5vl:7b`
  - [x] `POST /screen/intent` route → `ScreenIntentClassifier`; graceful degrade on error
- [x] `apps/macos/Sources/bagent/DaemonClient.swift`:
  - [x] `ScreenContextFields` + `ScreenIntentResponse` structs
  - [x] `chatStream` extended with `screenContext: ScreenContextFields?` param
  - [x] `screenIntent(message:)` async method → `POST /screen/intent`
- [x] `apps/macos/Sources/bagent/ChatViewModel.swift`:
  - [x] Screen pre-gate `looksLikeScreenTurn(_:)` (static, SK+EN keywords)
  - [x] `send()` calls `/screen/intent` when pre-gate passes → captures via `ScreenContextProvider` → passes `screenContext` to `chatStream`
- [x] `PermissionsManager.swift` — `hasScreenRecording` (CGPreflightScreenCaptureAccess) + `requestScreenRecording()` + `openScreenRecordingSettings()`; `hasAccessibility` (AXIsProcessTrusted) + `requestAccessibility()` + `openAccessibilitySettings()`
- [x] `SettingsView.swift` — Screen Recording + Accessibility rows in permissionsSection (dot + label + "Udeliť" button + explanation text)
- [x] `Info.plist` + `Makefile` — `NSScreenCaptureUsageDescription` added to both
- [x] `Package.swift` — `ScreenCaptureKit`, `Vision`, `ApplicationServices` frameworks linked
- [x] `skills/screen-context/SKILL.md` — rules: no invented UI content, pii:true, password exclusion, vision over OCR
- [x] Notch idle polish — collapsed idle = blank (no sparkles, no dot); dot visible when chat open or status != .ready (error always shown); sparkles visible on hover/voice/expanded

### Phase 7 deferred items
- [ ] Manual QA: `make bundle && open bagent.app` — grant Screen Recording + Accessibility in Settings; ask "čo je na obrazovke?" → vision model answers; "prečítaj výber" → AX selection used; verify no file written under `~/Library/Application Support/bagent/attachments` for screen frames
- [ ] Image paste QA: ⌘V with image in clipboard → `[image #1]` token + chip; send → thumbnail in bubble
- [ ] Unit tests: `is_screen_context` keyword combos; `ScreenIntent` JSON deserialisation incl. `action:"none"` (4 already in screen_intent.rs)
- [ ] Live BaseRT classifier round-trip test (`#[ignore]`)
- [ ] `basert pull qwen2.5vl:7b` — required for vision analysis (user must run once)

---

## Phase 8 — Codex Connector ✅

- [x] `crates/connectors/codex/` — subprocess wrapper with `--sandbox read-only`, stdin prompt, 120 s timeout (SIGTERM + SIGKILL), SHA-256 output hash
- [x] Deterministic task rater (`crates/agent/src/task_rater.rs`) — bilingual SK/EN keyword gates, 5 levels (LocalOnly → CodexRequired), 28 tests
- [x] Tool rule: `codex.run_task` → Ask (never Auto) in rules engine + YAML
- [x] Daemon routes: `GET /codex/status`, `POST /codex/rate-task`, `POST /codex/run-task`
- [x] Context packet privacy model — forbidden list enforced; user approves before dispatch
- [x] Approval-gated via existing REST poll modal (`pending_approvals` table + oneshot)
- [x] Codex binary path configurable in Settings → Codex section
- [x] Graceful "codex_not_found" degradation
- [x] Audit: description + level + privacy_risk + exit_code + timed_out + output_hash (no raw bodies)
- [x] Chat SSE: `task_rating` event emitted for CodexCandidate+ turns
- [x] Swift: `CodexRatingBadge` in `MessageBubble`; `testCodex()` in SettingsView
- [x] Skill file: `skills/codex-advanced-task/SKILL.md`
- [x] Docs updated: `ARCHITECTURE.md`, `DATA_MODEL.md`

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
- [ ] Onboarding flow (permissions, BaseRT guide, language pref)
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

## Phase 13A — Safe Local File + App Access ✅

- [x] `crates/connectors/filesystem/` crate: `PathPolicy`, `search_files`, `read_text`, `metadata`, `open` (argv-only + async exec)
- [x] `PathPolicy::default_for_user_home()`: allowed roots (Desktop/Documents/Downloads/Pictures/Movies/Music/iCloud Drive), denied roots (.ssh/.gnupg/Keychains/1Password/Bitwarden/Chrome/Brave/Firefox/.Trash/system dirs)
- [x] `DANGEROUS_EXTENSIONS` list — blocks .app/.sh/.py/.scpt/.pkg/.dmg etc. from open
- [x] `open.rs`: pure `build_*_argv` functions (test-safe) + async exec via `/usr/bin/open` only, never `sh -c`
- [x] `search.rs`: WalkDir walk, filename/content/path scoring, Slovak diacritics, binary skip, 500-char line truncation
- [x] `crates/agent/src/file_intent.rs`: `FileIntent`/`FileAction` + `FileIntentClassifier` (BaseRT JSON, SK/EN few-shots)
- [x] `crates/rules/src/lib.rs`: filesystem/macos rules (auto/ask/forbidden)
- [x] `crates/agent/src/context_planner.rs`: `is_file_search()` (placed after `is_mail_search`), file skill names
- [x] `crates/agent/src/prompt.rs`: `PromptTrace` file_* fields
- [x] `crates/daemon/src/main.rs`: `AppState.fs`, 10 new routes, handlers, `fetch_tool_context` file branch, `FileActionPlan`, `save/load_last_file_ref`, `merge_session_metadata`, SSE `file_found`/`file_opened` events
- [x] `skills/file-search/SKILL.md`, `skills/file-open/SKILL.md`, `skills/app-open-control/SKILL.md`
- [x] `apps/macos/Sources/bagent/DaemonClient.swift`: `FileRef`, `FileSearchRequest/Response/Result`, SSE event cases, REST methods
- [x] `apps/macos/Sources/bagent/ChatViewModel.swift`: `ChatMessage.fileRef`, switch cases
- [x] `apps/macos/Sources/bagent/SettingsView.swift`: Full Disk Access copy updated for file search
- [x] All workspace tests pass (`cargo test --workspace`)
- [x] Workspace and Swift build clean
- [ ] Chat UI file cards (Open/Reveal/Open With buttons) — deferred
- [ ] PDF/docx/xlsx content indexing during bulk walk — deferred (filename-only in v1; on-demand via read_text)
- [ ] Interactive Ask approval on direct REST open routes — deferred (Ask via chat path; REST returns 409)

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

---

## Scheduled Automations (2026-07) — DONE

Implemented end to end (see `docs/AUTOMATIONS.md`; issues #1–#14):

- [x] `bagent-automations` crate: typed schedules, IANA-zone recurrence, DST policies, validation, catch-up window
- [x] launchd daemon residency — scheduling continues after the app exits
- [x] shared agent execution service (`agent_exec.rs`): one loop for chat + automations, unattended write gating, fail-closed unknown tools
- [x] persistence + typed CRUD API (`V13`), approval provenance (`V14`), 50-run retention with audited cleanup
- [x] daemon scheduler: atomic claims, 24h single catch-up, overlap/stale skip records, restart recovery, 2-run concurrency
- [x] daemon-wide `/events` SSE + background approval preemption in the notch
- [x] `/automations` notch surface: list, detail (runs + full-output reuse), step editor for once/hourly/daily/weekday/weekly
- [ ] follow-ups: live manual validation of sleep/wake + unattended write approval paths; per-automation model selection; fixed-phase every-N-hours anchoring
