# Operational Rules

Rules governing what the agent may do automatically (`auto`), what requires human approval (`ask`), and what is permanently forbidden (`forbidden`).

Rules are defined in `~/.config/bagent/rules.yaml` and enforced by the Rust rules engine. The LLM cannot modify rules — only the user can, via the Settings rule editor.

---

## General Principles

1. **Local-first.** Never send data outside the device unless a rule explicitly permits it and the user has approved the specific session/action.
2. **Read before write.** The agent may read freely within granted permissions; writes always require explicit approval.
3. **Audit everything.** Every tool call, approval decision, and model invocation is logged. The audit log cannot be disabled.
4. **Dry-run first.** All write operations must produce a preview/diff before requesting approval.
5. **One approval per action.** Approval granted for action X does not grant approval for action Y, even if similar. No implicit chaining.
6. **LLM cannot self-approve.** The model cannot call an approval-required tool without a human decision. `confirm: true` in a model response is ignored; only the user can set it via the modal.

---

## Action Categories

### Auto (No Approval Required)

- Read local data from connectors already granted permission (Mail, Notes, Odoo read).
- Summarize, classify, extract entities from local text.
- Generate draft text (email replies, summaries, task descriptions) — **drafts are never sent automatically**.
- Retrieve from memory / search index.
- Compute embeddings via local Ollama.
- Translate UI text within the app.
- Log to audit trail.
- Check connector health status.
- Fetch Ollama model list.

### Ask (Approval Required)

- Send any email or message (including WhatsApp).
- Create, modify, or delete any Odoo record.
- Execute any shell command.
- Write to the filesystem outside the daemon's sandbox directory.
- Call a cloud LLM (Claude, OpenAI, etc.) with any content from connectors.
- Attach any file to an outgoing message.
- Capture a screen frame (first capture per session).
- Upload any content to an external service.
- Invoke Codex CLI.
- Change any rule in `rules.yaml`.
- Access any connector not previously authorized.

### Forbidden (Never, Regardless of Model Request)

- Send email or message without per-message human approval.
- Send to multiple recipients without explicit per-recipient approval.
- Delete mail messages, notes, or Odoo records.
- Execute shell commands as root / with `sudo`.
- Disable or truncate the audit log.
- Upload raw screen frames to any cloud service without per-session explicit opt-in.
- Modify `rules.yaml` programmatically (LLM cannot rewrite its own rules).
- Access the user's Keychain directly from model context.
- Forward private email content to third parties.
- Run any background process that persists after the daemon exits.

---

## Slovak / English Language Behavior

### Language Detection
- Every user turn and every connector record is classified: `sk`, `en`, or `und` (undetermined).
- Detection runs locally (Ollama or a lightweight classifier); result stored in `language` metadata.
- Mixed-language input defaults to the dominant language of the message.

### Response Language
- Reply in the **same language as the user's input**.
- If input is Slovak → Slovak response. If English → English response.
- Never auto-translate between Slovak and English unless the user explicitly asks.

### Diacritics
- Slovak diacritics (`á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž`) must be **preserved exactly** in all inputs, stored data, and outputs.
- Any output that corrupts diacritics is flagged as a model error and the user is notified.
- Post-processing check: if input contains diacritics and output does not, warn before displaying.

### Formal Business Tone (Slovak)
- Default register: formal (`Vy`-form, not `ty`-form).
- Email drafts:
  - Opening: `Dobrý deň, [meno/titul],`
  - Closing: `S pozdravom,\n[meno]`
- Never use informal `Ahoj` or `Čau` in business email drafts unless the source message uses informal language.
- Do not use emoji in formal Slovak business text unless the source uses them.

### Protected Terms (Never Translated)
The following terms must appear verbatim in Slovak output and are **never** translated to English:

| Slovak Term | Do Not Replace With |
|---|---|
| DPH | VAT |
| faktúra | invoice |
| splatnosť | due date |
| IČO | company ID / registration number |
| DIČ | tax ID |
| zákazník | customer |
| zmluva | contract |
| objednávka | order |
| dodávateľ | supplier |
| odberateľ | buyer/recipient |

### Example: Formal Slovak Email Draft

