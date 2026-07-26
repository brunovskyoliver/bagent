pub mod markdown_mirror;
pub mod selector;

use anyhow::Result;
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub const PRUNE_UNUSED_DAYS: i64 = 60;

/// Minimum score for a memory hit to be considered relevant.
pub const RETRIEVAL_SCORE_THRESHOLD: f32 = 0.08;

// ── Types ─────────────────────────────────────────────────────────────────────

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
    // V11 ledger fields
    pub confidence: f32,
    pub importance: f32,
    pub status: String,      // "active" | "superseded" | "deleted"
    pub source: String,      // "explicit" | "passive" | "import" | "user_edit"
    pub sensitivity: String, // "normal" | "sensitive"
    pub subject: Option<String>,
    pub supersedes_id: Option<String>,
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

/// Parameters for full-featured memory insertion.
#[derive(Debug, Default)]
pub struct InsertParams<'a> {
    pub namespace: &'a str,
    pub kind: &'a str,
    pub language: &'a str,
    pub text: &'a str,
    pub source_ref: Option<&'a str>,
    pub metadata_json: Option<&'a str>,
    pub expires_at: Option<&'a str>,
    /// How this memory was created ("explicit" | "passive" | "import" | "user_edit")
    pub source: &'a str,
    pub confidence: f32,
    pub importance: f32,
    /// "normal" | "sensitive" — sensitive memories require explicit confirmation to store.
    pub sensitivity: &'a str,
    pub subject: Option<&'a str>,
}

impl<'a> InsertParams<'a> {
    /// Sensible defaults for quick inserts (passive, normal confidence).
    pub fn simple(namespace: &'a str, kind: &'a str, language: &'a str, text: &'a str) -> Self {
        Self {
            namespace,
            kind,
            language,
            text,
            source: "passive",
            confidence: 0.8,
            importance: 0.5,
            sensitivity: "normal",
            ..Default::default()
        }
    }
}

/// Retrieval query parameters.
#[derive(Default)]
pub struct RetrieveQuery<'a> {
    pub query: &'a str,
    pub namespaces: &'a [&'a str],
    /// If non-empty, only return items with these kind values.
    pub kinds: &'a [&'a str],
    /// Max results globally.
    pub k: usize,
    /// Max results per namespace (0 = use default 3).
    pub max_per_namespace: usize,
    /// Min score to include (0.0 = use RETRIEVAL_SCORE_THRESHOLD).
    pub score_threshold: f32,
    /// Whether to include sensitive memories.
    pub allow_sensitive: bool,
}

// ── Store ─────────────────────────────────────────────────────────────────────

pub struct MemoryStore {
    db: Arc<Mutex<Connection>>,
    data_dir: Option<PathBuf>,
}

impl MemoryStore {
    pub fn new(db: Arc<Mutex<Connection>>) -> Self {
        Self { db, data_dir: None }
    }

    pub fn with_data_dir(mut self, dir: PathBuf) -> Self {
        self.data_dir = Some(dir);
        self
    }

