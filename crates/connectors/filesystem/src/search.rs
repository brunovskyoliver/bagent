use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::path::Path;
use std::time::SystemTime;
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;

use crate::path_policy::PathPolicy;
use crate::types::{
    FileKind, FileMetadataResponse, FileSearchRequest, FileSearchResponse, FileSearchResult,
    MatchType, ReadTextRequest, ReadTextResponse,
};

/// Max file size considered for content search (10 MB).
const MAX_CONTENT_SEARCH_BYTES: u64 = 10 * 1024 * 1024;
/// Max bytes to scan for binary detection.
const BINARY_PROBE_BYTES: usize = 8 * 1024;
/// Max matched-line length returned to caller.
const MAX_LINE_CHARS: usize = 500;
/// Max matches returned per file during content search.
const MAX_MATCHES_PER_FILE: usize = 2;

// ── Scoring ───────────────────────────────────────────────────────────────────

fn score_match(
    basename: &str,
    path_str: &str,
    query_lower: &str,
    mtime: Option<SystemTime>,
    depth: usize,
    match_type: &MatchType,
) -> f32 {
    let basename_lower = basename.to_lowercase();

    let mut score: f32 = match match_type {
        MatchType::Content => {
            // Content match base score; filename relevance adds a bonus
            if basename_lower == query_lower {
                0.75
            } else if basename_lower.contains(query_lower) {
                0.60
            } else {
                0.45
            }
        }
        MatchType::FileName => {
            if basename_lower == query_lower {
                1.0
            } else if basename_lower.starts_with(query_lower) {
                0.9
            } else {
                0.7
            }
        }
        MatchType::Path => 0.4,
    };

    // Recency bonus (up to +0.10): modified within the last 30 days
    if let Some(mt) = mtime {
        if let Ok(age) = mt.elapsed() {
            let days = age.as_secs() / 86400;
            if days < 30 {
                score += 0.10 * (1.0 - days as f32 / 30.0);
            }
        }
    }

    // Depth bonus (up to +0.05): shallower = better
    score += 0.05 / (1.0 + depth as f32);

    score.clamp(0.0, 1.0)
}

// ── Binary detection ──────────────────────────────────────────────────────────

fn looks_binary(path: &Path) -> bool {
    match std::fs::File::open(path) {
        Ok(mut f) => {
            use std::io::Read;
            let mut buf = vec![0u8; BINARY_PROBE_BYTES];
            let n = f.read(&mut buf).unwrap_or(0);
            buf[..n].contains(&0u8)
        }
        Err(_) => true, // can't open → treat as binary
    }
}

// ── Diacritic folding ─────────────────────────────────────────────────────────

/// NFD-decompose and strip combining diacritical marks, then lowercase.
/// "zákazník" → "zakaznik", allowing Slovak → ASCII-folded filename matching
/// on the walkdir fallback path (Spotlight folds automatically).
pub fn fold_diacritics(s: &str) -> String {
    s.nfd()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

// ── Spotlight (mdfind) backend ────────────────────────────────────────────────

/// Run `mdfind -onlyin <root> <term>` and collect paths.
/// Returns raw absolute path strings; caller must policy-check them.
fn mdfind_raw(root: &Path, term: &str) -> Vec<String> {
    use std::process::Command;
    match Command::new("mdfind")
        .arg("-onlyin")
        .arg(root)
        .arg(term)
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        _ => vec![],
    }
}

