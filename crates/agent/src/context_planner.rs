//! ContextPlanner — decides what kind of task a user turn represents and
//! what context (memory namespaces, skills) is needed before building the prompt.
//!
//! Uses deterministic rules first (keyword/diacritic/trigger gates) and falls
//! back to an Ollama JSON classifier only when rules are low-confidence.
//!
//! Design principle: **fail closed** — if uncertain, request fewer memories and
//! fewer skills so the prompt stays small and accurate.

use anyhow::Result;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

use crate::feedback::has_explicit_trigger;

// ── Public types ─────────────────────────────────────────────────────────────

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

impl Default for ResponseLanguageHint {
    fn default() -> Self {
        Self::EnglishDefault
    }
}

/// The output of the planning layer — consumed by SkillSelector, MemorySelector,
/// and PromptBuilder before any LLM call is made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPlan {
    /// Canonical task category.
    pub task_type: String,
    /// Language hint for the assembled prompt.
    pub response_language_hint: ResponseLanguageHint,
    /// Whether memory retrieval should run at all.
    pub needs_memory: bool,
    /// Which memory namespaces are relevant.
    pub memory_namespaces: Vec<String>,
    /// Which memory kinds are relevant.
    pub memory_kinds: Vec<String>,
    /// Whether cross-session conversation recall should be injected.
    pub needs_conversation_recall: bool,
    /// Candidate skill names for SkillSelector to consider.
    pub candidate_skill_names: Vec<String>,
    /// 0.0–1.0 planner confidence for the winning intent.
    pub confidence: f32,
    /// Where the winning intent came from: `llm`, `deterministic`, or `merged`.
    #[serde(default)]
    pub intent_source: String,
    /// Connector selected by the intent router, when the task is connector-backed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_connector: Option<String>,
    /// True when the router thinks the assistant should ask a clarification before acting.
    #[serde(default)]
    pub clarification_needed: bool,
}

impl Default for ContextPlan {
    /// Safe / fail-closed default: no memory, no recall, no skills.
    fn default() -> Self {
        Self {
            task_type: "general".to_string(),
            response_language_hint: ResponseLanguageHint::EnglishDefault,
            needs_memory: false,
            memory_namespaces: vec![],
            memory_kinds: vec![],
            needs_conversation_recall: false,
            candidate_skill_names: vec![],
            confidence: 0.5,
            intent_source: "deterministic".to_string(),
            selected_connector: None,
            clarification_needed: false,
        }
    }
}

/// Extra turn context available to the intent router.
///
/// Kept stringly-typed on purpose: the planner only needs compact hints, not
/// connector data. This avoids coupling the agent crate to daemon connector types.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlannerRuntimeContext {
    pub recent_context: String,
    pub last_connector_refs: Vec<String>,
    pub connector_statuses: Vec<String>,
    pub available_skill_names: Vec<String>,
}

// ── Planner ──────────────────────────────────────────────────────────────────

pub struct ContextPlanner {
    ollama: OllamaClient,
    classifier_model: String,
}

impl ContextPlanner {
    pub fn new(ollama: OllamaClient, classifier_model: String) -> Self {
        Self {
            ollama,
            classifier_model,
        }
    }

    /// Plan context for a single user turn.
    ///
    /// `language`: `"sk"` or `"en"` from the daemon's diacritic heuristic.
    /// `has_mail_ctx`: true when the turn already triggered mail tool approval.
    pub async fn plan(
        &self,
        user_turn: &str,
        language: &str,
        has_mail_ctx: bool,
        runtime: &PlannerRuntimeContext,
    ) -> ContextPlan {
        let det = deterministic_plan(user_turn, language, has_mail_ctx);

        // LLM-first: every normal turn gets a local classifier pass. If it fails
        // or returns a weak/general answer, merge back to deterministic signals.
        match self.llm_plan(user_turn, language, runtime).await {
            Ok(llm_plan) => merge_plans(det, llm_plan),
            Err(_) => det,
        }
    }

    // ── LLM fallback ─────────────────────────────────────────────────────────

