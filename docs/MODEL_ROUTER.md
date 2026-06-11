# Model Router

The model router selects the appropriate model for each task based on task type, privacy requirements, language, and available backends.

---

## Routing Table

| Task Type | Route | Model (default) | Reason |
|---|---|---|---|
| Language detection | Ollama local | `qwen2.5:7b` | Fast, stays local, good SK/EN |
| Classification (intent, category) | Ollama local | `qwen2.5:7b` | Low latency, private |
| Summarization (mail, notes, Odoo) | Ollama local | `qwen2.5:7b` | Private content; SK/EN |
| Slovak business text generation | Ollama local | `qwen2.5:7b` | Best local SK quality |
| Short Q&A / entity extraction | Ollama local | `qwen2.5:7b` | Fast |
| Embeddings (memory indexing) | Ollama local | `bge-m3` or `nomic-embed-text` | Multilingual SK/EN |
| Screen OCR understanding | Ollama local | `minicpm-v` or `llava` | Vision; local only |
| Coding / refactoring | Codex CLI | (Codex uses its own model) | Specialized coding agent |
| Complex multi-step reasoning | Cloud LLM (opt-in) | `claude-opus-4-7` | When local model insufficient |
| Long document analysis (opt-in) | Cloud LLM (opt-in) | `claude-opus-4-7` | Context length |

> **Default**: route to Ollama local. Cloud is opt-in only, requires user approval per session.

---

## Recommended Models

### Chat / Generation

| Model | Size | SK Quality | Notes |
|---|---|---|---|
| `qwen2.5:7b` | ~4.5 GB | ★★★★ | Default; good SK/EN; fast on M-series |
| `qwen2.5:14b` | ~9 GB | ★★★★★ | Better quality; slower |
| `llama3.1:8b` | ~4.7 GB | ★★★ | Fallback; weaker SK diacritics |
| `mistral:7b` | ~4.1 GB | ★★★ | Fallback |

### Embeddings (Multilingual)

| Model | Dim | SK Support | Notes |
|---|---|---|---|
| `bge-m3` | 1024 | ★★★★★ | Best multilingual; recommended |
| `nomic-embed-text` | 768 | ★★★ | English-primary; acceptable for SK |

### Vision

| Model | Notes |
|---|---|
| `minicpm-v:8b` | Lightweight; screen understanding |
| `llava:13b` | Higher quality; more RAM |

---

## Fallback Behavior

```
Primary route unavailable → try fallback → notify user if all paths fail
```

| Condition | Fallback |
|---|---|
| Ollama not running | Show "Start Ollama" button; block until started or user cancels |
| Requested model not installed | Show `ollama pull <model>` instruction; suggest smaller available model |
| Codex CLI not found | Degrade to Ollama with coding-focused system prompt; warn user quality is lower |
| Cloud LLM API key missing | Prompt user to add key in Settings → Keychain; do not attempt call |
| Cloud LLM rate limited / error | Show error; do not silently retry with different model; let user decide |
| All paths fail | Return error to user with diagnostic info; log to audit |

---

## Privacy Filter

Applied **before every cloud model call**. Never applied to Ollama calls (stays local).

### Steps

1. **PII detection**: regex + named entity recognition scan on all text to be sent.
   - Detected types: email address, phone, Slovak ID (IČO/DIČ format), IBAN, birth number (`rodné číslo`), full name patterns.
2. **Connector PII flag**: if any connector in the context has `pii_present = true`, escalate to `Ask` approval level regardless of task type.
3. **Redaction**: replace detected PII tokens with `[REDACTED_EMAIL]`, `[REDACTED_PHONE]`, `[REDACTED_ICO]`, etc.
4. **User confirmation**: show list of redacted fields in the approval modal; user may proceed or cancel.
5. **Audit**: log cloud call with `redacted_fields` count and connector sources.

### Never Send to Cloud (by default)

- Raw email bodies.
- Raw `.emlx` content.
- Odoo partner PII (email, phone, bank, IČO/DIČ).
- Screen frames.
- Keychain-sourced content.
- Content from connectors with `pii_present = true` (unless user approves with redaction).

---

## Prompt Templates

### Base System Prompt

Applied to all Ollama chat calls.

```
You are a private, local AI assistant running on a MacBook.
You help the user with tasks in Slovak and English.
You are precise, reliable, and concise.
You never make up information — if you are uncertain, say so.
You never translate Slovak legal or business terms to English.
You always preserve Slovak diacritics exactly.
```

