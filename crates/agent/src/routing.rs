//! Routing layer — decides which private connectors (Apple Mail / files /
//! WhatsApp) a user turn may search, with what filters and limits.
//!
//! Pipeline per turn:
//!   1. `deterministic_hints` — cheap bilingual keyword rules.
//!   2. `RouteClassifier` — qwen3:0.6b slim JSON classification (think off).
//!   3. `build_route` — pure merge + validation → `RoutePlan`. The app owns
//!      the plan; model output is a recommendation, never an instruction.
//!   4. On weak/invalid classification the daemon retries with the main model
//!      (`RouteClassifier::classify_careful`, think on) and re-merges.
//!
//! Privacy posture is fail-closed: no private source is searched unless the
//! turn explicitly names one or carries a personal-data signal.

use anyhow::{bail, Result};
use chrono::Datelike;
use ollama_connector::OllamaClient;
use serde::{Deserialize, Serialize};

// ── Sources & capability profiles ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    AppleMail,
    Files,
    Whatsapp,
}

impl Source {
    pub fn as_str(&self) -> &'static str {
        match self {
            Source::AppleMail => "apple_mail",
            Source::Files => "files",
            Source::Whatsapp => "whatsapp",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "apple_mail" | "mail" | "email" => Some(Source::AppleMail),
            "files" | "file" | "filesystem" => Some(Source::Files),
            "whatsapp" | "wa" => Some(Source::Whatsapp),
            _ => None,
        }
    }

    pub const ALL: [Source; 3] = [Source::AppleMail, Source::Files, Source::Whatsapp];
}

/// Static capability profile for one connector tool. Used to build the
/// classifier prompt, validate plans, and label audit logs.
pub struct ToolProfile {
    pub name: &'static str,
    pub source: Source,
    pub domains: &'static [&'static str],
    pub can_answer: &'static [&'static str],
    /// Filters the executor accepts for this tool; anything else is dropped.
    pub filters: &'static [&'static str],
    pub privacy: &'static str,
    pub cost: &'static str,
    pub returns: &'static [&'static str],
}

pub const TOOL_PROFILES: [ToolProfile; 3] = [
    ToolProfile {
        name: "apple_mail_search",
        source: Source::AppleMail,
        domains: &["email", "attachments", "invoices", "receipts", "sent/received messages"],
        can_answer: &[
            "Find an email from someone",
            "Search my emails for a topic",
            "Find an attachment someone sent me",
            "Find invoices or receipts in email",
        ],
        filters: &["query", "person", "date_from", "date_to", "has_attachment"],
        privacy: "sensitive",
        cost: "medium",
        returns: &["subject", "sender", "date", "snippet", "message_id", "attachments"],
    },
    ToolProfile {
        name: "file_search",
        source: Source::Files,
        domains: &["local files", "documents", "downloads", "PDFs", "spreadsheets"],
        can_answer: &[
            "Where is my resume?",
            "Find the PDF I downloaded",
            "Search documents about taxes",
        ],
        filters: &["query", "file_type", "date_from", "date_to"],
        privacy: "sensitive",
        cost: "low",
        returns: &["path", "name", "kind", "modified_at", "snippet"],
    },
    ToolProfile {
        name: "whatsapp_search",
        source: Source::Whatsapp,
        domains: &["chats", "messages from contacts", "conversations"],
        can_answer: &[
            "What did X say about ...",
            "Find the message from X",
            "Search my chat with X",
        ],
        filters: &["query", "person", "date_from", "date_to"],
        privacy: "sensitive",
        cost: "medium",
        returns: &["chat", "contact", "date", "snippet"],
    },
];

pub fn profile_for(source: Source) -> &'static ToolProfile {
    TOOL_PROFILES.iter().find(|p| p.source == source).unwrap()
}

// ── Deterministic hints ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct SourceHint {
    pub source: Source,
    /// 0.0–1.0 rule confidence.
    pub score: f32,
    /// True when the user named the source outright ("in my email", "on WhatsApp").
    pub explicit: bool,
}

