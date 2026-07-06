//! Evidence — the only shape connector results may take on their way to the
//! main LLM. Sanitized, size-capped, ranked and deduplicated outside the model.

use serde::{Deserialize, Serialize};

use crate::routing::Source;

pub const SNIPPET_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub source: Source,
    pub title: String,
    pub author: Option<String>,
    /// ISO-8601 date (or "YYYY-MM-DD") when known.
    pub date: Option<String>,
    pub snippet: String,
    /// Connector-local identifier (mail rowid, file path, chat id).
    pub object_id: String,
    /// Human-readable location (mailbox, folder, chat name).
    pub location: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Ranker score, 0.0–1.0-ish (unnormalized sum of signal weights).
    pub confidence: f32,
}

impl Evidence {
    pub fn new(source: Source, title: impl Into<String>, object_id: impl Into<String>) -> Self {
        Self {
            source,
            title: title.into(),
            author: None,
            date: None,
            snippet: String::new(),
            object_id: object_id.into(),
            location: None,
            metadata: serde_json::Value::Null,
            confidence: 0.0,
        }
    }

    pub fn with_snippet(mut self, snippet: impl Into<String>) -> Self {
        let s: String = snippet.into();
        self.snippet = s.chars().take(SNIPPET_MAX_CHARS).collect();
        self
    }
}

// ── Ranking ──────────────────────────────────────────────────────────────────

/// Deterministic first-pass ranking + dedup. `priority_of(source)` maps a
/// source to its route-plan priority (1 = best). Truncates to `max_items`.
pub fn rank(
    query: &str,
    person: Option<&str>,
    priority_of: impl Fn(Source) -> u8,
    mut items: Vec<Evidence>,
    max_items: usize,
) -> Vec<Evidence> {
    let query_low = query.to_lowercase();
    let terms: Vec<&str> = query_low.split_whitespace().filter(|t| t.len() > 1).collect();
    let person_low = person.map(|p| p.to_lowercase());

    for ev in items.iter_mut() {
        ev.confidence = lexical_score(&query_low, &terms, person_low.as_deref(), &priority_of, ev);
    }
    items.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());

    // Dedup: same object, or near-identical snippet (normalized prefix hash).
    let mut seen_ids = std::collections::HashSet::new();
    let mut seen_snippets = std::collections::HashSet::new();
    items.retain(|ev| {
        let id_key = (ev.source, ev.object_id.clone());
        if !seen_ids.insert(id_key) {
            return false;
        }
        let norm: String = ev
            .snippet
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(120)
            .collect();
        norm.is_empty() || seen_snippets.insert(norm)
    });

    items.truncate(max_items);
    items
}

fn lexical_score(
    query_low: &str,
    terms: &[&str],
    person_low: Option<&str>,
    priority_of: &impl Fn(Source) -> u8,
    ev: &Evidence,
) -> f32 {
    let haystack = format!(
        "{} {} {}",
        ev.title.to_lowercase(),
        ev.snippet.to_lowercase(),
        ev.location.as_deref().unwrap_or("").to_lowercase()
    );
    let mut score = 0.0f32;

    // Keyword overlap
    if !terms.is_empty() {
        let hits = terms.iter().filter(|t| haystack.contains(**t)).count();
        score += 0.4 * (hits as f32 / terms.len() as f32);
    }
    // Exact phrase
    if terms.len() > 1 && !query_low.trim().is_empty() && haystack.contains(query_low.trim()) {
        score += 0.2;
    }
    // Person match against author
    if let (Some(p), Some(a)) = (person_low, ev.author.as_deref()) {
        if a.to_lowercase().contains(p) {
            score += 0.25;
        }
    }
    // Tool priority (1 = best)
    let prio = priority_of(ev.source);
    score += 0.15 / prio.max(1) as f32;
    // Recency: light boost for dates within ~30 days
    if let Some(days) = age_days(ev.date.as_deref()) {
        if days <= 30 {
            score += 0.1 * (1.0 - days as f32 / 30.0);
        }
    }
    score
}

fn age_days(date: Option<&str>) -> Option<i64> {
    let d = date?;
    let parsed = chrono::NaiveDate::parse_from_str(&d[..10.min(d.len())], "%Y-%m-%d").ok()?;
    Some((chrono::Utc::now().date_naive() - parsed).num_days().max(0))
}

