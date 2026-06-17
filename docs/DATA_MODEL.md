# Data Model

All data stored in a single SQLite database at `~/Library/Application Support/bagent/bagent.db`.
Schema migrations managed by `refinery` crate. SQLCipher encryption keyed from Keychain.

---

## Table Definitions

### `messages`

Email and other message-type records from connectors.

```sql
CREATE TABLE messages (
    id            TEXT PRIMARY KEY,           -- UUID v7
    source        TEXT NOT NULL,              -- 'apple_mail' | 'whatsapp' | ...
    external_id   TEXT NOT NULL,              -- connector-native ID (e.g. Mail message-id)
    language      TEXT NOT NULL DEFAULT 'und',-- 'sk' | 'en' | 'und'
    subject       TEXT,
    body          TEXT,                       -- stripped plaintext (no raw HTML)
    body_html     TEXT,                       -- raw HTML if available (not indexed)
    sender        TEXT,                       -- RFC 5322 address
    recipients    TEXT,                       -- JSON array of addresses
    received_at   TEXT NOT NULL,              -- ISO 8601
    indexed_at    TEXT,                       -- when body was extracted and indexed
    thread_id     TEXT,                       -- connector thread grouping key
    mailbox       TEXT,                       -- 'INBOX' | 'Sent' | ...
    is_read       INTEGER NOT NULL DEFAULT 0,
    UNIQUE(source, external_id)
);

CREATE INDEX messages_received_at ON messages(received_at DESC);
CREATE INDEX messages_language    ON messages(language);
CREATE INDEX messages_sender      ON messages(sender);

CREATE VIRTUAL TABLE messages_fts USING fts5(
    id UNINDEXED,
    subject,
    body,
    sender,
    content='messages',
    content_rowid='rowid',
    tokenize='unicode61'
);
```

---

### `notes`

Notes from Apple Notes or other note connectors.

```sql
CREATE TABLE notes (
    id          TEXT PRIMARY KEY,             -- UUID v7
    source      TEXT NOT NULL,               -- 'apple_notes'
    external_id TEXT NOT NULL,
    language    TEXT NOT NULL DEFAULT 'und',
    title       TEXT,
    body        TEXT,                        -- Markdown (converted from HTML)
    folder      TEXT,
    created_at  TEXT,
    updated_at  TEXT NOT NULL,
    indexed_at  TEXT,
    UNIQUE(source, external_id)
);

CREATE VIRTUAL TABLE notes_fts USING fts5(
    id UNINDEXED,
    title,
    body,
    content='notes',
    content_rowid='rowid',
    tokenize='unicode61'
);
```

---

### `tool_calls`

Record of every tool invocation attempt.

```sql
CREATE TABLE tool_calls (
    id           TEXT PRIMARY KEY,            -- UUID v7
    tool         TEXT NOT NULL,               -- tool name e.g. 'mail_list_inbox'
    connector    TEXT NOT NULL,
    args_json    TEXT NOT NULL,               -- JSON of tool arguments
    side_effect  TEXT NOT NULL,               -- SideEffectClass enum value
    started_at   TEXT NOT NULL,               -- ISO 8601
    finished_at  TEXT,
    status       TEXT NOT NULL DEFAULT 'pending',  -- 'pending'|'running'|'success'|'error'|'denied'|'timeout'
    result_json  TEXT,
    error        TEXT,
    approval_id  TEXT REFERENCES approvals(id),
    audit_id     TEXT REFERENCES audit_entries(id),
    session_id   TEXT NOT NULL
);

CREATE INDEX tool_calls_session ON tool_calls(session_id, started_at DESC);
CREATE INDEX tool_calls_tool    ON tool_calls(tool, started_at DESC);
```

---

### `approvals`

Human approval decisions for tool calls and cloud model access.

```sql
CREATE TABLE approvals (
    id            TEXT PRIMARY KEY,           -- UUID v7
    session_id    TEXT NOT NULL,
    tool_call_id  TEXT REFERENCES tool_calls(id),
    action_type   TEXT NOT NULL,             -- 'tool_exec' | 'cloud_llm' | 'send_email' | 'odoo_write' | ...
    request_json  TEXT NOT NULL,             -- full context shown to user (args, dry_run_diff, etc.)
    dry_run_diff  TEXT,                      -- unified diff for write operations
    decision      TEXT,                      -- 'allow' | 'deny' | 'timeout' | NULL (pending)
    decided_by    TEXT NOT NULL DEFAULT 'human',
    decided_at    TEXT,
    reason        TEXT,                      -- optional user-provided note
    expires_at    TEXT NOT NULL,             -- ISO 8601; auto-deny after this
    created_at    TEXT NOT NULL
);

CREATE INDEX approvals_pending ON approvals(decision) WHERE decision IS NULL;
```

---

### `memory_items`

Agent memory: facts, preferences, extracted entities, glossary, style profiles, corrections, and contacts.

