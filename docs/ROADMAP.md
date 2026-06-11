# Roadmap

Each phase follows the structure: **Goal · Deliverables · Risks · Acceptance Criteria**.

---

## Phase 0 — Research / Spike

**Goal:** Validate key unknowns before committing to the architecture. No shipping code.

**Deliverables:**
- [ ] Notch geometry measurements per device class (14", 16" MBP, non-notch fallback).
- [ ] Prototype `NSPanel` anchored to notch region; verify z-ordering and Mission Control behaviour.
- [ ] ScreenCaptureKit cost benchmark: frame capture CPU/memory at 1 fps vs 5 fps.
- [ ] Ollama latency benchmark: `qwen2.5:7b` first-token latency on M1/M2/M3; Slovak diacritics roundtrip test.
- [ ] Apple Mail SQLite schema snapshot (`Envelope Index`); identify message body path pattern for `.emlx`.
- [ ] Apple Notes SQLite schema snapshot (`NoteStore.sqlite`); AppleScript read test.
- [ ] Odoo XML-RPC handshake test against a sandbox instance.
- [ ] Document findings in `docs/spikes/`.

**Risks:**
- Notch geometry changes across OS updates — hardcode insets per device model string with fallback.
- Ollama may be too slow for interactive use on older hardware — define minimum acceptable latency (< 800 ms TTFT).
- Apple Mail DB schema undocumented and may change — build adapter layer with version detection.

**Acceptance Criteria:**
- [ ] NSPanel appears correctly anchored to notch on at least one physical device.
- [ ] Ollama produces correct `Dobrý deň` and `č š ž ľ ť ď` in output without corruption.
- [ ] At least one Mail message body successfully extracted from `.emlx`.
- [ ] Written spike notes committed under `docs/spikes/`.

---

## Phase 1 — Notch UI Shell

**Goal:** Working macOS app with notch panel UI, global hotkey, and stub chat interface.

**Deliverables:**
- [ ] `apps/macos/` Xcode project scaffolded.
- [ ] `NSStatusItem` (fallback) + notch `NSPanel` (primary).
- [ ] Global hotkey `⌥Space` toggles panel.
- [ ] Chat UI: `TextEditor` input, placeholder output area, send button.
- [ ] Panel collapses to pill on dismiss; animates expand/collapse.
- [ ] App sandboxing config + entitlements plist (minimal set for Phase 1).
- [ ] Dark mode support.

**Risks:**
- `NSPanel` at `.mainMenu + 1` level may interfere with Spotlight, system panels — test carefully.
- Accessibility permission prompt at wrong time confuses users — defer until Phase 7.
- SwiftUI + AppKit mixing can cause layout bugs — establish a clear AppKit-hosts-SwiftUI boundary early.

**Acceptance Criteria:**
- [ ] `⌥Space` shows/hides panel from any app.
- [ ] Panel displays correctly under notch on MacBook Pro (notch device) and in menu-bar area on non-notch Mac.
- [ ] Typing in input field does not steal focus from underlying app when panel is dismissed.
- [ ] App passes `codesign --verify` and launches without system alerts.

---

## Phase 2 — Rust Backend + IPC

**Goal:** Daemon process running alongside the app, communicating over local HTTP/SSE.

**Deliverables:**
- [ ] `crates/daemon/` Cargo workspace scaffolded.
- [ ] `axum` server on `127.0.0.1:0`; port + bearer token written to `~/Library/Application Support/bagent/`.
- [ ] Endpoints: `GET /health`, `POST /chat` (stub echo), `GET /approvals/pending`.
- [ ] Swift `DaemonClient` reading port/token; calling `/health` on startup.
- [ ] App launches daemon binary as child process; restarts on crash.
- [ ] SQLite DB initialized with schema migrations via `refinery`.
- [ ] `audit_entries` table; every chat request logged.
- [ ] Basic Settings tab: daemon status indicator.

**Risks:**
- Port file race condition if app starts before daemon is ready — use retry loop with 50 ms backoff × 20.
- Daemon crash loop could drain battery — exponential backoff on restart, max 3 restarts/minute.
- Token stored in `Application Support` (plaintext) before Keychain integration — add Keychain in this phase.

**Acceptance Criteria:**
- [ ] `/health` returns `200 { status: "ok" }` within 2 s of app launch.
- [ ] Echo chat response appears in SwiftUI output area via SSE.
- [ ] Daemon PID survives app backgrounding; dies when app quits.
- [ ] Audit entry created for each chat request.
- [ ] Bearer token stored in Keychain, not filesystem.