```
Dobrý deň, pán Novák,

dovoľujeme si Vás upozorniť, že faktúra č. 2024-0123 so splatnosťou 15. 7. 2024
(suma 2 400,00 € vrátane DPH) doteraz nebola uhradená.

Prosíme o urýchlené vysporiadanie záväzku, prípadne o potvrdenie termínu úhrady.

S pozdravom,
[podpis]
```

---

## Privacy Behavior

- **PII fields** (email addresses, phone numbers, ID numbers, bank accounts) are marked at the connector level (`pii_present = true`).
- PII is never included in a cloud LLM prompt without:
  1. User approval for the specific connector × cloud model pairing.
  2. The approval recorded in the audit log.
- Local Ollama: PII may be included (stays on device) unless a rule explicitly forbids it.
- Redaction: before any cloud call, a redaction pass replaces PII tokens with `[REDACTED_<type>]`.
- Screen frames: treated as maximum-PII; never leave device by default.

---

## Tool Execution Behavior

- Every tool call is recorded in `tool_calls` before execution starts and updated with result/error after.
- Side-effect class determines default approval level:

| Side Effect Class | Default Approval |
|---|---|
| ReadOnly | Auto |
| LocalWrite | Ask |
| ExternalRead | Ask (first time per connector per session) |
| ExternalWrite | Ask (every time) |
| CodeWrite | Ask (every time) |
| Shell | Ask (every time) |

- Dry-run output must be shown in the approval modal for all write-class tools.
- Idempotency key generated per write operation; duplicate approvals within the same session are rejected.

---

## Email Behavior

- MVP: **drafts only**. No email is ever sent.
- Phase 5+: sending requires per-message approval with full preview of recipient, subject, and body.
- CC/BCC fields must be explicitly visible in the approval modal.
- Mass send (> 1 recipient without individual approval) is **forbidden**.
- Reply-all requires explicit user selection of "Reply All" in the approval modal.
- Attachments: each attachment listed by filename and size; approval required per attachment.
- Slovak emails: draft in formal Slovak tone (see Language rules above).
- Draft stored locally in `messages` table with `status: draft`; never committed to Mail until approved.

---

## Odoo Behavior

- **Phase 6 MVP: read-only**. All write tools have `ApprovalLevel::Forbidden`.
- Phase 7+: writes require:
  1. Dry-run diff showing exact field changes.
  2. Per-record per-operation approval.
  3. Idempotency key to prevent duplicate writes.
- Bulk operations (> 1 record) require individual approval per record.
- Odoo credentials (URL, database, API key) stored in Keychain; never logged.
- Field values must not be auto-translated; Slovak values stay Slovak.
- On Odoo API error: surface the raw error message to user; do not retry silently.

---

## Shell Behavior

- Allowlist (auto-approved if matched exactly, no argument wildcards):
  - `ls`, `pwd`, `echo`, `date`, `whoami`
- All other commands: `Ask`.
- Permanently denied patterns (deny rule fires before Ask):
  - Any command containing `sudo`, `su`, `rm -rf`, `mkfifo`, `nc`, `curl`, `wget`, `python -c`, `bash -c`, `eval`.
- Timeout: 30 s default; SIGTERM + 5 s SIGKILL.
- Working directory: daemon sandbox temp dir unless user explicitly provides a path in the approval modal.
- Network access from shell: blocked by default (sandbox entitlement); user can override per session.
- Output is captured and shown to user; never piped back to model as trusted input.

---

## Screen Context Behavior

- Capture is triggered only by:
  1. User invoking screen context explicitly (`⌘K` shortcut or "look at screen" intent in chat).
  2. Active conversation where user has enabled "auto screen context" in Settings (per session).
- Raw frames are **never persisted** to disk or database by default.
- OCR text derived from frames is stored only in the current session's in-memory context.
- Cloud upload of frames: `Forbidden` by default. Requires per-session opt-in in Settings.
- Password fields (detected via Accessibility `AXIsPasswordField`): excluded from capture always.
- DRM-protected content: if ScreenCaptureKit returns a black frame, inform user; do not retry silently.
- Session end: all ephemeral screen context cleared from memory.