### Slovak Business Assistant System Prompt

Applied when `language == "sk"` and task is email drafting or business text.

```
Si profesionálny asistent pre slovensky hovoriacich podnikateľov.
Odpovedaj vždy formálnym spôsobom (Vy-forma).
Zachovaj diakritiku: á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž.
Neprekladaj tieto termíny: DPH, faktúra, splatnosť, IČO, DIČ, zmluva, objednávka, zákazník, dodávateľ, odberateľ.
Obchodné e-maily začínaj s "Dobrý deň," a konči s "S pozdravom,".
Teplota odpovede: presná, žiadne domýšľanie.
```

### Language Detection Prompt

```
Detect the primary language of the following text.
Respond with exactly one of: "sk", "en", "und".
Do not add any other text.

Text: {{text}}
```

### Summarization Prompt (SK input)

```
Zhrň nasledujúcu správu v 2–3 vetách.
Zachovaj formálny tón.
Nezachovávaj pozdravy ani podpisy.
Neprekladaj odborné termíny.
Zachovaj diakritiku.

Správa:
{{message_body}}
```

### Summarization Prompt (EN input)

```
Summarize the following message in 2–3 sentences.
Keep a formal tone.
Do not include greetings or signatures.
Do not translate technical or domain terms.

Message:
{{message_body}}
```

### Codex Task Prompt (passed to Codex CLI stdin)

```json
{
  "task": "{{task_description}}",
  "context": "{{context_files_summary}}",
  "constraints": [
    "Do not modify files outside the provided working directory.",
    "Output a unified diff of all changes.",
    "Do not add explanatory comments unless asked."
  ]
}
```

---

## Slovak / English Handling Details

### Temperature Settings

| Task | Temperature |
|---|---|
| Slovak business email draft | 0.3 |
| Slovak summarization | 0.2 |
| English summarization | 0.3 |
| Classification / language detect | 0.0 |
| Creative / open-ended | 0.7 |
| Embeddings | n/a |

### Diacritics Enforcement

1. After every Ollama generation: compare input diacritics presence to output.
2. If input contains ≥ 1 diacritic character and output contains 0 diacritic characters: flag as potential diacritics corruption.
3. Log warning in audit; notify user with "Model response may have lost diacritics — review before use."

### Glossary Lock

Implemented as a post-processing pass:

```rust
const PROTECTED_TERMS: &[(&str, &str)] = &[
    ("VAT", "DPH"),
    ("invoice", "faktúra"),
    ("due date", "splatnosť"),
    ("company ID", "IČO"),
    ("tax ID", "DIČ"),
    ("contract", "zmluva"),
    ("order", "objednávka"),
    ("customer", "zákazník"),
    ("supplier", "dodávateľ"),
];
```

Applied only when `output_language == "sk"` and `input_language == "sk"`. Case-insensitive replace; whole-word match only.

---

## Routing Decision Flow

```
User turn received
    │
    ├─ language_detect(turn)  →  "sk" | "en" | "und"
    │
    ├─ classify_intent(turn)  →  task_type
    │
    ├─ check_privacy_filter(context)
    │       │
    │       ├─ pii_present?  →  must use Ollama OR user approves cloud
    │       └─ no pii        →  may use cloud if task_type → cloud
    │
    ├─ lookup_routing_table(task_type, privacy)  →  backend
    │
    ├─ apply_prompt_template(task_type, language)
    │
    ├─ call_backend(backend, prompt)
    │
    ├─ post_process(output, language)
    │       ├─ diacritics_check (if sk)
    │       └─ glossary_lock (if sk)
    │
    └─ return response
```

---

## Configuration

Stored in `connectors` table, kind `model_router`:

```json
{
  "default_chat_model": "qwen2.5:7b",
  "embedding_model": "bge-m3",
  "vision_model": "minicpm-v:8b",
  "ollama_base_url": "http://localhost:11434",
  "cloud_enabled": false,
  "cloud_provider": "anthropic",
  "cloud_model": "claude-opus-4-7",
  "cloud_api_key_keychain_id": "bagent.cloud.apikey",
  "privacy_filter_enabled": true,
  "diacritics_check_enabled": true,
  "glossary_lock_enabled": true,
  "default_language": "sk"
}
```