/// Search files using macOS Spotlight (`mdfind`) across multiple roots and terms.
///
/// - Runs each (root × term) pair as a separate `mdfind` call.
/// - Deduplicates results across terms.
/// - Filters every hit through the `PathPolicy` (denied roots, allowed roots).
/// - Enriches results with kind / mtime / mime.
/// - Content snippets are **not** extracted here to keep latency low; callers
///   that need content should call `read_text_sync` on interesting results.
pub fn search_files_spotlight(
    policy: &PathPolicy,
    roots: &[std::path::PathBuf],
    terms: &[String],
    extensions: Option<&[String]>,
    max_results: usize,
) -> Vec<FileSearchResult> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut results: Vec<FileSearchResult> = Vec::new();

    'outer: for term in terms {
        for root in roots {
            for raw_path in mdfind_raw(root, term) {
                if results.len() >= max_results {
                    break 'outer;
                }
                if seen.contains(&raw_path) {
                    continue;
                }

                let p = std::path::Path::new(&raw_path);

                // Policy checks
                let canonical = match std::fs::canonicalize(p) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if policy.is_denied(&canonical) {
                    continue;
                }
                if !policy.is_under_allowed_root(&canonical) {
                    continue;
                }
                if policy.contains_hidden_component(&canonical) {
                    continue;
                }

                // Extension filter
                if let Some(exts) = extensions {
                    let ext = canonical
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if !canonical.is_dir() && !exts.iter().any(|e| e.to_lowercase() == ext) {
                        continue;
                    }
                }

                let file_name = canonical
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                let mime = if canonical.is_file() {
                    Some(
                        mime_guess::from_path(&canonical)
                            .first_or_octet_stream()
                            .to_string(),
                    )
                } else {
                    None
                };
                let mtime = mtime_sys(&canonical);
                let depth = canonical
                    .strip_prefix(root)
                    .map(|r| r.components().count())
                    .unwrap_or(0);
                let score = score_match(
                    &file_name,
                    &raw_path,
                    &term.to_lowercase(),
                    mtime,
                    depth,
                    &MatchType::FileName,
                );

                seen.insert(raw_path.clone());
                results.push(FileSearchResult {
                    path: canonical.to_string_lossy().into_owned(),
                    display_name: file_name,
                    parent: canonical.parent().map(|p| p.to_string_lossy().into_owned()),
                    kind: file_kind(&canonical),
                    mime,
                    size_bytes: std::fs::metadata(&canonical).ok().map(|m| m.len()),
                    modified_at: mtime_str(&canonical),
                    match_type: MatchType::FileName,
                    matched_line: None,
                    line_number: None,
                    score,
                });
            }
        }
    }

    // Sort: score desc, then mtime desc
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.modified_at.cmp(&a.modified_at))
    });
    results.truncate(max_results);
    results
}

// ── mtime helper ─────────────────────────────────────────────────────────────

fn mtime_str(path: &Path) -> Option<String> {
    let mtime = std::fs::metadata(path).ok()?.modified().ok()?;
    let dt: DateTime<Utc> = mtime.into();
    Some(dt.to_rfc3339())
}

fn mtime_sys(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

// ── FileKind helper ───────────────────────────────────────────────────────────

fn file_kind(path: &Path) -> FileKind {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return FileKind::Other,
    };
    if meta.file_type().is_symlink() {
        return FileKind::Symlink;
    }
    if meta.is_dir() {
        // .app bundles are "packages"
        if path.extension().and_then(|e| e.to_str()) == Some("app") {
            return FileKind::Package;
        }
        return FileKind::Directory;
    }
    FileKind::File
}

// ── Depth helper ─────────────────────────────────────────────────────────────

fn path_depth(base: &Path, path: &Path) -> usize {
    path.strip_prefix(base)
        .map(|rel| rel.components().count())
        .unwrap_or(0)
}

// ── search_files ─────────────────────────────────────────────────────────────

