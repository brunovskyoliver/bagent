use super::{
    EvidenceBudget, EvidenceIntent, EvidencePlan, EvidenceRequirement, VerificationLevel,
    EVIDENCE_SCHEMA_VERSION,
};

pub(crate) struct EvidencePlanner;

impl EvidencePlanner {
    pub(crate) fn plan(intent: EvidenceIntent) -> EvidencePlan {
        let operational_intent = match &intent {
            EvidenceIntent::AnalyzeQuotedEvidence { intent } => intent.as_ref(),
            intent => intent,
        };
        let (requirements, budget) = match operational_intent {
            EvidenceIntent::MailLatestHeaders { count, .. } => (
                vec![EvidenceRequirement::MailHeaders { count: *count }],
                EvidenceBudget {
                    mail_list_attempts: 1,
                    mail_body_attempts: 0,
                    web_search_attempts: 0,
                    web_fetch_attempts: 0,
                    max_parallel_fetches: 0,
                    optional_exploration_rounds: 0,
                },
            ),
            EvidenceIntent::MailLatestContent { count, .. } => (
                vec![
                    EvidenceRequirement::MailHeaders { count: *count },
                    EvidenceRequirement::MailBodies { count: *count },
                ],
                EvidenceBudget {
                    mail_list_attempts: 1,
                    // Reserve one retry per requested body where the global
                    // ten-attempt cap permits it. A retry must not consume the
                    // slot for the next distinct message.
                    mail_body_attempts: count.saturating_mul(2).min(10),
                    web_search_attempts: 0,
                    web_fetch_attempts: 0,
                    max_parallel_fetches: 0,
                    optional_exploration_rounds: 0,
                },
            ),
            EvidenceIntent::MailTargeted { needs_content, .. } => (
                vec![EvidenceRequirement::TargetedMail {
                    needs_content: *needs_content,
                }],
                EvidenceBudget {
                    mail_list_attempts: 1,
                    mail_body_attempts: u8::from(*needs_content).saturating_mul(2),
                    web_search_attempts: 0,
                    web_fetch_attempts: 0,
                    max_parallel_fetches: 0,
                    optional_exploration_rounds: 0,
                },
            ),
            EvidenceIntent::WebDirectPage { .. } => (
                vec![EvidenceRequirement::DirectPage],
                EvidenceBudget {
                    mail_list_attempts: 0,
                    mail_body_attempts: 0,
                    web_search_attempts: 0,
                    web_fetch_attempts: 1,
                    max_parallel_fetches: 1,
                    optional_exploration_rounds: 0,
                },
            ),
            EvidenceIntent::WebFact { verification, .. } => (
                vec![EvidenceRequirement::FetchedSources {
                    count: match verification {
                        VerificationLevel::SingleAuthoritative => 1,
                        VerificationLevel::Corroborated => 2,
                    },
                }],
                EvidenceBudget {
                    mail_list_attempts: 0,
                    mail_body_attempts: 0,
                    web_search_attempts: 2,
                    web_fetch_attempts: 5,
                    max_parallel_fetches: 2,
                    optional_exploration_rounds: 1,
                },
            ),
            EvidenceIntent::AnalyzeQuotedEvidence { .. } => {
                unreachable!("quoted analysis intents are unwrapped before planning")
            }
        };
        EvidencePlan {
            version: EVIDENCE_SCHEMA_VERSION,
            intent,
            requirements,
            budget,
        }
    }
}
