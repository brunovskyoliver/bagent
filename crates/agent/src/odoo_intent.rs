use anyhow::Result;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

/// LLM-classified intent for an Odoo-related user turn.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OdooIntent {
    /// The primary action the user wants.
    pub action: OdooAction,
    /// Free-form search query (company name, contact name, invoice number…).
    pub query: Option<String>,
    /// True when the user only wants open / unpaid / unresolved records.
    #[serde(default)]
    pub open_only: bool,
    /// Specific record ID when the user references a known record (rare).
    pub record_id: Option<i64>,
    /// True when the user explicitly asks to open the record in a browser/Safari.
    #[serde(default)]
    pub wants_open: bool,
}

/// Structured action classification for Odoo turns.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OdooAction {
    #[default]
    None,
    SearchContacts,
    GetInvoices,
    ListTickets,
    GetRecord,
    Open,
}

pub struct OdooIntentClassifier {
    ollama: OllamaClient,
    model: String,
}

impl OdooIntentClassifier {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Classify the user turn into a structured Odoo intent.
    ///
    /// `context` is a short snippet of recent turns (last ~4) plus an optional
    /// `[LastFoundOdooRecord]` line for coreference resolution.
    /// Pass empty string when there is no prior history.
    ///
    /// Returns `OdooIntent { action: None, .. }` when the turn is not about Odoo.
    pub async fn classify(&self, user_turn: &str, context: &str) -> Result<OdooIntent> {
        let context_block = if context.is_empty() {
            String::new()
        } else {
            format!(
                "Prior conversation for resolving pronouns only. Do not classify it:\n\
                 {context}\n\
                 End prior conversation.\n\n"
            )
        };

        let prompt = format!(
            "{context_block}User message: \"{user_turn}\"\n\n\
             Classify ONLY the user message as an Odoo CRM intent. Return JSON ONLY:\n\
             {{\n\
               \"action\": \"none|search_contacts|get_invoices|list_tickets|get_record|open\",\n\
               \"query\": null,\n\
               \"open_only\": false,\n\
               \"record_id\": null,\n\
               \"wants_open\": false\n\
             }}\n\n\
             Rules:\n\
             - search_contacts: user asks for contact/partner/company info from Odoo. Slovak: \"nájdi kontakt\", \"kto je zákazník\", \"IČO firmy\", \"partner\", \"adresa zákazníka\".\n\
             - get_invoices: user asks about invoices/bills in Odoo. Slovak: \"faktúr\", \"neuhradené faktúry\", \"splatné faktúry\", \"vystavené faktúry\", \"invoice\".\n\
             - list_tickets: user asks about their helpdesk tickets/tasks in Odoo. Slovak: \"helpdesk tikety\", \"moje tikety\", \"otvorené tikety\", \"úlohy v odoo\", \"ticket\".\n\
             - get_record: user asks for a specific record by ID or very specific reference.\n\
             - open: user wants to open/show a record in browser or Safari. Slovak: \"otvor v safari\", \"otvoriť v prehliadači\", \"ukáž záznam\". Set wants_open=true.\n\
             - none: not about Odoo data.\n\
             - query: the search term (company name, contact name, invoice number…). Extract from the message.\n\
             - open_only: true when user specifically asks for open/unpaid/unresolved records only.\n\
             - wants_open: true when user wants to open the found record in Safari/browser.\n\n\
             Examples:\n\
             \"nájdi kontakt Tenenet\" -> {{\"action\":\"search_contacts\",\"query\":\"Tenenet\",\"open_only\":false,\"record_id\":null,\"wants_open\":false}}\n\
             \"aké mám neuhradené faktúry?\" -> {{\"action\":\"get_invoices\",\"query\":null,\"open_only\":true,\"record_id\":null,\"wants_open\":false}}\n\
             \"moje helpdesk tikety\" -> {{\"action\":\"list_tickets\",\"query\":null,\"open_only\":false,\"record_id\":null,\"wants_open\":false}}\n\
             \"otvorené tikety\" -> {{\"action\":\"list_tickets\",\"query\":null,\"open_only\":true,\"record_id\":null,\"wants_open\":false}}\n\
             \"otvor to v safari\" -> {{\"action\":\"open\",\"query\":null,\"open_only\":false,\"record_id\":null,\"wants_open\":true}}\n\
             \"show me that invoice in browser\" -> {{\"action\":\"open\",\"query\":null,\"open_only\":false,\"record_id\":null,\"wants_open\":true}}\n\
             \"aké faktúry mám od Tenenet\" -> {{\"action\":\"get_invoices\",\"query\":\"Tenenet\",\"open_only\":false,\"record_id\":null,\"wants_open\":false}}",
        );

        let raw = self.ollama.generate_raw(&self.model, &prompt, 0.0).await?;
        tracing::debug!(raw_odoo_intent = %raw, "odoo_intent raw");
        let intent: OdooIntent = serde_json::from_str(clean_json(&raw)).unwrap_or_default();
        Ok(intent)
    }
}