/// Bilingual (EN + SK) keyword rules. Each entry: (patterns, source, score, explicit).
/// Ordered strongest-first; a source keeps its best-scoring hit.
const HINT_RULES: &[(&[&str], Source, f32, bool)] = &[
    // Explicit source mentions
    (
        &["in my email", "my emails", "email", "e-mail", "mail", "inbox", "mailbox",
          "v mojom maile", "v maili", "v mailoch", "schránk", "schrank", "pošt", "post box"],
        Source::AppleMail, 0.9, true,
    ),
    (
        &["whatsapp", "whatspp", "whatsap", "na whatsappe", "cez whatsapp", " wa "],
        Source::Whatsapp, 0.95, true,
    ),
    (
        &["on my mac", "in my files", "my files", "file", "folder", "priečinok", "priecinok",
          "súbor", "subor", "downloaded", "stiahnut", "stiahol", "desktop", "documents folder",
          "plocha", "downloads"],
        Source::Files, 0.85, true,
    ),
    // Strong implicit signals
    (
        &["sent me", "poslal mi", "poslala mi", "posielal mi", "emailed me"],
        Source::AppleMail, 0.7, false,
    ),
    (&["sent me", "poslal mi", "poslala mi"], Source::Whatsapp, 0.45, false),
    (
        &["attachment", "attached", "príloh", "priloh", "invoice", "receipt",
          "faktúr", "faktur", "bloček", "blocek"],
        Source::AppleMail, 0.6, false,
    ),
    (&["invoice", "receipt", "faktúr", "faktur"], Source::Files, 0.4, false),
    (
        &["message from", "what did", "čo povedal", "co povedal", "čo písal", "co pisal",
          "čo mi písal", "chat with", "chat s", "správa od", "sprava od", "wrote", "napísal mi",
          "napisal mi", "mentioned", "spomenul", "spomenula"],
        Source::Whatsapp, 0.7, false,
    ),
    (&["what did", "čo povedal", "co povedal", "mentioned", "spomenul", "spomenula"], Source::AppleMail, 0.35, false),
    (
        &["pdf", "docx", ".doc", "xlsx", "spreadsheet", "presentation", "prezentáci",
          "prezentaci", "tabuľk", "tabulk", "resume", "cv", "životopis", "zivotopis"],
        Source::Files, 0.7, false,
    ),
    (&["contract", "zmluv", "taxes", "dane", "daňov", "danov"], Source::Files, 0.4, false),
    (&["contract", "zmluv"], Source::AppleMail, 0.4, false),
    (&["meeting link", "odkaz na meeting", "link"], Source::AppleMail, 0.4, false),
    (&["meeting link", "link"], Source::Whatsapp, 0.35, false),
];

pub fn deterministic_hints(text: &str) -> Vec<SourceHint> {
    let low = text.to_lowercase();
    let mut best: std::collections::HashMap<Source, SourceHint> = Default::default();
    for (patterns, source, score, explicit) in HINT_RULES {
        if patterns.iter().any(|p| low.contains(p)) {
            let e = best.entry(*source).or_insert(SourceHint {
                source: *source,
                score: 0.0,
                explicit: false,
            });
            e.score = e.score.max(*score);
            e.explicit |= *explicit;
        }
    }
    let mut hints: Vec<SourceHint> = best.into_values().collect();
    hints.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    hints
}

/// True when the turn refers to the user's own data — the precondition for
/// touching any private source when no source was explicitly named.
pub fn personal_signal(text: &str) -> bool {
    let low = format!(" {} ", text.to_lowercase());
    [
        " my ", " mine ", " me ", " i downloaded", " i saved", " i got", " i received",
        " môj ", " moj ", " moja ", " moje ", " mi ", " mne ", " som ", " sme ",
        "sent me", " sent", "poslal", "poslala", "písal", "pisal", "povedal", "povedala",
        "napísal", "napisal", "spomenul", "spomenula", "said", " say", "wrote",
        "mentioned", "what did", "čo ", "co ",
    ]
    .iter()
    .any(|p| low.contains(p))
}

/// Destructive / externally-visible verbs — never executed by the routing
/// layer; the plan degrades to a clarification.
pub fn destructive_signal(text: &str) -> bool {
    let low = text.to_lowercase();
    [
        "delete", "remove", "send ", "share", "forward", "upload", "move ", "rename",
        "modify", "vymaž", "vymaz", "zmaž", "zmaz", "odstráň", "odstran", "pošli",
        "posli", "odošli", "odosli", "prepošli", "preposli", "presuň", "presun",
        "premenuj", "zdieľaj", "zdielaj", "uprav ",
    ]
    .iter()
    .any(|p| low.contains(p))
}

