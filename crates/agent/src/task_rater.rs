use serde::{Deserialize, Serialize};

/// Deterministic task-rating module.
///
/// Classifies user task descriptions into five levels that govern whether the
/// local Ollama model is sufficient, or whether the Codex external-reasoning
/// harness should be offered (always with explicit user approval).
///
/// The rating is **purely deterministic** — keyword gates, no LLM calls.
/// Privacy and safety decisions must not depend on a generative model.

// ── Public types ──────────────────────────────────────────────────────────────

/// Where the task sits on the local-vs-Codex complexity axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskLevel {
    /// Use local Ollama only. One-source, simple operation.
    #[default]
    LocalOnly,
    /// Local model should handle this. Codex unnecessary unless user asks.
    LocalPreferred,
    /// Codex may help, but only with approval. Cross-source or moderately complex.
    CodexCandidate,
    /// Prefer Codex: broad, multi-source, ambiguous, or careful reasoning needed.
    CodexRecommended,
    /// Codex only, with explicit approval and privacy warning.
    CodexRequired,
}

impl std::fmt::Display for TaskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalOnly => write!(f, "LocalOnly"),
            Self::LocalPreferred => write!(f, "LocalPreferred"),
            Self::CodexCandidate => write!(f, "CodexCandidate"),
            Self::CodexRecommended => write!(f, "CodexRecommended"),
            Self::CodexRequired => write!(f, "CodexRequired"),
        }
    }
}

/// Privacy sensitivity of the context that would be sent to Codex.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyRisk {
    #[default]
    Low,
    Medium,
    High,
    Sensitive,
}

impl std::fmt::Display for PrivacyRisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "Low"),
            Self::Medium => write!(f, "Medium"),
            Self::High => write!(f, "High"),
            Self::Sensitive => write!(f, "Sensitive"),
        }
    }
}

/// Which context items should be built for the Codex packet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextScope {
    /// Do not send anything to Codex.
    #[default]
    None,
    /// Send only summaries and metadata, no raw bodies.
    SummariesOnly,
    /// Send summaries plus selected record references (user reviews which).
    SelectedRecords,
    /// User has explicitly approved the full packet contents.
    UserApprovedPacket,
}

/// Full rating result for a user task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRating {
    pub level: TaskLevel,
    /// 0–100 complexity score driving the level classification.
    pub score: u8,
    /// Human-readable reasons for the rating (shown in approval text and UI hints).
    pub reasons: Vec<String>,
    /// True when the level is CodexCandidate or above.
    pub codex_recommended: bool,
    /// True when user approval is required before proceeding.
    pub requires_approval: bool,
    pub privacy_risk: PrivacyRisk,
    pub suggested_context_scope: ContextScope,
}

// ── TaskRater ─────────────────────────────────────────────────────────────────

/// Unit-struct rater — no state, deterministic, sync.
///
/// Mirrors the `PromptBuilder` pattern used elsewhere in this crate: a stateless
/// unit struct whose only public method does the work.
#[derive(Default)]
pub struct TaskRater;

impl TaskRater {
    pub fn new() -> Self {
        Self
    }

