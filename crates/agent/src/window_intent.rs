use anyhow::Result;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

/// LLM-classified intent for an AeroSpace window-management user turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowIntent {
    /// "focus_workspace" | "open_app" | "move_app" | "focus_app" | "none"
    pub action: String,
    /// Target workspace number or name (e.g. "3", "main").
    /// `None` means "current workspace" (no move needed).
    pub workspace: Option<String>,
    /// Application name if the user specified one (e.g. "Mail", "Safari").
    pub app: Option<String>,
}

impl Default for WindowIntent {
    fn default() -> Self {
        Self {
            action: "none".to_string(),
            workspace: None,
            app: None,
        }
    }
}

pub struct WindowIntentClassifier {
    ollama: OllamaClient,
    model: String,
}

impl WindowIntentClassifier {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Classify whether the user is asking AeroSpace to manage windows / workspaces.
    ///
    /// `context` is a short snippet of recent conversation turns. Pass empty string
    /// when there is no prior history. Used to resolve app/workspace references from
    /// prior turns (e.g. "presun ho na plochu 2" → "ho" = last mentioned app).
    ///
    /// Returns `WindowIntent { action: "none", .. }` when the turn is not about
    /// window management, so callers can cheaply bail out.
    pub async fn classify(&self, user_turn: &str, context: &str) -> Result<WindowIntent> {
        let context_block = if context.is_empty() {
            String::new()
        } else {
            format!(
                "== Conversation context (resolve references from this) ==\n\
                 {context}\n\
                 == End context ==\n\n\
                 If the current message uses pronouns or references (SK: ho/ju/ten/tú; EN: it/that app), \
                 resolve them to the entity named in the context above.\n\n"
            )
        };
        let prompt = format!(
            "{context_block}User message: \"{user_turn}\"\n\n\
             Decide whether this message asks to control windows or workspaces with AeroSpace window manager.\n\
             Respond with JSON ONLY, no markdown, no explanation:\n\
             {{\n\
               \"action\": \"focus_workspace|open_app|move_app|focus_app|none\",\n\
               \"workspace\": null,\n\
               \"app\": null\n\
             }}\n\n\
             Rules:\n\
             - action=\"focus_workspace\": switch to / focus a workspace. \
               SK: \"prepni na plochu\", \"choď na plochu\", \"plocha 3\". \
               EN: \"switch to workspace\", \"go to workspace\", \"focus workspace\".\n\
             - action=\"open_app\": open an application, optionally on a workspace. \
               SK: \"otvor [app] na ploche\", \"spusti\". EN: \"open [app] on workspace\".\n\
             - action=\"move_app\": move an app's window to another workspace. \
               SK: \"presuň\", \"presun okno\". EN: \"move to workspace\", \"send to workspace\".\n\
             - action=\"focus_app\": switch focus to an already-running app (no workspace change). \
               SK: \"zameraj na\", \"prepni na [app]\". EN: \"focus\", \"switch to [app]\".\n\
             - action=\"none\": no AeroSpace / window / workspace action requested.\n\
             - workspace: the number or name if stated (e.g. \"3\", \"main\"). null if not mentioned.\n\
             - app: the application display name if mentioned (e.g. \"Mail\", \"Safari\", \"Terminal\"). \
               null if not specified.\n\
             - Slovak workspace synonyms: \"plocha\", \"plôšku\", \"pracovná plocha\", \"workspace\".",
        );

        let raw = self.ollama.generate_raw(&self.model, &prompt, 0.0).await?;
        let intent: WindowIntent = serde_json::from_str(clean_json(&raw))
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