/// Deterministic date-range extraction for common relative phrases.
/// Returns (date_from, date_to) as ISO "YYYY-MM-DD".
// ponytail: keyword table only (today/yesterday/last week/month names) —
// swap for a date-parsing crate if users need arbitrary phrases.
pub fn parse_date_hint(text: &str, today: chrono::NaiveDate) -> Option<(String, String)> {
    let low = text.to_lowercase();
    let day = |d: chrono::NaiveDate| d.format("%Y-%m-%d").to_string();
    if low.contains("today") || low.contains("dnes") {
        return Some((day(today), day(today)));
    }
    if low.contains("yesterday") || low.contains("včera") || low.contains("vcera") {
        let y = today - chrono::Duration::days(1);
        return Some((day(y), day(y)));
    }
    if low.contains("last week") || low.contains("minulý týždeň") || low.contains("minuly tyzden") {
        return Some((day(today - chrono::Duration::days(14)), day(today)));
    }
    if low.contains("this week") || low.contains("tento týždeň") || low.contains("tento tyzden") {
        return Some((day(today - chrono::Duration::days(7)), day(today)));
    }
    if low.contains("last month") || low.contains("minulý mesiac") || low.contains("minuly mesiac") {
        return Some((day(today - chrono::Duration::days(31)), day(today)));
    }
    const MONTHS: [(&str, u32); 24] = [
        ("january", 1), ("january", 1), ("february", 2), ("februar", 2), ("march", 3),
        ("marec", 3), ("april", 4), ("apríl", 4), ("may", 5), ("máj", 5), ("june", 6),
        ("jún", 6), ("july", 7), ("júl", 7), ("august", 8), ("august", 8), ("september", 9),
        ("september", 9), ("october", 10), ("októb", 10), ("november", 11), ("november", 11),
        ("december", 12), ("december", 12),
    ];
    for (name, m) in MONTHS {
        if low.contains(name) {
            let year = if m > today.month() { today.year() - 1 } else { today.year() };
            let from = chrono::NaiveDate::from_ymd_opt(year, m, 1)?;
            let to = if m == 12 {
                chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)?
            } else {
                chrono::NaiveDate::from_ymd_opt(year, m + 1, 1)?
            } - chrono::Duration::days(1);
            return Some((day(from), day(to)));
        }
    }
    None
}

/// File extension hinted by the turn, if any.
pub fn parse_file_type_hint(text: &str) -> Option<String> {
    let low = text.to_lowercase();
    for (kw, ext) in [
        ("pdf", "pdf"), ("docx", "docx"), (".doc", "doc"), ("xlsx", "xlsx"),
        ("spreadsheet", "xlsx"), ("tabuľk", "xlsx"), ("tabulk", "xlsx"),
        ("presentation", "pptx"), ("prezentáci", "pptx"), ("prezentaci", "pptx"),
    ] {
        if low.contains(kw) {
            return Some(ext.to_string());
        }
    }
    None
}

// ── Classifier (qwen3:0.6b, slim schema) ─────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentType {
    Question,
    Find,
    Summarize,
    Compare,
    Open,
    Read,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSource {
    pub source: Source,
    pub confidence: f32,
}

/// Validated slim classification. The app builds the actual tool plan from
/// this + deterministic hints; the model never dictates filters or limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteClassification {
    pub intent_type: IntentType,
    pub local_data_needed: bool,
    pub privacy_level: String,
    pub candidate_sources: Vec<CandidateSource>,
    pub person: Option<String>,
    pub clarification_needed: bool,
}

/// Raw wire format — permissive strings so one bad enum value doesn't throw
/// away the whole classification.
#[derive(Debug, Deserialize)]
struct RawClassification {
    #[serde(default)]
    intent_type: String,
    #[serde(default)]
    local_data_needed: bool,
    #[serde(default)]
    privacy_level: String,
    #[serde(default)]
    candidate_sources: Vec<RawCandidate>,
    #[serde(default)]
    person: Option<String>,
    #[serde(default)]
    clarification_needed: bool,
}

#[derive(Debug, Deserialize)]
struct RawCandidate {
    #[serde(default)]
    source: String,
    #[serde(default)]
    confidence: f32,
}

/// Strict validation of raw model output. Unknown sources are dropped;
/// confidences are clamped; an unknown intent_type maps to Unknown.
pub fn validate_classification(raw_json: &str) -> Result<RouteClassification> {
    let raw: RawClassification = serde_json::from_str(raw_json)?;
    let intent_type = match raw.intent_type.trim().to_lowercase().as_str() {
        "question" => IntentType::Question,
        "find" => IntentType::Find,
        "summarize" => IntentType::Summarize,
        "compare" => IntentType::Compare,
        "open" => IntentType::Open,
        "read" => IntentType::Read,
        _ => IntentType::Unknown,
    };
    let privacy_level = match raw.privacy_level.trim().to_lowercase().as_str() {
        "personal" => "personal",
        "sensitive" => "sensitive",
        _ => "none",
    }
    .to_string();
    let mut candidate_sources: Vec<CandidateSource> = raw
        .candidate_sources
        .into_iter()
        .filter_map(|c| {
            Source::parse(&c.source).map(|source| CandidateSource {
                source,
                // Cap below explicit-hint scores: small models are chronically
                // overconfident, and a model guess must never outrank a
                // deterministic hint into a single-source route.
                confidence: c.confidence.clamp(0.0, 0.9),
            })
        })
        .collect();
    candidate_sources.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    candidate_sources.dedup_by_key(|c| c.source);
    let person = raw
        .person
        .filter(|p| !p.trim().is_empty() && p.trim().to_lowercase() != "null" && p.len() < 80);
    if candidate_sources.is_empty() && raw.local_data_needed {
        bail!("local_data_needed with no valid candidate sources");
    }
    Ok(RouteClassification {
        intent_type,
        local_data_needed: raw.local_data_needed,
        privacy_level,
        candidate_sources,
        person,
        clarification_needed: raw.clarification_needed,
    })
}