---

## Phase 3 — Ollama Integration

**Goal:** Real LLM responses via local Ollama; Slovak diacritics verified.

**Deliverables:**
- [ ] `crates/connectors/ollama` HTTP client; streaming ndjson parser.
- [ ] Model router stub: all requests → Ollama.
- [ ] Model picker in Settings (fetches from `/api/tags`).
- [ ] Default model: `qwen2.5:7b`.
- [ ] Slovak diacritics preserved end-to-end: `á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž`.
- [ ] Ollama up/down status in `/health` and Settings.
- [ ] Context window management: sliding window, summarize older turns.
- [ ] Embedding endpoint wired (`nomic-embed-text` or `bge-m3`).

**Risks:**
- `qwen2.5:7b` may hallucinate Slovak diacritics — test with fixture corpus before releasing.
- Users without Ollama installed get a broken experience — show clear install instructions in onboarding.
- Streaming + SSE → Swift concurrent access bugs — use actor-isolated stream state.

**Acceptance Criteria:**
- [ ] Slovak test fixture: input `"Ahoj, ako sa máš?"` → response contains valid Slovak text with diacritics intact.
- [ ] Streaming: first token visible in UI < 1 s on M-series Mac with `qwen2.5:7b` loaded.
- [ ] Model picker correctly lists installed Ollama models.
- [ ] If Ollama is down, UI shows clear error (not a crash or silent hang).

---

## Phase 4 — Read-Only Apple Mail + Notes

**Goal:** Agent can read, summarize, and answer questions about the user's emails and notes.

**Deliverables:**
- [ ] `crates/connectors/apple_mail`: reads `Envelope Index` SQLite + `.emlx` body extraction.
- [ ] `crates/connectors/apple_notes`: reads via `NSAppleScript` JXA or `NoteStore.sqlite`.
- [ ] Tools: `mail_list_inbox`, `mail_get_message`, `notes_search`, `notes_get_note`.
- [ ] Full Disk Access permission requested on first use with explanation dialog.
- [ ] Language detection per message; `language` stored in `messages` table.
- [ ] Slovak email summarization: prompt template preserves formal business tone.
- [ ] Incremental sync: only index new/changed since last sync; track `indexed_at`.
- [ ] Privacy: mail bodies never sent to cloud model without explicit user opt-in per session.

**Risks:**
- `Envelope Index` schema changes across macOS versions — add version guard; fail gracefully with "unsupported Mail version" error.
- Full Disk Access prompt is irreversible (user must go to System Settings) — guide with screenshot in onboarding.
- Large mailboxes (100k+ messages) can make initial index slow — run in background with progress indicator.
- `.emlx` format (binary plist header + raw RFC 2822) — parse carefully; handle multipart MIME.

**Acceptance Criteria:**
- [ ] `mail_list_inbox` returns last 20 unread messages.
- [ ] Slovak email body extracted and summarized in Slovak with formal tone.
- [ ] Notes search returns results for a Slovak query.
- [ ] No mail body appears in Ollama prompt without user confirming (audit shows the gate).
- [ ] Incremental sync completes in < 5 s for 100 new messages.

---

## Phase 5 — Rules Engine + Approval Framework

**Goal:** Declarative rules governing what the agent may do; human-in-the-loop modals for uncertain actions.

**Deliverables:**
- [ ] `crates/rules/`: YAML rule loader, hot-reload via `notify` crate, matcher.
- [ ] Default ruleset (`rules.yaml`) committed to app bundle as template.
- [ ] Rule evaluation integrated into tool dispatcher.
- [ ] Approval modal in SwiftUI: action description, tool name, args preview, dry-run diff, `[Allow] [Deny] [Edit]`.
- [ ] `/approvals/pending` polling in Swift (1 s interval); show badge on status item.
- [ ] Audit entries for every approval decision (allow/deny/timeout).
- [ ] Settings tab: rule YAML editor with syntax highlighting (basic) and validation.
- [ ] Slovak approval messages: modal text and audit entries in the language of the triggering content.

**Risks:**
- Rule hot-reload race with in-flight tool calls — use `RwLock`; reload after current turn completes.
- Users accidentally deny everything with a bad rule — add "reset to defaults" button.
- Modal appearing on wrong `NSScreen` (multi-monitor) — always present on screen containing notch panel.