/// Search local files by name and/or content. Synchronous — caller must `spawn_blocking`.
///
/// Strategy:
/// 1. If `req.terms` is non-empty, try Spotlight (`mdfind`) first — it indexes PDF/docx/xlsx
///    content and folds diacritics automatically. Fast and accurate.
/// 2. Fall back to walkdir with `fold_diacritics` matching when Spotlight returns nothing
///    or when `req.terms` is empty (single-query / backwards-compat path).
pub fn search_files_sync(
    policy: &PathPolicy,
    req: FileSearchRequest,
) -> Result<FileSearchResponse> {
    // Resolve effective terms: prefer explicit `terms`, fall back to `query`.
    let effective_terms: Vec<String> = if !req.terms.is_empty() {
        req.terms.clone()
    } else if !req.query.is_empty() {
        vec![req.query.clone()]
    } else {
        vec![]
    };

    let query_lower = req.query.to_lowercase();
    let max_results = req.max_results.min(policy.max_search_results);
    let max_depth = req.max_depth.unwrap_or(10);

    // Resolve search roots
    let roots: Vec<std::path::PathBuf> = if let Some(ref raw_roots) = req.roots {
        raw_roots
            .iter()
            .filter_map(|r| {
                policy
                    .validate_search_root(r, req.include_hidden)
                    .map_err(|e| tracing::warn!("invalid search root '{}': {}", r, e))
                    .ok()
            })
            .collect()
    } else {
        policy.allowed_roots.clone()
    };

    if roots.is_empty() {
        return Ok(FileSearchResponse {
            query: req.query.clone(),
            results: vec![],
            truncated: false,
        });
    }

    // ── Spotlight path (preferred when multi-term or whenever mdfind is useful) ──
    // Try Spotlight first; if it returns results, use them and skip walkdir.
    if !effective_terms.is_empty() {
        let ext_slice: Option<&[String]> = req.extensions.as_deref();
        let spotlight_results =
            search_files_spotlight(policy, &roots, &effective_terms, ext_slice, max_results);
        if !spotlight_results.is_empty() {
            let truncated = spotlight_results.len() >= max_results;
            return Ok(FileSearchResponse {
                query: req.query.clone(),
                results: spotlight_results,
                truncated,
            });
        }
    }

    // ── walkdir fallback ─────────────────────────────────────────────────────
    // Matches by folded-diacritic lowercase so Slovak queries find ASCII-folded filenames.
    let query_folded = fold_diacritics(&query_lower);
    // Also collect folded versions of each term for multi-term walkdir matching.
    let terms_folded: Vec<String> = effective_terms.iter().map(|t| fold_diacritics(t)).collect();

    let mut results: Vec<FileSearchResult> = Vec::new();

    for root in &roots {
        if results.len() >= max_results {
            break;
        }

        let include_hidden_local = req.include_hidden;
        let policy_clone = policy.denied_roots.clone();
        let walk = WalkDir::new(root)
            .max_depth(max_depth)
            .follow_links(false)
            .into_iter()
            .filter_entry(move |e| {
                let p = e.path();
                let depth = e.depth();
                // Skip hidden entries when not included (but never filter the root itself at depth 0)
                if depth > 0 && !include_hidden_local {
                    if e.file_name()
                        .to_str()
                        .map(|s| s.starts_with('.'))
                        .unwrap_or(false)
                    {
                        return false;
                    }
                }
                // Skip denied paths and their children
                !policy_clone.iter().any(|d| p.starts_with(d))
            });

        'entries: for entry in walk.flatten() {
            if results.len() >= max_results {
                break;
            }

            let path = entry.path();

            // Skip denied paths
            if policy.is_denied(path) {
                continue;
            }

            // Skip hidden when not included — but only check components below the walk root
            if !req.include_hidden {
                let rel = path.strip_prefix(root).unwrap_or(path);
                if rel.components().any(|c| {
                    use std::path::Component;
                    if let Component::Normal(s) = c {
                        s.to_str().map(|s| s.starts_with('.')).unwrap_or(false)
                    } else {
                        false
                    }
                }) {
                    continue;
                }
            }

            let file_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            let path_str = path.to_string_lossy();

            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            // Extension filter
            if let Some(ref exts) = req.extensions {
                if !path.is_dir() && !exts.iter().any(|e| e.to_lowercase() == ext) {
                    continue;
                }
            }

            let mtime = mtime_sys(path);
            let depth = path_depth(root, path);

            // ── Filename / path match ─────────────────────────────────────────
            if req.search_names {
                let basename_lower = file_name.to_lowercase();
                let basename_folded = fold_diacritics(&basename_lower);
                let path_lower = path_str.to_lowercase();
                let path_folded = fold_diacritics(&path_lower);

                // Match against both the plain query and any multi-term alternative.
                // Uses folded strings so Slovak queries find ASCII-folded filenames.
                let name_hit = basename_folded == query_folded
                    || basename_folded.contains(&query_folded)
                    || terms_folded
                        .iter()
                        .any(|t| basename_folded.contains(t.as_str()));
                let path_hit = !name_hit
                    && (path_folded.contains(&query_folded)
                        || terms_folded
                            .iter()
                            .any(|t| path_folded.contains(t.as_str())));

                let (matched, match_type) = if name_hit {
                    (true, MatchType::FileName)
                } else if path_hit {
                    (true, MatchType::Path)
                } else {
                    (false, MatchType::FileName)
                };

                if matched {
                    let score = score_match(
                        file_name,
                        &path_str,
                        &query_lower,
                        mtime,
                        depth,
                        &match_type,
                    );
                    let mime = if path.is_file() {
                        Some(
                            mime_guess::from_path(path)
                                .first_or_octet_stream()
                                .to_string(),
                        )
                    } else {
                        None
                    };
                    results.push(FileSearchResult {
                        path: path.to_string_lossy().into_owned(),
                        display_name: file_name.to_string(),
                        parent: path.parent().map(|p| p.to_string_lossy().into_owned()),
                        kind: file_kind(path),
                        mime,
                        size_bytes: std::fs::metadata(path).ok().map(|m| m.len()),
                        modified_at: mtime_str(path),
                        match_type,
                        matched_line: None,
                        line_number: None,
                        score,
                    });
                    if results.len() >= max_results {
                        break 'entries;
                    }
                    continue; // already added via name match; don't also content-search
                }
            }

            // ── Content search ────────────────────────────────────────────────
            if req.search_contents && path.is_file() {
                // Skip directories, large files, and binaries
                let size = std::fs::metadata(path).ok().map(|m| m.len()).unwrap_or(0);
                if size > MAX_CONTENT_SEARCH_BYTES {
                    continue;
                }
                if looks_binary(path) {
                    continue;
                }

                let content = match std::fs::read(path) {
                    Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                    Err(_) => continue,
                };

                let mut file_match_count = 0;
                for (line_idx, line) in content.lines().enumerate() {
                    if file_match_count >= MAX_MATCHES_PER_FILE {
                        break;
                    }
                    if results.len() >= max_results {
                        break 'entries;
                    }
                    let line_lower = line.to_lowercase();
                    let line_folded = fold_diacritics(&line_lower);
                    let content_hit = line_lower.contains(&query_lower)
                        || line_folded.contains(&query_folded)
                        || terms_folded
                            .iter()
                            .any(|t| line_folded.contains(t.as_str()));
                    if content_hit {
                        let truncated_line: String = line.chars().take(MAX_LINE_CHARS).collect();
                        let mtime2 = mtime;
                        let score = score_match(
                            file_name,
                            &path_str,
                            &query_lower,
                            mtime2,
                            depth,
                            &MatchType::Content,
                        );
                        let mime = Some(
                            mime_guess::from_path(path)
                                .first_or_octet_stream()
                                .to_string(),
                        );
                        results.push(FileSearchResult {
                            path: path.to_string_lossy().into_owned(),
                            display_name: file_name.to_string(),
                            parent: path.parent().map(|p| p.to_string_lossy().into_owned()),
                            kind: file_kind(path),
                            mime,
                            size_bytes: Some(size),
                            modified_at: mtime_str(path),
                            match_type: MatchType::Content,
                            matched_line: Some(truncated_line),
                            line_number: Some((line_idx + 1) as u64),
                            score,
                        });
                        file_match_count += 1;
                    }
                }
            }
        }
    }

    // Sort: score desc, then mtime desc
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.modified_at.cmp(&a.modified_at))
    });

    let truncated = results.len() >= max_results;
    results.truncate(max_results);

    Ok(FileSearchResponse {
        query: req.query,
        results,
        truncated,
    })
}

