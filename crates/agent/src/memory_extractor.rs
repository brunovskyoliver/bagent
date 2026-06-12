use anyhow::Result;
use bagent_memory::MemoryStore;
use ollama_connector::{Message, OllamaClient};
use serde::Deserialize;
use std::sync::Arc;

pub const IMPORTANCE_THRESHOLD: f32 = 0.6;

#[derive(Debug, Deserialize)]
struct ExtractedItem {
    kind: String,
    text: String,
    importance: f32,
    namespace: String,
}

#[derive(Debug, Deserialize)]
struct ExtractionResult {
    #[serde(default)]
    items: Vec<ExtractedItem>,
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
                    if item.importance < IMPORTANCE_THRESHOLD {
                        continue;
                    }
                    let valid_kind = matches!(
                        item.kind.as_str(),
                        "fact" | "entity" | "preference" | "instruction" | "correction" | "sk_glossary"
                    );
                    if !valid_kind {
                        continue;
                    }
                    let _ = memory
                        .insert(&item.namespace, &item.kind, language, &item.text, None, None, None)
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
            r#"You are a memory extraction assistant. Analyze this conversation turn and extract any information worth remembering long-term about the user or their preferences.

User: {user_turn}
Assistant: {assistant_reply}

Return JSON with this exact structure:
{{
  "items": [
    {{
      "kind": "fact|entity|preference|instruction",
      "text": "concise factual statement in {language}",
      "importance": 0.0-1.0,
      "namespace": "global|user_pref"
    }}
  ]
}}

Rules:
- Only include items with genuine long-term value (importance >= 0.6)
- "fact": objective facts about the user's business/context
- "entity": named people, companies, projects the user references
- "preference": how the user likes things done
- "instruction": behavioral rules the user implies
- namespace "user_pref" for preferences/instructions, "global" for facts/entities
- If nothing worth remembering, return {{"items": []}}
- Return ONLY valid JSON, no markdown."#
        );

        let raw = self.ollama.summarize(&self.model, &[Message::user(&prompt)]).await?;

        // Strip markdown code fences if present
        let json_str = raw
            .trim()
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        let result: ExtractionResult = serde_json::from_str(json_str)?;
        Ok(result.items)
    }
}