/// Optional second pass: one batched bge-m3 embedding call over the query and
/// the (already lexically ranked) items; blends cosine into the score.
pub async fn embed_rerank(
    ollama: &ollama_connector::OllamaClient,
    embed_model: &str,
    query: &str,
    items: &mut Vec<Evidence>,
) -> anyhow::Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let mut texts = vec![query.to_string()];
    texts.extend(
        items
            .iter()
            .map(|e| format!("{} {}", e.title, e.snippet).chars().take(512).collect()),
    );
    let vecs = ollama.embed_batch(embed_model, &texts).await?;
    if vecs.len() != items.len() + 1 {
        anyhow::bail!("embed batch size mismatch");
    }
    let qv = &vecs[0];
    for (ev, v) in items.iter_mut().zip(vecs[1..].iter()) {
        ev.confidence = 0.5 * ev.confidence + 0.5 * cosine(qv, v);
    }
    items.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    Ok(())
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// Render ranked evidence as the prompt block the synthesizer sees.
/// Numbered, source-tagged, size-limited — never raw connector dumps.
pub fn render_evidence_block(items: &[Evidence]) -> String {
    let mut out = String::from("## Evidence (local search results)\n");
    for (i, ev) in items.iter().enumerate() {
        let src = match ev.source {
            Source::AppleMail => "Apple Mail",
            Source::Files => "File",
            Source::Whatsapp => "WhatsApp",
        };
        out.push_str(&format!(
            "[{}] {} — {}{}{}{}\n    {}\n",
            i + 1,
            src,
            ev.title,
            ev.author
                .as_deref()
                .map(|a| format!(" | from: {a}"))
                .unwrap_or_default(),
            ev.date
                .as_deref()
                .map(|d| format!(" | {d}"))
                .unwrap_or_default(),
            ev.location
                .as_deref()
                .map(|l| format!(" | {l}"))
                .unwrap_or_default(),
            ev.snippet.replace('\n', " "),
        ));
    }
    out
}

/// Full tool-context block for the synthesizer: grounding rules + what was
/// searched + ranked evidence + optional full content of the top match.
/// This is injected as a system layer — the model never sees raw connector data.
pub fn render_evidence_context(
    searched: &[(Source, usize)],
    items: &[Evidence],
    full_content: Option<&str>,
) -> String {
    let searched_line = searched
        .iter()
        .map(|(s, n)| {
            let name = match s {
                Source::AppleMail => "Apple Mail",
                Source::Files => "local files",
                Source::Whatsapp => "WhatsApp",
            };
            format!("{name} ({n} results)")
        })
        .collect::<Vec<_>>()
        .join(", ");

    let mut out = format!(
        "## Local search evidence\n\
         Searched: {searched_line}.\n\
         Rules for answering:\n\
         - Answer using ONLY the evidence below. Do not invent emails, files, \
           messages, dates, names, or sources.\n\
         - Mention the source app for each finding and cite its provenance \
           (email subject/date, file path, WhatsApp chat/date).\n\
         - If the evidence is insufficient, say what was searched and that no \
           match was found — do not guess.\n\
         - If multiple matches are plausible, present the most likely ones and \
           say why.\n\
         - Do not reveal private details beyond what the question needs.\n\n"
    );
    if items.is_empty() {
        out.push_str("No matching results were found in the searched sources.\n");
    } else {
        out.push_str(&render_evidence_block(items));
    }
    if let Some(content) = full_content {
        out.push_str("\n### Full content of top match\n");
        out.push_str(content);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(source: Source, id: &str, title: &str, snippet: &str) -> Evidence {
        Evidence::new(source, title, id).with_snippet(snippet)
    }

    #[test]
    fn ranks_keyword_matches_higher() {
        let items = vec![
            ev(Source::Files, "a", "vacation.jpg", "beach photo"),
            ev(Source::Files, "b", "contract_maria.pdf", "contract with maria draft"),
        ];
        let ranked = rank("contract", Some("maria"), |_| 1, items, 12);
        assert_eq!(ranked[0].object_id, "b");
    }

    #[test]
    fn dedups_same_object_and_near_identical_snippets() {
        let items = vec![
            ev(Source::AppleMail, "1", "Invoice June", "your invoice for june is attached"),
            ev(Source::AppleMail, "1", "Invoice June", "your invoice for june is attached"),
            ev(Source::Whatsapp, "c9", "Chat", "Your invoice for June is attached!"),
        ];
        let ranked = rank("invoice june", None, |_| 1, items, 12);
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn truncates_to_max_and_caps_snippets() {
        let items: Vec<Evidence> = (0..20)
            .map(|i| {
                ev(
                    Source::Files,
                    &format!("f{i}"),
                    &format!("doc {i}"),
                    &format!("{i} unique snippet {}", "x".repeat(1000)),
                )
            })
            .collect();
        assert!(items.iter().all(|e| e.snippet.chars().count() <= SNIPPET_MAX_CHARS));
        let ranked = rank("doc", None, |_| 1, items, 12);
        assert_eq!(ranked.len(), 12);
    }

    #[test]
    fn priority_breaks_ties() {
        let items = vec![
            ev(Source::Whatsapp, "w", "address note", "the address is here"),
            ev(Source::AppleMail, "m", "address note", "the address is right here"),
        ];
        let ranked = rank(
            "address",
            None,
            |s| if s == Source::AppleMail { 1 } else { 2 },
            items,
            12,
        );
        assert_eq!(ranked[0].source, Source::AppleMail);
    }

    #[test]
    fn render_includes_provenance() {
        let mut e = ev(Source::AppleMail, "42", "Zmluva", "podpísaná zmluva v prílohe");
        e.author = Some("maria@example.com".into());
        e.date = Some("2026-06-01".into());
        let block = render_evidence_block(&[e]);
        assert!(block.contains("[1] Apple Mail — Zmluva"));
        assert!(block.contains("maria@example.com"));
        assert!(block.contains("2026-06-01"));
    }
}
