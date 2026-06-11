# Connectors

Each connector is described with: **Purpose · Permissions · Read Actions · Write Actions · Approval Level · Failure Modes · MVP Scope · Future Scope**.

---

## Apple Mail

**Purpose:** Read and summarize emails; generate reply drafts; support Slovak business correspondence.

**Permissions Required:**
- Full Disk Access (to read `~/Library/Mail/` without AppleScript).
- Automation → Mail (for AppleScript/JXA send path in future phases).

**Read Actions:**
- `mail_list_inbox(account?, limit?, unread_only?)` — list message metadata from `Envelope Index` SQLite.
- `mail_get_message(message_id)` — extract body from `.emlx` file; parse MIME parts; strip quoted history optionally.
- `mail_search(query, mailbox?, date_range?)` — FTS5 search over indexed message bodies.
- `mail_get_thread(thread_id)` — reconstruct conversation thread.

**Write Actions (Phase 5+):**
- `mail_create_draft(to, subject, body, cc?, bcc?, attachments?)` — creates draft in Mail via AppleScript/JXA. Never sends. Stored locally first.
- `mail_send_draft(draft_id)` — sends a previously approved draft. Requires per-message approval.

**Approval Level:**
- Reads: `Auto` (after initial Full Disk Access grant).
- Draft creation: `Ask` (show full preview).
- Send: `Ask` (per-message, every time — no batching).

**Failure Modes:**
- `Envelope Index` locked (Mail app active + writing): retry with 200 ms backoff × 5; inform user if fails.
- `.emlx` parse error (corrupt file): skip and log; do not surface raw binary to model.
- Mail schema version mismatch (macOS update): return `ConnectorError::SchemaVersion`; show "Mail connector needs update" in UI.
- Automation permission denied: surface System Settings deep link.

**MVP Scope (Phase 4):**
- Read inbox only (no sent/drafts/other mailboxes).
- List last 50 unread messages.
- Get message body for summarization.
- Slovak language detection and summary.

**Future Scope:**
- All mailboxes + smart mailbox support.
- Rule-based auto-labeling (local model only).
- Draft → send flow with approval.
- Attachment download and analysis (with approval).
- Multiple accounts.

---

## Apple Notes

**Purpose:** Search and retrieve notes for context; support knowledge base queries in Slovak and English.

**Permissions Required:**
- Automation → Notes (for AppleScript/JXA read path, preferred).
- Full Disk Access (for direct `NoteStore.sqlite` path as fallback).

**Read Actions:**
- `notes_list(folder?, limit?)` — list note titles and modification dates.
- `notes_search(query)` — full-text search via AppleScript `whose name contains` or direct FTS5 on `NoteStore.sqlite`.
- `notes_get_note(note_id)` — retrieve note body (HTML → Markdown conversion).

**Write Actions (Future):**
- `notes_create_note(title, body, folder?)` — create note via AppleScript. Requires approval.
- `notes_append_to_note(note_id, text)` — append text to existing note. Requires approval.

**Approval Level:**
- Reads: `Auto` (after Automation permission grant).
- Writes: `Ask`.

**Failure Modes:**
- `NoteStore.sqlite` format is undocumented and changes between OS versions — prefer AppleScript path; fall back to SQLite with version guard.
- Notes with rich content (tables, attachments): extract plain text only; flag if content was truncated.
- iCloud sync lag: note may not be locally available immediately; retry after 2 s.

**MVP Scope (Phase 4):**
- List and search notes.
- Get note body for context injection.
- Slovak language metadata.

**Future Scope:**
- Create notes as agent memory/output.
- Folder organization.
- Shared notes support.
- Attachment handling.

---

## Odoo

**Purpose:** Read CRM, invoice, contact, and task data for business context and assistant responses.

**Permissions Required:**
- Network access to Odoo instance URL.
- Keychain entry: `bagent.odoo.url`, `bagent.odoo.db`, `bagent.odoo.username`, `bagent.odoo.apikey`.

**Read Actions:**
- `odoo_search_contacts(query?, domain?)` — search `res.partner` records.
- `odoo_get_contact(partner_id)` — full partner record.
- `odoo_list_invoices(state?, partner_id?, date_range?)` — list `account.move` records.
- `odoo_get_invoice(invoice_id)` — full invoice with lines, DPH, splatnosť, IČO.
- `odoo_list_opportunities(stage?, assigned_to?)` — CRM leads (`crm.lead`).
- `odoo_get_opportunity(lead_id)` — full CRM record.
- `odoo_list_tasks(project_id?, assigned_to?, state?)` — project tasks.
- `odoo_get_task(task_id)` — full task record.