```sql
-- V4 base schema
CREATE TABLE memory_items (
    id            TEXT PRIMARY KEY,           -- UUID v7
    namespace     TEXT NOT NULL,              -- 'global' | 'user_pref' | 'sk_glossary' | 'style_profile'
                                              --   | 'contacts' | 'corrections' | 'negative_rules' | ...
    kind          TEXT NOT NULL,              -- 'fact' | 'entity' | 'preference' | 'instruction'
                                              --   | 'correction' | 'sk_glossary' | 'style_profile'
                                              --   | 'contact' | 'project' | 'workflow' | 'negative_rule' | 'profile'
    language      TEXT NOT NULL DEFAULT 'und',
    text          TEXT NOT NULL,              -- the memory content
    source_ref    TEXT,                       -- optional: e.g. 'message:<id>'
    metadata_json TEXT,                       -- additional structured data
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    expires_at    TEXT,                       -- NULL = permanent
    -- V11 ledger columns:
    confidence    REAL    NOT NULL DEFAULT 0.8,   -- extractor confidence [0,1]
    importance    REAL    NOT NULL DEFAULT 0.5,   -- importance score [0,1]; feeds retrieval ranking
    status        TEXT    NOT NULL DEFAULT 'active',   -- 'active' | 'superseded' | 'deleted'
    source        TEXT    NOT NULL DEFAULT 'passive',  -- 'passive' | 'explicit' | 'user_edit' | 'import'
    sensitivity   TEXT    NOT NULL DEFAULT 'normal',   -- 'normal' | 'sensitive'
    subject       TEXT,                        -- optional subject tag (person, project, entity)
    supersedes_id TEXT                         -- id of the item this row supersedes
);

-- Retrieval always hard-filters on (status, namespace, kind)
CREATE INDEX idx_memory_items_status_ns_kind ON memory_items (status, namespace, kind);
CREATE INDEX memory_items_language  ON memory_items(language);

CREATE VIRTUAL TABLE memory_fts USING fts5(
    id UNINDEXED,
    text,
    content='memory_items',
    content_rowid='rowid',
    tokenize='unicode61'
);
```

**Retrieval invariants:**
- Only `status='active'` rows are returned by `retrieve_filtered`.
- Only `sensitivity='normal'` rows are returned (sensitive items require explicit opt-in never currently granted).
- Passive extraction is blocked for `sensitivity='sensitive'` items regardless of confidence/importance.
- Explicit/user_edit insertion against an active passive item in the same namespace (cosine > 0.75) → supersedes the passive item (`status='superseded'`).
- `prune()` only hard-deletes rows with `status IN ('deleted','superseded')` older than the prune window.

---

### `connectors`

Configuration and sync state for each connector instance.

```sql
CREATE TABLE connectors (
    id           TEXT PRIMARY KEY,            -- e.g. 'apple_mail', 'odoo_prod', 'ollama'
    kind         TEXT NOT NULL,               -- connector type enum
    display_name TEXT NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1,
    config_json  TEXT NOT NULL DEFAULT '{}',  -- non-secret config (URL, model names, etc.)
    pii_present  INTEGER NOT NULL DEFAULT 0,  -- 1 = connector data contains PII
    can_read     INTEGER NOT NULL DEFAULT 1,
    can_write    INTEGER NOT NULL DEFAULT 0,
    last_sync_at TEXT,
    sync_cursor  TEXT,                        -- connector-specific continuation token
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
```

---

### `audit_entries`

Append-only audit log. Never UPDATE or DELETE.

```sql
CREATE TABLE audit_entries (
    id           TEXT PRIMARY KEY,            -- UUID v7
    seq          INTEGER NOT NULL UNIQUE,     -- monotonically increasing
    prev_hash    TEXT NOT NULL,               -- SHA-256 of previous row's JSON representation
    session_id   TEXT,
    actor        TEXT NOT NULL,               -- 'user' | 'agent' | 'system' | 'daemon'
    action       TEXT NOT NULL,               -- e.g. 'tool_call', 'approval_decision', 'model_invoke', 'sync'
    connector    TEXT,
    tool         TEXT,
    model        TEXT,
    language     TEXT,
    payload_json TEXT NOT NULL,               -- full context (redacted PII)
    outcome      TEXT,                        -- 'success' | 'error' | 'denied' | 'timeout'
    created_at   TEXT NOT NULL               -- ISO 8601 with microsecond precision
    -- NO foreign keys — audit is independent; referenced records may be deleted but audit stays
);

CREATE INDEX audit_entries_session    ON audit_entries(session_id, seq DESC);
CREATE INDEX audit_entries_created_at ON audit_entries(created_at DESC);
CREATE INDEX audit_entries_action     ON audit_entries(action, created_at DESC);
```

Hash chain: `prev_hash = SHA256(JSON(previous_row))` where `JSON(row)` = canonical JSON of all columns except `prev_hash`. First row: `prev_hash = "genesis"`.

---

### `embeddings`

Vector embeddings for semantic search over messages, notes, and memory items.