/// Async wrapper (spawns blocking).
pub async fn search_files(
    policy: PathPolicy,
    req: FileSearchRequest,
) -> Result<FileSearchResponse> {
    tokio::task::spawn_blocking(move || search_files_sync(&policy, req))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking: {}", e))?
}

// ── read_text ─────────────────────────────────────────────────────────────────

/// Read a safe text snippet from a file. Synchronous — caller must `spawn_blocking`.
pub fn read_text_sync(policy: &PathPolicy, req: ReadTextRequest) -> Result<ReadTextResponse> {
    let canonical = policy.validate_read_path(&req.path)?;

    if canonical.is_dir() {
        return Err(anyhow::anyhow!("path is a directory"));
    }

    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let mime = Some(
        mime_guess::from_path(&canonical)
            .first_or_octet_stream()
            .to_string(),
    );

    // For PDF/docx/xlsx: delegate to the attachment extractor
    let is_document = matches!(ext.as_str(), "pdf" | "docx" | "xlsx" | "xls" | "doc");

    if is_document {
        let mime_str = mime.as_deref().unwrap_or("application/octet-stream");
        let result = bagent_attachments::extract(&canonical, mime_str)?;
        let content = result
            .extracted_text
            .unwrap_or_else(|| "[text extraction failed]".to_string());
        let max_chars = req
            .max_bytes
            .unwrap_or(policy.max_read_bytes)
            .min(policy.hard_max_read_bytes);
        let truncated = content.len() > max_chars;
        let content: String = content.chars().take(max_chars).collect();
        return Ok(ReadTextResponse {
            path: canonical.to_string_lossy().into_owned(),
            mime,
            truncated,
            content,
            pii: true,
        });
    }

    // Binary check
    if looks_binary(&canonical) {
        return Err(anyhow::anyhow!("binary file: {}", canonical.display()));
    }

    // Plain text read
    let max_chars = req
        .max_bytes
        .unwrap_or(policy.max_read_bytes)
        .min(policy.hard_max_read_bytes);

    let raw = std::fs::read(&canonical)?;
    let text = String::from_utf8_lossy(&raw).into_owned();

    let truncated = text.len() > max_chars;
    let content: String = text.chars().take(max_chars).collect();

    Ok(ReadTextResponse {
        path: canonical.to_string_lossy().into_owned(),
        mime,
        truncated,
        content,
        pii: true,
    })
}

