use anyhow::Result;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

/// LLM-classified intent for a local-filesystem user turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIntent {
    pub action: FileAction,
    /// Search query (for Search/Read actions).
    pub query: Option<String>,
    /// Explicit file/folder path if stated by the user.
    pub path: Option<String>,
    /// Explicit root directories if stated (e.g. "v dokumentoch").
    pub roots: Option<Vec<String>>,
    /// App name if user asked to open with a specific app.
    pub app: Option<String>,
    /// File extension filters if stated (e.g. ["pdf", "txt"]).
    pub extensions: Option<Vec<String>>,
    #[serde(default)]
    pub wants_content_search: bool,
    #[serde(default = "default_true")]
    pub wants_filename_search: bool,
    #[serde(default)]
    pub include_hidden: bool,
    pub max_results: Option<usize>,
}

fn default_true() -> bool {
    true
}

impl Default for FileIntent {
    fn default() -> Self {
        Self {
            action: FileAction::None,
            query: None,
            path: None,
            roots: None,
            app: None,
            extensions: None,
            wants_content_search: false,
            wants_filename_search: true,
            include_hidden: false,
            max_results: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FileAction {
    #[default]
    None,
    Search,
    Read,
    Open,
    OpenWith,
    Reveal,
    OpenFolder,
    OpenApp,
    FocusApp,
}

pub struct FileIntentClassifier {
    ollama: OllamaClient,
    model: String,
}

impl FileIntentClassifier {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Classify whether the user is asking about local files/apps.
    ///
    /// `context` is a short snippet of recent conversation turns including a
    /// `[LastFoundFile]` line when available. Pass empty string when no history.
    ///
    /// Returns `FileIntent { action: None, .. }` when the turn is not file-related.
    pub async fn classify(&self, user_turn: &str, context: &str) -> Result<FileIntent> {
        let context_block = if context.is_empty() {
            String::new()
        } else {
            format!(
                "== Predchádzajúci kontext (vyriešiť z neho zámená / references) ==\n\
                 {context}\n\
                 == Koniec kontextu ==\n\n\
                 Ak aktuálna správa používa zámená alebo odkazuje na niečo (SK: ho/ju/ten súbor/tú \
                 zmluvu; EN: it/that file/that document), vyriešiť to na entitu z kontextu.\n\n"
            )
        };

        let prompt = format!(
            r#"{context_block}Správa používateľa: "{user_turn}"

Rozhodni, či táto správa súvisí s lokálnymi súbormi, priečinkami alebo spúšťaním aplikácií.
Odpoveď IBA JSON, bez markdown, bez vysvetlenia:
{{
  "action": "none|search|read|open|open_with|reveal|open_folder|open_app|focus_app",
  "query": null,
  "path": null,
  "roots": null,
  "app": null,
  "extensions": null,
  "wants_content_search": false,
  "wants_filename_search": true,
  "include_hidden": false,
  "max_results": null
}}

Pravidlá:
- action="none": správa nesúvisí so súbormi ani aplikáciami.
- action="search": hľadaj súbory podľa názvu alebo obsahu.
  SK: "nájdi", "vyhľadaj", "kde mám", "kde je", "hľadaj".
  EN: "find", "search", "look for", "where is".
- action="read": prečítaj obsah konkrétneho súboru.
  SK: "prečítaj", "zobraz obsah", "čo je v". EN: "read", "show content of", "what's in".
- action="open": otvor súbor/priečinok predvolenou aplikáciou.
  SK: "otvor", "spusti súbor". EN: "open", "launch file".
- action="open_with": otvor súbor konkrétnou aplikáciou.
  SK: "otvor v", "otvor to v Preview/Exceli". EN: "open in", "open with Preview/Excel".
- action="reveal": ukáž súbor vo Finderi.
  SK: "ukáž vo Finderi", "odhali vo Finderi". EN: "reveal in Finder", "show in Finder".
- action="open_folder": otvor priečinok.
  SK: "otvor priečinok", "otvor adresár". EN: "open folder", "open directory".
- action="open_app": spusti aplikáciu.
  SK: "otvor aplikáciu", "spusti", "otvor Mail/Preview/Excel". EN: "open Mail", "launch Excel".
- action="focus_app": prepni na aplikáciu (už beží).
  SK: "prepni na", "zameraj na aplikáciu". EN: "focus", "switch to app".

- query: čo hľadáme (kľúčové slovo, fráza, názov súboru). null pre open/reveal/app akcie.
- roots: priečinky na prehľadanie ak sú uvedené: "~/Documents", "~/Downloads" atď. null = všetky.
- app: názov aplikácie (napr. "Preview", "Microsoft Excel", "Mail"). null ak nie je uvedená.
- extensions: zoznam prípon bez bodky ["pdf","txt"]. null ak nie je uvedené.
- wants_content_search: true ak chce hľadať vo vnútri súborov (obsah/text).
- wants_filename_search: true (default) ak chce hľadať podľa názvu súboru.
- include_hidden: false (default). true len ak user explicitne chce hidden súbory.

Príklady:
- "nájdi faktúru s DPH" → {{"action":"search","query":"faktúra DPH","wants_content_search":true,"wants_filename_search":true}}
- "vyhľadaj v dokumentoch slovo zmluva" → {{"action":"search","query":"zmluva","roots":["~/Documents"],"wants_content_search":true,"wants_filename_search":false}}
- "kde mám zmluvu s TENENET?" → {{"action":"search","query":"TENENET zmluva","wants_content_search":true,"wants_filename_search":true}}
- "nájdi PDF obsahujúce IBAN" → {{"action":"search","query":"IBAN","extensions":["pdf"],"wants_content_search":true,"wants_filename_search":true}}
- "open that PDF in Preview" → {{"action":"open_with","app":"Preview"}}
- "ukáž vo Finderi" → {{"action":"reveal"}}
- "otvor priečinok s faktúrou" → {{"action":"open_folder"}}
- "open Mail" → {{"action":"open_app","app":"Mail"}}
- "otvor ho" → {{"action":"open"}}
- "prepni na Finder" → {{"action":"focus_app","app":"Finder"}}
- "find files containing splatnosť" → {{"action":"search","query":"splatnosť","wants_content_search":true,"wants_filename_search":false}}
- "search Documents for DPH" → {{"action":"search","query":"DPH","roots":["~/Documents"],"wants_content_search":true,"wants_filename_search":true}}"#
        );

        let raw = self.ollama.generate_json(&self.model, &prompt, 0.0).await?;
        let intent: FileIntent = serde_json::from_str(clean_json(&raw)).unwrap_or_default();
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> FileIntent {
        serde_json::from_str(json).unwrap_or_default()
    }

    #[test]
    fn default_action_is_none() {
        let i = FileIntent::default();
        assert_eq!(i.action, FileAction::None);
    }

    #[test]
    fn parses_search_action() {
        let i = parse(
            r#"{"action":"search","query":"DPH","wants_content_search":true,"wants_filename_search":true}"#,
        );
        assert_eq!(i.action, FileAction::Search);
        assert_eq!(i.query.as_deref(), Some("DPH"));
        assert!(i.wants_content_search);
        assert!(i.wants_filename_search);
    }

    #[test]
    fn parses_search_with_roots() {
        let i = parse(
            r#"{"action":"search","query":"zmluva","roots":["~/Documents"],"wants_content_search":true,"wants_filename_search":false}"#,
        );
        assert_eq!(i.action, FileAction::Search);
        assert_eq!(
            i.roots.as_deref(),
            Some(["~/Documents".to_string()].as_slice())
        );
        assert!(i.wants_content_search);
        assert!(!i.wants_filename_search);
    }

    #[test]
    fn parses_open_with_app() {
        let i = parse(r#"{"action":"open_with","app":"Preview"}"#);
        assert_eq!(i.action, FileAction::OpenWith);
        assert_eq!(i.app.as_deref(), Some("Preview"));
    }

    #[test]
    fn parses_reveal() {
        let i = parse(r#"{"action":"reveal"}"#);
        assert_eq!(i.action, FileAction::Reveal);
    }

    #[test]
    fn parses_open_app() {
        let i = parse(r#"{"action":"open_app","app":"Mail"}"#);
        assert_eq!(i.action, FileAction::OpenApp);
        assert_eq!(i.app.as_deref(), Some("Mail"));
    }

    #[test]
    fn parses_focus_app() {
        let i = parse(r#"{"action":"focus_app","app":"Finder"}"#);
        assert_eq!(i.action, FileAction::FocusApp);
        assert_eq!(i.app.as_deref(), Some("Finder"));
    }

    #[test]
    fn falls_back_to_default_on_bad_json() {
        let i = parse("not valid json");
        assert_eq!(i.action, FileAction::None);
    }

    #[test]
    fn parses_extensions_filter() {
        let i = parse(
            r#"{"action":"search","query":"IBAN","extensions":["pdf"],"wants_content_search":true,"wants_filename_search":true}"#,
        );
        assert_eq!(
            i.extensions.as_deref(),
            Some(["pdf".to_string()].as_slice())
        );
    }

    #[test]
    #[ignore = "requires Ollama + classifier model"]
    fn live_classify_sk_search() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ollama = OllamaClient::new("http://127.0.0.1:11434");
            let clf = FileIntentClassifier::new(ollama, "qwen2.5:0.5b".to_string());
            let intent = clf.classify("nájdi faktúru s DPH", "").await.unwrap();
            assert_eq!(intent.action, FileAction::Search);
        });
    }

    #[test]
    #[ignore = "requires Ollama + classifier model"]
    fn live_classify_reveal() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ollama = OllamaClient::new("http://127.0.0.1:11434");
            let clf = FileIntentClassifier::new(ollama, "qwen2.5:0.5b".to_string());
            let intent = clf.classify("ukáž vo Finderi", "").await.unwrap();
            assert_eq!(intent.action, FileAction::Reveal);
        });
    }

    #[test]
    #[ignore = "requires Ollama + classifier model"]
    fn live_classify_open_app() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ollama = OllamaClient::new("http://127.0.0.1:11434");
            let clf = FileIntentClassifier::new(ollama, "qwen2.5:0.5b".to_string());
            let intent = clf.classify("open Mail", "").await.unwrap();
            assert_eq!(intent.action, FileAction::OpenApp);
            assert_eq!(intent.app.as_deref(), Some("Mail"));
        });
    }

    #[test]
    #[ignore = "requires Ollama + classifier model"]
    fn live_classify_coreference_open_it() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ollama = OllamaClient::new("http://127.0.0.1:11434");
            let clf = FileIntentClassifier::new(ollama, "qwen2.5:0.5b".to_string());
            let context = "[LastFoundFile]: /Users/me/Documents/faktura.pdf\n[User]: nájdi faktúru";
            let intent = clf.classify("otvor ho", context).await.unwrap();
            assert_eq!(intent.action, FileAction::Open);
        });
    }
}