fn classifier_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "intent_type": {"type": "string", "enum": ["question","find","summarize","compare","open","read","unknown"]},
            "local_data_needed": {"type": "boolean"},
            "privacy_level": {"type": "string", "enum": ["none","personal","sensitive"]},
            "candidate_sources": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "source": {"type": "string", "enum": ["apple_mail","files","whatsapp"]},
                        "confidence": {"type": "number"}
                    },
                    "required": ["source", "confidence"]
                }
            },
            "person": {"type": ["string", "null"]},
            "clarification_needed": {"type": "boolean"}
        },
        "required": ["intent_type", "local_data_needed", "privacy_level", "candidate_sources", "clarification_needed"]
    })
}

pub fn classifier_prompt(user_turn: &str, context: &str) -> String {
    let context_block = if context.is_empty() {
        String::new()
    } else {
        format!("Prior conversation (for pronoun resolution only):\n{context}\n\n")
    };
    format!(
        "You are a fast local intent classifier for a private desktop agent. \
         Classify the user request and suggest which local sources may hold the answer. \
         Do not answer the user. Return JSON only.\n\n\
         Available sources:\n\
         - apple_mail: local emails and attachments\n\
         - files: local macOS files\n\
         - whatsapp: local WhatsApp messages\n\n\
         Rules:\n\
         - Prefer no sources (empty candidate_sources, local_data_needed=false) when the \
           request is general knowledge.\n\
         - Use private sources only when the user clearly asks about their own local data.\n\
         - \"sent me\"/\"poslal mi\" → apple_mail first, maybe whatsapp.\n\
         - \"downloaded\"/\"file\"/\"PDF\"/\"stiahol\" → files.\n\
         - \"chat\"/\"whatsapp\"/\"what did X say\"/\"čo písal\" → whatsapp.\n\
         - \"email\"/\"inbox\"/\"attachment\"/\"faktúra v maili\" → apple_mail.\n\
         - person: the contact/sender name the user mentioned, else null.\n\
         - The user may write Slovak or English.\n\n\
         {context_block}User request: \"{user_turn}\""
    )
}

pub struct RouteClassifier {
    ollama: OllamaClient,
    model: String,
}

impl RouteClassifier {
    pub fn new(ollama: OllamaClient, model: String) -> Self {
        Self { ollama, model }
    }