fn clean_json(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix("```json").unwrap_or(s);
    let s = s.strip_prefix("```").unwrap_or(s);
    let s = s.strip_suffix("```").unwrap_or(s);
    s.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_default_is_none() {
        let i = OdooIntent::default();
        assert_eq!(i.action, OdooAction::None);
        assert!(!i.open_only);
        assert!(!i.wants_open);
    }

    #[test]
    fn deserialize_search_contacts() {
        let raw = r#"{"action":"search_contacts","query":"Tenenet","open_only":false,"record_id":null,"wants_open":false}"#;
        let i: OdooIntent = serde_json::from_str(raw).unwrap();
        assert_eq!(i.action, OdooAction::SearchContacts);
        assert_eq!(i.query.as_deref(), Some("Tenenet"));
    }

    #[test]
    fn deserialize_open_only_invoices() {
        let raw = r#"{"action":"get_invoices","query":null,"open_only":true,"record_id":null,"wants_open":false}"#;
        let i: OdooIntent = serde_json::from_str(raw).unwrap();
        assert_eq!(i.action, OdooAction::GetInvoices);
        assert!(i.open_only);
    }

    #[test]
    fn deserialize_open() {
        let raw = r#"{"action":"open","query":null,"open_only":false,"record_id":null,"wants_open":true}"#;
        let i: OdooIntent = serde_json::from_str(raw).unwrap();
        assert_eq!(i.action, OdooAction::Open);
        assert!(i.wants_open);
    }

    #[test]
    fn deserialize_none() {
        let raw = r#"{"action":"none","query":null,"open_only":false,"record_id":null,"wants_open":false}"#;
        let i: OdooIntent = serde_json::from_str(raw).unwrap();
        assert_eq!(i.action, OdooAction::None);
    }

    #[test]
    fn deserialize_bad_json_falls_back_to_default() {
        let raw = "not json at all";
        let i: OdooIntent = serde_json::from_str(clean_json(raw)).unwrap_or_default();
        assert_eq!(i.action, OdooAction::None);
    }

    #[tokio::test]
    #[ignore = "requires Ollama"]
    async fn live_classify_invoices() {
        let ollama = OllamaClient::new(ollama_connector::DEFAULT_BASE_URL);
        let clf = OdooIntentClassifier::new(ollama, "qwen2.5:7b".into());
        let intent = clf
            .classify("aké mám neuhradené faktúry?", "")
            .await
            .unwrap();
        println!("{intent:?}");
        assert_eq!(intent.action, OdooAction::GetInvoices);
        assert!(intent.open_only);
    }

    #[tokio::test]
    #[ignore = "requires Ollama"]
    async fn live_classify_contacts() {
        let ollama = OllamaClient::new(ollama_connector::DEFAULT_BASE_URL);
        let clf = OdooIntentClassifier::new(ollama, "qwen2.5:7b".into());
        let intent = clf
            .classify("nájdi kontakt Tenenet s.r.o.", "")
            .await
            .unwrap();
        println!("{intent:?}");
        assert_eq!(intent.action, OdooAction::SearchContacts);
        assert_eq!(intent.query.as_deref(), Some("Tenenet s.r.o."));
    }
}
