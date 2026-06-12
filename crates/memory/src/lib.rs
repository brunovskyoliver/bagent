use anyhow::Result;
use chrono::Utc;
use ollama_connector::OllamaClient;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub const DEFAULT_EMBED_MODEL: &str = "bge-m3";
pub const DEDUP_COSINE_THRESHOLD: f32 = 0.92;
pub const PRUNE_UNUSED_DAYS: i64 = 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub namespace: String,
    pub kind: String,
    pub language: String,
    pub text: String,
    pub source_ref: Option<String>,
    pub metadata_json: Option<String>,
    pub last_used_at: Option<String>,
    pub use_count: i64,
    pub created_at: String,
    pub updated_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MemoryHit {
    pub item: MemoryItem,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatTurnHit {
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub score: f32,
}

pub struct MemoryStore {
    db: Arc<Mutex<Connection>>,
    ollama: OllamaClient,
    embed_model: String,
}

impl MemoryStore {
    pub fn new(db: Arc<Mutex<Connection>>, ollama: OllamaClient) -> Self {
        Self {
            db,
            ollama,
            embed_model: DEFAULT_EMBED_MODEL.to_string(),
        }
    }

    pub fn with_embed_model(mut self, model: &str) -> Self {
        self.embed_model = model.to_string();
        self
    }

    /// Insert a memory item. Returns None if deduplication blocks the insert.
    pub async fn insert(
        &self,
        namespace: &str,
        kind: &str,
        language: &str,
        text: &str,
        source_ref: Option<&str>,
        metadata_json: Option<&str>,
        expires_at: Option<&str>,
    ) -> Result<Option<String>> {
        // Dedup: check cosine similarity against existing items in same namespace
        if let Ok(embedding) = self.ollama.embed(&self.embed_model, text).await {
            let db = self.db.lock().unwrap();
            if self.is_duplicate(&db, namespace, &embedding, DEDUP_COSINE_THRESHOLD)? {
                tracing::debug!("memory insert deduplicated for namespace={namespace}");
                return Ok(None);
            }
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        {
            let db = self.db.lock().unwrap();
            db.execute(
                "INSERT INTO memory_items \
                 (id, namespace, kind, language, text, source_ref, metadata_json, created_at, updated_at, expires_at) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                rusqlite::params![
                    id, namespace, kind, language, text,
                    source_ref, metadata_json, now, now, expires_at
                ],
            )?;
        }

        // Embed asynchronously (best-effort; retrieval degrades to FTS only if missing)
        if let Ok(embedding) = self.ollama.embed(&self.embed_model, text).await {
            let db = self.db.lock().unwrap();
            let _ = self.store_embedding(&db, &id, namespace, &embedding);
        }

        Ok(Some(id))
    }

    /// Hybrid BM25 + cosine retrieval. Returns top-k hits across namespaces.
    /// Per-namespace cap applied (max 3 per namespace).
    pub async fn retrieve(
        &self,
        query: &str,
        namespaces: &[&str],
        k: usize,
    ) -> Result<Vec<MemoryHit>> {
        let query_embedding = self.ollama.embed(&self.embed_model, query).await.ok();

        let mut hits: Vec<MemoryHit> = Vec::new();

        // Placeholder for FTS queries: ?1 = search term, ?2..?N+1 = namespaces, ?N+2 = limit
        let fts_ns_placeholder = namespaces
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(",");
        // Placeholder for cosine/fallback queries: ?1..?N = namespaces, ?N+1 = limit
        let cos_ns_placeholder = namespaces
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");

        let db = self.db.lock().unwrap();

        // BM25 retrieval via FTS5
        let fts_sql = format!(
            "SELECT mi.id, mi.namespace, mi.kind, mi.language, mi.text, mi.source_ref, \
                    mi.metadata_json, mi.last_used_at, mi.use_count, mi.created_at, mi.updated_at, \
                    mi.expires_at, bm25(memory_fts) as bm25_score \
             FROM memory_fts \
             JOIN memory_items mi ON memory_fts.id = mi.id \
             WHERE memory_fts MATCH ?1 AND mi.namespace IN ({fts_ns_placeholder}) \
               AND (mi.expires_at IS NULL OR mi.expires_at > datetime('now')) \
             ORDER BY bm25_score \
             LIMIT ?{last}",
            fts_ns_placeholder = fts_ns_placeholder,
            last = namespaces.len() + 2
        );

        // Fallback: plain scan ordered by use_count (no FTS params needed for ?1)
        let fallback_sql = format!(
            "SELECT mi.id, mi.namespace, mi.kind, mi.language, mi.text, mi.source_ref, \
                    mi.metadata_json, mi.last_used_at, mi.use_count, mi.created_at, mi.updated_at, \
                    mi.expires_at, 0.0 as bm25_score \
             FROM memory_items mi \
             WHERE mi.namespace IN ({cos_ns_placeholder}) \
               AND (mi.expires_at IS NULL OR mi.expires_at > datetime('now')) \
             ORDER BY mi.use_count DESC \
             LIMIT ?{last}",
            cos_ns_placeholder = cos_ns_placeholder,
            last = namespaces.len() + 1
        );

        let limit = (k * 3) as i64;

        // Try FTS; fall back to plain scan on error (e.g. special chars in query)
        let fts_ok = if let Ok(mut stmt) = db.prepare(&fts_sql) {
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(format!("{query}*"))];
            for ns in namespaces {
                params.push(Box::new(ns.to_string()));
            }
            params.push(Box::new(limit));
            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            if let Ok(mut rows) = stmt.query(params_refs.as_slice()) {
                while let Ok(Some(row)) = rows.next() {
                    if let Ok(item) = row_to_item(row) {
                        let bm25_score: f64 = row.get(12).unwrap_or(0.0);
                        let bm25_norm = (bm25_score.abs() as f32).min(10.0) / 10.0;
                        hits.push(MemoryHit {
                            item,
                            score: bm25_norm * 0.4,
                        });
                    }
                }
                true
            } else {
                false
            }
        } else {
            false
        };

        if !fts_ok {
            if let Ok(mut stmt) = db.prepare(&fallback_sql) {
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = namespaces
                    .iter()
                    .map(|ns| Box::new(ns.to_string()) as Box<dyn rusqlite::ToSql>)
                    .collect();
                params.push(Box::new(limit));
                let params_refs: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                if let Ok(mut rows) = stmt.query(params_refs.as_slice()) {
                    while let Ok(Some(row)) = rows.next() {
                        if let Ok(item) = row_to_item(row) {
                            hits.push(MemoryHit { item, score: 0.0 });
                        }
                    }
                }
            }
        }

        // Cosine component (additive if embedding available)
        if let Some(ref qe) = query_embedding {
            let cos_sql = format!(
                "SELECT item_id, vector FROM embeddings WHERE namespace IN ({cos_ns_placeholder})",
                cos_ns_placeholder = cos_ns_placeholder
            );
            if let Ok(mut cos_stmt) = db.prepare(&cos_sql) {
                let ns_params: Vec<&dyn rusqlite::ToSql> = namespaces
                    .iter()
                    .map(|ns| ns as &dyn rusqlite::ToSql)
                    .collect();
                if let Ok(mut cos_rows) = cos_stmt.query(ns_params.as_slice()) {
                    while let Ok(Some(row)) = cos_rows.next() {
                        let item_id: String = row.get(0)?;
                        let blob: Vec<u8> = row.get(1)?;
                        let stored = blob_to_f32(&blob);
                        let cos = cosine_similarity(qe, &stored);
                        // Apply recency decay by looking up created_at
                        let decay = self.recency_decay(&db, &item_id);
                        let cos_contrib = cos * 0.6 * decay;
                        if let Some(hit) = hits.iter_mut().find(|h| h.item.id == item_id) {
                            hit.score += cos_contrib;
                        } else if cos_contrib > 0.1 {
                            // Semantic-only hit (not in FTS results)
                            if let Ok(Some(item)) = self.get(&db, &item_id) {
                                hits.push(MemoryHit {
                                    item,
                                    score: cos_contrib,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Sort descending by score
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Per-namespace cap (max 3) then global top-k
        let mut ns_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let final_hits: Vec<MemoryHit> = hits
            .into_iter()
            .filter(|h| {
                let count = ns_counts.entry(h.item.namespace.clone()).or_insert(0);
                if *count < 3 {
                    *count += 1;
                    true
                } else {
                    false
                }
            })
            .take(k)
            .collect();

        // Update last_used_at + use_count for returned items
        for hit in &final_hits {
            let now = Utc::now().to_rfc3339();
            let _ = db.execute(
                "UPDATE memory_items SET last_used_at = ?1, use_count = use_count + 1 WHERE id = ?2",
                rusqlite::params![now, hit.item.id],
            );
        }

        Ok(final_hits)
    }

    /// Retrieve relevant past chat turns via hybrid BM25+cosine over chat_turns_fts.
    /// Returns up to k turns (role, content truncated to 300 chars, created_at).
    /// Used for cross-session conversation injection.
    pub async fn retrieve_turns(
        &self,
        query: &str,
        exclude_session_id: Option<&str>,
        k: usize,
    ) -> Result<Vec<(String, String, String)>> {
        Ok(self
            .retrieve_turn_candidates(query, exclude_session_id, k)
            .await?
            .into_iter()
            .map(|h| (h.role, h.content, h.created_at))
            .collect())
    }

    /// Retrieve relevant past chat turns for diagnostics/simulation. Normal prompt
    /// assembly should treat these as candidates, not authoritative context.
    pub async fn retrieve_turn_candidates(
        &self,
        query: &str,
        exclude_session_id: Option<&str>,
        k: usize,
    ) -> Result<Vec<ChatTurnHit>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }

        let query_embedding = self.ollama.embed(&self.embed_model, query).await.ok();
        let db = self.db.lock().unwrap();

        let exclude = exclude_session_id.unwrap_or("");

        // BM25 via FTS5
        let fts_sql =
            "SELECT ct.id, ct.role, ct.content, ct.created_at, bm25(chat_turns_fts) as score \
                       FROM chat_turns_fts \
                       JOIN chat_turns ct ON chat_turns_fts.id = ct.id \
                       WHERE chat_turns_fts MATCH ?1 \
                         AND ct.role IN ('user','assistant') \
                         AND (?2 = '' OR ct.session_id != ?2) \
                       ORDER BY score \
                       LIMIT ?3";

        let limit = (k * 3) as i64;
        let mut results: Vec<(String, String, String, String, f32)> = Vec::new();

        if let Ok(mut stmt) = db.prepare(fts_sql) {
            if let Ok(mut rows) = stmt.query(rusqlite::params![format!("{query}*"), exclude, limit])
            {
                while let Ok(Some(row)) = rows.next() {
                    let id: String = row.get(0).unwrap_or_default();
                    let role: String = row.get(1).unwrap_or_default();
                    let content: String = row.get(2).unwrap_or_default();
                    let created_at: String = row.get(3).unwrap_or_default();
                    let score: f64 = row.get(4).unwrap_or(0.0);
                    results.push((
                        id,
                        role,
                        content,
                        created_at,
                        (score.abs() as f32).min(10.0) / 10.0 * 0.4,
                    ));
                }
            }
        }

        // Cosine boost
        if let Some(ref qe) = query_embedding {
            let cos_sql = "SELECT item_id, vector FROM embeddings WHERE source = 'chat_turn'";
            if let Ok(mut cos_stmt) = db.prepare(cos_sql) {
                if let Ok(mut cos_rows) = cos_stmt.query([]) {
                    while let Ok(Some(row)) = cos_rows.next() {
                        let item_id: String = row.get(0).unwrap_or_default();
                        let blob: Vec<u8> = row.get(1).unwrap_or_default();
                        let stored = blob_to_f32(&blob);
                        let cos = cosine_similarity(qe, &stored) * 0.6;
                        if let Some(r) = results.iter_mut().find(|r| r.0 == item_id) {
                            r.4 += cos;
                        } else if cos > 0.15 {
                            // Semantic-only hit
                            if let Ok(mut s) = db.prepare(
                                "SELECT id, role, content, created_at FROM chat_turns WHERE id = ?1"
                            ) {
                                if let Ok(mut rows2) = s.query(rusqlite::params![item_id]) {
                                    if let Ok(Some(r2)) = rows2.next() {
                                        let id2: String = r2.get(0).unwrap_or_default();
                                        let role2: String = r2.get(1).unwrap_or_default();
                                        let content2: String = r2.get(2).unwrap_or_default();
                                        let at2: String = r2.get(3).unwrap_or_default();
                                        results.push((id2, role2, content2, at2, cos));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        results.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));

        Ok(results
            .into_iter()
            .take(k)
            .map(|(_, role, content, created_at, score)| {
                let truncated = if content.len() > 300 {
                    let end = content.floor_char_boundary(300);
                    format!("{}…", &content[..end])
                } else {
                    content
                };
                ChatTurnHit {
                    role,
                    content: truncated,
                    created_at,
                    score,
                }
            })
            .collect())
    }

    /// Embed a chat turn and store it in embeddings with source='chat_turn'.
    pub async fn embed_chat_turn(&self, turn_id: &str, content: &str) -> Result<()> {
        let embedding = self.ollama.embed(&self.embed_model, content).await?;
        let db = self.db.lock().unwrap();
        let blob = f32_to_blob(&embedding);
        let now = Utc::now().to_rfc3339();
        db.execute(
            "INSERT OR REPLACE INTO embeddings \
             (item_id, namespace, model, dim, vector, created_at, source) \
             VALUES (?1, 'chat_turn', ?2, ?3, ?4, ?5, 'chat_turn')",
            rusqlite::params![turn_id, self.embed_model, embedding.len() as i64, blob, now],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        let db = self.db.lock().unwrap();
        let n = db.execute(
            "DELETE FROM memory_items WHERE id = ?1",
            rusqlite::params![id],
        )?;
        let _ = db.execute(
            "DELETE FROM embeddings WHERE item_id = ?1",
            rusqlite::params![id],
        );
        Ok(n > 0)
    }

    /// Remove items not retrieved in PRUNE_UNUSED_DAYS days (or expired).
    pub fn prune(&self) -> Result<usize> {
        let db = self.db.lock().unwrap();
        let n = db.execute(
            "DELETE FROM memory_items WHERE \
             (last_used_at IS NOT NULL AND last_used_at < datetime('now', ?1)) \
             OR (expires_at IS NOT NULL AND expires_at < datetime('now'))",
            rusqlite::params![format!("-{PRUNE_UNUSED_DAYS} days")],
        )?;
        if n > 0 {
            tracing::info!("memory prune removed {n} stale items");
        }
        Ok(n)
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn store_embedding(
        &self,
        db: &Connection,
        item_id: &str,
        namespace: &str,
        vec: &[f32],
    ) -> Result<()> {
        let blob = f32_to_blob(vec);
        let now = Utc::now().to_rfc3339();
        db.execute(
            "INSERT OR REPLACE INTO embeddings (item_id, namespace, model, dim, vector, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![item_id, namespace, self.embed_model, vec.len() as i64, blob, now],
        )?;
        Ok(())
    }

    fn is_duplicate(
        &self,
        db: &Connection,
        namespace: &str,
        query_vec: &[f32],
        threshold: f32,
    ) -> Result<bool> {
        let mut stmt = db.prepare(
            "SELECT e.vector FROM embeddings e \
             JOIN memory_items mi ON e.item_id = mi.id \
             WHERE e.namespace = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![namespace])?;
        while let Some(row) = rows.next()? {
            let blob: Vec<u8> = row.get(0)?;
            let stored = blob_to_f32(&blob);
            if cosine_similarity(query_vec, &stored) >= threshold {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn recency_decay(&self, db: &Connection, item_id: &str) -> f32 {
        let created_at: Option<String> = db
            .query_row(
                "SELECT created_at FROM memory_items WHERE id = ?1",
                rusqlite::params![item_id],
                |r| r.get(0),
            )
            .ok();
        if let Some(ts) = created_at {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&ts) {
                let age_days = (Utc::now() - dt.with_timezone(&Utc)).num_days() as f32;
                return (-age_days / 30.0).exp();
            }
        }
        1.0
    }

    fn get(&self, db: &Connection, id: &str) -> Result<Option<MemoryItem>> {
        let mut stmt = db.prepare(
            "SELECT id, namespace, kind, language, text, source_ref, metadata_json, \
                    last_used_at, use_count, created_at, updated_at, expires_at \
             FROM memory_items WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_item(row)?))
        } else {
            Ok(None)
        }
    }
}

// ── Vec math ─────────────────────────────────────────────────────────────────

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

fn f32_to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn blob_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn row_to_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryItem> {
    Ok(MemoryItem {
        id: row.get(0)?,
        namespace: row.get(1)?,
        kind: row.get(2)?,
        language: row.get(3)?,
        text: row.get(4)?,
        source_ref: row.get(5)?,
        metadata_json: row.get(6)?,
        last_used_at: row.get(7)?,
        use_count: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        expires_at: row.get(11)?,
    })
}