    /// Rate a task description and optional context-source list.
    ///
    /// # Arguments
    /// * `description` – the user's raw message or task description.
    /// * `context_sources` – optional declared sources from the caller
    ///   (e.g. `["mail", "odoo", "notes"]`). Merged with sources detected in
    ///   the description text.
    /// * `privacy_hint` – optional caller hint; `"pii"` forces at least `High`.
    pub fn rate(
        &self,
        description: &str,
        context_sources: &[String],
        privacy_hint: Option<&str>,
    ) -> TaskRating {
        let low = description.to_lowercase();

        // ── Hard-gate: external system mutation → immediate CodexRequired ─────
        if is_external_mutation(&low) {
            return TaskRating {
                level: TaskLevel::CodexRequired,
                score: 95,
                reasons: vec![
                    "external system mutation detected (write/update to Odoo or similar)"
                        .to_string(),
                ],
                codex_recommended: true,
                requires_approval: true,
                privacy_risk: privacy_from_text(&low, context_sources, privacy_hint, true),
                suggested_context_scope: ContextScope::UserApprovedPacket,
            };
        }

        // ── Hard-gate: bulk messaging / mass replies → CodexRequired ─────────
        if is_bulk_messaging(&low) {
            return TaskRating {
                level: TaskLevel::CodexRequired,
                score: 90,
                reasons: vec!["bulk messaging or mass-reply to contacts detected".to_string()],
                codex_recommended: true,
                requires_approval: true,
                privacy_risk: privacy_from_text(&low, context_sources, privacy_hint, false),
                suggested_context_scope: ContextScope::UserApprovedPacket,
            };
        }

        // ── Additive complexity score ─────────────────────────────────────────
        let mut score: i32 = 0;
        let mut reasons: Vec<String> = Vec::new();

        // Source count — description text + caller-declared
        let source_count = count_sources(&low, context_sources);
        match source_count {
            0 | 1 => {} // no boost for single-source
            2 => {
                score += 20;
                reasons.push("multi-source task (2 sources)".to_string());
            }
            3 => {
                score += 40;
                reasons.push(format!("multi-source task ({source_count} sources)"));
            }
            _ => {
                score += 45;
                reasons.push(format!("multi-source task ({source_count} sources)"));
            }
        }

        // Complexity signals (+20 each)
        if has_reconcile(&low) {
            score += 20;
            reasons.push("reconciliation task".to_string());
        }
        if has_investigate(&low) {
            score += 20;
            reasons.push("investigation / root-cause analysis".to_string());
        }
        if has_timeline(&low) {
            score += 15;
            reasons.push("timeline or communication history reconstruction".to_string());
        }
        if has_contradiction(&low) {
            score += 15;
            reasons.push("conflict / contradiction detection".to_string());
        }
        if has_brief_or_digest(&low) {
            score += 20;
            reasons.push("client brief or business digest".to_string());
        }
        if has_compare(&low) {
            score += 20;
            reasons.push("comparison across documents or records".to_string());
        }
        if has_all_context(&low) {
            score += 15;
            reasons.push("requesting all context about a subject".to_string());
        }
        if has_action_plan(&low) {
            score += 15;
            reasons.push("multi-step action plan requested".to_string());
        }
        if has_explicit_codex(&low) {
            score += 20;
            reasons.push("user explicitly requested Codex".to_string());
        }

        // Financial / document signals (+10)
        if has_invoice_or_payment(&low) {
            score += 10;
            reasons.push("invoice / payment / financial document".to_string());
        }
        if has_dispute(&low) {
            score += 10;
            reasons.push("dispute or legal issue".to_string());
        }

        // Business drafting with a named customer / client (+15)
        if has_business_draft_with_customer(&low) {
            score += 15;
            reasons.push("business reply or draft for a specific customer".to_string());
        }

        // Mild reducer: explicit single-item reference with no complexity signal
        if is_single_item_simple(&low) {
            score -= 5;
        }

        // Clamp to 0–100
        let score = score.clamp(0, 100) as u8;

        // Score → level thresholds (calibrated against the SK/EN test fixture)
        let level = if score >= 85 {
            TaskLevel::CodexRequired
        } else if score >= 60 {
            TaskLevel::CodexRecommended
        } else if score >= 30 {
            TaskLevel::CodexCandidate
        } else if score >= 10 {
            TaskLevel::LocalPreferred
        } else {
            TaskLevel::LocalOnly
        };

        let codex_recommended = !matches!(level, TaskLevel::LocalOnly | TaskLevel::LocalPreferred);
        let requires_approval = codex_recommended;

        let privacy_risk = privacy_from_text(&low, context_sources, privacy_hint, false);

        let suggested_context_scope = match &level {
            TaskLevel::LocalOnly | TaskLevel::LocalPreferred => ContextScope::None,
            TaskLevel::CodexCandidate => ContextScope::SummariesOnly,
            TaskLevel::CodexRecommended => ContextScope::SelectedRecords,
            TaskLevel::CodexRequired => ContextScope::UserApprovedPacket,
        };

        TaskRating {
            level,
            score,
            reasons,
            codex_recommended,
            requires_approval,
            privacy_risk,
            suggested_context_scope,
        }
    }
}

// ── Gate helpers ──────────────────────────────────────────────────────────────