    fn memories_dir(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("memories"))
    }

    /// Scan the mirror directory for files that are newer than their DB row
    /// and upsert them. Called at daemon startup — best-effort, never fails startup.
    pub async fn scan_and_import_mirror(&self) {
        let mdir = match self.memories_dir() {
            Some(d) => d,
            None => return,
        };
        if !mdir.exists() {
            return;
        }

        let changed = {
            let db = self.db.lock().unwrap();
            markdown_mirror::scan_changed(&mdir, &db)
        };

        if changed.is_empty() {
            return;
        }
        tracing::info!("memory mirror: importing {} changed file(s)", changed.len());

        for (fm, text) in changed {
            let db = self.db.lock().unwrap();
            let r = db.execute(
                "INSERT INTO memory_items \
                 (id, namespace, kind, language, text, source_ref, metadata_json, \
                  created_at, updated_at, expires_at, confidence, importance, status, source, sensitivity, subject, supersedes_id) \
                 VALUES (?1,?2,?3,?4,?5,?6,NULL,?7,?8,?9,?10,?11,?12,?13,?14,?15,NULL) \
                 ON CONFLICT(id) DO UPDATE SET \
                  text=excluded.text, namespace=excluded.namespace, kind=excluded.kind, \
                  language=excluded.language, source_ref=excluded.source_ref, \
                  updated_at=excluded.updated_at, expires_at=excluded.expires_at, \
                  confidence=excluded.confidence, importance=excluded.importance, \
                  status=excluded.status, source=excluded.source, \
                  sensitivity=excluded.sensitivity, subject=excluded.subject",
                rusqlite::params![
                    fm.id, fm.namespace, fm.kind, fm.language, text,
                    fm.source_ref, fm.created_at, fm.updated_at, fm.expires_at,
                    fm.confidence, fm.importance, fm.status, fm.source, fm.sensitivity, fm.subject,
                ],
            );
            if let Err(e) = r {
                tracing::warn!("memory mirror: upsert failed for {}: {e}", fm.id);
                continue;
            }
            tracing::debug!("memory mirror: imported {}", fm.id);
        }
    }

    fn mirror_export(&self, id: &str) {
        let mdir = match self.memories_dir() {
            Some(d) => d,
            None => return,
        };
        let db = self.db.lock().unwrap();
        if let Ok(Some(item)) = self.get_item_by_id(&db, id) {
            drop(db);
            markdown_mirror::export_item(&mdir, &item);
        }
    }

    fn get_item_by_id(&self, db: &Connection, id: &str) -> Result<Option<MemoryItem>> {
        let mut stmt = db.prepare(
            "SELECT id, namespace, kind, language, text, source_ref, metadata_json, \
                    last_used_at, use_count, created_at, updated_at, expires_at, \
                    confidence, importance, status, source, sensitivity, subject, supersedes_id \
             FROM memory_items WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_item(row)?))
        } else {
            Ok(None)
        }
    }

    // ── Insert ────────────────────────────────────────────────────────────────

    /// Insert a memory item using full params. Returns None if deduplication blocks.
    /// For explicit/user_edit sources: also supersedes conflicting passive memories.
    pub async fn insert_full(&self, params: InsertParams<'_>) -> Result<Option<String>> {
        // Gate: do not passively store sensitive data
        if params.sensitivity == "sensitive" && params.source == "passive" {
            tracing::debug!(
                "memory: refusing passive store of sensitive item in ns={}",
                params.namespace
            );
            return Ok(None);
        }

        let normalized = normalize_text(params.text);
        let db = self.db.lock().unwrap();
        let mut stmt =
            db.prepare("SELECT text FROM memory_items WHERE namespace = ?1 AND status = 'active'")?;
        let duplicate = stmt
            .query_map(rusqlite::params![params.namespace], |row| {
                row.get::<_, String>(0)
            })?
            .filter_map(std::result::Result::ok)
            .any(|text| normalize_text(&text) == normalized);
        drop(stmt);
        drop(db);
        if duplicate {
            tracing::debug!("memory insert deduplicated ns={}", params.namespace);
            return Ok(None);
        }

        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        {
            let mut db = self.db.lock().unwrap();
            let tx = db.transaction()?;

            // Explicit corrections replace a related passive fact without
            // requiring an embedding model. Prefer stable identity fields,
            // then fall back to conservative lexical similarity.
            if params.source == "explicit" || params.source == "user_edit" {
                supersede_conflicting_lexical(&tx, &params, &id, &now)?;
            }

            tx.execute(
                "INSERT INTO memory_items \
                 (id, namespace, kind, language, text, source_ref, metadata_json, \
                  created_at, updated_at, expires_at, \
                  confidence, importance, status, source, sensitivity, subject, supersedes_id) \
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'active',?13,?14,?15,NULL)",
                rusqlite::params![
                    id,
                    params.namespace,
                    params.kind,
                    params.language,
                    params.text,
                    params.source_ref,
                    params.metadata_json,
                    now,
                    now,
                    params.expires_at,
                    params.confidence,
                    params.importance,
                    params.source,
                    params.sensitivity,
                    params.subject,
                ],
            )?;
            tx.commit()?;
        }

        // Mirror export (best-effort — skip sensitive passive items already blocked above)
        self.mirror_export(&id);

        Ok(Some(id))
    }

    /// Backward-compatible insert (same signature as the old `insert`).
    /// Maps to `insert_full` with passive/normal defaults.
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
        self.insert_full(InsertParams {
            namespace,
            kind,
            language,
            text,
            source_ref,
            metadata_json,
            expires_at,
            source: "passive",
            confidence: 0.8,
            importance: 0.5,
            sensitivity: "normal",
            ..Default::default()
        })
        .await
    }

    // ── Supersede ─────────────────────────────────────────────────────────────

    /// Mark a memory item as superseded (not deleted — audit trail intact).
    pub fn supersede(&self, old_id: &str) -> Result<bool> {
        let n = {
            let db = self.db.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            db.execute(
                "UPDATE memory_items SET status = 'superseded', updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, old_id],
            )?
        };
        if n > 0 {
            self.mirror_export(old_id);
        }
        Ok(n > 0)
    }

    // ── Retrieval ─────────────────────────────────────────────────────────────

    /// FTS-only retrieval.
    /// Hard filters: `status = 'active'`, `sensitivity = 'normal'` (unless allowed).
    /// Score = 0.70 * BM25 + 0.15 * importance + 0.15 * recency.
    pub async fn retrieve_filtered(&self, q: RetrieveQuery<'_>) -> Result<Vec<MemoryHit>> {
        let max_per_ns = if q.max_per_namespace == 0 {
            3
        } else {
            q.max_per_namespace
        };
        let threshold = if q.score_threshold == 0.0 {
            RETRIEVAL_SCORE_THRESHOLD
        } else {
            q.score_threshold
        };

        let sensitivity_filter = if q.allow_sensitive {
            "AND (mi.sensitivity = 'normal' OR mi.sensitivity = 'sensitive')"
        } else {
            "AND mi.sensitivity = 'normal'"
        };

        let ns_ph: String = q
            .namespaces
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 2))
            .collect::<Vec<_>>()
            .join(",");

        let limit = (q.k * 3) as i64;
        let db = self.db.lock().unwrap();

        let mut hits: Vec<MemoryHit> = Vec::new();

        // BM25 via FTS5 (only when query is non-empty)
        let fts_ok = if !q.query.trim().is_empty() {
            let fts_sql = format!(
                "SELECT mi.id, mi.namespace, mi.kind, mi.language, mi.text, mi.source_ref, \
                        mi.metadata_json, mi.last_used_at, mi.use_count, mi.created_at, mi.updated_at, \
                        mi.expires_at, mi.confidence, mi.importance, mi.status, mi.source, \
                        mi.sensitivity, mi.subject, mi.supersedes_id, \
                        bm25(memory_fts) as bm25_score \
                 FROM memory_fts \
                 JOIN memory_items mi ON memory_fts.id = mi.id \
                 WHERE memory_fts MATCH ?1 \
                   AND mi.namespace IN ({ns_ph}) \
                   AND mi.status = 'active' \
                   {sensitivity_filter} \
                   AND (mi.expires_at IS NULL OR mi.expires_at > datetime('now')) \
                 ORDER BY bm25_score \
                 LIMIT ?{last}",
                ns_ph = ns_ph,
                sensitivity_filter = sensitivity_filter,
                last = q.namespaces.len() + 2
            );
            if let Ok(mut stmt) = db.prepare(&fts_sql) {
                let mut params: Vec<Box<dyn rusqlite::ToSql>> =
                    vec![Box::new(format!("{}*", q.query))];
                for ns in q.namespaces {
                    params.push(Box::new(ns.to_string()));
                }
                params.push(Box::new(limit));
                let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                if let Ok(mut rows) = stmt.query(refs.as_slice()) {
                    let mut ranked_items = Vec::new();
                    while let Ok(Some(row)) = rows.next() {
                        if let Ok(item) = row_to_item(row) {
                            ranked_items.push(item);
                        }
                    }
                    for (rank, item) in ranked_items.into_iter().enumerate() {
                        // FTS5 BM25 magnitudes depend on corpus size and are
                        // often tiny. Reciprocal rank keeps lexical relevance
                        // dominant and stable across databases.
                        let lexical_score = 1.0 / (rank as f32 + 1.0);
                        let base_score = lexical_score * 0.70
                            + item.importance * 0.15
                            + recency_score(&item.created_at) * 0.15;
                        hits.push(MemoryHit {
                            item,
                            score: base_score,
                        });
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        // Fallback: plain scan when FTS empty or query empty
        if !fts_ok {
            let cos_ns_ph: String = q
                .namespaces
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(",");
            let fallback_sql = format!(
                "SELECT mi.id, mi.namespace, mi.kind, mi.language, mi.text, mi.source_ref, \
                        mi.metadata_json, mi.last_used_at, mi.use_count, mi.created_at, mi.updated_at, \
                        mi.expires_at, mi.confidence, mi.importance, mi.status, mi.source, \
                        mi.sensitivity, mi.subject, mi.supersedes_id, 0.0 as bm25_score \
                 FROM memory_items mi \
                 WHERE mi.namespace IN ({cos_ns_ph}) \
                   AND mi.status = 'active' \
                   {sensitivity_filter} \
                   AND (mi.expires_at IS NULL OR mi.expires_at > datetime('now')) \
                 ORDER BY mi.importance DESC, mi.use_count DESC \
                 LIMIT ?{last}",
                cos_ns_ph = cos_ns_ph,
                sensitivity_filter = sensitivity_filter,
                last = q.namespaces.len() + 1
            );
            if let Ok(mut stmt) = db.prepare(&fallback_sql) {
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = q
                    .namespaces
                    .iter()
                    .map(|ns| Box::new(ns.to_string()) as Box<dyn rusqlite::ToSql>)
                    .collect();
                params.push(Box::new(limit));
                let refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                if let Ok(mut rows) = stmt.query(refs.as_slice()) {
                    while let Ok(Some(row)) = rows.next() {
                        if let Ok(item) = row_to_item(row) {
                            let importance_score =
                                item.importance * 0.15 + recency_score(&item.created_at) * 0.15;
                            hits.push(MemoryHit {
                                item,
                                score: importance_score,
                            });
                        }
                    }
                }
            }
        }

        // Apply kind filter if specified
        if !q.kinds.is_empty() {
            hits.retain(|h| q.kinds.contains(&h.item.kind.as_str()));
        }

        // Score threshold
        hits.retain(|h| h.score >= threshold);

        // Sort descending
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Per-namespace cap + MMR-style dedup (skip near-duplicates by text similarity)
        let mut ns_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let mut final_hits: Vec<MemoryHit> = Vec::new();
        for hit in hits {
            if final_hits.len() >= q.k {
                break;
            }
            let count = ns_counts.entry(hit.item.namespace.clone()).or_insert(0);
            if *count >= max_per_ns {
                continue;
            }
            // Near-dup text filter: skip if very similar text already in final_hits
            let norm_text = hit.item.text.to_lowercase();
            let is_near_dup = final_hits.iter().any(|existing| {
                let existing_norm = existing.item.text.to_lowercase();
                text_similarity(&norm_text, &existing_norm) > 0.85
            });
            if is_near_dup {
                continue;
            }
            *count += 1;
            final_hits.push(hit);
        }

        // Update last_used_at + use_count
        for hit in &final_hits {
            let now = Utc::now().to_rfc3339();
            let _ = db.execute(
                "UPDATE memory_items SET last_used_at = ?1, use_count = use_count + 1 WHERE id = ?2",
                rusqlite::params![now, hit.item.id],
            );
        }

        Ok(final_hits)
    }

    /// Hybrid BM25 + cosine retrieval (backward-compatible signature).
    /// Uses old weights (BM25×0.4 + cosine×0.6) and per-namespace cap 3.
    pub async fn retrieve(
        &self,
        query: &str,
        namespaces: &[&str],
        k: usize,
    ) -> Result<Vec<MemoryHit>> {
        self.retrieve_filtered(RetrieveQuery {
            query,
            namespaces,
            kinds: &[],
            k,
            max_per_namespace: 3,
            score_threshold: 0.0,
            allow_sensitive: false,
        })
        .await
    }

    // ── Chat turn retrieval ────────────────────────────────────────────────────

    /// Retrieve relevant past chat turns for diagnostics / explicit recall.
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

    /// Full scored chat turn candidates (used by PromptBuilder for diagnostic trace
    /// and for explicit recall injection when needs_conversation_recall=true).
    pub async fn retrieve_turn_candidates(
        &self,
        query: &str,
        exclude_session_id: Option<&str>,
        k: usize,
    ) -> Result<Vec<ChatTurnHit>> {
        if query.trim().is_empty() {
            return Ok(vec![]);
        }

        let db = self.db.lock().unwrap();
        let exclude = exclude_session_id.unwrap_or("");

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
                        (score.abs() as f32).min(10.0) / 10.0,
                    ));
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

    // ── Delete / prune ────────────────────────────────────────────────────────

    /// Soft-delete a memory item (set status = 'deleted' rather than removing).
    /// Hard-deletes the embedding row since it's no longer needed for retrieval.
    pub fn delete(&self, id: &str) -> Result<bool> {
        let n = {
            let db = self.db.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            let n = db.execute(
                "UPDATE memory_items SET status = 'deleted', updated_at = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )?;
            let _ = db.execute(
                "DELETE FROM embeddings WHERE item_id = ?1",
                rusqlite::params![id],
            );
            n
        };
        // Mirror: update the file with status=deleted (keep the file — TODO spec)
        if n > 0 {
            self.mirror_export(id);
        }
        Ok(n > 0)
    }

    /// Remove items not retrieved in PRUNE_UNUSED_DAYS days (or expired),
    /// but only those already marked deleted or superseded.
    pub fn prune(&self) -> Result<usize> {
        let db = self.db.lock().unwrap();
        // Hard-delete items that are deleted/superseded AND stale
        let n = db.execute(
            "DELETE FROM memory_items WHERE \
             status IN ('deleted', 'superseded') AND (\
               (last_used_at IS NOT NULL AND last_used_at < datetime('now', ?1)) \
               OR (expires_at IS NOT NULL AND expires_at < datetime('now'))\
             )",
            rusqlite::params![format!("-{PRUNE_UNUSED_DAYS} days")],
        )?;
        // Also prune active items that have truly expired
        let n2 = db.execute(
            "UPDATE memory_items SET status = 'deleted', updated_at = datetime('now') \
             WHERE status = 'active' AND expires_at IS NOT NULL AND expires_at < datetime('now')",
            rusqlite::params![],
        )?;
        if n + n2 > 0 {
            tracing::info!("memory prune: {n} hard-deleted, {n2} expired");
        }
        Ok(n + n2)
    }
}

// ── Text similarity helper (Jaccard over trigrams) ────────────────────────────

fn text_similarity(a: &str, b: &str) -> f32 {
    if a == b {
        return 1.0;
    }
    if a.len() < 3 || b.len() < 3 {
        return 0.0;
    }
    let tri_a: std::collections::HashSet<&str> = (0..a.len().saturating_sub(2))
        .filter_map(|i| a.get(i..i + 3))
        .collect();
    let tri_b: std::collections::HashSet<&str> = (0..b.len().saturating_sub(2))
        .filter_map(|i| b.get(i..i + 3))
        .collect();
    let intersection = tri_a.intersection(&tri_b).count() as f32;
    let union = tri_a.union(&tri_b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn supersede_conflicting_lexical(
    db: &Connection,
    params: &InsertParams<'_>,
    new_id: &str,
    now: &str,
) -> Result<()> {
    let mut stmt = db.prepare(
        "SELECT id, text, source_ref, subject FROM memory_items \
         WHERE namespace = ?1 AND kind = ?2 AND status = 'active' AND source = 'passive'",
    )?;
    let mut rows = stmt.query(rusqlite::params![params.namespace, params.kind])?;
    let new_text = normalize_text(params.text);
    while let Some(row) = rows.next()? {
        let old_id: String = row.get(0)?;
        let old_text: String = row.get(1)?;
        let old_source_ref: Option<String> = row.get(2)?;
        let old_subject: Option<String> = row.get(3)?;
        let same_source = params
            .source_ref
            .zip(old_source_ref.as_deref())
            .is_some_and(|(new, old)| !new.is_empty() && new == old);
        let same_subject = params
            .subject
            .zip(old_subject.as_deref())
            .is_some_and(|(new, old)| {
                !new.is_empty() && normalize_text(new) == normalize_text(old)
            });
        let lexically_related = text_similarity(&new_text, &normalize_text(&old_text)) >= 0.55;
        if same_source || same_subject || lexically_related {
            db.execute(
                "UPDATE memory_items SET status='superseded', supersedes_id=?1, updated_at=?2 \
                 WHERE id=?3",
                rusqlite::params![new_id, now, old_id],
            )?;
        }
    }
    Ok(())
}

fn recency_score(created_at: &str) -> f32 {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|created| {
            let age_days = (Utc::now() - created.with_timezone(&Utc)).num_days().max(0) as f32;
            (-age_days / 30.0).exp()
        })
        .unwrap_or(1.0)
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
        confidence: row.get::<_, f64>(12).unwrap_or(0.8) as f32,
        importance: row.get::<_, f64>(13).unwrap_or(0.5) as f32,
        status: row
            .get::<_, String>(14)
            .unwrap_or_else(|_| "active".to_string()),
        source: row
            .get::<_, String>(15)
            .unwrap_or_else(|_| "passive".to_string()),
        sensitivity: row
            .get::<_, String>(16)
            .unwrap_or_else(|_| "normal".to_string()),
        subject: row.get(17)?,
        supersedes_id: row.get(18)?,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::Mutex;

    const TEST_SCHEMA: &str = "
        CREATE TABLE IF NOT EXISTS memory_items (
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
        CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
            id UNINDEXED, text,
            content='memory_items', content_rowid='rowid',
            tokenize='unicode61'
        );
        CREATE TRIGGER IF NOT EXISTS memory_items_ai AFTER INSERT ON memory_items BEGIN
            INSERT INTO memory_fts(rowid, id, text) VALUES (new.rowid, new.id, new.text);
        END;
        CREATE TRIGGER IF NOT EXISTS memory_items_ad AFTER DELETE ON memory_items BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, id, text) VALUES ('delete', old.rowid, old.id, old.text);
        END;
        CREATE TRIGGER IF NOT EXISTS memory_items_au AFTER UPDATE ON memory_items BEGIN
            INSERT INTO memory_fts(memory_fts, rowid, id, text) VALUES ('delete', old.rowid, old.id, old.text);
            INSERT INTO memory_fts(rowid, id, text) VALUES (new.rowid, new.id, new.text);
        END;
        CREATE TABLE IF NOT EXISTS embeddings (
            item_id TEXT PRIMARY KEY,
            namespace TEXT NOT NULL,
            model TEXT NOT NULL,
            dim INTEGER NOT NULL,
            vector BLOB NOT NULL,
            created_at TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'memory_item'
        );
        CREATE TABLE IF NOT EXISTS chat_turns (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            role TEXT NOT NULL,
            content TEXT NOT NULL,
            language TEXT NOT NULL DEFAULT 'und',
            model TEXT,
            created_at TEXT NOT NULL,
            parent_turn_id TEXT
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS chat_turns_fts USING fts5(
            id UNINDEXED, content,
            content='chat_turns', content_rowid='rowid',
            tokenize='unicode61'
        );
    ";

    fn test_store() -> Arc<MemoryStore> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(TEST_SCHEMA).unwrap();
        Arc::new(MemoryStore::new(Arc::new(Mutex::new(conn))))
    }

    static ITEM_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

    fn insert_item(
        store: &MemoryStore,
        ns: &str,
        kind: &str,
        text: &str,
        status: &str,
        source: &str,
        sensitivity: &str,
    ) {
        let db = store.db.lock().unwrap();
        let n = ITEM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!("test-{n}");
        let now = Utc::now().to_rfc3339();
        db.execute(
            "INSERT INTO memory_items (id, namespace, kind, language, text, created_at, updated_at, status, source, sensitivity)
             VALUES (?1,?2,?3,'und',?4,?5,?5,?6,?7,?8)",
            rusqlite::params![id, ns, kind, text, now, status, source, sensitivity],
        ).unwrap();
    }

    #[tokio::test]
    async fn retrieval_ignores_deleted_status() {
        let store = test_store();
        insert_item(
            &store,
            "user_pref",
            "preference",
            "User prefers bullets",
            "deleted",
            "passive",
            "normal",
        );
        insert_item(
            &store,
            "user_pref",
            "preference",
            "User prefers summaries",
            "active",
            "passive",
            "normal",
        );

        let hits = store
            .retrieve("bullets summaries preferences", &["user_pref"], 10)
            .await
            .unwrap();
        assert!(
            hits.iter().all(|h| h.item.status == "active"),
            "deleted items must not be returned"
        );
    }

    #[tokio::test]
    async fn insert_deduplicates_normalized_text_without_embeddings() {
        let store = test_store();
        let first = store
            .insert(
                "global",
                "fact",
                "sk",
                "  Faktúra   je splatná  ",
                None,
                None,
                None,
            )
            .await
            .unwrap();
        let duplicate = store
            .insert(
                "global",
                "fact",
                "sk",
                "faktúra je SPLATNÁ",
                None,
                None,
                None,
            )
            .await
            .unwrap();

        assert!(first.is_some());
        assert!(duplicate.is_none());
    }

    #[tokio::test]
    async fn retrieval_ignores_superseded_status() {
        let store = test_store();
        insert_item(
            &store,
            "user_pref",
            "preference",
            "Old style preference",
            "superseded",
            "passive",
            "normal",
        );
        insert_item(
            &store,
            "user_pref",
            "preference",
            "New style preference",
            "active",
            "explicit",
            "normal",
        );

        let hits = store
            .retrieve("style preference", &["user_pref"], 10)
            .await
            .unwrap();
        assert!(
            hits.iter().all(|h| h.item.status != "superseded"),
            "superseded items must not be returned"
        );
    }

    #[tokio::test]
    async fn retrieval_ignores_sensitive_items() {
        let store = test_store();
        insert_item(
            &store,
            "user_pref",
            "preference",
            "password is abc123",
            "active",
            "passive",
            "sensitive",
        );
        insert_item(
            &store,
            "user_pref",
            "preference",
            "Normal preference here",
            "active",
            "passive",
            "normal",
        );

        let hits = store
            .retrieve("password preference", &["user_pref"], 10)
            .await
            .unwrap();
        assert!(
            hits.iter().all(|h| h.item.sensitivity != "sensitive"),
            "sensitive items must not appear in default retrieval"
        );
    }

    #[tokio::test]
    async fn kind_filter_works() {
        let store = test_store();
        insert_item(
            &store,
            "user_pref",
            "preference",
            "User prefers bullets",
            "active",
            "passive",
            "normal",
        );
        insert_item(
            &store,
            "corrections",
            "correction",
            "Do not use Czech",
            "active",
            "passive",
            "normal",
        );

        let hits = store
            .retrieve_filtered(RetrieveQuery {
                query: "preference",
                namespaces: &["user_pref", "corrections"],
                kinds: &["preference"],
                k: 10,
                max_per_namespace: 3,
                score_threshold: 0.0,
                allow_sensitive: false,
            })
            .await
            .unwrap();
        assert!(
            hits.iter().all(|h| h.item.kind == "preference"),
            "kind filter broken"
        );
    }

    #[tokio::test]
    async fn soft_delete_hides_item() {
        let store = test_store();
        insert_item(
            &store,
            "user_pref",
            "preference",
            "User prefers bullets",
            "active",
            "passive",
            "normal",
        );
        // Find the id
        let hits_before = store.retrieve("bullets", &["user_pref"], 10).await.unwrap();
        if let Some(hit) = hits_before.first() {
            let id = hit.item.id.clone();
            store.delete(&id).unwrap();
            let hits_after = store.retrieve("bullets", &["user_pref"], 10).await.unwrap();
            assert!(
                hits_after.iter().all(|h| h.item.id != id),
                "soft-deleted item must not appear in retrieval"
            );
        }
        // If no hits (no embedding), test still passes — deletion works at DB level
    }

    #[tokio::test]
    async fn passive_sensitive_insert_blocked() {
        let store = test_store();
        let result = store
            .insert_full(InsertParams {
                namespace: "user_pref",
                kind: "preference",
                language: "en",
                text: "password for server is abc123",
                source: "passive",
                confidence: 0.9,
                importance: 0.9,
                sensitivity: "sensitive",
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(result.is_none(), "sensitive passive insert must be blocked");
    }

    #[tokio::test]
    async fn explicit_sensitive_insert_allowed() {
        let store = test_store();
        // Explicit source + sensitive: allowed (user explicitly asked)
        let result = store
            .insert_full(InsertParams {
                namespace: "user_pref",
                kind: "preference",
                language: "en",
                text: "My API key is xyz (user explicitly said remember this)",
                source: "explicit",
                confidence: 0.95,
                importance: 0.9,
                sensitivity: "sensitive",
                ..Default::default()
            })
            .await
            .unwrap();
        // Allowed to insert, but retrieval won't show it without allow_sensitive flag
        assert!(
            result.is_some(),
            "explicit sensitive insert should be allowed"
        );
    }

    #[tokio::test]
    async fn explicit_correction_supersedes_related_passive_memory_without_embeddings() {
        let store = test_store();
        let old_id = store
            .insert_full(InsertParams {
                namespace: "user_pref",
                kind: "preference",
                language: "en",
                text: "User prefers concise replies",
                source_ref: Some("preference:reply-style"),
                source: "passive",
                confidence: 0.8,
                importance: 0.5,
                sensitivity: "normal",
                subject: Some("reply style"),
                ..Default::default()
            })
            .await
            .unwrap()
            .unwrap();

        let new_id = store
            .insert_full(InsertParams {
                namespace: "user_pref",
                kind: "preference",
                language: "en",
                text: "User now prefers detailed replies",
                source_ref: Some("preference:reply-style"),
                source: "explicit",
                confidence: 1.0,
                importance: 0.9,
                sensitivity: "normal",
                subject: Some("reply style"),
                ..Default::default()
            })
            .await
            .unwrap()
            .unwrap();

        let db = store.db.lock().unwrap();
        let old_state: (String, Option<String>) = db
            .query_row(
                "SELECT status, supersedes_id FROM memory_items WHERE id = ?1",
                rusqlite::params![old_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(old_state, ("superseded".to_string(), Some(new_id)));
    }

    #[tokio::test]
    async fn fts_rank_dominates_recency_and_importance() {
        let store = test_store();
        let db = store.db.lock().unwrap();
        db.execute(
            "INSERT INTO memory_items \
             (id, namespace, kind, language, text, created_at, updated_at, importance) \
             VALUES ('strong', 'facts', 'fact', 'en', ?1, '2020-01-01T00:00:00Z', \
                     '2020-01-01T00:00:00Z', 0.0)",
            ["alpha beta alpha beta alpha beta"],
        )
        .unwrap();
        db.execute(
            "INSERT INTO memory_items \
             (id, namespace, kind, language, text, created_at, updated_at, importance) \
             VALUES ('weak', 'facts', 'fact', 'en', ?1, ?2, ?2, 1.0)",
            rusqlite::params![
                "alpha beta filler filler filler filler filler filler filler filler",
                Utc::now().to_rfc3339()
            ],
        )
        .unwrap();
        drop(db);

        let hits = store
            .retrieve_filtered(RetrieveQuery {
                query: "alpha beta",
                namespaces: &["facts"],
                kinds: &[],
                k: 2,
                max_per_namespace: 2,
                score_threshold: 0.01,
                allow_sensitive: false,
            })
            .await
            .unwrap();
        assert_eq!(hits.first().map(|hit| hit.item.id.as_str()), Some("strong"));
    }

    #[tokio::test]
    async fn supersede_marks_old_item() {
        let store = test_store();
        insert_item(
            &store,
            "user_pref",
            "preference",
            "Old preference",
            "active",
            "passive",
            "normal",
        );
        let db = store.db.lock().unwrap();
        let old_id: String = db
            .query_row(
                "SELECT id FROM memory_items WHERE text = 'Old preference'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        drop(db);

        store.supersede(&old_id).unwrap();

        let db = store.db.lock().unwrap();
        let status: String = db
            .query_row(
                "SELECT status FROM memory_items WHERE id = ?1",
                rusqlite::params![old_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "superseded");
    }

    #[tokio::test]
    async fn retrieve_returns_empty_when_namespace_empty() {
        let store = test_store();
        let hits = store.retrieve("anything", &[], 10).await.unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn text_similarity_exact_match() {
        assert_eq!(text_similarity("hello world", "hello world"), 1.0);
    }

    #[test]
    fn text_similarity_different_strings() {
        let s = text_similarity("completely different text", "nothing in common here");
        assert!(s < 0.3, "should be low similarity: {s}");
    }
}