**Write Actions (Phase 6+ write tier, forbidden in MVP):**
- `odoo_create_activity(model, record_id, activity_type, note, deadline)` — log activity.
- `odoo_update_opportunity_stage(lead_id, stage_id)` — move CRM stage.
- `odoo_create_task(project_id, name, description, assigned_to?)` — create task.

**Approval Level:**
- Reads: `Ask` (first use per session; auto thereafter if user checks "remember for this session").
- Writes: `Ask` (every time, with dry-run diff showing exact field changes).

**Failure Modes:**
- Authentication failure: surface Keychain re-entry in Settings.
- Odoo version mismatch: detect version via `/web/webclient/version_info`; warn on unsupported versions.
- Network timeout: 10 s default; retry once; surface error with Odoo URL and HTTP status.
- Large result sets: paginate (default page size 80); stream results to UI.
- Field access denied: surface `xmlrpc.Fault` code + message; do not hallucinate a fallback value.

**MVP Scope (Phase 6):**
- Read: contacts, invoices, opportunities, tasks.
- All writes `Forbidden`.
- Keychain-based auth.
- Version detection for Odoo 16/17.

**Future Scope:**
- Create activities, tasks.
- Update opportunity stages.
- Sync tasks to/from agent memory.
- Custom model/field configuration.
- Webhook receiver for real-time updates.

---

## Ollama

**Purpose:** Local LLM inference for all private tasks — summarization, classification, embeddings, Slovak text generation, coding fallback.

**Permissions Required:**
- Network (loopback only: `localhost:11434`). No external network needed.

**Read Actions:**
- `ollama_chat(model, messages, stream?)` — streaming or blocking chat completion.
- `ollama_embed(model, text)` — compute embedding vector.
- `ollama_list_models()` — list installed models.
- `ollama_model_info(model)` — model metadata (context length, quantization).

**Write Actions:**
- None exposed to the agent tool layer. Model installation is user-initiated via Ollama CLI or the Ollama app.

**Approval Level:**
- All Ollama calls: `Auto` (stays local; no PII leaves device).

**Failure Modes:**
- Ollama not running: `/health` shows `ollama_up: false`; show "Start Ollama" button with `open ollama://` deep link.
- Model not installed: surface model name + `ollama pull <model>` instruction.
- Context window exceeded: truncate with sliding window strategy; log truncation in audit.
- Slow response (> 5 s TTFT): show animated indicator; allow user to cancel.
- Out of memory (OOM): Ollama returns 500; surface "Model too large" with suggested smaller model.

**MVP Scope (Phase 3):**
- Chat with streaming.
- Model picker.
- Basic embedding for memory indexing.
- Slovak diacritics verified.

**Future Scope:**
- Vision model support (`llava`, `minicpm-v`) for screen context.
- Multi-model routing (small model for classification, large for generation).
- Local fine-tuned Slovak business model.
- Automatic model download prompts.

---

## Codex CLI

**Purpose:** Coding and refactoring tasks delegated to the Codex CLI agent with full approval gating.

**Permissions Required:**
- Filesystem access to sandboxed working directory (temp dir).
- Subprocess execution.
- Optionally: network access if Codex needs to fetch context (blocked by default; ask user).

**Read Actions:**
- Codex reads files in the provided working directory during task execution.

**Write Actions:**
- `codex_run_task(task_description, working_dir?, context_files?)` — invokes Codex CLI subprocess; produces file diff for approval.

**Approval Level:**
- Every invocation: `Ask` (show task description + diff preview before any file is written).

**Failure Modes:**
- `codex` binary not found: show install instructions; degrade to local LLM with coding prompt.
- Codex timeout (> 120 s): SIGTERM → 5 s → SIGKILL; report partial output if any.
- Codex writes outside sandbox: monitor working dir with `FSEvents`; abort if write detected outside bounds.
- Non-zero exit: surface stderr to user; do not retry automatically.
- API key for Codex not configured: prompt user to enter key in Settings → Keychain.

**MVP Scope (Phase 8):**
- Single task invocation with approval.
- Sandbox working dir.
- Diff preview in approval modal.
- Timeout enforcement.

**Future Scope:**
- Multi-turn Codex sessions.
- Integration with screen context (point at code in editor).
- Direct write-back to repo with git diff preview.
- Codex for non-coding structured tasks (document generation).

---

## Ghostty / Shell

