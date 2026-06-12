use anyhow::Result;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

/// LLM-classified intent for a mail-related user turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailIntent {
    /// "list_recent" | "search" | "read_attachment" | "open" | "none"
    pub action: String,
    /// Sender email or display-name fragment to search for.
    pub sender: Option<String>,
    /// Subject keywords to search for.
    pub subject: Option<String>,
    /// Date in ISO format "YYYY-MM-DD". Model normalises Slovak "DD.MM.YYYY".
    pub date: Option<String>,
    /// Body content keywords (best-effort on locally-cached messages only).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// True when the user wants to read or analyse an email attachment.
    pub wants_attachment: bool,
}

impl Default for MailIntent {
    fn default() -> Self {
        Self {
            action: "none".to_string(),
            sender: None,
            subject: None,
            date: None,
            keywords: vec![],
            wants_attachment: false,
        }
    }
}

pub struct MailIntentClassifier {
    ollama: OllamaClient,
    model: String,
}

impl MailIntentClassifier {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Classify the user turn into a structured mail search intent.
    ///
    /// `context` is a short snippet of recent conversation turns (last ~4)
    /// formatted as `[User]: ...\n[Assistant]: ...\n`. Pass empty string when
    /// there is no prior history. The classifier uses it to resolve pronouns
    /// like "nej/jej/ho/tej firmy" back to entities mentioned earlier.
    ///
    /// Returns `MailIntent { action: "none", .. }` when the turn is not
    /// about reading / searching mail, so callers can cheaply bail out.
    pub async fn classify(&self, user_turn: &str, context: &str) -> Result<MailIntent> {
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let yesterday = (chrono::Utc::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d").to_string();
        let context_block = if context.is_empty() {
            String::new()
        } else {
            format!(
                "== PRIOR CONVERSATION (for pronoun resolution ONLY — do not classify this) ==\n\
                 {context}\n\
                 == END PRIOR CONVERSATION ==\n\n\
                 STEP 1 — Coreference resolution (mandatory):\n\
                 Before classifying, replace every pronoun or vague reference in the CURRENT MESSAGE with \
                 the concrete entity from the prior conversation above.\n\
                 SK pronouns to resolve: \"nej\" (her), \"jej\" (her/hers), \"ho\" (him/it), \"od neho\" (from him), \
                 \"od nej\" (from her), \"tej firmy\" (that company), \"toho emailu\" (that email), \
                 \"ten mail\" (that mail), \"tej osoby\" (that person).\n\
                 EN pronouns to resolve: \"her\", \"him\", \"it\", \"that person\", \"that company\", \"that email\", \"them\".\n\
                 Example: prior context mentions \"Katarína Horváthová\"; current message says \"od nej správa\" \
                 → treat it as \"správa od Katarína Horváthová\" → set sender=\"Katarína Horváthová\".\n\n\
                 STEP 2 — Classify the resolved message (as if the pronoun was never there).\n\n"
            )
        };
        let prompt = format!(
            "{context_block}User message: \"{user_turn}\"\n\n\
             Decide whether this message is asking to read or search Apple Mail.\n\
             Respond with JSON ONLY, no markdown, no explanation:\n\
             {{\n\
               \"action\": \"list_recent|search|read_attachment|open|none\",\n\
               \"sender\": null,\n\
               \"subject\": null,\n\
               \"date\": null,\n\
               \"keywords\": [],\n\
               \"wants_attachment\": false\n\
             }}\n\n\
             Action rules (CRITICAL — read carefully):\n\
             - action=\"list_recent\": ONLY when user wants a generic inbox overview with NO specific sender, company, subject, or content mentioned. Examples: \"show me my inbox\", \"aké mám maily\", \"čo mám nové\".\n\
             - action=\"search\": whenever ANY of the following is present: a sender name, company name, email address, subject keywords, content keywords, or attachment mention — even if the user also says \"recent\" or \"new\" or \"posledné\". Examples: \"recent mails from ryanair\" → search. \"nové emaily od Petra\" → search. \"what did apple send me\" → search.\n\
             - action=\"read_attachment\": user wants to read, analyse, or find a file attached to a mail.\n\
             - action=\"open\": user explicitly asks to open or show a specific email in the Mail app. SK: \"otvor\", \"otvoriť\", \"ukáž mi ten mail\", \"zobraz mail\". EN: \"open it\", \"open the email\", \"show me that email\". Fill sender/subject/keywords if the message also identifies which email.\n\
             - action=\"none\": the message is not about reading mail at all.\n\n\
             Examples:\n\
             - \"do i have any recent mails from ryanair?\" → {{\"action\":\"search\",\"sender\":\"ryanair\",\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             - \"nové maily od Petra\" → {{\"action\":\"search\",\"sender\":\"Peter\",\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             - \"what did apple send me?\" → {{\"action\":\"search\",\"sender\":\"apple\",\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             - \"show me my recent inbox\" → {{\"action\":\"list_recent\",\"sender\":null,\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             - \"akékoľvek nové správy?\" → {{\"action\":\"list_recent\",\"sender\":null,\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             - context mentions \"Katarína Horváthová\"; message \"mala by byt od nej sprava v maily\" → {{\"action\":\"search\",\"sender\":\"Katarína Horváthová\",\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             - context mentions \"firma Acme\"; message \"pozri emaily od nich\" → {{\"action\":\"search\",\"sender\":\"Acme\",\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             - context has [LastFoundMail] and user says \"ma tento mail aj nejake prilohy\" → {{\"action\":\"read_attachment\",\"sender\":null,\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":true}}\n\
             - context has [LastFoundMail] and user says \"co je v tej prilohe\" → {{\"action\":\"read_attachment\",\"sender\":null,\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":true}}\n\n\
             Field rules:\n\
             - date: set ONLY when the user explicitly states a date in the message. If no date is stated, ALWAYS set date to null.\n\
             - Relative dates: \"dnes\"/\"today\" → \"{today}\". \"včera\"/\"vcera\"/\"yesterday\" → \"{yesterday}\".\n\
             - Normalise Slovak dates: \"10.6.2026\" → \"2026-06-10\".\n\
             - Set wants_attachment=true for: príloha, prílohu, attachment, pdf, dokument, precitaj prilohu.\n\
             - keywords: concrete content words only (invoice numbers, amounts, company names, booking codes). Do NOT put sender names in keywords — use the sender field instead.\n\
             - sender: extract any company name, person name, or email domain that identifies who sent the mail. \
               If the user used a pronoun (nej/jej/ho/her/him) that you resolved to a name in STEP 1, put that resolved name here.",
        );

        let raw = self.ollama.generate_raw(&self.model, &prompt, 0.0).await?;
        tracing::debug!(raw_mail_intent = %raw, "mail_intent raw");
        let intent: MailIntent = serde_json::from_str(clean_json(&raw))
            .unwrap_or_default();
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
    use ollama_connector::{OllamaClient, DEFAULT_BASE_URL};

    /// Simulate second screenshot: user found mail, then asks about its attachments.
    /// Classifier must return action=read_attachment, wants_attachment=true,
    /// with NO sender/subject (so the daemon short-circuits to last_mail_ref rowid).
    #[tokio::test]
    #[ignore]
    async fn test_attachment_of_current_mail() {
        let ollama = OllamaClient::new(DEFAULT_BASE_URL);
        let classifier = MailIntentClassifier::new(ollama, "qwen2.5:7b".to_string());

        let context = "[LastFoundMail]: rowid=42 sender=\"katka@tenenet.sk\" subject=\"dochádzky\"\n\
                       [User]: katka@tenenet.sk by mal byt ten mail\n\
                       [Assistant]: Našiel som email: Od: Katarína Horváthová <katka@tenenet.sk>, Predmet: dochádzky";
        let user_turn = "ma tento mail aj nejake prilohy?";

        let intent = classifier.classify(user_turn, context).await.unwrap();
        println!("\n=== MailIntent (attachment of current mail) ===");
        println!("action:           {}", intent.action);
        println!("sender:           {:?}", intent.sender);
        println!("subject:          {:?}", intent.subject);
        println!("wants_attachment: {}", intent.wants_attachment);

        assert!(
            intent.action == "read_attachment" || intent.wants_attachment,
            "should request attachment, got action={} wants_attachment={}",
            intent.action, intent.wants_attachment
        );
        // sender/subject must be empty so the daemon short-circuits via last_mail_ref
        assert!(
            intent.sender.is_none() && intent.subject.is_none(),
            "sender and subject must be null for rowid short-circuit to trigger, got sender={:?} subject={:?}",
            intent.sender, intent.subject
        );
    }

    /// Simulate the screenshot scenario:
    ///   Turn 1: user asks "co vies o katarine horvathovej"
    ///   Turn 2: user says "mala by byt od nej sprava v maily"
    /// Classifier must resolve "nej" → "Katarína Horváthová" and emit action=search.
    #[tokio::test]
    #[ignore]
    async fn test_coreference_nej_to_sender() {
        let ollama = OllamaClient::new(DEFAULT_BASE_URL);
        let model = "qwen2.5:7b".to_string();
        let classifier = MailIntentClassifier::new(ollama, model);

        let context = "[User]: co vies o katarine horvathovej\n\
                       [Assistant]: V čosikmajšom kontexte nemám konkrétnych informácií o Katarine Horvathovej. Máte náhodou niečo konkrétnejšie?";
        let user_turn = "mala by byt od nej sprava v maily";

        let intent = classifier.classify(user_turn, context).await.unwrap();
        println!("\n=== MailIntent result ===");
        println!("action:          {}", intent.action);
        println!("sender:          {:?}", intent.sender);
        println!("subject:         {:?}", intent.subject);
        println!("date:            {:?}", intent.date);
        println!("keywords:        {:?}", intent.keywords);
        println!("wants_attachment:{}", intent.wants_attachment);

        assert_eq!(intent.action, "search", "should search, not list_recent or none");
        let sender = intent.sender.expect("sender must be resolved from context");
        // Check for name fragments tolerant of diacritics variations
        let s = sender.to_lowercase();
        assert!(
            s.contains("horv") || s.contains("katarín") || s.contains("katarin"),
            "sender should resolve to Katarína Horváthová, got: {sender}"
        );
    }
}