    async fn llm_plan(
        &self,
        user_turn: &str,
        language: &str,
        runtime: &PlannerRuntimeContext,
    ) -> Result<LlmPlanResult> {
        let runtime_block = if runtime == &PlannerRuntimeContext::default() {
            String::new()
        } else {
            format!(
                "\n## Runtime context\nRecent context:\n{}\nLast connector refs: {:?}\nConnector statuses: {:?}\nAvailable skills: {:?}\n",
                runtime.recent_context,
                runtime.last_connector_refs,
                runtime.connector_statuses,
                runtime.available_skill_names
            )
        };
        let prompt = format!(
            r#"You are a task classifier for a personal macOS assistant. Classify the user turn below.

User turn: "{user_turn}"
Detected language: {language}
{runtime_block}

## CRITICAL — Odoo vs local file search disambiguation

When the word "odoo" appears in the query, you MUST determine which intent is stronger:

**odoo_lookup** — the user wants data FROM the Odoo ERP system (customers, partners, orders,
invoices, CRM records stored in Odoo). Signals: "open orders", "partners", "customers in Odoo",
"what does Odoo show", "sales order", "CRM".

**file_search** — the user wants a local file that happens to live inside a folder whose name
contains "odoo" (e.g. ~/odoo-dev/, an "odoo development folder" on disk). Signals: "find me",
"search for", "in the folder", "excel", "xlsx", "pdf", "in my files", specific Slovak filename
words (rozúčtovanie, výpis, prehlad...).

Score BOTH intents independently on a 0.0–1.0 scale, then pick the higher-scoring one as task_type.

## Return ONLY a JSON object (no markdown):
{{
  "task_type": "sk_business_email|mail_search|invoice_analysis|odoo_lookup|window_control|file_search|screen_context|whatsapp|explicit_memory|conversation_recall|general",
  "confidence": 0.0,
  "selected_connector": null,
  "clarification_needed": false,
  "intent_scores": {{
    "odoo_lookup": 0.0,
    "file_search": 0.0,
    "whatsapp": 0.0,
    "mail_search": 0.0
  }},
  "needs_memory": true|false,
  "memory_namespaces": ["user_pref","style_profile","sk_glossary","contacts","corrections","negative_rules","global"],
  "needs_conversation_recall": true|false,
  "candidate_skill_names": ["sk-business-email","mail-search","invoice-analysis","odoo-readonly","aerospace-window-control","file-search","file-open","app-open-control","screen-context","whatsapp"],
  "response_language": "english_default|match_user|match_source|slovak_required"
}}

Rules:
- LLM-FIRST routing: infer intent from text, typos, and context. Do not require exact keywords.
- needs_memory=true only for tasks where user preferences, corrections, or glossary would help.
- "what did I say / čo som hovoril / what did we decide" → needs_conversation_recall=true, needs_memory=false.
- Simple facts ("what is 2+2", "what time is it") → needs_memory=false.
- Slovak email drafting/rewriting → sk-business-email + needs_memory=true with user_pref,style_profile,sk_glossary.
- Invoice / DPH / faktúra / splatnosť → invoice-analysis.
- Mail search / inbox / nájdi mail → mail-search.
- WhatsApp / WA / chat history / unread WhatsApp messages / send WhatsApp → whatsapp.
- Common misspellings like "whatspp", "whatsap", and "whatapp" mean WhatsApp.
- "most recent text/message with X", "latest text/message with X", or "last chat with X" means WhatsApp if the user mentions WhatsApp/WA or a close typo.
- If WhatsApp is explicitly mentioned, prefer whatsapp over mail_search even when the turn says "read", "message", "last", or "unread".
- Workspace / window / plocha / AeroSpace → aerospace-window-control.
- Local file search/open/reveal/app launch → file-search, file-open, app-open-control.
- Odoo ERP data lookup → odoo-readonly.
- candidate_skill_names max 3.
- selected_connector: one of "mail", "notes", "odoo", "filesystem", "screen", "whatsapp", or null.
- confidence: your confidence in task_type, 0.0-1.0.
- intent_scores: provide odoo_lookup, file_search, whatsapp, and mail_search scores for every query, even if one is 0.0."#
        );

        let raw = self
            .ollama
            .generate_json(&self.classifier_model, &prompt, 0.0)
            .await?;
        let result: LlmPlanResult = serde_json::from_str(&raw)?;
        Ok(result)
    }
}

// ── Deterministic rules ───────────────────────────────────────────────────────

