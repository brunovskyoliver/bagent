use anyhow::Result;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

/// LLM-classified intent for a screen-context user turn.
///
/// The classifier decides whether the user wants the agent to look at, read,
/// analyse, or find something on the current screen — and which context sources
/// are useful (full-frame screenshot, on-device OCR, or the Accessibility
/// selected-text).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenIntent {
    pub action: ScreenAction,
    /// True when the agent should capture and send a screenshot frame.
    #[serde(default)]
    pub wants_screen: bool,
    /// True when on-device OCR output should be sent alongside the screenshot.
    #[serde(default)]
    pub wants_ocr: bool,
    /// True when the AX selected-text should be included (cheaper than a screenshot).
    #[serde(default)]
    pub wants_selection: bool,
}

impl Default for ScreenIntent {
    fn default() -> Self {
        Self {
            action: ScreenAction::None,
            wants_screen: false,
            wants_ocr: false,
            wants_selection: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScreenAction {
    /// Not a screen-context turn — skip capture.
    #[default]
    None,
    /// User wants to see / know what is displayed on screen.
    View,
    /// User wants to analyse something visible on screen (chart, UI, document).
    Analyze,
    /// User wants to read text that appears on screen (article, error message, code).
    Read,
    /// User wants the agent to locate something specific on screen.
    Find,
}

pub struct ScreenIntentClassifier {
    ollama: OllamaClient,
    model: String,
}

impl ScreenIntentClassifier {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Classify whether the user wants the agent to access the current screen.
    ///
    /// Returns `ScreenIntent { action: None, .. }` when the turn is unrelated to the screen.
    pub async fn classify(&self, user_turn: &str, context: &str) -> Result<ScreenIntent> {
        let context_block = if context.is_empty() {
            String::new()
        } else {
            format!("== Predchádzajúci kontext ==\n{context}\n== Koniec kontextu ==\n\n")
        };

        let prompt = format!(
            r#"{context_block}Správa používateľa: "{user_turn}"

Rozhodni, či táto správa žiada agenta o prístup k aktuálnej obrazovke používateľa
(screenshot, čítanie textu z obrazovky, analýza UI/obsahu, hľadanie niečoho na obrazovke).

Odpoveď IBA JSON, bez markdown, bez vysvetlenia:
{{
  "action": "none|view|analyze|read|find",
  "wants_screen": false,
  "wants_ocr": false,
  "wants_selection": false
}}

Pravidlá:
- action="none": správa nesúvisí s obrazovkou. Vráť all false.
- action="view": chce vedieť čo je na obrazovke / vidieť stav. wants_screen=true.
  SK: "čo je na obrazovke", "čo vidíš", "pozri na obrazovku", "čo sa zobrazuje".
  EN: "what's on screen", "what do you see", "look at my screen", "what's displayed".
- action="analyze": chce analýzu viditeľného obsahu (graf, UI, dokument, kód).
  wants_screen=true, wants_ocr=true.
  SK: "analyzuj toto", "čo to znamená", "vysvetli to čo vidíš".
  EN: "analyze this", "what does this mean", "explain what you see".
- action="read": chce prečítať text z obrazovky (článok, chybová hláška, kód).
  wants_screen=true, wants_ocr=true.
  SK: "prečítaj to", "prečítaj toto", "prečítaj obrazovku", "čo tam píše".
  EN: "read this", "read the screen", "what does it say".
- action="find": hľadá niečo konkrétne na obrazovke.
  wants_screen=true, wants_ocr=true.
  SK: "nájdi na obrazovke", "kde je tlačidlo", "nájdi toto pole".
  EN: "find on screen", "where is the button", "locate this field".
- wants_selection=true iba keď chce prečítať/analyzovať VYBRATÝ text (SK: "prečítaj výber",
  "tento výber", "vybraný text"; EN: "read selection", "selected text", "this selection").
  Môže byť true aj keď wants_screen=true (oboje spolu).

Príklady:
- "čo je na obrazovke?" → {{"action":"view","wants_screen":true,"wants_ocr":false,"wants_selection":false}}
- "analyzuj tento graf" → {{"action":"analyze","wants_screen":true,"wants_ocr":true,"wants_selection":false}}
- "prečítaj tento článok" → {{"action":"read","wants_screen":true,"wants_ocr":true,"wants_selection":false}}
- "prečítaj výber" → {{"action":"read","wants_screen":false,"wants_ocr":false,"wants_selection":true}}
- "what's on my screen?" → {{"action":"view","wants_screen":true,"wants_ocr":false,"wants_selection":false}}
- "read this error message" → {{"action":"read","wants_screen":true,"wants_ocr":true,"wants_selection":false}}
- "find the submit button" → {{"action":"find","wants_screen":true,"wants_ocr":true,"wants_selection":false}}
- "nájdi email" → {{"action":"none","wants_screen":false,"wants_ocr":false,"wants_selection":false}}
- "write a business email" → {{"action":"none","wants_screen":false,"wants_ocr":false,"wants_selection":false}}"#
        );

        let raw = self.ollama.generate_json(&self.model, &prompt, 0.0).await?;
        let intent: ScreenIntent = serde_json::from_str(clean_json(&raw)).unwrap_or_default();
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

    fn parse(json: &str) -> ScreenIntent {
        serde_json::from_str(json).unwrap_or_default()
    }

    #[test]
    fn default_action_is_none() {
        let i = ScreenIntent::default();
        assert_eq!(i.action, ScreenAction::None);
        assert!(!i.wants_screen);
        assert!(!i.wants_ocr);
        assert!(!i.wants_selection);
    }

    #[test]
    fn parses_view_action() {
        let i = parse(
            r#"{"action":"view","wants_screen":true,"wants_ocr":false,"wants_selection":false}"#,
        );
        assert_eq!(i.action, ScreenAction::View);
        assert!(i.wants_screen);
        assert!(!i.wants_ocr);
    }

    #[test]
    fn parses_read_with_selection() {
        let i = parse(
            r#"{"action":"read","wants_screen":false,"wants_ocr":false,"wants_selection":true}"#,
        );
        assert_eq!(i.action, ScreenAction::Read);
        assert!(!i.wants_screen);
        assert!(i.wants_selection);
    }

    #[test]
    fn parses_none_for_unrelated() {
        // "nájdi email" should parse as None (not a screen turn)
        let i = parse(
            r#"{"action":"none","wants_screen":false,"wants_ocr":false,"wants_selection":false}"#,
        );
        assert_eq!(i.action, ScreenAction::None);
    }

    #[test]
    fn bad_json_falls_back_to_default() {
        let i = parse("not json at all");
        assert_eq!(i.action, ScreenAction::None);
    }
}