/// Count distinct data sources mentioned in the description + declared list.
fn count_sources(low: &str, declared: &[String]) -> usize {
    let mut sources = std::collections::HashSet::new();

    // Mail / email / Gmail / inbox
    if low.contains("mail")
        || low.contains("email")
        || low.contains("gmail")
        || low.contains("inbox")
        || low.contains("správ")
    {
        sources.insert("mail");
    }
    // Notes / poznámky (any Slovak case form: poznámka/poznámky/poznámok/poznámkach)
    if low.contains("note") || low.contains("notes") || low.contains("poznám") {
        sources.insert("notes");
    }
    // Odoo ERP
    if low.contains("odoo") {
        sources.insert("odoo");
    }
    // WhatsApp (any case suffix, e.g. "whatsappu", "whatsappa")
    if low.contains("whatsapp") {
        sources.insert("whatsapp");
    }
    // Files / PDF / local documents / Downloads / attachments
    if low.contains("file")
        || low.contains("pdf")
        || low.contains("súbor")
        || low.contains("dokument")
        || low.contains("downloads")
        || low.contains("príloha")
        || low.contains("attachment")
    {
        sources.insert("files");
    }
    // Reminders / calendar / tasks
    if low.contains("reminder")
        || low.contains("calendar")
        || low.contains("úloh")
        || low.contains("task")
        || low.contains("kalendár")
    {
        sources.insert("reminders");
    }
    // Screen / display
    if low.contains("screen") || low.contains("obrazovk") {
        sources.insert("screen");
    }

    // Merge caller-declared sources
    for s in declared {
        let sl = s.to_lowercase();
        match sl.as_str() {
            "mail" | "email" | "gmail" | "apple_mail" => {
                sources.insert("mail");
            }
            "notes" | "apple_notes" => {
                sources.insert("notes");
            }
            "odoo" => {
                sources.insert("odoo");
            }
            "whatsapp" => {
                sources.insert("whatsapp");
            }
            "files" | "filesystem" | "local_files" => {
                sources.insert("files");
            }
            "reminders" | "calendar" => {
                sources.insert("reminders");
            }
            "screen" => {
                sources.insert("screen");
            }
            _ => {}
        }
    }

    sources.len()
}

/// Hard gate: task requests writing to / mutating an external system directly.
/// Covers Slovak (`aktualizuj`, `zmeň`) and English variants.
fn is_external_mutation(low: &str) -> bool {
    let mutation_verb = low.contains("aktualizuj")
        || low.contains("zmeň v")
        || low.contains("update odoo")
        || low.contains("write to odoo")
        || low.contains("modify odoo")
        || low.contains("zapíš do")
        || low.contains("vlož do odoo");
    let mutation_obj = low.contains("odoo") || low.contains("crm") || low.contains("systém");
    (mutation_verb && mutation_obj)
        || low.contains("update odoo")
        || low.contains("aktualizuj odoo")
}

/// Hard gate: bulk messaging, mass WhatsApp replies, or mass email replies.
fn is_bulk_messaging(low: &str) -> bool {
    let has_bulk = low.contains("hromadné")
        || low.contains("hromadnú")
        || low.contains("hromadné")
        || low.contains("bulk")
        || low.contains("mass ")
        || low.contains("masov");
    let has_message = low.contains("odpoved")   // odpoveď / odpovede / odpovedí
        || low.contains("replies")
        || low.contains("reply")
        || low.contains("message")
        || low.contains("správ")
        || low.contains("whatsapp");
    has_bulk && has_message
}

fn has_reconcile(low: &str) -> bool {
    low.contains("reconcile")
        || low.contains("reconciliation")
        || low.contains("zosúlad")
        || low.contains("porovnaj s odoo")
        || low.contains("overiť faktúr")
}

fn has_investigate(low: &str) -> bool {
    low.contains("zisti prečo")
        || low.contains("investigate")
        || low.contains("preskúmaj")
        || low.contains("prečo tvrd")
        || low.contains("find out why")
        || low.contains("why did")
        || low.contains("analyze why")
        || low.contains("root cause")
}

fn has_timeline(low: &str) -> bool {
    low.contains("timeline")
        || low.contains("časová os")
        || low.contains("history of")
        || low.contains("communication history")
        || low.contains("chronológ")
}

fn has_contradiction(low: &str) -> bool {
    low.contains("contradiction")
        || low.contains("contradict")
        || low.contains("conflicts")
        || low.contains("conflict")
        || low.contains("rozpor")
        || low.contains("nezrovnalosť")
}

fn has_brief_or_digest(low: &str) -> bool {
    low.contains("brief")
        || low.contains("briefing")
        || low.contains("digest")
        || low.contains("týždenný")
        || low.contains("weekly")
        || low.contains("business overview")
}

