use anyhow::Result;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

/// Explicit trigger phrases that cause immediate memory capture.
const EXPLICIT_SK: &[&str] = &[
    "pamätaj si",
    "od teraz",
    "od tejto chvíle",
    "vždy odpovedaj",
    "nikdy neodpovedaj",
    "už nikdy",
    "vždy použi",
];
const EXPLICIT_EN: &[&str] = &[
    "remember",
    "from now on",
    "always",
    "never",
    "keep in mind",
    "make sure to",
    "stop doing",
    "don't",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectiveResult {
    pub directive: String,
    pub kind: String, // "preference" | "sk_glossary" | "style_profile"
    pub namespace: String,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrectionResult {
    pub is_correction: bool,
    pub what_was_wrong: Option<String>,
    pub correct_behavior: Option<String>,
    pub scope: String, // "global" | "sk_lang" | "this_session"
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleProfile {
    pub formality: String,            // "formal" | "informal"
    pub address_form: String,         // "Vy" | "Ty"
    pub brevity: String,              // "concise" | "detailed"
    pub response_length_pref: String, // "short" | "medium" | "long"
}

pub struct DirectiveExtractor {
    ollama: OllamaClient,
    model: String,
}

impl DirectiveExtractor {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Returns Some(result) if the turn contains an explicit memory trigger.
    pub async fn detect_and_extract(&self, user_turn: &str) -> Result<Option<DirectiveResult>> {
        if !has_explicit_trigger(user_turn) {
            return Ok(None);
        }
        let prompt = format!(
            "The user said: \"{user_turn}\"\n\n\
             Extract the memory directive from this message. Respond with JSON only, no markdown:\n\
             {{\n\
               \"directive\": \"<concise rule or fact to remember>\",\n\
               \"kind\": \"preference|sk_glossary|style_profile\",\n\
               \"namespace\": \"user_pref|sk_glossary|global\",\n\
               \"language\": \"sk|en|und\"\n\
             }}"
        );
        let response = self.ollama.generate_raw(&self.model, &prompt, 0.0).await?;
        let result: DirectiveResult = serde_json::from_str(clean_json(&response))?;
        Ok(Some(result))
    }
}

pub struct CorrectionClassifier {
    ollama: OllamaClient,
    model: String,
}

impl CorrectionClassifier {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Classifies whether the user turn is correcting the previous assistant turn.
    /// Confidence > 0.7 → store as correction memory.
    pub async fn classify(
        &self,
        prev_assistant: &str,
        user_turn: &str,
    ) -> Result<CorrectionResult> {
        let prompt = format!(
            "Previous assistant message:\n\"{prev_assistant}\"\n\n\
             User reply:\n\"{user_turn}\"\n\n\
             Is the user correcting the assistant? Respond with JSON only, no markdown:\n\
             {{\n\
               \"is_correction\": true|false,\n\
               \"what_was_wrong\": \"<what the assistant did wrong, or null>\",\n\
               \"correct_behavior\": \"<what the assistant should do instead, or null>\",\n\
               \"scope\": \"global|sk_lang|this_session\",\n\
               \"confidence\": 0.0\n\
             }}"
        );
        let response = self.ollama.generate_raw(&self.model, &prompt, 0.0).await?;
        let result: CorrectionResult = serde_json::from_str(clean_json(&response))?;
        Ok(result)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn has_explicit_trigger(text: &str) -> bool {
    let lower = text.to_lowercase();
    EXPLICIT_SK.iter().any(|t| lower.contains(t)) || EXPLICIT_EN.iter().any(|t| lower.contains(t))
}

fn clean_json(s: &str) -> &str {
    // Strip markdown code fences if model wraps in ```json ... ```
    let s = s.trim();
    let s = s.strip_prefix("```json").unwrap_or(s);
    let s = s.strip_prefix("```").unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}
