use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::MemoryItem;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryFrontmatter {
    pub id: String,
    pub namespace: String,
    pub kind: String,
    pub language: String,
    pub source: String,
    pub confidence: f32,
    pub importance: f32,
    pub status: String,
    pub sensitivity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

/// Export one memory item to `{memories_dir}/{namespace}/{id}.md`.
/// Sensitive items are silently skipped.
/// I/O errors are logged and swallowed — export is best-effort.
pub fn export_item(memories_dir: &Path, item: &MemoryItem) {
    if item.sensitivity == "sensitive" {
        return;
    }
    if let Err(e) = try_export(memories_dir, item) {
        tracing::warn!("memory mirror: export failed for {}: {e}", item.id);
    }
}

fn try_export(memories_dir: &Path, item: &MemoryItem) -> Result<()> {
    let ns_dir = memories_dir.join(&item.namespace);
    std::fs::create_dir_all(&ns_dir)?;
    let path = ns_dir.join(format!("{}.md", item.id));

    let fm = MemoryFrontmatter {
        id: item.id.clone(),
        namespace: item.namespace.clone(),
        kind: item.kind.clone(),
        language: item.language.clone(),
        source: item.source.clone(),
        confidence: item.confidence,
        importance: item.importance,
        status: item.status.clone(),
        sensitivity: item.sensitivity.clone(),
        subject: item.subject.clone(),
        created_at: item.created_at.clone(),
        updated_at: item.updated_at.clone(),
        expires_at: item.expires_at.clone(),
        source_ref: item.source_ref.clone(),
    };

    let yaml = serde_yaml::to_string(&fm)?;
    let content = format!("---\n{}---\n\n{}\n", yaml, item.text);
    std::fs::write(path, content)?;
    Ok(())
}

/// Scan `memories_dir` recursively for `.md` files whose frontmatter `updated_at`
/// is newer than the corresponding DB row (keyed by `id`).
/// Returns `(frontmatter, text)` pairs that should be upserted.
/// Never panics — individual file errors are logged and skipped.
pub fn scan_changed(
    memories_dir: &Path,
    db: &rusqlite::Connection,
) -> Vec<(MemoryFrontmatter, String)> {
    let mut out = Vec::new();

    let top = match std::fs::read_dir(memories_dir) {
        Ok(e) => e,
        Err(_) => return out,
    };

    for ns_entry in top.flatten() {
        let ns_path = ns_entry.path();
        if !ns_path.is_dir() {
            continue;
        }
        let files = match std::fs::read_dir(&ns_path) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for file_entry in files.flatten() {
            let fp = file_entry.path();
            if fp.extension().map_or(true, |e| e != "md") {
                continue;
            }
            match parse_memory_file(&fp) {
                Ok((fm, text)) => {
                    if fm.sensitivity == "sensitive" {
                        continue;
                    }
                    let db_updated: Option<String> = db
                        .query_row(
                            "SELECT updated_at FROM memory_items WHERE id = ?1",
                            rusqlite::params![fm.id],
                            |r| r.get(0),
                        )
                        .ok();

                    let import = match db_updated {
                        None => true,
                        Some(db_ts) => is_file_newer(&fm.updated_at, &db_ts),
                    };

                    if import {
                        out.push((fm, text));
                    }
                }
                Err(e) => {
                    tracing::warn!("memory mirror: skipping {}: {e}", fp.display());
                }
            }
        }
    }

    out
}

/// Parse a mirror `.md` file into `(frontmatter, body_text)`.
pub fn parse_memory_file(path: &Path) -> Result<(MemoryFrontmatter, String)> {
    let raw = std::fs::read_to_string(path)?;
    let content = raw.trim_start();
    let after_fence = content
        .strip_prefix("---")
        .ok_or_else(|| anyhow::anyhow!("missing opening ---"))?
        .trim_start_matches('\n');
    let close = after_fence
        .find("\n---")
        .ok_or_else(|| anyhow::anyhow!("missing closing ---"))?;
    let yaml_str = &after_fence[..close];
    let body = after_fence[close + 4..]
        .trim_start_matches('\n')
        .trim_end()
        .to_string();
    let fm: MemoryFrontmatter = serde_yaml::from_str(yaml_str)
        .map_err(|e| anyhow::anyhow!("invalid frontmatter YAML: {e}"))?;
    Ok((fm, body))
}

/// True when `file_ts` is more than 1 second newer than `db_ts`.
fn is_file_newer(file_ts: &str, db_ts: &str) -> bool {
    use chrono::DateTime;
    let f = DateTime::parse_from_rfc3339(file_ts);
    let d = DateTime::parse_from_rfc3339(db_ts);
    match (f, d) {
        (Ok(ft), Ok(dt)) => ft > dt + chrono::Duration::seconds(1),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"---
id: test-uuid-1234
namespace: user_pref
kind: preference
language: en
source: explicit
confidence: 0.9
importance: 0.8
status: active
sensitivity: normal
created_at: '2026-01-01T00:00:00+00:00'
updated_at: '2026-01-02T00:00:00+00:00'
---

User prefers bullet points over prose.
"#;

    #[test]
    fn round_trip_parse() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.md");
        std::fs::write(&path, SAMPLE).unwrap();
        let (fm, text) = parse_memory_file(&path).unwrap();
        assert_eq!(fm.id, "test-uuid-1234");
        assert_eq!(fm.namespace, "user_pref");
        assert_eq!(fm.kind, "preference");
        assert_eq!(fm.status, "active");
        assert!(text.contains("bullet points"));
    }

    #[test]
    fn is_file_newer_detects_newer() {
        assert!(is_file_newer(
            "2026-01-03T00:00:00+00:00",
            "2026-01-01T00:00:00+00:00"
        ));
        assert!(!is_file_newer(
            "2026-01-01T00:00:00+00:00",
            "2026-01-03T00:00:00+00:00"
        ));
        // within 1s epsilon: not newer
        assert!(!is_file_newer(
            "2026-01-01T00:00:00+00:00",
            "2026-01-01T00:00:00+00:00"
        ));
    }

    #[test]
    fn export_then_scan_round_trips_and_does_not_loop() {
        use crate::MemoryItem;
        use rusqlite::Connection;

        let tmp = tempfile::tempdir().unwrap();
        let item = MemoryItem {
            id: "round-trip-uuid".to_string(),
            namespace: "user_pref".to_string(),
            kind: "preference".to_string(),
            language: "en".to_string(),
            text: "User prefers concise bullet points.".to_string(),
            source_ref: None,
            metadata_json: None,
            last_used_at: None,
            use_count: 0,
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-02T00:00:00+00:00".to_string(),
            expires_at: None,
            confidence: 0.9,
            importance: 0.8,
            status: "active".to_string(),
            source: "explicit".to_string(),
            sensitivity: "normal".to_string(),
            subject: None,
            supersedes_id: None,
        };

        export_item(tmp.path(), &item);

        // Anti-loop: DB row at same updated_at → scan returns nothing
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE memory_items (id TEXT PRIMARY KEY, updated_at TEXT)")
            .unwrap();
        conn.execute(
            "INSERT INTO memory_items (id, updated_at) VALUES (?1, ?2)",
            rusqlite::params![item.id, item.updated_at],
        )
        .unwrap();
        assert!(
            scan_changed(tmp.path(), &conn).is_empty(),
            "export must not cause re-import of itself (anti-loop)"
        );

        // Fresh DB: scan imports it with fields intact
        let conn2 = Connection::open_in_memory().unwrap();
        conn2
            .execute_batch("CREATE TABLE memory_items (id TEXT PRIMARY KEY, updated_at TEXT)")
            .unwrap();
        let changed = scan_changed(tmp.path(), &conn2);
        assert_eq!(changed.len(), 1, "new item must be imported");
        assert_eq!(changed[0].0.id, item.id);
        assert_eq!(changed[0].0.namespace, "user_pref");
        assert_eq!(changed[0].0.status, "active");
        assert_eq!(changed[0].1, item.text);
    }

    #[test]
    fn sensitive_skipped_in_scan() {
        let sensitive = r#"---
id: s-1
namespace: secrets
kind: password
language: en
source: explicit
confidence: 0.9
importance: 0.9
status: active
sensitivity: sensitive
created_at: '2020-01-01T00:00:00+00:00'
updated_at: '2020-01-01T00:00:00+00:00'
---

my super secret password
"#;
        let tmp = tempfile::tempdir().unwrap();
        let ns_dir = tmp.path().join("secrets");
        std::fs::create_dir_all(&ns_dir).unwrap();
        std::fs::write(ns_dir.join("s-1.md"), sensitive).unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE memory_items (id TEXT PRIMARY KEY, updated_at TEXT)")
            .unwrap();

        let changed = scan_changed(tmp.path(), &conn);
        assert!(changed.is_empty(), "sensitive items must be skipped");
    }
}