```sql
-- Requires sqlite-vss or sqlite-vec extension loaded at runtime.

CREATE TABLE embeddings (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id   TEXT NOT NULL,     -- UUID of the source record
    namespace TEXT NOT NULL,     -- 'messages' | 'notes' | 'memory_items'
    model     TEXT NOT NULL,     -- embedding model name e.g. 'bge-m3'
    dim       INTEGER NOT NULL,  -- vector dimension e.g. 1024
    vector    BLOB NOT NULL      -- float32 array, little-endian
);

CREATE INDEX embeddings_namespace ON embeddings(namespace, item_id);

-- sqlite-vec virtual table for ANN search:
-- CREATE VIRTUAL TABLE vec_items USING vec0(
--     item_id TEXT,
--     namespace TEXT,
--     embedding FLOAT[1024]
-- );
```

---

### `sessions`

Chat sessions linking turns, tool calls, and approvals together.

```sql
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,             -- UUID v7
    started_at  TEXT NOT NULL,
    ended_at    TEXT,
    language    TEXT,                         -- dominant language of session
    summary     TEXT,                         -- agent-generated summary (on close)
    metadata_json TEXT
);
```

---

## Language Metadata

Every table that stores user-derived text includes a `language` column with values:
- `sk` — Slovak
- `en` — English
- `und` — undetermined / mixed

Language is set during indexing by the language detector (Ollama local call). It is never changed after initial classification unless the user explicitly triggers re-classification.

---

## Indexes and Search Strategy

### Full-Text Search (FTS5)

- `messages_fts`: subject + body + sender.
- `notes_fts`: title + body.
- `memory_fts`: memory text.
- Tokenizer: `unicode61` (handles Slovak diacritics correctly — unlike ASCII-only tokenizers).
- BM25 ranking via `bm25(table_fts)` in ORDER BY.

### Semantic Search

- ANN query via `sqlite-vec` virtual table.
- Hybrid retrieval: BM25 score + cosine similarity combined with `0.4 * bm25 + 0.6 * cosine` (tunable).
- Re-ranked by recency: `score * exp(-age_days / 30)`.

### Namespace Isolation

Each connector writes into its own namespace. Cross-connector queries must explicitly union namespaces. This prevents, e.g., a Notes search from accidentally returning Odoo record snippets.

---

## Migration Strategy

- Migrations in `crates/daemon/migrations/` numbered `V001__init.sql`, `V002__...sql`.
- `refinery` runs migrations at daemon startup; rolls back on error (SQLite transactions).
- Schema version stored in `refinery_schema_history` table (created automatically by refinery).
- Breaking schema changes (column rename, type change): always add new column + migration; deprecate old in next version.

---

## Phase 8 — Codex Task Harness

### Task Rating (`crates/agent/src/task_rater.rs`)

Deterministic scoring against bilingual SK/EN keyword gates. No LLM involvement in the
rating decision. Score maps to level:

| Score | `TaskLevel` | `ContextScope` | Approval required |
|---|---|---|---|
| 0–9 | `LocalOnly` | `None` | No |
| 10–29 | `LocalPreferred` | `None` | No |
| 30–59 | `CodexCandidate` | `SummariesOnly` | Yes (if run) |
| 60–84 | `CodexRecommended` | `SelectedRecords` | Yes |
| 85+ | `CodexRequired` | `UserApprovedPacket` | Yes |

`PrivacyRisk` levels: `Low` → `Medium` → `High` → `Sensitive`.
Raised by: raw mail/WhatsApp bodies, Odoo customer records, invoices, personal data, legal disputes, credentials, private notes, memory, screenshots.

### Context Packet (`CodexContextPacket`)

```rust
pub struct CodexContextPacket {
    pub user_request: String,
    pub allowed_context: Vec<ContextItem>,
    pub forbidden_context_types: Vec<String>,
    pub constraints: Vec<String>,
    pub expected_output: CodexExpectedOutput,
}

pub struct ContextItem {
    pub source: String,       // "apple_mail" | "apple_notes" | "odoo" | ...
    pub title: String,
    pub summary: String,      // truncated summary, never raw body by default
    pub record_ref: String,   // opaque ID for citation
    pub pii: bool,
}
```

Default `forbidden_context_types` (always present, never overridable by Codex):
`credentials`, `tokens`, `keychain`, `database_raw`, `memory_db`, `browser_stores`,
`ssh_keys`, `gnupg`, `password_managers`, `raw_mail_bodies` (unless explicitly approved),
`screenshots` (unless explicitly included), `unrelated_private_files`.

### Codex Audit Log

Every `/codex/run-task` attempt writes to the existing `audit_log` table with:
```json
{
  "action": "codex_run_task",
  "description": "<task description>",
  "level": "CodexRecommended",
  "privacy_risk": "Medium",
  "context_sources": ["apple_mail", "odoo"],
  "approval_id": "<uuid>",
  "exit_code": 0,
  "timed_out": false,
  "output_hash": "<sha256 hex>"
}
```
Raw private context bodies are **never** included in the audit payload.

### Codex Configuration

Stored in `UserDefaults` (SwiftUI layer) — not in the SQLite DB:
- `bagent.codex_path` — user-specified binary path; empty = auto-discover from `$PATH`

The daemon reads this on `/codex/run-task` via the `CodexConfig` passed at startup.
Binary presence is exposed via `/codex/status` and the `connectors.codex` field in `/health`.
