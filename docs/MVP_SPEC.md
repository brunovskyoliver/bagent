# MVP Specification

The MVP delivers a functional, local-first assistant for one primary use case: **summarize and draft replies to Slovak and English emails, running entirely on-device with Ollama**.

---

## Scope Boundary

**In MVP:**
- Notch / menu-bar UI with chat panel.
- Rust backend daemon with local HTTP/SSE IPC.
- Ollama chat (streaming, model picker).
- Read-only Apple Mail summaries.
- Slovak email summarization with formal tone.
- Approval framework scaffold (modal + audit — for cloud LLM opt-in decision in MVP).
- Audit log viewer.
- Settings screen (rules viewer, model config, API keys, language preference).

**Not in MVP:**
- Apple Notes connector.
- Odoo connector.
- Shell execution.
- Screen context / ScreenCaptureKit.
- Codex CLI integration.
- Cloud LLM (button exists but disabled until user adds key and approves).
- WhatsApp.
- Email sending (drafts only, no send path).
- Multilingual translation.
- Embeddings / semantic search (memory indexing wired but not exposed in UI).

---

## UI

### Notch Pill (Collapsed State)

```
╔═══════════════════════════════╗
║  ████ [camera notch area] ████║
║  ▌▌     [bagent pill]         ║   ← 28×8 pt pill, centered under notch
╚═══════════════════════════════╝
```

- Collapsed: a subtle dark pill. Pulses white when agent is thinking.
- Click or `⌥Space` → expands to chat panel.
- On non-notch Macs: status bar icon (magnifying glass or `✦`).

### Chat Panel (Expanded State)

```
┌─────────────────────────────────┐
│ bagent                      ✕   │  ← 400 × 520 pt
├─────────────────────────────────┤
│                                 │
│  [streaming response here]      │
│                                 │
│  ─────────────────────────────  │
│  Summarize unread mail          │
│  ─────────────────────────────  │
│  Draft reply to last invoice    │
│  ─────────────────────────────  │
├─────────────────────────────────┤
│  ┌───────────────────────────┐  │
│  │ Type a message...         │  │  ← TextEditor, max 4 lines before scroll
│  └───────────────────────────┘  │
│  [⌘↩ Send]  [●] qwen2.5:7b ▾   │
└─────────────────────────────────┘
```

- Panel slides down from notch (150 ms ease-out).
- Model picker (bottom right) lists installed Ollama models.
- `⌘↩` sends; `Escape` collapses.
- Typing indicator: animated dots when daemon is processing.
- Stop button replaces send during streaming.

### Approval Modal

```
┌─────────────────────────────────────────────┐
│  ⚠️  Approval Required                       │
├─────────────────────────────────────────────┤
│  Action:  Call cloud LLM (claude-opus-4-7)  │
│  Context: Summarize 3 email messages        │
│                                             │
│  Redacted fields: [REDACTED_EMAIL] ×2       │
│                                             │
│  This will send the selected content to     │
│  Anthropic's API. The content has been      │
│  redacted as shown above.                   │
│                                             │
│  [  Deny  ]  [  Allow  ]                    │
│                                             │
│  Auto-deny in: 58s                          │
└─────────────────────────────────────────────┘
```

- `NSAlert`-based or SwiftUI Sheet depending on phase.
- Always appears on the screen containing the notch panel.
- Never auto-approves.

### Settings Screen

Tabs:
1. **General**: language preference (Slovak / English / Auto), hotkey config.
2. **Models**: Ollama model picker, cloud LLM toggle + API key (SecureField → Keychain).
3. **Mail**: Full Disk Access status, inbox selection, sync now button.
4. **Rules**: YAML text editor (read-only in MVP; editable in Phase 5), reset to defaults.
5. **Audit**: scrollable list of recent audit entries, filter by action type.
6. **About**: version, daemon status, diagnostics button.

---

## Backend API (MVP Subset)

```
GET  /health
     → { "status": "ok", "version": "0.1.0", "ollama_up": true, "db_ok": true }

POST /chat
     body: { "messages": [{"role":"user","content":"..."}], "session_id": "uuid" }
     → SSE stream:
         data: {"type":"token","content":"Dobrý"}\n\n
         data: {"type":"token","content":" deň"}\n\n
         data: {"type":"done","tool_calls":[]}\n\n

GET  /approvals/pending
     → [{ "id":"uuid", "action_type":"cloud_llm", "request_json":{...}, "expires_at":"..." }]

POST /approvals/:id
     body: { "decision": "allow" | "deny", "reason": "optional" }
     → { "ok": true }

GET  /audit?limit=50&since=ISO8601
     → [{ "id", "seq", "action", "actor", "outcome", "created_at", "payload_json" }]

GET  /connectors
     → [{ "id":"apple_mail", "enabled":true, "last_sync_at":"...", "pii_present":true }]

POST /connectors/apple_mail/sync
     → { "queued": true }
```

---

## Chat Flows

### Flow 1: Summarize Unread Mail (Slovak)

