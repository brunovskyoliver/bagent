use anyhow::Result;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

/// LLM-classified intent for a WhatsApp user turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsappIntent {
    pub action: WhatsappAction,
    /// Contact name from coreference or explicit mention (e.g. "Peter", "Katka").
    pub contact_name: Option<String>,
    /// Explicit phone number if stated.
    pub phone: Option<String>,
    /// WhatsApp chat JID if the user stated it explicitly.
    pub chat_id: Option<String>,
    /// Search keywords (e.g. ["faktúra", "2026"]).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Date filter in ISO format (YYYY-MM-DD) after normalisation.
    pub date: Option<String>,
    /// Exact message text to send (only for DraftSend).
    pub message_text: Option<String>,
    /// Result limit hint.
    pub limit: Option<u32>,
}

impl Default for WhatsappIntent {
    fn default() -> Self {
        Self {
            action: WhatsappAction::None,
            contact_name: None,
            phone: None,
            chat_id: None,
            keywords: vec![],
            date: None,
            message_text: None,
            limit: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhatsappAction {
    #[default]
    None,
    ListRecent,
    Search,
    ReadHistory,
    DraftSend,
}

// ── Classifier ────────────────────────────────────────────────────────────────

pub struct WhatsappIntentClassifier {
    ollama: OllamaClient,
    model: String,
}

impl WhatsappIntentClassifier {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Classify whether the user is asking about WhatsApp.
    ///
    /// `context` is a short snippet of recent conversation turns including
    /// `[LastFoundWhatsapp]` when available, for coreference resolution.
    ///
    /// Returns `WhatsappIntent { action: None, .. }` when the turn is not WhatsApp-related.
    /// On classifier or parse failure, falls back to `Default` (action = None).
    pub async fn classify(&self, user_turn: &str, context: &str) -> Result<WhatsappIntent> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let yesterday = (chrono::Utc::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();

        let context_block = if context.is_empty() {
            String::new()
        } else {
            format!(
                "== Predchádzajúci kontext (vyriešiť zámená / coreference) ==\n\
                 {context}\n\
                 == Koniec kontextu ==\n\n\
                 Ak správa používa zámená (SK: jemu/jej/od neho/od nej/ho/ju; \
                 EN: him/her/them/it), vyriešiť na entitu z kontextu.\n\n"
            )
        };

        let prompt = format!(
            r#"{context_block}Dnešný dátum: {today}. Včera: {yesterday}.

Správa používateľa: "{user_turn}"

Rozhodni, či táto správa súvisí s WhatsApp správami, kontaktmi alebo chatmi.
Odpoveď IBA JSON, bez markdown, bez vysvetlenia:
{{
  "action": "none|list_recent|search|read_history|draft_send",
  "contact_name": null,
  "phone": null,
  "chat_id": null,
  "keywords": [],
  "date": null,
  "message_text": null,
  "limit": null
}}

Pravidlá:
- action="none": správa nesúvisí s WhatsApp.
- action="list_recent": používateľ chce vidieť posledné chaty / správy bez konkrétneho hľadania.
  SK: "posledné správy", "čo mi písali", "zoznam chatov".
  EN: "recent messages", "latest chats", "what did people write".
- action="search": hľadáme správy podľa kľúčových slov, dátumu alebo odosielateľa.
  SK: "kde mi písal o faktúre", "nájdi správu od Petra".
  EN: "find message about invoice", "search WhatsApp".
- action="read_history": prečítaj históriu chatu s konkrétnym kontaktom.
  SK: "čo mi písal Peter", "čo sme si písali s Katkou".
  EN: "what did Peter write", "show chat history with", "what did I talk about with Slavka",
  "latest message with Slavka", "latest text with Slavka", "most recent message with Slavka".
- action="draft_send": používateľ chce poslať správu.
  SK: "napíš Petrovi", "pošli mu správu", "odpovedz na WhatsApp".
  EN: "send WhatsApp to", "write to Peter on WhatsApp".
- contact_name: meno osoby (SK aj EN, zachovaj originálnu formu).
- Ak kontext obsahuje [LastFoundWhatsapp] a používateľ sa pýta na "last messages",
  "what did I talk about with X", "čo sme si písali s X" alebo podobné pokračovanie,
  klasifikuj ako read_history aj bez explicitného slova WhatsApp.
- Ak používateľ opraví kontakt po WhatsApp odpovedi, napr. "I have a chat with Slávka Múčková",
  klasifikuj ako read_history a nastav contact_name na uvedené meno.
- keywords: slová na hľadanie v obsahu správ (napr. ["faktúra", "platba"]).
- date: dátum v ISO YYYY-MM-DD (SK "dnes"→{today}, "včera"→{yesterday}, "10.6.2026"→"2026-06-10").
- message_text: PRESNÝ text na odoslanie (len pre draft_send, neinventuj obsah).
- Nikdy neinventuj meno kontaktu ani text správy.
- Ak si nie si istý, vráť action="none"."#
        );

        // Use generate_json for enforced JSON output (like file_intent.rs)
        let raw = self
            .ollama
            .generate_json(&self.model, &prompt, 0.0)
            .await
            .unwrap_or_default();

        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }
}
