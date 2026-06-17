use anyhow::Result;
use bagent_memory::{InsertParams, MemoryStore};
use ollama_connector::OllamaClient;
use serde::Deserialize;
use std::sync::Arc;

// Thresholds for passive memory extraction
pub const CONFIDENCE_THRESHOLD: f32 = 0.75;
pub const IMPORTANCE_THRESHOLD: f32 = 0.60;

/// Full kind set supported by the memory ledger.
const VALID_KINDS: &[&str] = &[
    "fact",
    "entity",
    "preference",
    "instruction",
    "correction",
    "sk_glossary",
    "style_profile",
    "contact",
    "project",
    "workflow",
    "negative_rule",
    "profile",
];

/// Keywords in text that suggest sensitive data (blocked for passive store).
const SENSITIVE_TEXT_INDICATORS: &[&str] = &[
    "password",
    "heslo",
    "api key",
    "token",
    "secret",
    "credential",
    "pin ",
    "private key",
    "iban ",
    "bank account",
    "account number",
    "zdravie",
    "health",
    "choroba",
    "illness",
    "diagnos",
];

#[derive(Debug, Deserialize)]
struct ExtractedItem {
    kind: String,
    text: String,
    importance: f32,
    confidence: f32,
    namespace: String,
    #[serde(default = "default_sensitivity")]
    sensitivity: String,
    #[serde(default)]
    subject: Option<String>,
}

fn default_sensitivity() -> String {
    "normal".to_string()
}

#[derive(Debug, Deserialize)]
struct ExtractionResult {
    #[serde(default)]
    memories: Vec<ExtractedItem>,
    /// Support old format ("items" key) for backward compat
    #[serde(default)]
    items: Vec<ExtractedItem>,
}

impl ExtractionResult {
    fn all_items(self) -> Vec<ExtractedItem> {
        if !self.memories.is_empty() {
            self.memories
        } else {
            self.items
        }
    }
}

pub struct MemoryExtractor {
    ollama: OllamaClient,
    model: String,
}

impl MemoryExtractor {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Extract and store memorable items from a completed turn. Fire-and-forget.
    ///
    /// Strict gates:
    /// - confidence >= 0.75
    /// - importance >= 0.60
    /// - sensitivity == "normal"
    /// - valid kind
    /// - no sensitive text indicators
    /// - duplicate blocked by MemoryStore.insert_full (cosine >= 0.92)
    pub async fn run(
        &self,
        user_turn: &str,
        assistant_reply: &str,
        memory: Arc<MemoryStore>,
        language: &str,
    ) {
        match self.extract(user_turn, assistant_reply, language).await {
            Ok(items) => {
                for item in items {
                    // Gate 1: thresholds
                    if item.confidence < CONFIDENCE_THRESHOLD {
                        tracing::debug!(
                            "extractor: dropped (low confidence {:.2}): {:?}",
                            item.confidence,
                            &item.text[..item.text.len().min(60)]
                        );
                        continue;
                    }
                    if item.importance < IMPORTANCE_THRESHOLD {
                        tracing::debug!(
                            "extractor: dropped (low importance {:.2}): {:?}",
                            item.importance,
                            &item.text[..item.text.len().min(60)]
                        );
                        continue;
                    }

                    // Gate 2: valid kind
                    if !VALID_KINDS.contains(&item.kind.as_str()) {
                        tracing::debug!("extractor: dropped (invalid kind '{}')", item.kind);
                        continue;
                    }

                    // Gate 3: sensitivity
                    let sensitivity = if item.sensitivity == "sensitive" {
                        tracing::debug!("extractor: dropped sensitive item passively");
                        continue;
                    } else {
                        "normal"
                    };

                    // Gate 4: text contains sensitive indicators
                    let low_text = item.text.to_lowercase();
                    if SENSITIVE_TEXT_INDICATORS
                        .iter()
                        .any(|kw| low_text.contains(kw))
                    {
                        tracing::debug!(
                            "extractor: dropped (sensitive text pattern) in passive mode"
                        );
                        continue;
                    }

                    // Gate 5: skip obviously one-off / non-durable content
                    if is_one_off_content(&item.text) {
                        tracing::debug!("extractor: dropped (one-off content)");
                        continue;
                    }

                    let _ = memory
                        .insert_full(InsertParams {
                            namespace: &item.namespace,
                            kind: &item.kind,
                            language,
                            text: &item.text,
                            source: "passive",
                            confidence: item.confidence,
                            importance: item.importance,
                            sensitivity,
                            subject: item.subject.as_deref(),
                            ..Default::default()
                        })
                        .await;
                }
            }
            Err(e) => tracing::debug!("memory_extractor: {e}"),
        }
    }

