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

Agent memory: facts, preferences, extracted entities, and summaries the agent has stored.

```sql
CREATE TABLE memory_items (
    id            TEXT PRIMARY KEY,           -- UUID v7
    namespace     TEXT NOT NULL,              -- 'global' | 'mail' | 'odoo' | 'user_pref' | ...
    kind          TEXT NOT NULL,              -- 'fact' | 'entity' | 'summary' | 'preference' | 'instruction'
    language      TEXT NOT NULL DEFAULT 'und',
    text          TEXT NOT NULL,              -- the memory content
    source_ref    TEXT,                       -- optional: e.g. 'message:<id>'
    metadata_json TEXT,                       -- additional structured data
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    expires_at    TEXT                        -- NULL = permanent
);

CREATE INDEX memory_items_namespace ON memory_items(namespace, kind);
CREATE INDEX memory_items_language  ON memory_items(language);

CREATE VIRTUAL TABLE memory_fts USING fts5(
    id UNINDEXED,
    text,
    content='memory_items',
    content_rowid='rowid',
    tokenize='unicode61'
);
```

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
