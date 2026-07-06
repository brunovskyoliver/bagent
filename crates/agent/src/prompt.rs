//! Stateless prompt assembly.
//!
//! Only current-turn context is allowed here: live connector data and ephemeral
//! attachment/screen context. The caller appends the current user turn.
//! Memory, corrections, summaries, past conversations, and selected skill bodies
//! are intentionally ignored so every LLM interaction starts clean.

use bagent_memory::MemoryHit;
use ollama_connector::Message;
use serde::{Deserialize, Serialize};

/// What kind of response language should the assistant target?
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseLanguageHint {
    /// Default: assistant speaks English unless the user writes Slovak.
    EnglishDefault,
    /// Mirror whatever language the user used in this turn.
    MatchUser,
    /// Match the language of the source content being worked on (mail, notes, etc.).
    MatchSourceContent,
    /// Slovak is required regardless of the user's input language.
    SlovakRequired,
    /// Specific language override from the user.
    UserSpecified(String),
}

/// A skill chosen for the current prompt turn.
/// Mirrors `bagent_skills::selector::SelectedSkill` so agent crate
/// doesn't depend on the skills crate directly.
#[derive(Debug, Clone)]
pub struct SelectedSkill {
    pub name: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltPrompt {
    pub messages: Vec<Message>,
    pub trace: PromptTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTrace {
    pub language: String,
    pub recall_policy: String,
    pub context_plan: Option<serde_json::Value>,
    pub layers: Vec<PromptLayerTrace>,
    pub memory_hits: Vec<PromptMemoryHitTrace>,
    pub correction_hits: Vec<PromptMemoryHitTrace>,
    pub past_turn_candidates: Vec<PromptPastTurnTrace>,
    pub selected_skill_names: Vec<String>,
    pub selected_memory_ids: Vec<String>,
    pub conversation_recall_injected: bool,
    pub memory_query: String,
    // Mail search diagnostics emitted by the daemon. Kept as JSON so the agent
    // crate does not depend on the Apple Mail adapter's concrete types.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mail_search_trace: Option<serde_json::Value>,
    // Phase 13A — File intent trace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_intent: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_tool_called: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_result_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_action_required_approval: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_action_approved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_action_denied_reason: Option<String>,
    // Phase 11 — WhatsApp intent trace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whatsapp_intent: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whatsapp_contact: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whatsapp_chat_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whatsapp_context_injected: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whatsapp_send_approval_id: Option<String>,
    // Reference resolver trace
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_resolution: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_connector: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standalone_query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_needs_live_fetch: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptLayerTrace {
    pub name: String,
    pub role: String,
    pub included: bool,
    pub chars: usize,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMemoryHitTrace {
    pub id: String,
    pub namespace: String,
    pub kind: String,
    pub score: f32,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptPastTurnTrace {
    pub role: String,
    pub created_at: String,
    pub score: f32,
    pub injected: bool,
    pub reason: String,
    pub preview: String,
}

// ── Builder ───────────────────────────────────────────────────────────────────

pub struct PromptBuilder;

impl PromptBuilder {
    pub fn new() -> Self {
        Self
    }

    /// Build the full message list up through layer 7 (session summary + history).
    /// Caller appends the user turn and submits to Ollama.
    ///
    /// All context (skills, memory, recall candidates) is pre-selected by the caller.
    pub async fn build(
        &self,
        _user_turn: &str,
        language: &str,
        _response_language_hint: &ResponseLanguageHint,
        selected_skills: &[SelectedSkill],
        selected_memory: &[MemoryHit],
        corrections: &[MemoryHit],
        tool_ctx: Option<String>,
        attachments_ctx: Option<String>,
        _history: Vec<Message>,
        _session_summary: Option<String>,
        recall_candidates: Vec<crate::ChatTurnHit>,
        _needs_conversation_recall: bool,
        context_plan: Option<serde_json::Value>,
        memory_query: &str,
    ) -> anyhow::Result<BuiltPrompt> {
        let mut messages: Vec<Message> = Vec::new();
        let mut layers: Vec<PromptLayerTrace> = Vec::new();

        if let Some(ctx) = tool_ctx {
            push_system_layer(&mut messages, &mut layers, "live_tool_context", &ctx);
        }

        if let Some(att) = attachments_ctx {
            push_system_layer(&mut messages, &mut layers, "attachment_context", &att);
        }

        let mut past_turn_traces: Vec<PromptPastTurnTrace> = Vec::new();
        for candidate in &recall_candidates {
            past_turn_traces.push(PromptPastTurnTrace {
                role: candidate.role.clone(),
                created_at: candidate.created_at.clone(),
                score: candidate.score,
                injected: false,
                reason: "stateless_prompt_no_recall".to_string(),
                preview: preview(&candidate.content, 300),
            });
        }

        let selected_skill_names: Vec<String> =
            selected_skills.iter().map(|s| s.name.clone()).collect();

        let trace = PromptTrace {
            language: language.to_string(),
            recall_policy: "stateless_no_recall".to_string(),
            context_plan,
            layers,
            memory_hits: selected_memory.iter().map(memory_hit_trace).collect(),
            correction_hits: corrections.iter().map(memory_hit_trace).collect(),
            past_turn_candidates: past_turn_traces,
            selected_skill_names,
            selected_memory_ids: Vec::new(),
            conversation_recall_injected: false,
            memory_query: memory_query.to_string(),
            mail_search_trace: None,
            file_intent: None,
            file_tool_called: None,
            file_result_count: None,
            file_action_required_approval: None,
            file_action_approved: None,
            file_action_denied_reason: None,
            whatsapp_intent: None,
            whatsapp_contact: None,
            whatsapp_chat_id: None,
            whatsapp_context_injected: None,
            whatsapp_send_approval_id: None,
            reference_resolution: None,
            resolved_connector: None,
            standalone_query: None,
            reference_needs_live_fetch: None,
        };

        Ok(BuiltPrompt { messages, trace })
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Formatting helpers ────────────────────────────────────────────────────────

fn push_system_layer(
    messages: &mut Vec<Message>,
    layers: &mut Vec<PromptLayerTrace>,
    name: &str,
    content: &str,
) {
    messages.push(Message::system(content));
    layers.push(PromptLayerTrace {
        name: name.to_string(),
        role: "system".to_string(),
        included: true,
        chars: content.len(),
        preview: preview(content, 240),
    });
}

fn memory_hit_trace(hit: &MemoryHit) -> PromptMemoryHitTrace {
    PromptMemoryHitTrace {
        id: hit.item.id.clone(),
        namespace: hit.item.namespace.clone(),
        kind: hit.item.kind.clone(),
        score: hit.score,
        preview: preview(&hit.item.text, 240),
    }
}

pub fn preview(s: &str, max: usize) -> String {
    let compact = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.len() <= max {
        compact
    } else {
        let end = compact.floor_char_boundary(max);
        format!("{}…", &compact[..end])
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_current_turn_context_layers_are_injected() {
        let recall = crate::ChatTurnHit {
            role: "assistant".to_string(),
            content: "Katka z TENENET poslala email s predmetom dochádzky.".to_string(),
            created_at: "2026-06-12T10:00:00Z".to_string(),
            score: 0.8,
        };

        let builder = PromptBuilder::new();
        let built = builder
            .build(
                "current turn",
                "sk",
                &ResponseLanguageHint::MatchUser,
                &[SelectedSkill {
                    name: "sk-business-email".to_string(),
                    body: "NEVER_INJECT_SKILL_BODY".to_string(),
                }],
                &[],
                &[],
                Some("LIVE_CONNECTOR_CONTEXT".to_string()),
                Some("ATTACHMENT_CONTEXT".to_string()),
                vec![Message::user("OLD_USER_TURN")],
                Some("OLD_SESSION_SUMMARY".to_string()),
                vec![recall],
                true,
                None,
                "current turn",
            )
            .await
            .unwrap();

        let sent_prompt = built
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(sent_prompt.contains("LIVE_CONNECTOR_CONTEXT"));
        assert!(sent_prompt.contains("ATTACHMENT_CONTEXT"));
        assert!(!sent_prompt.contains("OLD_USER_TURN"));
        assert!(!sent_prompt.contains("OLD_SESSION_SUMMARY"));
        assert!(!sent_prompt.contains("TENENET"));
        assert!(!sent_prompt.contains("NEVER_INJECT_SKILL_BODY"));
        assert_eq!(built.trace.layers.len(), 2);
        assert_eq!(built.trace.conversation_recall_injected, false);
    }

    #[tokio::test]
    async fn trace_keeps_skill_names_but_not_skill_body() {
        let skill = SelectedSkill {
            name: "sk-business-email".to_string(),
            body: "Some skill body.".to_string(),
        };
        let builder = PromptBuilder::new();
        let built = builder
            .build(
                "test",
                "sk",
                &ResponseLanguageHint::SlovakRequired,
                &[skill],
                &[],
                &[],
                None,
                None,
                vec![],
                None,
                vec![],
                false,
                None,
                "test",
            )
            .await
            .unwrap();

        assert!(built
            .trace
            .selected_skill_names
            .contains(&"sk-business-email".to_string()));
        assert!(built.messages.is_empty());
    }

    #[tokio::test]
    async fn selected_memory_is_traced_but_never_injected_or_selected() {
        use bagent_memory::{MemoryHit, MemoryItem};
        let item = MemoryItem {
            id: "mem_test_123".to_string(),
            namespace: "user_pref".to_string(),
            kind: "preference".to_string(),
            language: "en".to_string(),
            text: "User prefers bullet points.".to_string(),
            source_ref: None,
            metadata_json: None,
            last_used_at: None,
            use_count: 0,
            created_at: "2026-06-01T00:00:00Z".to_string(),
            updated_at: "2026-06-01T00:00:00Z".to_string(),
            expires_at: None,
            confidence: 0.9,
            importance: 0.7,
            status: "active".to_string(),
            source: "explicit".to_string(),
            sensitivity: "normal".to_string(),
            subject: None,
            supersedes_id: None,
        };
        let hit = MemoryHit { item, score: 0.85 };
        let builder = PromptBuilder::new();
        let built = builder
            .build(
                "test",
                "en",
                &ResponseLanguageHint::EnglishDefault,
                &[],
                &[hit],
                &[],
                None,
                None,
                vec![],
                None,
                vec![],
                false,
                None,
                "test",
            )
            .await
            .unwrap();

        let sent = built
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!sent.contains("User prefers bullet points."));
        assert!(built.trace.selected_memory_ids.is_empty());
        assert_eq!(built.trace.memory_hits.len(), 1);
    }
}