1. User types: `"Zhrň mi neprečítané správy"` or clicks suggestion chip.
2. Daemon detects language `sk`.
3. `mail_list_inbox(unread_only: true, limit: 10)` called (auto — read-only).
4. For each message: `mail_get_message(id)` called (auto).
5. Prompt assembled with Slovak summarization template (see MODEL_ROUTER.md).
6. Ollama `qwen2.5:7b` generates streaming summary.
7. Response streamed to UI.
8. Audit entry: action=`model_invoke`, model=`qwen2.5:7b`, language=`sk`, connector=`apple_mail`.

**Expected output format:**
```
Neprečítané správy (3):

1. Od: jan.novak@firma.sk | 10. 6. 2026
   Predmet: Faktúra č. 2024-0456 — upomienka
   Zhrnutie: Upomienka na úhradu faktúry vo výške 1 200 € so splatnosťou 5. 6. 2026.

2. Od: info@dodavatel.sk | 9. 6. 2026
   Predmet: Zmluva o spolupráci — podpis
   Zhrnutie: Žiadosť o podpis zmluvy o spolupráci na rok 2026.

3. Od: peter.kral@partner.sk | 8. 6. 2026
   Predmet: Stretnutie budúci týždeň
   Zhrnutie: Návrh stretnutia vo štvrtok 13. 6. o 10:00 v ich kancelárii.
```

### Flow 2: Draft Reply (Slovak Formal)

1. User types: `"Napíš odpoveď na fakturovú upomienku od Nováka"`.
2. Daemon fetches message from Jan Novák.
3. Prompt uses Slovak business assistant template.
4. Draft generated (never sent automatically).
5. Draft shown in UI with `[Copy Draft]` button.
6. Audit entry: action=`tool_call`, tool=`mail_draft_generated`, language=`sk`.

**Expected draft:**
```
Dobrý deň, pán Novák,

ďakujeme za Vašu správu. Úhradu faktúry č. 2024-0456 sme zaradili do spracovania
a platba bude realizovaná do 13. 6. 2026.

S pozdravom,
[podpis]
```

### Flow 3: Cloud LLM Opt-In

1. User asks a complex question requiring reasoning beyond `qwen2.5:7b` quality.
2. Model router decides: route to cloud.
3. Privacy filter runs; PII redacted.
4. Approval modal shown: "Send to claude-opus-4-7? Content: [summary]. Redacted: 2 email addresses."
5. User clicks Allow → cloud call made → response streamed.
6. Audit entry: action=`model_invoke`, model=`claude-opus-4-7`, redacted_fields=2.

---

## Slovak QA Fixtures

Stored in `fixtures/sk/` for regression testing diacritics and formal tone.

### Fixture 1 — Invoice Reminder (`faktúra-upomienka.txt`)

```
Od: jan.novak@firma.sk
Predmet: Upomienka — faktúra č. 2024-0123

Dobrý deň,

dovoľujeme si Vás upozorniť, že faktúra č. 2024-0123 na sumu 2 400,00 €
(vrátane DPH 20 %) so splatnosťou 1. 6. 2026 doteraz nebola uhradená.

Prosíme o vysporiadanie záväzku v čo najkratšom čase.
V prípade, že ste platbu realizovali, považujte túto správu za bezpredmetnú.

S pozdravom,
Ján Novák
IČO: 12345678 | DIČ: SK2023456789
```

Expected model output: summary in Slovak, diacritics intact, `DPH` not replaced with `VAT`, `splatnosťou` not replaced with `due date`.

### Fixture 2 — Meeting Request (`stretnutie.txt`)

```
Od: eva.kralova@partner.sk
Predmet: Stretnutie — návrh termínu

Dobrý deň,

radi by sme Vás pozvali na pracovné stretnutie vo veci obnovy zmluvy o dodávke.
Navrhujem termín: štvrtok 13. 6. 2026 o 10:00 v našich priestoroch (Bratislava, Hlavná 5).

Prosím o potvrdenie alebo návrh iného termínu.

S pozdravom,
Eva Králová
```

### Fixture 3 — Customer Complaint (`sťažnosť.txt`)

```
Od: zakaznik@email.sk
Predmet: Sťažnosť na kvalitu dodaného tovaru

Dobrý deň,

dňa 5. 6. 2026 sme prevzali objednávku č. OBJ-2026-0789. Pri kontrole sme zistili,
že 3 kusy výrobku sú poškodené a nezodpovedajú objednanej špecifikácii.

Žiadame o výmenu tovaru alebo vrátenie kúpnej ceny za poškodené kusy.
Doklady a fotodokumentáciu zasielame v prílohe.

S pozdravom,
Miroslav Horváth
```

---

## Acceptance Criteria for MVP

- [ ] App launches; notch panel appears; `⌥Space` toggles it.
- [ ] Ollama `qwen2.5:7b` responds in < 1 s TTFT on M-series Mac.
- [ ] Slovak diacritics intact in all 3 fixtures (zero corrupted characters).
- [ ] `faktúra` not replaced with `invoice` in Slovak summaries.
- [ ] Approval modal appears for cloud LLM call; auto-denies after 60 s timeout.
- [ ] Audit log shows every chat request, tool call, and approval decision.
- [ ] Mail sync completes without crashing when Full Disk Access is granted.
- [ ] App passes `spctl --assess` (notarization check, even if not yet distributed).
- [ ] Settings correctly saves model choice and language preference across restarts.
- [ ] No plaintext secrets in `~/Library/Application Support/bagent/`.
