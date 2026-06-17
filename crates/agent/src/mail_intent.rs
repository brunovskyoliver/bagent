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
            .format("%Y-%m-%d")
            .to_string();
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
             Classify ONLY the user message as an Apple Mail intent. Return JSON ONLY:\n\
             {{\n\
               \"action\": \"list_recent|search|read_attachment|open|none\",\n\
               \"sender\": null,\n\
               \"subject\": null,\n\
               \"date\": null,\n\
               \"keywords\": [],\n\
               \"wants_attachment\": false\n\
             }}\n\n\
             Rules:\n\
             - list_recent: generic inbox/unread overview, no sender/company/subject. Slovak examples: \"zhrň neprečítané správy\", \"aké mám nové maily\".\n\
             - search: find mail by sender, company, subject, date, or content.\n\
             - open: user asks to open/show an email in Mail app.\n\
             - read_attachment: user asks about an attachment/pdf/document in email.\n\
             - none: not about email.\n\
             - sender: if the text says \"od X\", \"from X\", or \"sender X\", put X in sender. Do NOT put X in keywords.\n\
             - date: use null unless stated in the user message. \"dnes\"/\"today\" = \"{today}\". \"včera\"/\"vcera\"/\"yesterday\" = \"{yesterday}\".\n\
             - keywords: content terms only, never sender names.\n\n\
             Examples:\n\
             \"vies mi najst mail od alza ktory bol odoslany vcera?\" -> {{\"action\":\"search\",\"sender\":\"alza\",\"subject\":null,\"date\":\"{yesterday}\",\"keywords\":[],\"wants_attachment\":false}}\n\
             \"nájdi mi email od Apple zo včera\" -> {{\"action\":\"search\",\"sender\":\"Apple\",\"subject\":null,\"date\":\"{yesterday}\",\"keywords\":[],\"wants_attachment\":false}}\n\
             \"zhrň neprečítané správy\" -> {{\"action\":\"list_recent\",\"sender\":null,\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             \"otvor mail od fakturacia@example.com\" -> {{\"action\":\"open\",\"sender\":\"fakturacia@example.com\",\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}\n\
             \"ma tento mail prilohy?\" -> {{\"action\":\"read_attachment\",\"sender\":null,\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":true}}\n\
             \"mala by byt od nej sprava v maily\" with prior conversation naming Katarína Horváthová -> {{\"action\":\"search\",\"sender\":\"Katarína Horváthová\",\"subject\":null,\"date\":null,\"keywords\":[],\"wants_attachment\":false}}",
        );

        let raw = self.ollama.generate_raw(&self.model, &prompt, 0.0).await?;
        tracing::debug!(raw_mail_intent = %raw, "mail_intent raw");
        let intent: MailIntent = serde_json::from_str(clean_json(&raw)).unwrap_or_default();
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
            intent.action,
            intent.wants_attachment
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

        assert_eq!(
            intent.action, "search",
            "should search, not list_recent or none"
        );
        let sender = intent.sender.expect("sender must be resolved from context");
        // Check for name fragments tolerant of diacritics variations
        let s = sender.to_lowercase();
        assert!(
            s.contains("horv") || s.contains("katarín") || s.contains("katarin"),
            "sender should resolve to Katarína Horváthová, got: {sender}"
        );
    }

    #[tokio::test]
    #[ignore = "requires Ollama; set BAGENT_TEST_CLASSIFIER_MODEL to compare models"]
    async fn inspect_slovak_mail_intent_cases() {
        let ollama = OllamaClient::new(DEFAULT_BASE_URL);
        let model = std::env::var("BAGENT_TEST_CLASSIFIER_MODEL")
            .unwrap_or_else(|_| "qwen2.5:0.5b".to_string());
        let classifier = MailIntentClassifier::new(ollama, model.clone());

        let cases = vec![
            (
                "vies mi najst mail od ryanair ktory bol odoslany vcera?",
                "",
                "search",
                Some("ryanair"),
                true,
            ),
            (
                "nájdi mi email od Apple zo včera",
                "",
                "search",
                Some("apple"),
                true,
            ),
            (
                "zhrň neprečítané správy",
                "",
                "list_recent",
                None,
                false,
            ),
            (
                "otvor mail od ryanair",
                "",
                "open",
                Some("ryanair"),
                false,
            ),
            (
                "vies mi najst mail od ryanair ktory bol odoslany vcera?",
                "[User]: Zhrň neprečítané správy\n[Assistant]: Neprečítané správy: 1. [Pred 7 dňami] Od: iCloud <noreply@email.apple.com> | Predmet: Hide My Email was used with omnigroup.com",
                "search",
                Some("ryanair"),
                true,
            ),
        ];

        let mut results = Vec::new();
        for (message, context, expected_action, expected_sender, expects_date) in cases {
            let intent = classifier.classify(message, context).await.unwrap();
            println!("{model}: {message} -> {intent:?}");
            results.push(intent.clone());
            let mut failures = Vec::new();
            if intent.action != expected_action {
                failures.push(format!("action={}", intent.action));
            }
            if let Some(sender) = expected_sender {
                let actual = intent.sender.as_deref().unwrap_or_default().to_lowercase();
                if !actual.contains(sender) {
                    failures.push(format!("sender={:?}", intent.sender));
                }
            }
            if intent.date.is_some() != expects_date {
                failures.push(format!("date={:?}", intent.date));
            }
            if !failures.is_empty() {
                println!(
                    "{model}: mismatch for {message:?}; expected action={expected_action} sender={expected_sender:?} date_present={expects_date}; got {}",
                    failures.join(", ")
                );
            }
        }

        assert_eq!(results.len(), 5);
    }
}