fn has_compare(low: &str) -> bool {
    low.contains("porovnaj")
        || low.contains("porovnanie")
        || low.contains("compare")
        || low.contains("comparison")
        || low.contains(" vs ")
        || low.contains("versus")
}

fn has_all_context(low: &str) -> bool {
    low.contains("all context")
        || low.contains("všetko o")
        || low.contains("everything about")
        || low.contains("celý kontext")
        || low.contains("kompletný prehľad")
        || low.contains("full picture")
}

fn has_action_plan(low: &str) -> bool {
    low.contains("action plan")
        || low.contains("akčný plán")
        || low.contains("multi-step")
        || low.contains("step by step plan")
}

fn has_explicit_codex(low: &str) -> bool {
    low.contains("codex")
        || low.contains("použi codex")
        || low.contains("external reasoning")
        || low.contains("advanced reasoning")
}

fn has_invoice_or_payment(low: &str) -> bool {
    low.contains("invoice")
        || low.contains("invoices")
        || low.contains("faktúr")    // faktúra/faktúry/faktúrou/faktúrach (all share "faktúr")
        || low.contains("payment")
        || low.contains("platba")
        || low.contains("zaplatil")
        || low.contains("zaplatit")
        || low.contains("overdue")
        || low.contains("po splatnosti")
        || low.contains("due date")
        || low.contains("splatnosť")
}

fn has_dispute(low: &str) -> bool {
    low.contains("dispute")
        || low.contains("spor")
        || low.contains("complaint")
        || low.contains("sťažnosť")
        || low.contains("legal")
        || low.contains("právny")
}

/// True when the user is drafting a reply/response **for a specific customer**.
/// Moderate complexity booster — more than simple summarise, less than multi-source.
fn has_business_draft_with_customer(low: &str) -> bool {
    let has_draft = low.contains("priprav odpoveď")
        || low.contains("navrhni odpoveď")
        || low.contains("prepare reply")
        || low.contains("prepare a reply")
        || low.contains("draft reply")
        || low.contains("draft a reply")
        || low.contains("draft an email")
        || low.contains("prepare an email")
        || low.contains("prepare response")
        || low.contains("write a reply")
        || low.contains("write a response");
    let has_customer = low.contains("zákazník")  // all forms share the root
        || low.contains("klient")
        || low.contains("customer")
        || low.contains("client");
    has_draft && has_customer
}

/// Mild downward nudge: explicit single-item references with no complexity signal.
fn is_single_item_simple(low: &str) -> bool {
    let single_marker = low.contains("tento email")
        || low.contains("tento mail")
        || low.contains("this email")
        || low.contains("this note")
        || low.contains("one email")
        || low.contains("one note")
        || low.contains("jednu poznámk")
        || low.contains("túto poznámk");
    // Only penalise when there are no complexity signals present
    single_marker && !has_compare(low) && !has_reconcile(low) && !has_investigate(low)
}