/// Async wrapper.
pub async fn read_text(policy: PathPolicy, req: ReadTextRequest) -> Result<ReadTextResponse> {
    tokio::task::spawn_blocking(move || read_text_sync(&policy, req))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking: {}", e))?
}

// ── metadata ──────────────────────────────────────────────────────────────────

/// Return file/folder metadata. Synchronous.
pub fn metadata_sync(policy: &PathPolicy, raw_path: &str) -> Result<FileMetadataResponse> {
    let expanded = policy.expand_tilde(raw_path)?;
    let canonical = std::fs::canonicalize(&expanded)
        .map_err(|e| anyhow::anyhow!("cannot resolve '{}': {}", raw_path, e))?;

    if policy.is_denied(&canonical) {
        return Err(anyhow::anyhow!("access denied: {}", canonical.display()));
    }
    if !policy.is_under_allowed_root(&canonical) {
        return Err(anyhow::anyhow!("path outside allowed roots"));
    }

    let file_name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();

    let mime = if canonical.is_file() {
        Some(
            mime_guess::from_path(&canonical)
                .first_or_octet_stream()
                .to_string(),
        )
    } else {
        None
    };

    Ok(FileMetadataResponse {
        path: canonical.to_string_lossy().into_owned(),
        display_name: file_name,
        parent: canonical.parent().map(|p| p.to_string_lossy().into_owned()),
        kind: file_kind(&canonical),
        mime,
        size_bytes: std::fs::metadata(&canonical).ok().map(|m| m.len()),
        modified_at: mtime_str(&canonical),
    })
}