fn deterministic_plan(user_turn: &str, language: &str, has_mail_ctx: bool) -> ContextPlan {
    let low = user_turn.to_lowercase();
    let is_sk = language == "sk";

    // Explicit recall triggers (check before memory triggers)
    if is_conversation_recall(&low) {
        return ContextPlan {
            task_type: "conversation_recall".to_string(),
            response_language_hint: if is_sk {
                ResponseLanguageHint::MatchUser
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: false,
            memory_namespaces: vec![],
            memory_kinds: vec![],
            needs_conversation_recall: true,
            candidate_skill_names: vec![],
            confidence: 0.9,
            ..Default::default()
        };
    }

    // Explicit memory triggers — store, not retrieve
    if has_explicit_trigger(user_turn) {
        return ContextPlan {
            task_type: "explicit_memory".to_string(),
            response_language_hint: if is_sk {
                ResponseLanguageHint::MatchUser
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: false,
            memory_namespaces: vec![],
            memory_kinds: vec![],
            needs_conversation_recall: false,
            candidate_skill_names: vec![],
            confidence: 0.95,
            ..Default::default()
        };
    }

    // AeroSpace / window control
    if is_window_control(&low) {
        return ContextPlan {
            task_type: "window_control".to_string(),
            response_language_hint: ResponseLanguageHint::EnglishDefault,
            needs_memory: false,
            memory_namespaces: vec![],
            memory_kinds: vec![],
            needs_conversation_recall: false,
            candidate_skill_names: vec!["aerospace-window-control".to_string()],
            confidence: 0.9,
            ..Default::default()
        };
    }

    // Odoo — when file-search signals also present, lower confidence so the LLM
    // disambiguator resolves whether "odoo" is the ERP system or a local folder name.
    if is_odoo(&low) {
        let ambiguous = has_local_file_signals(&low);
        return ContextPlan {
            task_type: "odoo_lookup".to_string(),
            response_language_hint: if is_sk {
                ResponseLanguageHint::MatchUser
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: true,
            memory_namespaces: vec!["user_pref".to_string(), "contacts".to_string()],
            memory_kinds: vec!["preference".to_string(), "contact".to_string()],
            needs_conversation_recall: false,
            candidate_skill_names: vec!["odoo-readonly".to_string()],
            // Ambiguous: drop below 0.6 threshold so llm_plan is invoked to score
            // odoo_lookup vs file_search intents explicitly.
            confidence: if ambiguous { 0.4 } else { 0.85 },
            ..Default::default()
        };
    }

    // Invoice / accounting analysis (check before mail/email — invoices often arrive by mail)
    if is_invoice(&low) {
        let mut namespaces = vec!["user_pref".to_string(), "sk_glossary".to_string()];
        let mut kinds = vec![
            "preference".to_string(),
            "sk_glossary".to_string(),
            "negative_rule".to_string(),
        ];
        if is_sk || has_sk_business_terms(&low) {
            namespaces.push("style_profile".to_string());
            kinds.push("style_profile".to_string());
        }
        return ContextPlan {
            task_type: "invoice_analysis".to_string(),
            response_language_hint: if is_sk || has_sk_business_terms(&low) {
                ResponseLanguageHint::MatchSourceContent
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: true,
            memory_namespaces: namespaces,
            memory_kinds: kinds,
            needs_conversation_recall: false,
            candidate_skill_names: vec!["invoice-analysis".to_string()],
            confidence: 0.88,
            ..Default::default()
        };
    }

    // Slovak business email drafting/rewriting
    if is_sk_business_email(&low, is_sk) {
        return ContextPlan {
            task_type: "sk_business_email".to_string(),
            response_language_hint: ResponseLanguageHint::SlovakRequired,
            needs_memory: true,
            memory_namespaces: vec![
                "user_pref".to_string(),
                "style_profile".to_string(),
                "sk_glossary".to_string(),
                "corrections".to_string(),
                "negative_rules".to_string(),
            ],
            memory_kinds: vec![
                "preference".to_string(),
                "style_profile".to_string(),
                "sk_glossary".to_string(),
                "correction".to_string(),
                "negative_rule".to_string(),
            ],
            needs_conversation_recall: false,
            candidate_skill_names: vec!["sk-business-email".to_string()],
            confidence: 0.9,
            ..Default::default()
        };
    }

    // WhatsApp messaging. Check before mail: generic words like "read",
    // "message", "last", and "unread" often appear in WhatsApp requests too.
    if is_whatsapp(&low) {
        return ContextPlan {
            task_type: "whatsapp".to_string(),
            response_language_hint: if is_sk {
                ResponseLanguageHint::MatchUser
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: true,
            memory_namespaces: vec!["contacts".to_string(), "user_pref".to_string()],
            memory_kinds: vec!["contact".to_string(), "preference".to_string()],
            needs_conversation_recall: false,
            candidate_skill_names: vec!["whatsapp".to_string()],
            confidence: 0.9,
            selected_connector: Some("whatsapp".to_string()),
            ..Default::default()
        };
    }

    // Mail search / open / inbox (including mail contexts already loaded)
    // Check BEFORE file_search so mail-referencing turns ("nájdi mail") aren't
    // accidentally routed to the filesystem planner.
    if is_mail_search(&low) || has_mail_ctx {
        let mut skills = vec!["mail-search".to_string()];
        // Add SK email skill if the source appears Slovak
        if is_sk || has_sk_business_terms(&low) {
            skills.push("sk-business-email".to_string());
        }
        return ContextPlan {
            task_type: "mail_search".to_string(),
            response_language_hint: if is_sk {
                ResponseLanguageHint::MatchUser
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: is_sk || has_sk_business_terms(&low),
            memory_namespaces: if is_sk || has_sk_business_terms(&low) {
                vec![
                    "contacts".to_string(),
                    "user_pref".to_string(),
                    "sk_glossary".to_string(),
                ]
            } else {
                vec!["contacts".to_string()]
            },
            memory_kinds: vec!["contact".to_string(), "preference".to_string()],
            needs_conversation_recall: false,
            candidate_skill_names: skills,
            confidence: 0.85,
            selected_connector: Some("mail".to_string()),
            ..Default::default()
        };
    }

    // Screen-context requests (checked before file_search so "find X on screen" doesn't
    // get routed to the filesystem planner).
    if is_screen_context(&low) {
        return ContextPlan {
            task_type: "screen_context".to_string(),
            response_language_hint: if is_sk {
                ResponseLanguageHint::MatchUser
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: false,
            memory_namespaces: vec![],
            memory_kinds: vec![],
            needs_conversation_recall: false,
            candidate_skill_names: vec!["screen-context".to_string()],
            confidence: 0.9,
            selected_connector: Some("screen".to_string()),
            ..Default::default()
        };
    }

    // Local file / folder / app-open requests (checked after mail so "nájdi mail" doesn't match)
    if is_file_search(&low) {
        return ContextPlan {
            task_type: "file_search".to_string(),
            response_language_hint: if is_sk {
                ResponseLanguageHint::MatchUser
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: false,
            memory_namespaces: vec![],
            memory_kinds: vec![],
            needs_conversation_recall: false,
            candidate_skill_names: vec![
                "file-search".to_string(),
                "file-open".to_string(),
                "app-open-control".to_string(),
            ],
            confidence: 0.88,
            selected_connector: Some("filesystem".to_string()),
            ..Default::default()
        };
    }

    // Simple factual / trivial turn — no memory, no skills
    if is_trivial(&low) {
        return ContextPlan {
            task_type: "general".to_string(),
            response_language_hint: if is_sk {
                ResponseLanguageHint::MatchUser
            } else {
                ResponseLanguageHint::EnglishDefault
            },
            needs_memory: false,
            memory_namespaces: vec![],
            memory_kinds: vec![],
            needs_conversation_recall: false,
            candidate_skill_names: vec![],
            confidence: 0.75,
            ..Default::default()
        };
    }

    // General turn with SK context — load style/pref
    if is_sk {
        return ContextPlan {
            task_type: "general".to_string(),
            response_language_hint: ResponseLanguageHint::MatchUser,
            needs_memory: true,
            memory_namespaces: vec![
                "user_pref".to_string(),
                "style_profile".to_string(),
                "sk_glossary".to_string(),
            ],
            memory_kinds: vec![
                "preference".to_string(),
                "style_profile".to_string(),
                "sk_glossary".to_string(),
            ],
            needs_conversation_recall: false,
            candidate_skill_names: vec![],
            confidence: 0.65,
            ..Default::default()
        };
    }

    // General English turn — low confidence, send to LLM fallback
    ContextPlan {
        task_type: "general".to_string(),
        response_language_hint: ResponseLanguageHint::EnglishDefault,
        needs_memory: true,
        memory_namespaces: vec!["user_pref".to_string(), "global".to_string()],
        memory_kinds: vec!["preference".to_string(), "correction".to_string()],
        needs_conversation_recall: false,
        candidate_skill_names: vec![],
        confidence: 0.5, // triggers LLM fallback
        ..Default::default()
    }
}

// ── Gate helpers ─────────────────────────────────────────────────────────────

fn is_conversation_recall(low: &str) -> bool {
    let triggers = [
        "what did i say",
        "what did we say",
        "what did we discuss",
        "what did we decide",
        "remind me what",
        "do you remember when",
        "čo som hovoril",
        "čo sme hovorili",
        "čo sme riešili",
        "čo sme sa dohodli",
        "čo si hovoril",
        "čo sme rozhodli",
        "what we talked about",
        "previous discussion",
        "remember our conversation",
        "remember we discussed",
    ];
    triggers.iter().any(|t| low.contains(t))
}

fn is_window_control(low: &str) -> bool {
    let kw = [
        "workspace",
        "plocha ",
        "plochu",
        "aerospace",
        "switch to desktop",
        "prepni na",
        "prepni okno",
        "focus ",
        "move window",
        "presun okno",
        "tile ",
        "float ",
        "fullscreen",
        "maximize window",
    ];
    kw.iter().any(|k| low.contains(k))
}

fn is_odoo(low: &str) -> bool {
    ["odoo", "crm", "objednávk", "partner", "zákazník"]
        .iter()
        .any(|k| low.contains(k))
}

fn is_invoice(low: &str) -> bool {
    let kw = [
        "faktúra",
        "faktura",
        "invoice",
        "dph",
        "ičo",
        "dič",
        "iban",
        "splatnosť",
        "splatnost",
        "upomienka",
        "payment",
        "platba",
        "zaúčtovanie",
        "záloha",
        "zaloha",
        "účtovníctvo",
        "accounting",
    ];
    kw.iter().any(|k| low.contains(k))
}

fn is_sk_business_email(low: &str, is_sk: bool) -> bool {
    let email_kw = [
        "napíš",
        "napíšem",
        "odpovedz",
        "odpoveď",
        "odpovez",
        "formuluj",
        "zopakuj",
        "draft",
        "compose",
        "reply to",
        "write a",
        "write an",
        "formal",
        "formálny",
        "formálnu",
    ];
    let business_kw = [
        "email",
        "e-mail",
        "správu",
        "mail",
        "faktúra",
        "faktura",
        "upomienka",
        "zmluva",
        "objednávka",
        "oferta",
    ];
    let has_email_action = email_kw.iter().any(|k| low.contains(k));
    let has_business_target = business_kw.iter().any(|k| low.contains(k));
    (has_email_action && has_business_target)
        || (is_sk && has_email_action && has_sk_business_terms(low))
}

fn is_file_search(low: &str) -> bool {
    let kw = [
        // Slovak — specific enough to not overlap with mail
        "nájdi súbor",
        "nájdi dokument",
        "nájdi faktúr",
        "nájdi zmluv",
        "vyhľadaj v",
        "kde mám súbor",
        "kde je súbor",
        "kde mám zmluv",
        "kde je zmluv",
        "otvor súbor",
        "otvor priečinok",
        "otvor adresár",
        "ukáž vo finderi",
        "odhali vo finderi",
        "otvor to v preview",
        "otvor to v exceli",
        "otvor to v word",
        "spusti aplikáciu",
        "prepni na finder",
        "zameraj na finder",
        "hľadaj súbor",
        // English — specific
        "find file",
        "find document",
        "find invoice",
        "find contract",
        "search files",
        "search documents",
        "search for file",
        "open file",
        "open folder",
        "open directory",
        "reveal in finder",
        "show in finder",
        "open in preview",
        "open in excel",
        "open in word",
        "open with preview",
        "open with excel",
        "open finder",
        "open calendar",
        "launch excel",
        "launch finder",
        "focus finder",
        // Generic file-related keywords
        "find files containing",
        "files containing",
        "documents containing",
    ];
    // Exact keywords didn't match; check for broader "find me X in Y folder" patterns.
    // is_odoo returns before is_file_search is reached, so these patterns are safe to
    // include here — they only fire for queries that don't also contain Odoo signals.
    kw.iter().any(|k| low.contains(k)) || has_local_file_signals(low)
}

/// Returns true when the query contains signals that it may be about a local file/folder,
/// used to detect ambiguity with `is_odoo` so confidence is lowered and the LLM decides.
fn has_local_file_signals(low: &str) -> bool {
    let has_find_verb = low.contains("find me")
        || low.contains("search for")
        || low.contains("look for")
        || low.contains("locate")
        || low.contains("get me")
        || low.contains("nájdi mi");

    let has_file_type = low.contains("excel")
        || low.contains("xlsx")
        || low.contains("pdf")
        || low.contains("docx")
        || low.contains("spreadsheet")
        || low.contains("tabuľka");

    let has_folder_ref = low.contains(" folder")
        || low.contains(" directory")
        || low.contains("priečinok")
        || low.contains("adresár")
        || low.contains("in the ")
        || low.contains("in my ");

    (has_find_verb && has_file_type) || (has_find_verb && has_folder_ref)
}

fn is_screen_context(low: &str) -> bool {
    let kw = [
        // Slovak
        "obrazovk", // obrazovka / obrazovke / obrazovku
        "na obrazovke",
        "vidíš",
        "čo vidíš",
        "pozri na",
        "pozri sem",
        "analyzuj toto",
        "analyzuj to",
        "prečítaj toto", // Note: "prečítaj" also appears in mail_search but mail wins first
        "prečítaj to",
        "čo tam píše",
        "čo sa zobrazuje",
        "nájdi na obrazovke",
        "vyber text",
        "prečítaj výber",
        "vybraný text",
        "tento výber",
        // English
        "what's on screen",
        "what's on my screen",
        "what is on screen",
        "what is on my screen",
        "what can you see",
        "on the screen",
        "look at my screen",
        "look at the screen",
        "what do you see",
        "analyze this", // broad but useful; placed after more specific checks above
        "analyse this",
        "read this",
        "read the screen",
        "what does it say",
        "what does this say",
        "find on screen",
        "find the button",
        "locate on screen",
        "read selection",
        "selected text",
    ];
    kw.iter().any(|k| low.contains(k))
}

fn is_mail_search(low: &str) -> bool {
    if is_whatsapp(low) {
        return false;
    }
    let kw = [
        "email",
        "mail",
        "inbox",
        "schránk",
        "doručen",
        "sender",
        "odosielate",
        "nazvom",
        "názvom",
        "mailbox",
        "prilohu",
        "prílohu",
        "nájdi mail",
        "open mail",
        "otvor mail",
        "show mail",
    ];
    let explicit_mail = kw.iter().any(|k| low.contains(k));
    let generic_read = [
        "posledné email",
        "posledne email",
        "posledné mail",
        "posledne mail",
        "recent email",
        "recent mail",
        "latest email",
        "latest mail",
        "read email",
        "read mail",
        "prečítaj email",
        "prečítaj mail",
        "prečítaj správu v mail",
        "správu v mail",
    ]
    .iter()
    .any(|k| low.contains(k));
    explicit_mail || generic_read
}

fn has_sk_business_terms(low: &str) -> bool {
    let terms = [
        "dph",
        "faktúra",
        "faktura",
        "splatnosť",
        "splatnost",
        "ičo",
        "dič",
        "iban",
        "zmluva",
        "objednávka",
        "upomienka",
        "odberateľ",
        "dodávateľ",
    ];
    terms.iter().any(|t| low.contains(t))
}

fn is_whatsapp(low: &str) -> bool {
    let explicit = [
        "whatsapp",
        "whatspp",
        "whatsap",
        "whatapp",
        " wa ",
        "wa:",
        "na whatsappe",
        "cez whatsapp",
        "na wa",
    ];
    let send_signals = [
        "napíš mu",
        "napíš jej",
        "napíš petrovi",
        "napíš katke",
        "pošli mu správu",
        "pošli jej správu",
        "odpovedz mu",
        "odpovedz jej",
        "write to ",
        "send to ",
        "message ",
    ];
    let find_signals = [
        "kde mi písal",
        "kde mi písala",
        "kde mi písali",
        "čo mi písal",
        "čo mi písala",
        "čo mi písali",
        "čo sme si písali",
        "nájdi správu od",
        "find message from",
        "chat history",
        "last messages with",
        "last message with",
        "latest messages with",
        "latest message with",
        "most recent messages with",
        "most recent message with",
        "most recent text with",
        "latest text with",
        "recent text with",
        "last text with",
        "what did i talk about with",
        "what did we talk about with",
        "what did i discuss with",
    ];
    if explicit.iter().any(|k| low.contains(k)) {
        return true;
    }
    // "pošli mu / napíš mu" without "mail/email" context → WhatsApp send intent
    let has_send = send_signals.iter().any(|k| low.contains(k));
    let has_mail = low.contains("mail") || low.contains("email");
    if has_send && !has_mail {
        return true;
    }
    // "kde mi písal" / "čo mi písal" → WhatsApp history
    if find_signals.iter().any(|k| low.contains(k)) && !has_mail {
        return true;
    }
    false
}

fn is_trivial(low: &str) -> bool {
    // Very short turns or pure arithmetic / knowledge-base questions
    if low.trim().len() < 15 {
        return true;
    }
    let trivial = [
        "what is ",
        "what's ",
        "how many ",
        "who is ",
        "when did ",
        "where is ",
        "define ",
        "translate ",
        "convert ",
        "what time",
        "what day",
        "weather",
    ];
    trivial.iter().any(|t| low.starts_with(t))
}

// ── LLM result + merge ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct LlmPlanResult {
    task_type: String,
    #[serde(default)]
    confidence: Option<f32>,
    #[serde(default)]
    selected_connector: Option<String>,
    #[serde(default)]
    clarification_needed: bool,
    /// Explicit per-intent confidence scores. Present when the LLM detects competing
    /// intents (e.g. odoo_lookup vs file_search). The highest-scoring intent overrides
    /// task_type so the LLM's own scoring is the authoritative decision.
    #[serde(default)]
    intent_scores: std::collections::HashMap<String, f32>,
    needs_memory: bool,
    #[serde(default)]
    memory_namespaces: Vec<String>,
    #[serde(default)]
    needs_conversation_recall: bool,
    #[serde(default)]
    candidate_skill_names: Vec<String>,
    #[serde(default)]
    response_language: String,
}

/// Merge a low-confidence deterministic plan with an LLM refinement.
/// LLM wins on task_type, skills, recall. Deterministic wins on confidence value.
/// When `intent_scores` is present, the highest-scoring intent overrides task_type.
fn merge_plans(det: ContextPlan, llm: LlmPlanResult) -> ContextPlan {
    let llm_confidence = llm.confidence.unwrap_or(0.7).clamp(0.0, 1.0);
    let response_language_hint = match llm.response_language.as_str() {
        "match_user" => ResponseLanguageHint::MatchUser,
        "match_source" => ResponseLanguageHint::MatchSourceContent,
        "slovak_required" => ResponseLanguageHint::SlovakRequired,
        _ => det.response_language_hint.clone(),
    };

    // If the LLM provided intent_scores, the winner by score is authoritative.
    // This handles the odoo-folder vs odoo-ERP ambiguity: the LLM rates both and
    // whichever score is higher becomes the final task_type regardless of what
    // task_type field the LLM also emitted.
    let scored_task_type = llm
        .intent_scores
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .filter(|(_, score)| **score >= 0.55)
        .map(|(k, _)| k.clone())
        .unwrap_or_else(|| llm.task_type.clone());

    // LLM-first, with guardrails: if the LLM is weak/general but deterministic
    // routing found a concrete connector task, keep the concrete task.
    let llm_is_weak_general = scored_task_type == "general" && det.task_type != "general";
    let task_type = if llm_confidence < 0.55 || llm_is_weak_general {
        det.task_type.clone()
    } else {
        scored_task_type
    };

    let memory_namespaces = if llm.memory_namespaces.is_empty() {
        det.memory_namespaces
    } else {
        llm.memory_namespaces
    };

    // Merge kinds: preserve det kinds if LLM didn't provide specifics
    let memory_kinds = if det.memory_kinds.is_empty() {
        default_kinds_for_namespaces(&memory_namespaces)
    } else {
        det.memory_kinds
    };

    // Candidate skills: canonical task skill first, then classifier + deterministic
    // suggestions. This prevents an LLM plan like task_type=whatsapp from keeping
    // stale unrelated skills such as mail-search.
    let mut skills: Vec<String> = default_skills_for_task(&task_type);
    let llm_skills: Vec<String> = if task_type == det.task_type && llm_confidence < 0.55 {
        det.candidate_skill_names.clone()
    } else {
        llm.candidate_skill_names
    };
    for s in llm_skills.iter().chain(det.candidate_skill_names.iter()) {
        if !skill_compatible_with_task(&task_type, s) {
            continue;
        }
        if !skills.contains(s) {
            skills.push(s.clone());
        }
    }
    skills.truncate(3);

    let selected_connector = llm
        .selected_connector
        .or_else(|| infer_connector_for_task(&task_type))
        .or(det.selected_connector);

    ContextPlan {
        task_type,
        response_language_hint,
        needs_memory: llm.needs_memory,
        memory_namespaces,
        memory_kinds,
        needs_conversation_recall: llm.needs_conversation_recall,
        candidate_skill_names: skills,
        confidence: llm_confidence.max(det.confidence),
        intent_source: if llm_confidence < 0.55 || llm_is_weak_general {
            "deterministic".to_string()
        } else if !det.candidate_skill_names.is_empty() {
            "merged".to_string()
        } else {
            "llm".to_string()
        },
        selected_connector,
        clarification_needed: llm.clarification_needed,
    }
}

fn infer_connector_for_task(task_type: &str) -> Option<String> {
    match task_type {
        "mail_search" => Some("mail".to_string()),
        "odoo_lookup" => Some("odoo".to_string()),
        "file_search" => Some("filesystem".to_string()),
        "screen_context" => Some("screen".to_string()),
        "whatsapp" => Some("whatsapp".to_string()),
        _ => None,
    }
}

fn default_skills_for_task(task_type: &str) -> Vec<String> {
    match task_type {
        "sk_business_email" => vec!["sk-business-email".to_string()],
        "mail_search" => vec!["mail-search".to_string()],
        "invoice_analysis" => vec!["invoice-analysis".to_string()],
        "odoo_lookup" => vec!["odoo-readonly".to_string()],
        "window_control" => vec!["aerospace-window-control".to_string()],
        "file_search" => vec![
            "file-search".to_string(),
            "file-open".to_string(),
            "app-open-control".to_string(),
        ],
        "screen_context" => vec!["screen-context".to_string()],
        "whatsapp" => vec!["whatsapp".to_string()],
        _ => vec![],
    }
}

fn skill_compatible_with_task(task_type: &str, skill: &str) -> bool {
    match task_type {
        "sk_business_email" => skill == "sk-business-email",
        "mail_search" => skill == "mail-search" || skill == "sk-business-email",
        "invoice_analysis" => skill == "invoice-analysis" || skill == "sk-business-email",
        "odoo_lookup" => skill == "odoo-readonly",
        "window_control" => skill == "aerospace-window-control",
        "file_search" => matches!(skill, "file-search" | "file-open" | "app-open-control"),
        "screen_context" => skill == "screen-context",
        "whatsapp" => skill == "whatsapp",
        _ => true,
    }
}

fn default_kinds_for_namespaces(namespaces: &[String]) -> Vec<String> {
    let mut kinds = vec![];
    for ns in namespaces {
        match ns.as_str() {
            "user_pref" => {
                kinds.push("preference".to_string());
            }
            "style_profile" => {
                kinds.push("style_profile".to_string());
            }
            "sk_glossary" => {
                kinds.push("sk_glossary".to_string());
            }
            "contacts" => {
                kinds.push("contact".to_string());
            }
            "corrections" => {
                kinds.push("correction".to_string());
            }
            "negative_rules" => {
                kinds.push("negative_rule".to_string());
            }
            _ => {}
        }
    }
    kinds
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(msg: &str, lang: &str) -> ContextPlan {
        deterministic_plan(msg, lang, false)
    }

    #[test]
    fn trivial_needs_no_memory() {
        let p = plan("what is 2+2?", "en");
        assert!(!p.needs_memory);
        assert!(!p.needs_conversation_recall);
    }

    #[test]
    fn explicit_memory_trigger_sk() {
        let p = plan("pamätaj si, že preferujem krátke zhrnutia", "sk");
        assert_eq!(p.task_type, "explicit_memory");
        assert!(
            !p.needs_memory,
            "explicit memory turn should not trigger retrieval"
        );
    }

    #[test]
    fn explicit_memory_trigger_en() {
        let p = plan("remember from now on I prefer bullet points", "en");
        assert_eq!(p.task_type, "explicit_memory");
        assert!(!p.needs_memory);
    }

    #[test]
    fn conversation_recall_sk() {
        let p = plan("čo som hovoril o Katke minulý týždeň?", "sk");
        assert_eq!(p.task_type, "conversation_recall");
        assert!(p.needs_conversation_recall);
        assert!(!p.needs_memory);
    }

    #[test]
    fn conversation_recall_en() {
        let p = plan("what did I say about Katka last week?", "en");
        assert_eq!(p.task_type, "conversation_recall");
        assert!(p.needs_conversation_recall);
    }

    #[test]
    fn sk_business_email_loads_style_profile() {
        let p = plan("napíš formálnu odpoveď na tento mail", "sk");
        assert_eq!(p.task_type, "sk_business_email");
        assert!(p.needs_memory);
        assert!(p.memory_namespaces.contains(&"style_profile".to_string()));
        assert!(p.memory_namespaces.contains(&"sk_glossary".to_string()));
        assert!(p
            .candidate_skill_names
            .contains(&"sk-business-email".to_string()));
    }

    #[test]
    fn mail_search_loads_mail_search_skill() {
        let p = plan("nájdi mail od Katky a otvor ho", "sk");
        assert_eq!(p.task_type, "mail_search");
        assert!(p.candidate_skill_names.contains(&"mail-search".to_string()));
    }

    #[test]
    fn explicit_whatsapp_beats_generic_mail_words() {
        let p = plan("can you list my last unread messages on whatsapp", "en");
        assert_eq!(p.task_type, "whatsapp");
        assert!(p.candidate_skill_names.contains(&"whatsapp".to_string()));
        assert!(!p.candidate_skill_names.contains(&"mail-search".to_string()));
    }

    #[test]
    fn misspelled_whatsapp_recent_text_routes_to_whatsapp() {
        let p = plan("whats the most recent text with Slavka in whatspp", "en");
        assert_eq!(p.task_type, "whatsapp");
        assert!(p.candidate_skill_names.contains(&"whatsapp".to_string()));
        assert_eq!(p.selected_connector.as_deref(), Some("whatsapp"));
    }

    #[test]
    fn mail_read_still_routes_to_mail_when_mail_is_explicit() {
        let p = plan("can you read my latest email", "en");
        assert_eq!(p.task_type, "mail_search");
        assert!(p.candidate_skill_names.contains(&"mail-search".to_string()));
    }

    #[test]
    fn invoice_loads_invoice_analysis_skill() {
        let p = plan("pozri faktúru a skontroluj DPH a splatnosť", "sk");
        assert_eq!(p.task_type, "invoice_analysis");
        assert!(p
            .candidate_skill_names
            .contains(&"invoice-analysis".to_string()));
        assert!(p.needs_memory);
        assert!(p.memory_namespaces.contains(&"sk_glossary".to_string()));
    }

    #[test]
    fn odoo_with_file_search_signals_is_ambiguous() {
        // "odoo" as a folder name + file-search verb → deterministic plan must have
        // low confidence so the LLM disambiguator is invoked to score both intents.
        let p = plan(
            "can you find me rozuctovanie excel in the odoo development folder?",
            "en",
        );
        assert_eq!(
            p.task_type, "odoo_lookup",
            "deterministic layer keeps odoo_lookup as initial guess"
        );
        assert!(
            p.confidence < 0.6,
            "ambiguous query must have confidence < 0.6 to trigger LLM scoring (got {})",
            p.confidence
        );
    }

    #[test]
    fn find_me_xlsx_in_downloads_folder_is_file_search() {
        // No "odoo", no invoice/mail terms → deterministic file_search via is_file_search keywords
        let p = plan(
            "find me the rozuctovanie xlsx in the downloads folder",
            "en",
        );
        assert_eq!(p.task_type, "file_search");
    }

    #[test]
    fn odoo_erp_query_has_high_confidence() {
        // Pure ERP query: no find-verb + folder — stays odoo_lookup with high confidence
        let p = plan("what are the open orders in odoo for partner ABC?", "en");
        assert_eq!(p.task_type, "odoo_lookup");
        assert!(
            p.confidence >= 0.8,
            "unambiguous ERP query should have high confidence"
        );
    }

    #[test]
    fn window_control_selects_aerospace_skill() {
        let p = plan("prepni na plochu 3", "sk");
        assert_eq!(p.task_type, "window_control");
        assert!(p
            .candidate_skill_names
            .contains(&"aerospace-window-control".to_string()));
        assert!(!p.needs_memory);
    }

    #[test]
    fn weather_needs_no_memory() {
        let p = plan("what is the weather like?", "en");
        assert!(!p.needs_memory);
        assert!(!p.needs_conversation_recall);
    }

    #[test]
    fn write_reply_to_katka_no_recall() {
        let p = plan("write a reply to Katka", "en");
        assert!(
            !p.needs_conversation_recall,
            "writing a reply should not inject old chat turns"
        );
    }

    #[test]
    fn merge_keeps_llm_task_type() {
        let det = ContextPlan {
            task_type: "general".to_string(),
            confidence: 0.5,
            ..Default::default()
        };
        let llm = LlmPlanResult {
            task_type: "sk_business_email".to_string(),
            confidence: Some(0.82),
            selected_connector: None,
            clarification_needed: false,
            intent_scores: std::collections::HashMap::new(),
            needs_memory: true,
            memory_namespaces: vec!["user_pref".to_string(), "sk_glossary".to_string()],
            needs_conversation_recall: false,
            candidate_skill_names: vec!["sk-business-email".to_string()],
            response_language: "slovak_required".to_string(),
        };
        let merged = merge_plans(det, llm);
        assert_eq!(merged.task_type, "sk_business_email");
        assert_eq!(
            merged.response_language_hint,
            ResponseLanguageHint::SlovakRequired
        );
    }

    #[test]
    fn merge_keeps_deterministic_connector_when_llm_is_general() {
        let det = plan("whats the most recent text with Slavka in whatspp", "en");
        let llm = LlmPlanResult {
            task_type: "general".to_string(),
            confidence: Some(0.9),
            selected_connector: None,
            clarification_needed: false,
            intent_scores: [
                ("odoo_lookup".to_string(), 0.0),
                ("file_search".to_string(), 0.0),
                ("whatsapp".to_string(), 0.0),
                ("mail_search".to_string(), 0.0),
            ]
            .into_iter()
            .collect(),
            needs_memory: false,
            memory_namespaces: vec![],
            needs_conversation_recall: false,
            candidate_skill_names: vec![],
            response_language: "english_default".to_string(),
        };
        let merged = merge_plans(det, llm);
        assert_eq!(merged.task_type, "whatsapp");
        assert!(merged
            .candidate_skill_names
            .contains(&"whatsapp".to_string()));
    }

    #[test]
    fn whatsapp_task_gets_whatsapp_skill_even_if_llm_returns_wrong_skills() {
        let det = plan("whats my latest message with Slavka on whatsapp", "en");
        let llm = LlmPlanResult {
            task_type: "whatsapp".to_string(),
            confidence: Some(1.0),
            selected_connector: Some("whatsapp".to_string()),
            clarification_needed: false,
            intent_scores: [("whatsapp".to_string(), 1.0)].into_iter().collect(),
            needs_memory: false,
            memory_namespaces: vec![],
            needs_conversation_recall: false,
            candidate_skill_names: vec![
                "sk-business-email".to_string(),
                "mail-search".to_string(),
                "invoice-analysis".to_string(),
            ],
            response_language: "english_default".to_string(),
        };
        let merged = merge_plans(det, llm);
        assert_eq!(merged.task_type, "whatsapp");
        assert_eq!(
            merged.candidate_skill_names.first().map(String::as_str),
            Some("whatsapp")
        );
        assert!(!merged
            .candidate_skill_names
            .contains(&"mail-search".to_string()));
    }

    #[test]
    #[ignore = "requires Ollama + qwen2.5:0.5b"]
    fn llm_fallback_parses_json() {
        // Run with: cargo test -p bagent-agent -- --include-ignored
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let ollama = OllamaClient::new("http://127.0.0.1:11434");
            let planner = ContextPlanner::new(ollama, "qwen2.5:0.5b".to_string());
            let p = planner
                .plan(
                    "draft a formal reply to this Slovak invoice reminder",
                    "en",
                    false,
                    &PlannerRuntimeContext::default(),
                )
                .await;
            assert!(p.needs_memory);
        });
    }
}