**Acceptance Criteria:**
- [ ] A rule `action: deny` for `shell_exec` prevents shell tool from being called; audit shows denial.
- [ ] An `action: ask` rule presents the approval modal; Allow proceeds, Deny cancels with audit entry.
- [ ] Timeout (60 s) auto-denies; audit records `decision: timeout`.
- [ ] Rule YAML changes take effect without app restart.
- [ ] Settings rule editor rejects invalid YAML with inline error.

---

## Phase 6 — Odoo Connector

**Goal:** Agent can read contacts, invoices, opportunities, and tasks from Odoo.

**Deliverables:**
- [ ] `crates/connectors/odoo`: XML-RPC client (`/xmlrpc/2/common`, `/xmlrpc/2/object`).
- [ ] Auth: URL + database + username + API key stored in Keychain.
- [ ] Tools: `odoo_search_contacts`, `odoo_get_invoice`, `odoo_list_opportunities`, `odoo_get_task`.
- [ ] Read-only MVP — all write tools `ApprovalLevel::Forbidden` in Phase 6.
- [ ] Language handling: Odoo field values in Slovak/English preserved; no auto-translation.
- [ ] Settings tab: Odoo connection config + test connection button.
- [ ] Privacy filter: Odoo PII (`pii_present = true`) never routed to cloud without approval.

**Risks:**
- Odoo XML-RPC can be slow for large record sets — paginate; cache aggressively.
- Field names differ between Odoo versions (14/16/17) — detect version via `/web/webclient/version_info`.
- Credential storage: API key visible in Settings form — use `SecureField` in SwiftUI; never log.

**Acceptance Criteria:**
- [ ] Test connection succeeds against a real Odoo instance.
- [ ] `odoo_get_invoice` returns invoice number, partner name, amount, due date, `splatnosť`, `DPH`.
- [ ] Slovak field values preserved verbatim (no translation).
- [ ] PII fields (partner email, phone) trigger privacy filter gate before any Ollama call.
- [ ] No Odoo write attempted; all write tool registrations marked `Forbidden`.

---

## Phase 7 — Screen Context

**Goal:** Agent knows what is on screen and what text is selected; provides contextual assistance.

**Deliverables:**
- [ ] `ScreenCaptureKit` integration: on-demand frame capture (1 fps when panel open).
- [ ] Active app + window title via `NSWorkspace.shared.frontmostApplication`.
- [ ] Selected text via Accessibility API (`AXSelectedText`).
- [ ] Vision model fallback: frame sent to local vision-capable Ollama model (e.g. `llava` or `minicpm-v`) when OCR insufficient.
- [ ] Screen context attached to chat turn as `screen_context: { app, title, selected_text, ocr_text? }`.
- [ ] Permission prompt: Screen Recording requested on first screen-context use.
- [ ] Privacy: frames ephemeral by default; never stored; never cloud-uploaded without per-session opt-in.
- [ ] User can pin a frame (stored encrypted locally for session duration only).

**Risks:**
- ScreenCaptureKit requires `com.apple.security.temporary-exception.*` or entitlement changes — test on real device.
- Vision model adds significant memory pressure if run alongside chat model — allow user to disable.
- DRM-protected content (Netflix, etc.) returns black frame — handle gracefully, inform user.
- False positives: sensitive content (passwords, banking) captured unintentionally — auto-redact known password-field patterns.

**Acceptance Criteria:**
- [ ] Selected text in any app appears in chat context without user copy-pasting.
- [ ] Active app name shown in chat context header.
- [ ] OCR text from a screenshot of a Slovak document contains diacritics correctly.
- [ ] No screen frame persists after chat session ends (verify via DB audit).
- [ ] Cloud upload of frame correctly blocked unless user enables per-session.

---

## Phase 8 — Codex Connector

**Goal:** Agent can perform coding tasks via Codex CLI, with full approval gating.

**Deliverables:**
- [ ] `crates/connectors/codex`: subprocess wrapper for `codex` binary.
- [ ] Sandboxed temp working directory per invocation.
- [ ] Tools: `codex_run_task` (single approval-gated tool).
- [ ] Diff preview shown in approval modal before any file is written.
- [ ] Timeout: 120 s SIGTERM + 5 s SIGKILL.
- [ ] Codex binary path configurable; graceful "Codex CLI not found" message.
- [ ] Audit: every Codex invocation logged with task description, working dir, exit code, diff summary.

