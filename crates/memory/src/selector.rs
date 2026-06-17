//! MemorySelector — thin wrapper over MemoryStore::retrieve_filtered that
//! enforces the global limits and token budget defined in the brief.

use crate::{MemoryHit, MemoryStore};
use anyhow::Result;
use std::sync::Arc;

/// Hard limits for memory injection per prompt.
pub const MAX_MEMORY_CARDS: usize = 6;
pub const MAX_PER_NAMESPACE: usize = 3;
/// Rough char budget for the memory section of the prompt (~900–1200 tokens).
pub const MAX_MEMORY_CHARS: usize = 4800;

/// Parameters driven by the ContextPlan.
pub struct SelectQuery<'a> {
    pub query: &'a str,
    pub namespaces: &'a [String],
    pub kinds: &'a [String],
    /// Override global max if the planner wants fewer cards.
    pub max_cards: Option<usize>,
}

/// Select relevant memory cards. Returns [] when nothing passes the score threshold.
pub async fn select(store: &Arc<MemoryStore>, q: SelectQuery<'_>) -> Result<Vec<MemoryHit>> {
    if q.namespaces.is_empty() {
        return Ok(vec![]);
    }

    let ns_refs: Vec<&str> = q.namespaces.iter().map(|s| s.as_str()).collect();
    let kind_refs: Vec<&str> = q.kinds.iter().map(|s| s.as_str()).collect();
    let max_cards = q.max_cards.unwrap_or(MAX_MEMORY_CARDS);

    let hits = store
        .retrieve_filtered(crate::RetrieveQuery {
            query: q.query,
            namespaces: &ns_refs,
            kinds: &kind_refs,
            k: max_cards,
            max_per_namespace: MAX_PER_NAMESPACE,
            score_threshold: 0.0,
            allow_sensitive: false,
        })
        .await?;

    // Token budget cap: truncate if accumulated text would exceed budget
    let mut result = Vec::new();
    let mut total_chars = 0usize;
    for hit in hits {
        let chars = hit.item.text.len();
        if total_chars + chars > MAX_MEMORY_CHARS {
            break;
        }
        total_chars += chars;
        result.push(hit);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryStore;
    use ollama_connector::OllamaClient;
    use rusqlite::Connection;
    use std::sync::Mutex;

    fn test_store() -> Arc<MemoryStore> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(TEST_SCHEMA).unwrap();
        Arc::new(MemoryStore::new(
            Arc::new(Mutex::new(conn)),
            OllamaClient::new("http://127.0.0.1:9"), // unreachable — no embed calls in these tests
        ))
    }

    // Minimal schema matching V4 + V11 columns
    const TEST_SCHEMA: &str = "
        CREATE TABLE memory_items (
            id TEXT PRIMARY KEY,
            namespace TEXT NOT NULL,
            kind TEXT NOT NULL,
            language TEXT NOT NULL DEFAULT 'und',
            text TEXT NOT NULL,
            source_ref TEXT,
            metadata_json TEXT,
            last_used_at TEXT,
            use_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            expires_at TEXT,
            confidence REAL NOT NULL DEFAULT 0.8,
            importance REAL NOT NULL DEFAULT 0.5,
            status TEXT NOT NULL DEFAULT 'active',
            source TEXT NOT NULL DEFAULT 'passive',
            sensitivity TEXT NOT NULL DEFAULT 'normal',
            subject TEXT,
            supersedes_id TEXT
        );
        CREATE VIRTUAL TABLE memory_fts USING fts5(
            id UNINDEXED, text,
            content='memory_items', content_rowid='rowid',
            tokenize='unicode61'
        );
        CREATE TABLE embeddings (
            item_id TEXT PRIMARY KEY,
            namespace TEXT NOT NULL,
            model TEXT NOT NULL,
            dim INTEGER NOT NULL,
            vector BLOB NOT NULL,
            created_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'memory_item'
        );
        CREATE TABLE chat_turns (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            language TEXT NOT NULL DEFAULT 'und',
            model TEXT,
            created_at TEXT NOT NULL,
            parent_turn_id TEXT
        );
        CREATE VIRTUAL TABLE chat_turns_fts USING fts5(
            id UNINDEXED, content,
            content='chat_turns', content_rowid='rowid',
            tokenize='unicode61'
        );
    ";

    fn insert_active(
        conn: &Connection,
        id: &str,
        ns: &str,
        kind: &str,
        text: &str,
        status: &str,
        sensitivity: &str,
    ) {
        conn.execute(
            "INSERT INTO memory_items (id, namespace, kind, language, text, created_at, updated_at, status, sensitivity)
             VALUES (?1,?2,?3,'und',?4,datetime('now'),datetime('now'),?5,?6)",
            rusqlite::params![id, ns, kind, text, status, sensitivity],
        ).unwrap();
        conn.execute(
            "INSERT INTO memory_fts(rowid, id, text) SELECT rowid, id, text FROM memory_items WHERE id = ?1",
            rusqlite::params![id],
        ).unwrap();
    }

    // Tests that require DB access use the test helpers in lib.rs via
    // the public MemoryStore API — we can't access private fields here.
    // The selector logic itself is tested indirectly via the integration
    // tests in the daemon crate and in lib.rs's own tests.

    #[tokio::test]
    async fn returns_empty_for_no_namespaces() {
        let store = test_store();
        let hits = select(
            &store,
            SelectQuery {
                query: "test",
                namespaces: &[],
                kinds: &[],
                max_cards: None,
            },
        )
        .await
        .unwrap();
        assert!(hits.is_empty());
    }
}
