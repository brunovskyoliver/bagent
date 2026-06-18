use anyhow::Result;
use bagent_memory::MemoryHit;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

/// Structured last-known object from a connector, passed to the resolver.
/// Keep values short and already-sanitised by the daemon.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReferenceCandidate {
    pub connector: String,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub label: Option<String>,
    pub timestamp: Option<i64>,
    pub summary: Option<String>,
}

/// Output of the local reference resolver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceResolution {
    pub is_followup: bool,
    pub resolved_connector: Option<String>,
    pub resolved_entity_type: Option<String>,
    pub resolved_entity_id: Option<String>,
    pub resolved_contact_name: Option<String>,
    pub requested_operation: String,
    pub source_needed: String,
    pub target_connector: Option<String>,
    pub target_ref: Option<String>,
    pub reason: String,
    pub confidence: f32,
    pub needs_live_fetch: bool,
    pub standalone_query: String,
}

impl Default for ReferenceResolution {
    fn default() -> Self {
        Self {
            is_followup: false,
            resolved_connector: None,
            resolved_entity_type: None,
            resolved_entity_id: None,
            resolved_contact_name: None,
            requested_operation: "none".to_string(),
            source_needed: "none".to_string(),
            target_connector: None,
            target_ref: None,
            reason: String::new(),
            confidence: 0.0,
            needs_live_fetch: false,
            standalone_query: String::new(),
        }
    }
}

pub struct ReferenceResolver {
    ollama: OllamaClient,
    model: String,
}

impl ReferenceResolver {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    pub async fn resolve(
        &self,
        user_turn: &str,
        recent_context: &str,
        candidates: &[ReferenceCandidate],
        resolver_lessons: &[String],
        current_datetime: &str,
    ) -> Result<ReferenceResolution> {
        let candidates_json = serde_json::to_string(candidates).unwrap_or_else(|_| "[]".into());
        let lessons = format_resolver_lessons(resolver_lessons);
        let prompt = format!(
            r#"You resolve implicit references for a local personal assistant.

Current date/time: {current_datetime}
Current user turn: "{user_turn}"

Recent same-session conversation:
{recent_context}

Structured last connector references:
{candidates_json}

Relevant resolver lessons learned from explicit corrections:
{lessons}

Return JSON only. Decide whether the current turn needs a live source record, can be answered from current context, or is standalone.

Rules:
- Use ONLY recent same-session conversation and structured connector references.
- Do NOT use older cross-session memory.
- If the turn is standalone, set is_followup=false, source_needed="none", and requested_operation="none".
- Resolve from the current user turn, recent same-session conversation, and structured connector references.
- Prefer full_record when the user asks for details, rows, body content, attachments, or exact facts that were only summarized or listed earlier.
- If recent context contains only metadata/list summaries and the new turn asks for content details, target the original connector record instead of answering from the summary.
- Prefer more_history when a chat-style connector needs additional messages for tone, theme, timing, or conversation analysis.
- For exact timing questions, set requested_operation="fetch_timestamp".
- For "today/todays mailbox", resolve to mail with requested_operation="list_by_date"; this is not a follow-up, but it still selects mail.
- target_connector must match the connector to fetch from. target_ref should be the structured entity_id when available.
- reason must briefly state the evidence used, not a guess.
- Keep standalone_query concise and explicit.
- If confidence is below 0.60, leave connector fields null unless the user named a connector explicitly."#
        );

        let raw = self
            .ollama
            .generate_json_schema(&self.model, &prompt, reference_resolution_schema(), 0.0)
            .await?;
        let mut resolved = normalize_resolution(serde_json::from_str(&raw)?);
        let safe_borderline_fetch = resolved.confidence >= 0.50
            && resolved.target_connector.is_some()
            && matches!(
                resolved.source_needed.as_str(),
                "full_record" | "more_history"
            );
        if resolved.confidence < 0.60 && !connector_named(user_turn) && !safe_borderline_fetch {
            resolved.resolved_connector = None;
            resolved.resolved_entity_type = None;
            resolved.resolved_entity_id = None;
            resolved.target_connector = None;
            resolved.target_ref = None;
            resolved.source_needed = "none".to_string();
            resolved.needs_live_fetch = false;
        }
        Ok(resolved)
    }
}