/// Compute privacy risk from the description, declared sources, and hint.
fn privacy_from_text(
    low: &str,
    declared: &[String],
    hint: Option<&str>,
    is_mutation: bool,
) -> PrivacyRisk {
    // Risk levels: 0=Low 1=Medium 2=High 3=Sensitive
    let mut risk: u8 = 0;

    // Credentials / legal → Sensitive
    if low.contains("credential")
        || low.contains("keychain")
        || low.contains("password")
        || low.contains("heslo")
        || low.contains("legal")
        || low.contains("právny")
        || low.contains("súdny")
    {
        risk = risk.max(3);
    }

    // WhatsApp / Gmail → High (private messaging channels)
    if low.contains("whatsapp") || low.contains("gmail") {
        risk = risk.max(2);
    }

    // Odoo customer records → High
    if low.contains("odoo") || low.contains("zákazník") || low.contains("klient") {
        risk = risk.max(2);
    }

    // Invoices / payments / personal data → Medium-High
    if low.contains("faktúr")
        || low.contains("invoice")
        || low.contains("payment")
        || low.contains("platba")
        || low.contains("personal")
        || low.contains("osobné")
    {
        risk = risk.max(1);
    }

    // Mail / notes bodies → Medium
    if low.contains("mail")
        || low.contains("email")
        || low.contains("note")
        || low.contains("poznám")
    {
        risk = risk.max(1);
    }

    // Declared WhatsApp / Gmail sources → High
    for s in declared {
        let sl = s.to_lowercase();
        if sl == "whatsapp" || sl == "gmail" {
            risk = risk.max(2);
        }
        if sl == "odoo" {
            risk = risk.max(2);
        }
        // Mail / notes → at least Medium
        if sl == "mail" || sl == "email" || sl == "notes" {
            risk = risk.max(1);
        }
    }

    // PII hint from caller
    if hint == Some("pii") {
        risk = risk.max(2);
    }

    // Mutation → High minimum
    if is_mutation {
        risk = risk.max(2);
    }

    match risk {
        0 => PrivacyRisk::Low,
        1 => PrivacyRisk::Medium,
        2 => PrivacyRisk::High,
        _ => PrivacyRisk::Sensitive,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rate(desc: &str) -> TaskRating {
        TaskRater::new().rate(desc, &[], None)
    }

    fn rate_with(desc: &str, sources: &[&str]) -> TaskRating {
        let srcs: Vec<String> = sources.iter().map(|s| s.to_string()).collect();
        TaskRater::new().rate(desc, &srcs, None)
    }

    // ── Required SK/EN fixture from spec ──────────────────────────────────────

    #[test]
    fn sk_summarize_single_email_is_local_only() {
        let r = rate("zhrň mi tento email");
        assert_eq!(r.level, TaskLevel::LocalOnly, "score={}", r.score);
    }

    #[test]
    fn sk_find_last_email_is_local_only() {
        let r = rate("nájdi posledný email od Katky");
        assert_eq!(r.level, TaskLevel::LocalOnly, "score={}", r.score);
    }

    #[test]
    fn sk_prepare_reply_to_customer_is_local_preferred() {
        let r = rate("priprav odpoveď zákazníkovi podľa tohto emailu");
        assert_eq!(r.level, TaskLevel::LocalPreferred, "score={}", r.score);
    }

    #[test]
    fn sk_compare_invoice_mail_pdf_is_codex_candidate() {
        let r = rate("porovnaj faktúru z mailu s PDF v Downloads");
        assert_eq!(r.level, TaskLevel::CodexCandidate, "score={}", r.score);
    }

    #[test]
    fn sk_investigate_payment_across_mail_odoo_notes_is_codex_recommended() {
        let r =
            rate("zisti prečo klient tvrdí, že faktúru už zaplatil, použi mail, Odoo a poznámky");
        assert_eq!(r.level, TaskLevel::CodexRecommended, "score={}", r.score);
    }

    #[test]
    fn sk_weekly_digest_from_four_sources_is_codex_recommended() {
        let r = rate("sprav týždenný digest z mailov, Odoo, poznámok a WhatsAppu");
        assert_eq!(r.level, TaskLevel::CodexRecommended, "score={}", r.score);
    }

    #[test]
    fn sk_bulk_replies_is_codex_required() {
        let r = rate("navrhni hromadné odpovede zákazníkom na základe emailov a Odoo");
        assert_eq!(r.level, TaskLevel::CodexRequired, "score={}", r.score);
    }

    #[test]
    fn sk_update_odoo_is_codex_required() {
        let r = rate("aktualizuj Odoo podľa mailov");
        assert_eq!(r.level, TaskLevel::CodexRequired, "score={}", r.score);
    }

    #[test]
    fn en_summarize_one_note_is_local_only() {
        let r = rate("summarize this one note");
        assert_eq!(r.level, TaskLevel::LocalOnly, "score={}", r.score);
    }

    #[test]
    fn en_client_brief_from_four_sources_is_codex_recommended() {
        let r = rate("prepare a client brief from emails, notes, Odoo and local files");
        assert_eq!(r.level, TaskLevel::CodexRecommended, "score={}", r.score);
    }

    #[test]
    fn en_reconcile_invoices_is_codex_recommended() {
        let r = rate("reconcile open invoices across Odoo, Mail and PDFs");
        assert_eq!(r.level, TaskLevel::CodexRecommended, "score={}", r.score);
    }

    #[test]
    fn en_bulk_whatsapp_replies_is_codex_required() {
        let r = rate("draft bulk WhatsApp replies for overdue invoices");
        assert_eq!(r.level, TaskLevel::CodexRequired, "score={}", r.score);
    }

    // ── Additional correctness checks ─────────────────────────────────────────

    #[test]
    fn en_update_odoo_mutation_is_required() {
        let r = rate("update Odoo based on emails");
        assert_eq!(r.level, TaskLevel::CodexRequired, "score={}", r.score);
    }

    #[test]
    fn sk_open_email_is_local_only() {
        let r = rate("otvor email od Petra");
        assert_eq!(r.level, TaskLevel::LocalOnly, "score={}", r.score);
    }

    #[test]
    fn en_explain_screen_is_local_only() {
        let r = rate("explain what is on my screen");
        assert_eq!(r.level, TaskLevel::LocalOnly, "score={}", r.score);
    }

    #[test]
    fn en_translate_message_is_local_only() {
        let r = rate("translate this short message to English");
        assert_eq!(r.level, TaskLevel::LocalOnly, "score={}", r.score);
    }

    #[test]
    fn en_compare_two_docs_is_codex_candidate() {
        let r = rate("compare the two supplier offers from the emails and the PDF attachments");
        assert_eq!(r.level, TaskLevel::CodexCandidate, "score={}", r.score);
    }

    #[test]
    fn sk_prepare_client_brief_is_codex_recommended() {
        let r = rate("priprav kompletný brief pre klienta z mailov, poznámok a Odoo");
        assert_eq!(r.level, TaskLevel::CodexRecommended, "score={}", r.score);
    }

    #[test]
    fn codex_recommended_has_approval_required() {
        let r = rate("prepare a client brief from emails, notes, Odoo and local files");
        assert!(r.requires_approval);
        assert!(r.codex_recommended);
        assert_eq!(r.suggested_context_scope, ContextScope::SelectedRecords);
    }

    #[test]
    fn local_only_has_no_approval() {
        let r = rate("summarize this one note");
        assert!(!r.requires_approval);
        assert!(!r.codex_recommended);
        assert_eq!(r.suggested_context_scope, ContextScope::None);
    }

    #[test]
    fn codex_required_has_user_approved_packet_scope() {
        let r = rate("aktualizuj Odoo podľa mailov");
        assert_eq!(r.suggested_context_scope, ContextScope::UserApprovedPacket);
    }

    #[test]
    fn declared_sources_count_toward_multi_source() {
        // "summarize the customer situation" alone — not multi-source
        let r1 = rate("summarize the customer situation");
        // Same text but caller declares mail + odoo → now 2 sources
        let r2 = rate_with("summarize the customer situation", &["mail", "odoo"]);
        assert!(
            r2.score > r1.score,
            "declared sources should raise score: {} vs {}",
            r2.score,
            r1.score
        );
    }

    #[test]
    fn pii_hint_raises_privacy_risk() {
        let srcs: Vec<String> = vec![];
        let r = TaskRater::new().rate("summarize this note", &srcs, Some("pii"));
        assert!(
            matches!(r.privacy_risk, PrivacyRisk::High | PrivacyRisk::Sensitive),
            "expected High+ got {:?}",
            r.privacy_risk
        );
    }

    #[test]
    fn whatsapp_source_raises_privacy_risk_to_high() {
        let r = rate("sprav digest z mailov a WhatsAppu");
        assert!(
            matches!(r.privacy_risk, PrivacyRisk::High | PrivacyRisk::Sensitive),
            "expected High+ got {:?}",
            r.privacy_risk
        );
    }

    #[test]
    fn reasons_not_empty_for_complex_tasks() {
        let r = rate("reconcile open invoices across Odoo, Mail and PDFs");
        assert!(!r.reasons.is_empty(), "expected reasons for complex task");
    }

    #[test]
    fn reasons_contain_multi_source() {
        let r = rate("sprav týždenný digest z mailov, Odoo, poznámok a WhatsAppu");
        assert!(
            r.reasons.iter().any(|s| s.contains("multi-source")),
            "expected multi-source in reasons, got: {:?}",
            r.reasons
        );
    }

    #[test]
    fn sk_full_investigation_reasons_mention_investigation() {
        let r =
            rate("zisti prečo klient tvrdí, že faktúru už zaplatil, použi mail, Odoo a poznámky");
        assert!(
            r.reasons.iter().any(|s| s.contains("investigation")),
            "reasons: {:?}",
            r.reasons
        );
    }

    #[test]
    fn codex_candidate_scope_is_summaries_only() {
        let r = rate("porovnaj faktúru z mailu s PDF v Downloads");
        assert_eq!(r.suggested_context_scope, ContextScope::SummariesOnly);
    }
}
