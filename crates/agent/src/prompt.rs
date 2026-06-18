//! Layered system prompt assembly.
//!
//! Layer order (highest authority → appended first):
//!   1. Core identity, safety rules, and language policy
//!   2. Selected skills (bounded section, bodies truncated)
//!   3. Relevant user memory (filtered, formatted with id/kind/confidence)
//!   4. Live tool data (mail/notes/odoo)
//!   5. Attachment context
//!   6. Session summary
//!   7. Recent session history
//!   8. Current user turn ← added by caller
//!
//! The builder now accepts pre-selected context from the planning layer
//! (ContextPlanner → SkillSelector → MemorySelector).
//! It no longer runs its own memory retrieval.

use bagent_memory::MemoryHit;
use ollama_connector::Message;
use serde::{Deserialize, Serialize};

// Re-exports for daemon usage
pub use crate::context_planner::ResponseLanguageHint;

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

// ── Base persona ──────────────────────────────────────────────────────────────

/// Core identity, safety rules, and language policy.
/// Language policy: English default; Slovak-aware preservation rules always active.
const BASE_IDENTITY: &str = "\
You are bagent — a personal macOS assistant built into the system status bar.\n\
\n\
## Safety rules\n\
- You are a conversational assistant, NOT an email client. Do not format responses as emails \
  (no \"Dobrý deň,\" / \"S pozdravom,\" / sign-off) unless the user explicitly asks you to draft or reply to an email.\n\
- Be concise and accurate. Never invent information you do not have.\n\
- If live data (mail, notes, attachments) is present in the context, work directly with it \
  — do not ask for information you already have.\n\
- When a found email is shown (starts with \"Našiel som email:\"), reproduce the full header block \
  (Od/Komu/Prijaté/Predmet) exactly as provided. Never substitute unknown fields with guesses.\n\
- Reproduce email body text verbatim. Never mix in content from other contexts or past turns.\n\
- If the email body says \"TELO EMAILU SA NEPODARILO NAČÍTAŤ\", say exactly that — never invent contents.\n\
\n\
## Language policy\n\
- Default conversation language: English, unless the user writes in Slovak or explicitly asks for Slovak.\n\
- Task output language: match the user's request or source content (email/invoice/note language).\n\
- Slovak content handling: always understand, preserve meaning, preserve diacritics, and preserve business terms.\n\
- Slovak diacritics to always preserve: á č ď é í ľ ĺ ň ó ô ŕ š ť ú ý ž.\n\
- Never translate these Slovak business/legal terms — keep them verbatim in all outputs:\n\
  DPH, faktúra, splatnosť, IČO, DIČ, IBAN, zmluva, objednávka, odberateľ, dodávateľ, upomienka, záloha, dobropis.\n\
- Never mix Czech into Slovak business output.\n\
\n\
## Memory usage rules\n\
- Use memory only as user-specific context hints. Do not invent preferences not present in memory.\n\
- If memory is absent, proceed without assuming user preferences.\n\
- If memories conflict, follow the newest explicit correction or ask the user.\n\
- Do not quote private source content (emails, notes, documents) unless the user asked for it.\n\
\n\
## Slovak email drafting (only when asked)\n\
When the user explicitly asks to draft or reply to a Slovak business email:\n\
- Use formal Slovak with \"Dobrý deň,\" opening and \"S pozdravom,\" closing.\n\
- Use the \"Vy\" form (capitalized). No informal greetings.\n\
- Preserve all Slovak business terms verbatim.\n\
- Never use Czech expressions in Slovak output.";

// ── Language hint helpers ─────────────────────────────────────────────────────