    /// Fast classification (think off). One retry on invalid JSON.
    pub async fn classify(&self, user_turn: &str, context: &str) -> Result<RouteClassification> {
        let prompt = classifier_prompt(user_turn, context);
        for attempt in 0..2 {
            let raw = self
                .ollama
                .generate_json_schema(&self.model, &prompt, classifier_schema(), 0.0)
                .await?;
            match validate_classification(&raw) {
                Ok(c) => return Ok(c),
                Err(e) if attempt == 0 => {
                    tracing::debug!(error = %e, "route classifier invalid JSON, retrying");
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    /// Careful fallback on the main model (think on). Same schema — the app
    /// builds the plan either way, the bigger model just classifies better.
    pub async fn classify_careful(
        &self,
        main_model: &str,
        user_turn: &str,
        context: &str,
    ) -> Result<RouteClassification> {
        let prompt = classifier_prompt(user_turn, context);
        let raw = self
            .ollama
            .generate_json_schema_think(main_model, &prompt, classifier_schema(), 0.0, true)
            .await?;
        validate_classification(&raw)
    }
}

// ── Route plan ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct RouteBudgets {
    pub max_tool_calls: usize,
    pub max_results_per_tool: usize,
    pub default_limit: usize,
    pub max_private_sources: usize,
    pub max_parallel_searches: usize,
    pub max_evidence_items: usize,
    /// Confidence at/above which a single top source wins outright.
    pub single_source_confidence: f32,
    /// Minimum merged score for a source to join a parallel search.
    pub min_source_score: f32,
}

impl Default for RouteBudgets {
    fn default() -> Self {
        Self {
            max_tool_calls: 6,
            max_results_per_tool: 10,
            default_limit: 5,
            max_private_sources: 3,
            max_parallel_searches: 3,
            max_evidence_items: 12,
            single_source_confidence: 0.75,
            min_source_score: 0.3,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannedSearch {
    pub source: Source,
    /// Free-text query terms (already stripped of routing keywords).
    pub query: String,
    pub person: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub file_type: Option<String>,
    pub limit: usize,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoutePlan {
    /// No private data touched. `offer_search` names sources the user could
    /// explicitly ask for (privacy gate fail-closed).
    NoTools { offer_search: Vec<Source> },
    /// Destructive/ambiguous request — confirm before doing anything.
    Clarify { reason: String },
    /// Read-only searches to execute, priority order = vec order.
    Search {
        searches: Vec<PlannedSearch>,
        intent_type: IntentType,
        /// Read the top-ranked hit's full content when it clearly wins.
        read_top: bool,
    },
}

/// True when the daemon should retry classification with the main model.
pub fn needs_careful_pass(
    classification: Option<&RouteClassification>,
    hints: &[SourceHint],
) -> bool {
    match classification {
        None => true, // invalid JSON after retry
        Some(c) => {
            let low_conf = c
                .candidate_sources
                .first()
                .map(|s| s.confidence < 0.5)
                .unwrap_or(c.local_data_needed);
            (low_conf && hints.is_empty())
                || matches!(c.intent_type, IntentType::Summarize | IntentType::Compare)
        }
    }
}

/// Pure merge of deterministic hints + (optional) classification into a
/// validated plan. All limits are clamped by `budgets` here — nothing the
/// model says can widen them.
pub fn build_route(
    user_turn: &str,
    hints: &[SourceHint],
    classification: Option<&RouteClassification>,
    budgets: &RouteBudgets,
    today: chrono::NaiveDate,
) -> RoutePlan {
    if destructive_signal(user_turn) {
        return RoutePlan::Clarify {
            reason: "destructive_or_outbound_action".into(),
        };
    }

    // Merge: per-source score = max(hint score, classifier confidence).
    let mut scores: std::collections::HashMap<Source, f32> = Default::default();
    let mut any_explicit = false;
    for h in hints {
        scores.insert(h.source, h.score);
        any_explicit |= h.explicit;
    }
    if let Some(c) = classification {
        for cand in &c.candidate_sources {
            let e = scores.entry(cand.source).or_insert(0.0);
            *e = e.max(cand.confidence);
        }
    }

    let classifier_wants_local = classification.map(|c| c.local_data_needed).unwrap_or(false);
    let intent_type = classification
        .map(|c| c.intent_type)
        .unwrap_or(IntentType::Find);

    // Privacy gate — fail closed. Private sources only on explicit mention,
    // or a personal-data signal in the turn plus at least one candidate.
    let personal = personal_signal(user_turn);
    let justified = any_explicit || (personal && (!scores.is_empty() || classifier_wants_local));
    if scores.is_empty() || !justified {
        let mut offer: Vec<Source> = scores.keys().copied().collect();
        if offer.is_empty() && classifier_wants_local {
            offer = classification
                .map(|c| c.candidate_sources.iter().map(|s| s.source).collect())
                .unwrap_or_default();
        }
        return RoutePlan::NoTools { offer_search: offer };
    }

    if classification.map(|c| c.clarification_needed).unwrap_or(false) && scores.is_empty() {
        return RoutePlan::Clarify {
            reason: "classifier_requested_clarification".into(),
        };
    }

    // Source selection — confidence-gated parallelism.
    let mut ranked: Vec<(Source, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let top_score = ranked[0].1;
    let second_score = ranked.get(1).map(|r| r.1).unwrap_or(0.0);
    let single = top_score >= budgets.single_source_confidence
        && (top_score - second_score) >= 0.25;
    let selected: Vec<(Source, f32)> = if single {
        vec![ranked[0]]
    } else {
        ranked
            .into_iter()
            .filter(|(_, s)| *s >= budgets.min_source_score)
            .take(
                budgets
                    .max_parallel_searches
                    .min(budgets.max_private_sources)
                    .min(budgets.max_tool_calls),
            )
            .collect()
    };

    let person = classification.and_then(|c| c.person.clone());
    let (date_from, date_to) = parse_date_hint(user_turn, today)
        .map(|(f, t)| (Some(f), Some(t)))
        .unwrap_or((None, None));
    let file_type = parse_file_type_hint(user_turn);
    let query = content_query(user_turn, person.as_deref());

    let searches = selected
        .iter()
        .enumerate()
        .map(|(i, (source, _))| PlannedSearch {
            source: *source,
            query: query.clone(),
            person: person.clone(),
            date_from: date_from.clone(),
            date_to: date_to.clone(),
            file_type: if *source == Source::Files {
                file_type.clone()
            } else {
                None
            },
            limit: budgets.default_limit.min(budgets.max_results_per_tool),
            priority: (i + 1) as u8,
        })
        .collect();

    RoutePlan::Search {
        searches,
        intent_type,
        read_top: matches!(
            intent_type,
            IntentType::Summarize | IntentType::Read | IntentType::Open | IntentType::Find
        ),
    }
}

/// Extract content search terms from the turn: drop stopwords, routing
/// keywords, and the person's name (it goes into the person filter).
pub fn content_query(text: &str, person: Option<&str>) -> String {
    const STOP: &[&str] = &[
        // EN
        "the", "a", "an", "in", "on", "at", "of", "for", "to", "from", "my", "me", "i",
        "is", "was", "did", "do", "does", "what", "where", "which", "who", "find", "search",
        "look", "open", "read", "show", "summarize", "please", "can", "you", "about", "that",
        "sent", "say", "said", "me?", "and", "or", "with", "week", "last", "this", "yesterday",
        "today", "downloaded", "email", "emails", "mail", "mails", "inbox", "whatsapp", "chat",
        "file", "files", "folder", "message", "messages", "thread",
        // recency qualifiers — pure noise for keyword AND-match (search sorts by date DESC).
        "most", "recent", "recently", "latest", "newest", "new", "any", "get",
        "najnovší", "najnovsi", "najnovšiu", "najnovsiu", "posledný", "posledny",
        "poslednú", "poslednu", "nedávny", "nedavny",
        // SK
        "nájdi", "najdi", "vyhľadaj", "vyhladaj", "kde", "je", "čo", "co", "mi", "môj", "moj",
        "moja", "moje", "od", "v", "na", "z", "zo", "s", "so", "o", "a", "alebo", "ktorý",
        "ktory", "poslal", "poslala", "písal", "pisal", "povedal", "povedala", "spomenul",
        "spomenula", "otvor", "prečítaj", "precitaj", "zhrň", "zhrn", "mail", "maile", "maili",
        "správu", "spravu", "správa", "sprava", "súbor", "subor", "prosím", "prosim", "včera",
        "vcera", "dnes", "minulý", "minuly", "týždeň", "tyzden", "stiahol", "stiahla", "som",
    ];
    let person_low = person.map(|p| p.to_lowercase());
    text.split(|c: char| !c.is_alphanumeric() && c != 'á' && !c.is_alphabetic())
        .map(|w| w.trim().to_lowercase())
        .filter(|w| w.len() > 1)
        .filter(|w| !STOP.contains(&w.as_str()))
        .filter(|w| {
            person_low
                .as_ref()
                .map(|p| !p.contains(w.as_str()))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 7, 5).unwrap()
    }

    fn clf(
        intent: IntentType,
        local: bool,
        sources: &[(Source, f32)],
        person: Option<&str>,
    ) -> RouteClassification {
        RouteClassification {
            intent_type: intent,
            local_data_needed: local,
            privacy_level: if local { "personal" } else { "none" }.into(),
            candidate_sources: sources
                .iter()
                .map(|(s, c)| CandidateSource {
                    source: *s,
                    confidence: *c,
                })
                .collect(),
            person: person.map(String::from),
            clarification_needed: false,
        }
    }

    fn route(turn: &str, c: Option<&RouteClassification>) -> RoutePlan {
        build_route(
            turn,
            &deterministic_hints(turn),
            c,
            &RouteBudgets::default(),
            today(),
        )
    }

    fn sources_of(plan: &RoutePlan) -> Vec<Source> {
        match plan {
            RoutePlan::Search { searches, .. } => searches.iter().map(|s| s.source).collect(),
            _ => vec![],
        }
    }

    // ── Spec test cases 1–12 ─────────────────────────────────────────────────

    #[test]
    fn t1_what_did_john_say_prefers_whatsapp() {
        let c = clf(
            IntentType::Find,
            true,
            &[(Source::Whatsapp, 0.7), (Source::AppleMail, 0.4)],
            Some("John"),
        );
        let plan = route("What did John say about the trip?", Some(&c));
        let s = sources_of(&plan);
        assert_eq!(s.first(), Some(&Source::Whatsapp));
    }

    #[test]
    fn t2_contract_maria_sent_me_mail_first() {
        let c = clf(
            IntentType::Find,
            true,
            &[(Source::AppleMail, 0.7), (Source::Files, 0.4)],
            Some("Maria"),
        );
        let plan = route("Find the contract Maria sent me.", Some(&c));
        let s = sources_of(&plan);
        assert_eq!(s.first(), Some(&Source::AppleMail));
        assert!(s.contains(&Source::Files));
    }

    #[test]
    fn t3_where_is_my_resume_files_first() {
        let c = clf(IntentType::Find, true, &[(Source::Files, 0.8)], None);
        let plan = route("Where is my resume?", Some(&c));
        let s = sources_of(&plan);
        assert_eq!(s.first(), Some(&Source::Files));
    }

    #[test]
    fn t4_meeting_link_sarah_mail_and_whatsapp() {
        let c = clf(
            IntentType::Find,
            true,
            &[(Source::AppleMail, 0.55), (Source::Whatsapp, 0.5)],
            Some("Sarah"),
        );
        let plan = route("Find the meeting link Sarah sent.", Some(&c));
        let s = sources_of(&plan);
        assert!(s.contains(&Source::AppleMail));
        assert!(s.contains(&Source::Whatsapp));
    }

    #[test]
    fn t5_pdf_taxes_downloaded_files_first_with_date_and_type() {
        let c = clf(IntentType::Find, true, &[(Source::Files, 0.8)], None);
        let plan = route("Find the PDF about taxes I downloaded last week.", Some(&c));
        match &plan {
            RoutePlan::Search { searches, .. } => {
                assert_eq!(searches[0].source, Source::Files);
                assert_eq!(searches[0].file_type.as_deref(), Some("pdf"));
                assert!(searches[0].date_from.is_some());
            }
            other => panic!("expected search, got {other:?}"),
        }
    }

    #[test]
    fn t6_general_question_no_tools() {
        let c = clf(IntentType::Question, false, &[], None);
        let plan = route("What is Qwen3?", Some(&c));
        assert!(matches!(plan, RoutePlan::NoTools { .. }));
    }

    #[test]
    fn t7_search_my_emails_invoices_june_mail_only_with_dates() {
        let c = clf(IntentType::Find, true, &[(Source::AppleMail, 0.9)], None);
        let plan = route("Search my emails for invoices from June.", Some(&c));
        match &plan {
            RoutePlan::Search { searches, .. } => {
                assert_eq!(searches.len(), 1);
                assert_eq!(searches[0].source, Source::AppleMail);
                assert_eq!(searches[0].date_from.as_deref(), Some("2026-06-01"));
                assert_eq!(searches[0].date_to.as_deref(), Some("2026-06-30"));
            }
            other => panic!("expected search, got {other:?}"),
        }
    }

    #[test]
    fn t8_search_whatsapp_alex_dinner_whatsapp_only() {
        let c = clf(
            IntentType::Find,
            true,
            &[(Source::Whatsapp, 0.9)],
            Some("Alex"),
        );
        let plan = route("Search WhatsApp for what Alex said about dinner.", Some(&c));
        let s = sources_of(&plan);
        assert_eq!(s, vec![Source::Whatsapp]);
    }

    #[test]
    fn t9_delete_requires_clarification() {
        let c = clf(IntentType::Unknown, true, &[(Source::Files, 0.8)], None);
        let plan = route("Delete the files you found.", Some(&c));
        assert!(matches!(plan, RoutePlan::Clarify { .. }));
    }

    #[test]
    fn t10_open_downloaded_file_reads_top() {
        let c = clf(IntentType::Open, true, &[(Source::Files, 0.9)], None);
        let plan = route("Open the file I downloaded yesterday about taxes.", Some(&c));
        match &plan {
            RoutePlan::Search {
                searches, read_top, ..
            } => {
                assert_eq!(searches[0].source, Source::Files);
                assert!(*read_top);
            }
            other => panic!("expected search, got {other:?}"),
        }
    }

    #[test]
    fn t11_summarize_thread_mail_with_read() {
        let c = clf(
            IntentType::Summarize,
            true,
            &[(Source::AppleMail, 0.9)],
            Some("Peter"),
        );
        let plan = route(
            "Summarize the email thread from Peter about the contract.",
            Some(&c),
        );
        match &plan {
            RoutePlan::Search {
                searches, read_top, ..
            } => {
                assert_eq!(searches[0].source, Source::AppleMail);
                assert!(*read_top);
                assert_eq!(searches[0].person.as_deref(), Some("Peter"));
            }
            other => panic!("expected search, got {other:?}"),
        }
    }

    #[test]
    fn t12_lisa_mentioned_address_parallel_small_limits() {
        let c = clf(
            IntentType::Find,
            true,
            &[(Source::Whatsapp, 0.55), (Source::AppleMail, 0.5)],
            Some("Lisa"),
        );
        let plan = route("Find where Lisa mentioned the address.", Some(&c));
        match &plan {
            RoutePlan::Search { searches, .. } => {
                assert!(searches.len() >= 2);
                assert!(searches.iter().all(|s| s.limit <= 5));
            }
            other => panic!("expected search, got {other:?}"),
        }
    }

    // ── Validation / gate behaviour ──────────────────────────────────────────

    #[test]
    fn rejects_unknown_sources_and_clamps_confidence() {
        let raw = r#"{"intent_type":"find","local_data_needed":true,"privacy_level":"personal",
            "candidate_sources":[{"source":"icloud","confidence":0.9},
                                 {"source":"apple_mail","confidence":7.0}],
            "person":null,"clarification_needed":false}"#;
        let c = validate_classification(raw).unwrap();
        assert_eq!(c.candidate_sources.len(), 1);
        assert_eq!(c.candidate_sources[0].source, Source::AppleMail);
        assert!(c.candidate_sources[0].confidence <= 1.0);
    }

    #[test]
    fn invalid_json_is_error() {
        assert!(validate_classification("not json").is_err());
    }

    #[test]
    fn all_invented_sources_with_local_needed_is_error() {
        let raw = r#"{"intent_type":"find","local_data_needed":true,"privacy_level":"personal",
            "candidate_sources":[{"source":"dropbox","confidence":0.9}],
            "person":null,"clarification_needed":false}"#;
        assert!(validate_classification(raw).is_err());
    }

    #[test]
    fn privacy_gate_blocks_general_turn_even_if_classifier_wants_tools() {
        // Classifier hallucinates a private search for a general question.
        let c = clf(IntentType::Question, true, &[(Source::AppleMail, 0.9)], None);
        let plan = route("What is the capital of France?", Some(&c));
        assert!(matches!(plan, RoutePlan::NoTools { .. }));
    }

    #[test]
    fn limits_never_exceed_budget() {
        let c = clf(
            IntentType::Find,
            true,
            &[
                (Source::AppleMail, 0.6),
                (Source::Whatsapp, 0.6),
                (Source::Files, 0.6),
            ],
            None,
        );
        let plan = route("find the invoice someone sent me", Some(&c));
        if let RoutePlan::Search { searches, .. } = &plan {
            let b = RouteBudgets::default();
            assert!(searches.len() <= b.max_parallel_searches);
            assert!(searches.iter().all(|s| s.limit <= b.max_results_per_tool));
        } else {
            panic!("expected search");
        }
    }

    #[test]
    fn no_classifier_falls_back_to_hints_only() {
        let plan = route("nájdi faktúru v mojom maili od Alzy", None);
        let s = sources_of(&plan);
        assert!(s.contains(&Source::AppleMail));
    }

    #[test]
    fn careful_pass_triggers() {
        assert!(needs_careful_pass(None, &[]));
        let c = clf(IntentType::Summarize, true, &[(Source::AppleMail, 0.9)], None);
        assert!(needs_careful_pass(Some(&c), &[]));
        let weak = clf(IntentType::Find, true, &[(Source::Files, 0.2)], None);
        assert!(needs_careful_pass(Some(&weak), &[]));
        let strong = clf(IntentType::Find, true, &[(Source::Files, 0.9)], None);
        assert!(!needs_careful_pass(Some(&strong), &[]));
    }

    #[test]
    fn date_hints_parse() {
        let t = today();
        assert_eq!(
            parse_date_hint("emails from june", t),
            Some(("2026-06-01".into(), "2026-06-30".into()))
        );
        assert!(parse_date_hint("yesterday's file", t).is_some());
        assert!(parse_date_hint("no dates here", t).is_none());
    }

    #[test]
    fn content_query_strips_noise() {
        let q = content_query("Find the contract Maria sent me.", Some("Maria"));
        assert_eq!(q, "contract");
    }

    #[test]
    fn content_query_drops_recency_qualifiers() {
        // "most recent" must not survive: keywords are AND-matched, and these
        // words appear in no email, so they'd zero out an otherwise-valid search.
        let q = content_query(
            "can you find me the most recent email from radoslava.nemethova@tenenet.sk?",
            None,
        );
        for noise in ["most", "recent"] {
            assert!(!q.split_whitespace().any(|w| w == noise), "leaked {noise}: {q}");
        }
        assert!(q.contains("radoslava") && q.contains("nemethova"), "lost sender tokens: {q}");
    }

    /// End-to-end: live qwen3:0.6b classification merged through build_route
    /// must produce sane plans for the spec queries. Requires Ollama.
    #[tokio::test]
    #[ignore = "requires Ollama + qwen3:0.6b"]
    async fn live_classifier_routes_spec_queries() {
        let ollama = ollama_connector::OllamaClient::new(ollama_connector::DEFAULT_BASE_URL);
        let clf = RouteClassifier::new(ollama, "qwen3:0.6b".to_string());
        let budgets = RouteBudgets::default();
        let t = chrono::Local::now().date_naive();

        // (query, must_include, must_exclude, expect_tools)
        let cases: [(&str, &[Source], &[Source], bool); 5] = [
            (
                "Search my emails for invoices from June.",
                &[Source::AppleMail],
                &[Source::Whatsapp],
                true,
            ),
            (
                "Search WhatsApp for what Alex said about dinner.",
                &[Source::Whatsapp],
                &[],
                true,
            ),
            ("Where is my resume?", &[Source::Files], &[], true),
            ("What is Qwen3?", &[], &[], false),
            (
                "Find the contract Maria sent me.",
                &[Source::AppleMail],
                &[],
                true,
            ),
        ];

        for (query, include, exclude, expect_tools) in cases {
            let classification = clf.classify(query, "").await.ok();
            let hints = deterministic_hints(query);
            let plan = build_route(query, &hints, classification.as_ref(), &budgets, t);
            let sources = match &plan {
                RoutePlan::Search { searches, .. } => {
                    searches.iter().map(|s| s.source).collect::<Vec<_>>()
                }
                _ => vec![],
            };
            if expect_tools {
                for s in include {
                    assert!(sources.contains(s), "{query}: missing {s:?} in {sources:?}");
                }
                for s in exclude {
                    assert!(!sources.contains(s), "{query}: unexpected {s:?}");
                }
            } else {
                assert!(
                    sources.is_empty(),
                    "{query}: expected no tools, got {sources:?}"
                );
            }
        }
    }
}