**Purpose:** Execute allowed shell commands for system tasks; display output to user.

**Permissions Required:**
- Subprocess execution.
- Filesystem access (sandboxed working dir by default).
- Network (blocked by default; ask per session).

**Read Actions:**
- `shell_exec(command, working_dir?, timeout_seconds?)` — run command in sandboxed shell. Output captured.

**Write Actions:**
- Same tool; side effect depends on command. Shell connector does not distinguish internally — the rules engine classifies by command pattern.

**Approval Level:**
- Allowlisted commands: `Auto`.
- All others: `Ask`.
- Denied patterns: `Forbidden` (see RULES.md).

**Failure Modes:**
- Timeout: SIGTERM + SIGKILL; output truncated at 50 KB.
- Non-zero exit: surface stderr; do not auto-retry.
- Command in deny list: blocked before subprocess spawn; audit entry created.
- Working dir doesn't exist: return error; do not create silently.

**MVP Scope (Phase 2 scaffolding, Phase 5 for approval flow):**
- Allowlist read-only commands.
- Approval modal for everything else.
- Timeout enforcement.

**Future Scope:**
- Interactive session in a Ghostty tab (via AppleScript or shell integration protocol).
- Named sandbox profiles (e.g. "project X working dir").
- Command history in memory.

---

## Screen Context

**Purpose:** Provide the agent with awareness of what is on screen and what the user has selected, enabling contextual assistance without copy-paste.

**Permissions Required:**
- Screen Recording (ScreenCaptureKit).
- Accessibility (for `AXSelectedText`, active app).

**Read Actions:**
- `screen_get_active_app()` — frontmost app name + bundle ID via `NSWorkspace`.
- `screen_get_selected_text()` — currently selected text via Accessibility API.
- `screen_capture_frame(display?)` — one-shot screenshot via ScreenCaptureKit.
- `screen_ocr_frame(frame)` — run Vision framework OCR on a captured frame (local, on-device).

**Write Actions:**
- None. Screen context is read-only always.

**Approval Level:**
- `screen_get_active_app`: `Auto`.
- `screen_get_selected_text`: `Auto` (after Accessibility permission granted).
- `screen_capture_frame`: `Ask` (first capture per session; auto thereafter if user enables in Settings).
- Cloud upload of frame: `Forbidden` unless per-session opt-in.

**Failure Modes:**
- Screen Recording permission denied: show System Settings deep link.
- ScreenCaptureKit returns black frame (DRM content): inform user; do not retry silently.
- OCR fails on low-contrast text: return partial results with confidence score; do not fabricate text.
- Selected text unavailable (app doesn't support Accessibility): fall back to clipboard suggestion.

**MVP Scope (Phase 7):**
- Active app + selected text.
- One-shot frame capture with user trigger.
- Local OCR (Vision framework).
- No cloud upload.

**Future Scope:**
- Continuous low-fps ambient awareness (opt-in, local only).
- Vision model inference (Ollama `llava`) for complex UI understanding.
- Cursor/selection tracking for proactive suggestions.
- Multi-display support.

---

## WhatsApp (Future)

**Purpose:** Read WhatsApp messages for context and assist with drafting replies. Send only with explicit per-message approval.

**Permissions Required:**
- Accessibility (inspect WhatsApp macOS app UI elements).
- Screen Recording (as fallback for reading message content if Accessibility is insufficient).

**Read Actions (planned):**
- `whatsapp_get_recent_chats()` — list recent chats via Accessibility UI traversal.
- `whatsapp_get_messages(chat_id, limit?)` — read messages from a specific chat.

**Write Actions (planned):**
- `whatsapp_send_message(chat_id, text)` — type and send message via Accessibility. Every send requires approval.

**Approval Level:**
- Reads: `Ask` (privacy-sensitive; PII present).
- Sends: `Ask` (per-message, every time, with full preview).
- Simulating UI clicks without approval: `Forbidden`.

**Failure Modes:**
- WhatsApp app not open: return error with instructions to open app.
- Accessibility layout changes after WhatsApp update: connector breaks; degrade gracefully with "WhatsApp connector needs update".
- Screenshot-only fallback: raw frame may contain messages from other chats in sidebar — scope carefully to focused chat window.

**MVP Scope:** Not included. Phase 10+ or separate release.

**Future Scope:**
- Official WhatsApp Business API (if credentials available) as primary path.
- Accessibility path as fallback only.
- Webhook/notification listener for incoming messages (if feasible via Accessibility).
- Group chat support (with explicit per-group enablement).