fn language_hint_instruction(hint: &ResponseLanguageHint, language: &str) -> Option<String> {
    match hint {
        ResponseLanguageHint::EnglishDefault => None, // base persona already says English default
        ResponseLanguageHint::MatchUser => {
            if language == "sk" {
                Some("Respond in Slovak — the user is writing in Slovak.".to_string())
            } else {
                None
            }
        }
        ResponseLanguageHint::MatchSourceContent => {
            Some("Match the language of the source content (email, invoice, note) in your output. \
                  For Slovak source content, respond in Slovak and preserve all Slovak terms verbatim.".to_string())
        }
        ResponseLanguageHint::SlovakRequired => {
            Some("This task requires Slovak output. Respond in formal Slovak. \
                  Preserve diacritics and all Slovak business terms verbatim.".to_string())
        }
        ResponseLanguageHint::UserSpecified(lang) => {
            Some(format!("Respond in {lang} as the user requested."))
        }
    }
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
        response_language_hint: &ResponseLanguageHint,
        selected_skills: &[SelectedSkill],
        selected_memory: &[MemoryHit],
        corrections: &[MemoryHit],
        tool_ctx: Option<String>,
        attachments_ctx: Option<String>,
        history: Vec<Message>,
        session_summary: Option<String>,
        recall_candidates: Vec<crate::ChatTurnHit>,
        needs_conversation_recall: bool,
        context_plan: Option<serde_json::Value>,
        memory_query: &str,
    ) -> anyhow::Result<BuiltPrompt> {
        let mut messages: Vec<Message> = Vec::new();
        let mut layers: Vec<PromptLayerTrace> = Vec::new();

        // Layer 1 — core identity + language policy
        push_system_layer(
            &mut messages,
            &mut layers,
            "identity_language_policy",
            BASE_IDENTITY,
        );

        // Layer 1b — language hint (only when non-default)
        if let Some(hint_text) = language_hint_instruction(response_language_hint, language) {
            push_system_layer(&mut messages, &mut layers, "language_hint", &hint_text);
        }

        // Layer 2 — selected skills
        if !selected_skills.is_empty() {
            let skill_block = format_skills_block(selected_skills);
            push_system_layer(&mut messages, &mut layers, "selected_skills", &skill_block);
        }

        // Layer 3a — corrections + sk_glossary
        if !corrections.is_empty() {
            let block = format_memory_block("## Active corrections and glossary", corrections);
            push_system_layer(&mut messages, &mut layers, "corrections_glossary", &block);
        }

        // Layer 3b — relevant user memory
        if !selected_memory.is_empty() {
            let block = format_memory_block("## Relevant user memory", selected_memory);
            push_system_layer(&mut messages, &mut layers, "retrieved_memory", &block);
        }

        // Layer 4 — live tool data
        if let Some(ctx) = tool_ctx {
            push_system_layer(&mut messages, &mut layers, "live_tool_context", &ctx);
        }

        // Layer 5 — attachment context
        if let Some(att) = attachments_ctx {
            push_system_layer(&mut messages, &mut layers, "attachment_context", &att);
        }

        // Layer 6 — session summary
        if let Some(summary) = session_summary {
            push_system_layer(
                &mut messages,
                &mut layers,
                "session_summary",
                &format!("## Session summary\n{summary}"),
            );
        }

        // Layer 7 — conversation recall (injected only when explicitly planned)
        let mut past_turn_traces: Vec<PromptPastTurnTrace> = Vec::new();
        for candidate in &recall_candidates {
            let injected = needs_conversation_recall;
            let reason = if injected {
                "explicit_recall_requested".to_string()
            } else {
                "automatic_cross_session_recall_disabled".to_string()
            };
            past_turn_traces.push(PromptPastTurnTrace {
                role: candidate.role.clone(),
                created_at: candidate.created_at.clone(),
                score: candidate.score,
                injected,
                reason,
                preview: preview(&candidate.content, 300),
            });
        }
        let recall_policy = if needs_conversation_recall {
            "explicit_recall_injected"
        } else {
            "cross_session_chat_recall_disabled_by_default"
        };
        if needs_conversation_recall && !recall_candidates.is_empty() {
            let recall_block: String = recall_candidates
                .iter()
                .map(|h| format!("[{}] {}", h.role, h.content))
                .collect::<Vec<_>>()
                .join("\n");
            push_system_layer(
                &mut messages,
                &mut layers,
                "conversation_recall",
                &format!("## Relevant past conversation\n{recall_block}"),
            );
        }

        // Layer 8 — recent session history
        if !history.is_empty() {
            layers.push(PromptLayerTrace {
                name: "recent_session_history".to_string(),
                role: "mixed".to_string(),
                included: true,
                chars: history.iter().map(|m| m.content.len()).sum(),
                preview: preview(
                    &history
                        .iter()
                        .map(|m| format!("[{}] {}", m.role, m.content))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    240,
                ),
            });
        }
        messages.extend(history);

        let selected_skill_names: Vec<String> =
            selected_skills.iter().map(|s| s.name.clone()).collect();
        let selected_memory_ids: Vec<String> = selected_memory
            .iter()
            .chain(corrections.iter())
            .map(|h| h.item.id.clone())
            .collect();

        let trace = PromptTrace {
            language: language.to_string(),
            recall_policy: recall_policy.to_string(),
            context_plan,
            layers,
            memory_hits: selected_memory.iter().map(memory_hit_trace).collect(),
            correction_hits: corrections.iter().map(memory_hit_trace).collect(),
            past_turn_candidates: past_turn_traces,
            selected_skill_names,
            selected_memory_ids,
            conversation_recall_injected: needs_conversation_recall
                && !recall_candidates.is_empty(),
            memory_query: memory_query.to_string(),
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

fn format_skills_block(skills: &[SelectedSkill]) -> String {
    let mut parts = vec!["## Selected skills\n".to_string()];
    for skill in skills {
        parts.push(format!("### {}\n{}", skill.name, skill.body));
    }
    parts.join("\n")
}

fn format_memory_block(header: &str, hits: &[MemoryHit]) -> String {
    let lines: Vec<String> = hits
        .iter()
        .map(|h| {
            format!(
                "- [id:{} | {} | confidence:{:.2}] {}",
                h.item.id, h.item.kind, h.item.confidence, h.item.text
            )
        })
        .collect();
    format!("{header}\n{}", lines.join("\n"))
}

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

    async fn build_simple(
        user_turn: &str,
        lang: &str,
        hint: ResponseLanguageHint,
        needs_recall: bool,
    ) -> BuiltPrompt {
        let builder = PromptBuilder::new();
        builder
            .build(
                user_turn,
                lang,
                &hint,
                &[],    // no skills
                &[],    // no memory
                &[],    // no corrections
                None,   // no tool ctx
                None,   // no attachments
                vec![], // no history
                None,   // no summary
                vec![], // no recall candidates
                needs_recall,
                None,
                user_turn,
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn past_chat_candidates_are_not_injected_by_default() {
        // Simulate a recall candidate
        let recall = crate::ChatTurnHit {
            role: "assistant".to_string(),
            content: "Katka z TENENET poslala email s predmetom dochádzky.".to_string(),
            created_at: "2026-06-12T10:00:00Z".to_string(),
            score: 0.8,
        };

        let builder = PromptBuilder::new();
        let built = builder
            .build(
                "katka dochádzky",
                "sk",
                &ResponseLanguageHint::MatchUser,
                &[],
                &[],
                &[],
                None,
                None,
                vec![],
                None,
                vec![recall],
                false, // needs_conversation_recall = false
                None,
                "katka dochádzky",
            )
            .await
            .unwrap();

        let sent_prompt = built
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !sent_prompt.contains("TENENET"),
            "past chat content must not be injected into model messages when recall not needed"
        );
        assert!(
            built
                .trace
                .past_turn_candidates
                .iter()
                .any(|c| c.preview.contains("TENENET") && !c.injected),
            "past chat should remain visible as a non-injected debug candidate"
        );
        assert_eq!(built.trace.conversation_recall_injected, false);
    }

    #[tokio::test]
    async fn past_chat_injected_when_recall_requested() {
        let recall = crate::ChatTurnHit {
            role: "assistant".to_string(),
            content: "We decided to postpone the invoice.".to_string(),
            created_at: "2026-06-10T10:00:00Z".to_string(),
            score: 0.85,
        };

        let builder = PromptBuilder::new();
        let built = builder
            .build(
                "what did we decide about the invoice?",
                "en",
                &ResponseLanguageHint::EnglishDefault,
                &[],
                &[],
                &[],
                None,
                None,
                vec![],
                None,
                vec![recall],
                true, // needs_conversation_recall = true
                None,
                "invoice",
            )
            .await
            .unwrap();

        let sent_prompt = built
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            sent_prompt.contains("postpone the invoice"),
            "recall content must appear in prompt when needs_conversation_recall=true"
        );
        assert_eq!(built.trace.conversation_recall_injected, true);
    }

    #[tokio::test]
    async fn trace_includes_selected_skill_names() {
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
        let skill_layer = built
            .trace
            .layers
            .iter()
            .find(|l| l.name == "selected_skills");
        assert!(skill_layer.is_some(), "skills layer must be present");
    }

    #[tokio::test]
    async fn trace_includes_selected_memory_ids() {
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

        assert!(built
            .trace
            .selected_memory_ids
            .contains(&"mem_test_123".to_string()));
    }

    #[tokio::test]
    async fn english_default_no_extra_language_layer() {
        let built = build_simple(
            "how are you?",
            "en",
            ResponseLanguageHint::EnglishDefault,
            false,
        )
        .await;
        let has_lang_hint = built.trace.layers.iter().any(|l| l.name == "language_hint");
        assert!(
            !has_lang_hint,
            "English default should not add an extra language hint layer"
        );
    }

    #[tokio::test]
    async fn slovak_required_adds_language_layer() {
        let built = build_simple(
            "napíš email",
            "sk",
            ResponseLanguageHint::SlovakRequired,
            false,
        )
        .await;
        let has_lang_hint = built.trace.layers.iter().any(|l| l.name == "language_hint");
        assert!(
            has_lang_hint,
            "SlovakRequired should add a language hint layer"
        );
    }

    #[tokio::test]
    async fn memory_formatted_with_id_kind_confidence() {
        use bagent_memory::{MemoryHit, MemoryItem};
        let item = MemoryItem {
            id: "mem_xyz".to_string(),
            namespace: "sk_glossary".to_string(),
            kind: "sk_glossary".to_string(),
            language: "sk".to_string(),
            text: "Preserve DPH verbatim.".to_string(),
            source_ref: None,
            metadata_json: None,
            last_used_at: None,
            use_count: 0,
            created_at: "2026-06-01T00:00:00Z".to_string(),
            updated_at: "2026-06-01T00:00:00Z".to_string(),
            expires_at: None,
            confidence: 0.95,
            importance: 0.8,
            status: "active".to_string(),
            source: "explicit".to_string(),
            sensitivity: "normal".to_string(),
            subject: None,
            supersedes_id: None,
        };
        let hit = MemoryHit { item, score: 0.9 };
        let builder = PromptBuilder::new();
        let built = builder
            .build(
                "test",
                "sk",
                &ResponseLanguageHint::SlovakRequired,
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

        let mem_layer = built
            .trace
            .layers
            .iter()
            .find(|l| l.name == "retrieved_memory");
        assert!(mem_layer.is_some());
        let sent = built
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(sent.contains("mem_xyz"), "memory id must appear in prompt");
        assert!(sent.contains("sk_glossary"), "kind must appear in prompt");
        assert!(sent.contains("0.95"), "confidence must appear in prompt");
    }
}