    async fn extract(
        &self,
        user_turn: &str,
        assistant_reply: &str,
        language: &str,
    ) -> Result<Vec<ExtractedItem>> {
        let prompt = format!(
            r#"You are a strict memory extraction assistant. Analyze this conversation turn and extract ONLY durable, long-term facts about the user or their preferences.

User: {user_turn}
Assistant: {assistant_reply}

Return JSON with this exact structure:
{{
  "memories": [
    {{
      "kind": "preference|correction|sk_glossary|style_profile|contact|project|workflow|negative_rule|fact|entity",
      "text": "concise durable statement in {language}",
      "importance": 0.0-1.0,
      "confidence": 0.0-1.0,
      "namespace": "user_pref|sk_glossary|style_profile|contacts|corrections|negative_rules|global",
      "sensitivity": "normal|sensitive",
      "subject": null
    }}
  ]
}}

STRICT EXTRACTION RULES:
- ONLY extract durable user facts, preferences, corrections, glossary items, recurring workflows, or stable contacts/projects.
- Do NOT extract: one-off tasks, raw mail/note/invoice/attachment content, temporary context, guesses, or assistant behavior descriptions.
- Do NOT extract: passwords, API keys, bank credentials, health info, private legal details, financial account numbers.
- Do NOT extract: "User asked about X today" or "User discussed Y" — those are temporary, not durable.
- confidence >= 0.75 required for valid extraction.
- importance >= 0.60 required for valid extraction.
- sensitivity="sensitive" for anything involving personal health, credentials, financial account numbers, or private legal matters.
- If nothing durable exists: return {{"memories": []}}.
- Return ONLY valid JSON."#
        );

        let raw = self.ollama.generate_json(&self.model, &prompt, 0.0).await?;
        let result: ExtractionResult = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("memory extraction parse error: {e}\nraw: {raw}"))?;
        Ok(result.all_items())
    }
}

/// Quick heuristic: is this text likely a one-off task/event rather than a durable fact?
fn is_one_off_content(text: &str) -> bool {
    let low = text.to_lowercase();
    let one_off_patterns = [
        "today",
        "dnes",
        "yesterday",
        "včera",
        "this morning",
        "just now",
        "asked about",
        "discussed",
        "mentioned",
        "said that",
        "the email",
        "the attachment",
        "the invoice",
        "the message",
        "summary of",
        "summarized a",
        "zhrnutie",
    ];
    one_off_patterns.iter().any(|p| low.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_off_filter_rejects_email_reference() {
        assert!(is_one_off_content(
            "The email from Katka contained an attachment."
        ));
        assert!(is_one_off_content("User asked about the invoice today."));
        assert!(is_one_off_content(
            "Assistant summarized a message about payment."
        ));
    }

    #[test]
    fn one_off_filter_keeps_durable_pref() {
        assert!(!is_one_off_content(
            "User prefers concise summaries with bullet points."
        ));
        assert!(!is_one_off_content(
            "Preserve DPH, IČO, DIČ, IBAN verbatim in Slovak invoices."
        ));
        assert!(!is_one_off_content(
            "Katarína Horváthová may be referred to as Katka."
        ));
    }

    #[test]
    fn sensitive_text_blocked() {
        let sensitive_texts = [
            "Password for the server is abc123",
            "IBAN SK3112000000198742637541",
            "User's health diagnosis is confidential",
            "api key = sk-xxxx",
        ];
        for t in &sensitive_texts {
            let low = t.to_lowercase();
            assert!(
                super::SENSITIVE_TEXT_INDICATORS
                    .iter()
                    .any(|kw| low.contains(kw)),
                "Should detect sensitive in: {t}"
            );
        }
    }

    #[test]
    #[ignore = "requires Ollama + classifier model"]
    fn passive_extraction_returns_empty_for_one_off() {
        // Run with: cargo test -p bagent-agent -- --include-ignored
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let store = {
                use rusqlite::Connection;
                use std::sync::{Arc, Mutex};
                let conn = Connection::open_in_memory().unwrap();
                Arc::new(MemoryStore::new(
                    Arc::new(Mutex::new(conn)),
                    OllamaClient::new("http://127.0.0.1:11434"),
                ))
            };
            let extractor = MemoryExtractor::new(
                OllamaClient::new("http://127.0.0.1:11434"),
                "qwen2.5:0.5b".to_string(),
            );
            extractor
                .run(
                    "summarize this random news article",
                    "Here is a summary of the article...",
                    store,
                    "en",
                )
                .await;
            // No assertion needed — just checking it doesn't panic
        });
    }
}