/// Async wrapper.
pub async fn metadata(policy: PathPolicy, raw_path: String) -> Result<FileMetadataResponse> {
    tokio::task::spawn_blocking(move || metadata_sync(&policy, &raw_path))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking: {}", e))?
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_policy::PathPolicy;
    use std::fs;
    use std::io::Write;

    fn test_policy(root: &std::path::Path) -> PathPolicy {
        let canonical_root = fs::canonicalize(root).unwrap_or(root.to_path_buf());
        let mut p = PathPolicy::default_for_user_home().unwrap();
        p.allowed_roots = vec![canonical_root];
        p
    }

    fn temp_dir_with_files() -> (tempfile::TempDir, PathPolicy) {
        let dir = tempfile::tempdir().unwrap();
        let policy = test_policy(dir.path());
        (dir, policy)
    }

    #[test]
    fn fold_diacritics_strips_accents() {
        // Slovak diacritics → ASCII equivalents
        assert_eq!(fold_diacritics("zákazník"), "zakaznik");
        assert_eq!(fold_diacritics("splatnosť"), "splatnost");
        assert_eq!(fold_diacritics("faktúra"), "faktura");
        assert_eq!(fold_diacritics("prehľad"), "prehlad");
        // Folded strings compare equal regardless of diacritic source
        assert_eq!(
            fold_diacritics("zákazník"),
            fold_diacritics("zakaznik"),
            "diacritic and ASCII-folded forms should match"
        );
    }

    #[test]
    fn multi_term_finds_file_matching_any_term() {
        let (dir, policy) = temp_dir_with_files();
        fs::write(dir.path().join("prehlad_zakaznikov.txt"), "obsah").unwrap();
        fs::write(dir.path().join("unrelated.txt"), "nothing").unwrap();

        // Terms include folded Slovak — walkdir fallback should match
        let req = FileSearchRequest {
            query: "prehlad".to_string(),
            terms: vec!["prehlad".to_string(), "zakaznik".to_string()],
            search_names: true,
            max_results: 10,
            ..Default::default()
        };
        let resp = search_files_sync(&policy, req).unwrap();
        assert!(
            resp.results
                .iter()
                .any(|r| r.display_name.contains("prehlad")),
            "should find file matching any of the terms"
        );
    }

    #[test]
    fn finds_filename_match() {
        let (dir, policy) = temp_dir_with_files();
        fs::write(dir.path().join("faktura_jun.txt"), "obsah").unwrap();
        fs::write(dir.path().join("other.txt"), "other").unwrap();

        let req = FileSearchRequest {
            query: "faktura".to_string(),
            max_results: 10,
            ..Default::default()
        };
        let resp = search_files_sync(&policy, req).unwrap();
        assert!(resp
            .results
            .iter()
            .any(|r| r.display_name.contains("faktura")));
    }

    #[test]
    fn finds_content_phrase_match() {
        let (dir, policy) = temp_dir_with_files();
        fs::write(dir.path().join("doc.txt"), "Cena bez DPH je 100 EUR").unwrap();
        fs::write(dir.path().join("other.txt"), "nothing here").unwrap();

        let req = FileSearchRequest {
            query: "DPH".to_string(),
            search_names: false,
            search_contents: true,
            max_results: 10,
            ..Default::default()
        };
        let resp = search_files_sync(&policy, req).unwrap();
        let hit = resp.results.iter().find(|r| r.display_name == "doc.txt");
        assert!(hit.is_some(), "should find doc.txt by content");
        assert!(hit
            .unwrap()
            .matched_line
            .as_deref()
            .unwrap_or("")
            .contains("DPH"));
    }

    #[test]
    fn finds_slovak_diacritics() {
        let (dir, policy) = temp_dir_with_files();
        fs::write(dir.path().join("zmluva.txt"), "Splatnosť faktúry je 30 dní").unwrap();

        let req = FileSearchRequest {
            query: "splatnosť".to_string(),
            search_names: false,
            search_contents: true,
            max_results: 10,
            ..Default::default()
        };
        let resp = search_files_sync(&policy, req).unwrap();
        assert!(
            !resp.results.is_empty(),
            "should find file with Slovak content"
        );
    }

    #[test]
    fn respects_extension_filter() {
        let (dir, policy) = temp_dir_with_files();
        fs::write(dir.path().join("contract.txt"), "zmluva").unwrap();
        fs::write(dir.path().join("contract.pdf"), "zmluva pdf").unwrap();

        let req = FileSearchRequest {
            query: "contract".to_string(),
            extensions: Some(vec!["txt".to_string()]),
            max_results: 10,
            ..Default::default()
        };
        let resp = search_files_sync(&policy, req).unwrap();
        assert!(resp.results.iter().all(|r| r.path.ends_with(".txt")));
    }

    #[test]
    fn respects_max_results() {
        let (dir, policy) = temp_dir_with_files();
        for i in 0..10 {
            fs::write(dir.path().join(format!("file_{i}.txt")), "content").unwrap();
        }
        let req = FileSearchRequest {
            query: "file".to_string(),
            max_results: 3,
            ..Default::default()
        };
        let resp = search_files_sync(&policy, req).unwrap();
        assert!(resp.results.len() <= 3);
    }

    #[test]
    fn skips_binary_files() {
        let (dir, policy) = temp_dir_with_files();
        // Write a file with NUL bytes = binary
        let mut f = fs::File::create(dir.path().join("binary.bin")).unwrap();
        f.write_all(b"hello\x00world").unwrap();

        let req = FileSearchRequest {
            query: "hello".to_string(),
            search_names: false,
            search_contents: true,
            max_results: 10,
            ..Default::default()
        };
        let resp = search_files_sync(&policy, req).unwrap();
        assert!(
            resp.results.iter().all(|r| r.display_name != "binary.bin"),
            "binary file should not appear in content search"
        );
    }

    #[test]
    fn truncates_long_matched_line() {
        let (dir, policy) = temp_dir_with_files();
        let long_line = format!("{}DPH{}", "x".repeat(300), "y".repeat(300));
        fs::write(dir.path().join("long.txt"), &long_line).unwrap();

        let req = FileSearchRequest {
            query: "DPH".to_string(),
            search_names: false,
            search_contents: true,
            max_results: 10,
            ..Default::default()
        };
        let resp = search_files_sync(&policy, req).unwrap();
        if let Some(hit) = resp.results.iter().find(|r| r.display_name == "long.txt") {
            let line = hit.matched_line.as_deref().unwrap_or("");
            assert!(
                line.chars().count() <= MAX_LINE_CHARS,
                "matched line should be truncated to {MAX_LINE_CHARS} chars"
            );
        }
    }

    #[test]
    fn does_not_search_denied_dir() {
        let home = dirs::home_dir().unwrap();
        let ssh_dir = home.join(".ssh");
        if !ssh_dir.exists() {
            return; // can't test if dir doesn't exist
        }

        let policy = PathPolicy::default_for_user_home().unwrap();
        // Try to validate .ssh as a search root — should fail
        let result = policy.validate_search_root("~/.ssh", false);
        assert!(result.is_err(), ".ssh should be denied as a search root");
    }
}