pub fn select_resolver_lessons(hits: &[MemoryHit], max: usize) -> Vec<String> {
    hits.iter()
        .filter(|h| {
            h.item.namespace == "resolver_lessons"
                && h.item.kind == "routing_lesson"
                && h.item.source == "explicit"
                && h.item.status == "active"
                && h.item.sensitivity == "normal"
        })
        .take(max)
        .map(|h| h.item.text.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn format_resolver_lessons(lessons: &[String]) -> String {
    if lessons.is_empty() {
        "[]".to_string()
    } else {
        lessons
            .iter()
            .take(3)
            .map(|lesson| format!("- {lesson}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn normalize_resolution(mut resolved: ReferenceResolution) -> ReferenceResolution {
    resolved.confidence = resolved.confidence.clamp(0.0, 1.0);

    if resolved.resolved_connector.is_none() {
        match resolved.resolved_entity_type.as_deref() {
            Some("mail") | Some("message") => {
                resolved.resolved_connector = Some("mail".to_string());
            }
            Some("chat") => {
                resolved.resolved_connector = Some("whatsapp".to_string());
            }
            Some("file") => {
                resolved.resolved_connector = Some("filesystem".to_string());
            }
            Some("record") => {
                resolved.resolved_connector = Some("odoo".to_string());
            }
            _ => {}
        }
    }

    if resolved.target_connector.is_none() {
        resolved.target_connector = resolved.resolved_connector.clone();
    }
    if resolved.target_ref.is_none() {
        resolved.target_ref = resolved.resolved_entity_id.clone();
    }

    if !matches!(
        resolved.source_needed.as_str(),
        "none" | "current_context" | "full_record" | "more_history"
    ) {
        resolved.source_needed = if resolved.needs_live_fetch {
            "full_record".to_string()
        } else {
            "none".to_string()
        };
    }

    if matches!(
        resolved.source_needed.as_str(),
        "full_record" | "more_history"
    ) {
        resolved.needs_live_fetch = true;
    } else if resolved.source_needed == "none" {
        resolved.needs_live_fetch = false;
    }

    resolved
}

fn connector_named(s: &str) -> bool {
    let low = s.to_lowercase();
    [
        "whatsapp", "whatspp", "whatsap", "mail", "email", "inbox", "mailbox", "odoo", "file",
        "finder",
    ]
    .iter()
    .any(|needle| low.contains(needle))
}

fn reference_resolution_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "is_followup": { "type": "boolean" },
            "resolved_connector": {
                "type": ["string", "null"],
                "enum": ["mail", "whatsapp", "filesystem", "odoo", "screen", null]
            },
            "resolved_entity_type": {
                "type": ["string", "null"],
                "enum": ["chat", "message", "mail", "file", "record", "topic", null]
            },
            "resolved_entity_id": { "type": ["string", "null"] },
            "resolved_contact_name": { "type": ["string", "null"] },
            "requested_operation": {
                "type": "string",
                "enum": [
                    "answer_from_current_context",
                    "fetch_timestamp",
                    "fetch_more_history",
                    "fetch_full_record",
                    "list_by_date",
                    "analyze_tone",
                    "open",
                    "none"
                ]
            },
            "source_needed": {
                "type": "string",
                "enum": ["none", "current_context", "full_record", "more_history"]
            },
            "target_connector": {
                "type": ["string", "null"],
                "enum": ["mail", "whatsapp", "filesystem", "odoo", "screen", null]
            },
            "target_ref": { "type": ["string", "null"] },
            "reason": { "type": "string" },
            "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
            "needs_live_fetch": { "type": "boolean" },
            "standalone_query": { "type": "string" }
        },
        "required": [
            "is_followup",
            "resolved_connector",
            "resolved_entity_type",
            "resolved_entity_id",
            "resolved_contact_name",
            "requested_operation",
            "source_needed",
            "target_connector",
            "target_ref",
            "reason",
            "confidence",
            "needs_live_fetch",
            "standalone_query"
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_confidence_without_connector_clears_target() {
        let mut r = ReferenceResolution {
            is_followup: true,
            resolved_connector: Some("whatsapp".into()),
            resolved_entity_type: Some("chat".into()),
            resolved_entity_id: Some("abc".into()),
            confidence: 0.4,
            source_needed: "more_history".into(),
            target_connector: Some("whatsapp".into()),
            target_ref: Some("abc".into()),
            needs_live_fetch: true,
            ..Default::default()
        };
        if r.confidence < 0.60 && !super::connector_named("what about that") {
            r.resolved_connector = None;
            r.resolved_entity_type = None;
            r.resolved_entity_id = None;
            r.target_connector = None;
            r.target_ref = None;
            r.source_needed = "none".into();
            r.needs_live_fetch = false;
        }
        assert_eq!(r.resolved_connector, None);
        assert!(!r.needs_live_fetch);
    }

    #[test]
    fn connector_named_handles_mailbox_and_whatsapp_typos() {
        assert!(connector_named("whats in my todays mailbox"));
        assert!(connector_named("most recent text in whatspp"));
        assert!(!connector_named("when was this"));
    }

    #[test]
    fn entity_type_mail_normalizes_missing_connector() {
        let r = normalize_resolution(ReferenceResolution {
            is_followup: true,
            resolved_connector: None,
            resolved_entity_type: Some("mail".into()),
            resolved_entity_id: Some("42".into()),
            source_needed: "full_record".into(),
            confidence: 0.62,
            ..Default::default()
        });
        assert_eq!(r.resolved_connector.as_deref(), Some("mail"));
        assert_eq!(r.target_connector.as_deref(), Some("mail"));
        assert_eq!(r.target_ref.as_deref(), Some("42"));
        assert!(r.needs_live_fetch);
    }

    #[test]
    fn borderline_full_record_resolution_is_safe_to_fetch() {
        let resolved = normalize_resolution(ReferenceResolution {
            is_followup: true,
            resolved_connector: Some("mail".into()),
            resolved_entity_type: Some("mail".into()),
            resolved_entity_id: Some("42".into()),
            source_needed: "full_record".into(),
            confidence: 0.55,
            ..Default::default()
        });
        let safe_borderline_fetch = resolved.confidence >= 0.50
            && resolved.target_connector.is_some()
            && matches!(
                resolved.source_needed.as_str(),
                "full_record" | "more_history"
            );
        assert!(safe_borderline_fetch);
    }

    #[test]
    fn resolver_lesson_selection_is_feedback_gated() {
        fn hit(namespace: &str, kind: &str, source: &str, text: &str) -> MemoryHit {
            MemoryHit {
                item: bagent_memory::MemoryItem {
                    id: text.to_string(),
                    namespace: namespace.to_string(),
                    kind: kind.to_string(),
                    language: "und".to_string(),
                    text: text.to_string(),
                    source_ref: None,
                    metadata_json: None,
                    last_used_at: None,
                    use_count: 0,
                    created_at: "now".to_string(),
                    updated_at: "now".to_string(),
                    expires_at: None,
                    confidence: 0.9,
                    importance: 0.8,
                    status: "active".to_string(),
                    source: source.to_string(),
                    sensitivity: "normal".to_string(),
                    subject: None,
                    supersedes_id: None,
                },
                score: 0.9,
            }
        }

        let lessons = select_resolver_lessons(
            &[
                hit(
                    "resolver_lessons",
                    "routing_lesson",
                    "explicit",
                    "fetch body",
                ),
                hit("resolver_lessons", "routing_lesson", "passive", "ignore"),
                hit("corrections", "correction", "explicit", "ignore"),
            ],
            3,
        );
        assert_eq!(lessons, vec!["fetch body"]);
    }
}