**Risks:**
- Codex CLI API may change (it is a CLI, not a stable library) — pin version; test on update.
- Sandboxed temp dir must not have access to sensitive files — use `FileManager` temp dir with no symlinks to home.
- LLM may try to chain Codex calls to bypass approval — tool dispatcher enforces one-approval-per-invocation; no chaining without re-approval.

**Acceptance Criteria:**
- [ ] Codex invocation blocked until user approves in modal.
- [ ] Diff of proposed changes displayed in modal before approval.
- [ ] Files modified only within temp sandbox; main codebase untouched unless user explicitly provides a path.
- [ ] Timeout fires correctly; process tree killed (not just parent).
- [ ] Audit entry includes full args and diff hash.

---

## Phase 9 — Slovak / English Polish

**Goal:** First-class bilingual support; production-quality Slovak business output.

**Deliverables:**
- [ ] Language detector integrated in agent runtime: classify each user turn and each connector record.
- [ ] Formal Slovak tone prompt template locked in model router.
- [ ] Glossary lock: `DPH`, `faktúra`, `splatnosť`, `IČO`, `DIČ`, `zmluva`, `objednávka`, `zákazník` never translated.
- [ ] Diacritics regression test suite: 50+ Slovak sentences roundtripped through each model.
- [ ] Language metadata stored in `messages`, `memory_items`, `audit_entries`.
- [ ] UI strings: Slovak and English localizable strings (`Localizable.strings`); Slovak locale added.
- [ ] Date/number formatting: Slovak locale (`sk_SK`) for amounts and dates in summaries.
- [ ] Formal greeting/closing enforced: `Dobrý deň, …` / `S pozdravom` in email drafts.

**Risks:**
- Ollama models may occasionally drop diacritics under load — add post-processing check with warning to user.
- Machine translation of legal terms could cause compliance issues — `deny` rule hardcoded against translation tool for Slovak legal terms.
- `sk_SK` locale not available on all macOS installations — fallback to `sk`.

**Acceptance Criteria:**
- [ ] Diacritics test suite passes 100% (zero corrupted characters) for `qwen2.5:7b`.
- [ ] Slovak invoice summary contains `DPH` not `VAT`, `faktúra` not `invoice`, `splatnosť` not `due date`.
- [ ] Email draft in response to Slovak input starts with `Dobrý deň` and ends with `S pozdravom`.
- [ ] Language detection correctly classifies 10/10 mixed Slovak/English test inputs.
- [ ] UI renders correctly with Slovak locale active (dates as `11. 6. 2026`, amounts as `1 234,56 €`).

---

## Phase 10 — Packaging, Security Hardening, Beta

**Goal:** App ready for beta distribution; security audit passed; notarized and auto-updating.

**Deliverables:**
- [ ] Hardened Runtime enabled; all entitlements justified and minimal.
- [ ] Notarization pipeline: `xcrun notarytool submit` in CI (GitHub Actions).
- [ ] `bagentd` universal binary (arm64 + x86_64).
- [ ] Sparkle 2.x auto-update; appcast hosted; Ed25519 signature verification.
- [ ] SQLite encrypted via SQLCipher (key derived from Keychain secret).
- [ ] Audit log hash-chain verification tool (CLI: `bagentd --verify-audit`).
- [ ] Crash reporter (opt-in): Sentry or in-house with zero PII.
- [ ] Onboarding flow: permission explanations, Ollama install guide, language preference.
- [ ] Beta `.dmg` distributed to 5–10 test users.
- [ ] Threat model reviewed against OWASP LLM Top 10.

**Risks:**
- Notarization may reject entitlements — test on clean Apple ID with fresh provisioning profile.
- SQLCipher migration from plain SQLite requires careful key management — document recovery path.
- Auto-update delivering a broken daemon could brick the app — staged rollout (10% → 50% → 100%).
- Beta users may have non-standard Ollama setups — add diagnostics command `⌘D` in Settings.

**Acceptance Criteria:**
- [ ] App passes `spctl --assess --verbose bagent.app` (Gatekeeper check).
- [ ] Auto-update delivers a new version within 24 h of appcast publish.
- [ ] Audit log hash chain verifies clean on a fresh install + 100 operations.
- [ ] Zero plaintext API keys in `Application Support` directory.
- [ ] Beta user installs succeed on macOS 14 and macOS 15 without manual steps.
- [ ] Security checklist in `SECURITY.md` marked complete for all Phase 10 items.
