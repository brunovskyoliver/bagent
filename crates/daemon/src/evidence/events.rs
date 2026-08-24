use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    BodyOrigin, CanonicalGroundedAnswer, CanonicalOutcomeStatus, Completeness, EvidenceBundle,
    EvidenceContribution, EvidenceCounts, EvidenceIntent, EvidenceOperation, ExecutionStatus,
    FailureCode, RecoveryKind, ValidationOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidencePhase {
    FindingMail,
    Reading,
    Searching,
    Verifying,
    LoadingSynthesisModel,
    PreparingAnswer,
    Repairing,
    FallingBack,
    Validating,
    DeterministicRendering,
}

impl From<super::SynthesisPhase> for EvidencePhase {
    fn from(phase: super::SynthesisPhase) -> Self {
        match phase {
            super::SynthesisPhase::LoadingSynthesisModel => Self::LoadingSynthesisModel,
            super::SynthesisPhase::PreparingAnswer => Self::PreparingAnswer,
            super::SynthesisPhase::Repairing => Self::Repairing,
            super::SynthesisPhase::Validating => Self::Validating,
            super::SynthesisPhase::DeterministicRendering => Self::DeterministicRendering,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidencePhaseEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub turn_id: String,
    pub phase: EvidencePhase,
    pub completed: Option<u16>,
    pub total: Option<u16>,
    pub model_id: Option<String>,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub fallback: bool,
    pub repair: bool,
    pub failure_reason: Option<String>,
}

impl EvidencePhaseEvent {
    pub(crate) fn acquisition(
        turn_id: &str,
        phase: EvidencePhase,
        completed: Option<u16>,
        total: Option<u16>,
    ) -> Self {
        Self {
            event_type: "evidence_phase".to_string(),
            turn_id: turn_id.to_string(),
            phase,
            completed,
            total,
            model_id: None,
            duration_ms: 0,
            timed_out: false,
            fallback: false,
            repair: false,
            failure_reason: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LogicalActivityState {
    InProgress,
    Succeeded,
    Failed,
    Denied,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LogicalActivityEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub turn_id: String,
    pub activity_id: String,
    pub normalized_operation: String,
    pub argument_hash: String,
    pub execution_status: LogicalActivityState,
    pub contribution: EvidenceContribution,
    pub evidence_count: u16,
    pub source_domains: Vec<String>,
    pub duration_ms: u64,
    pub attempt_count: u8,
    pub retries: u8,
    pub duplicates_suppressed: u8,
    pub failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_origin: Option<BodyOrigin>,
}

impl LogicalActivityEvent {
    pub(crate) fn started(turn_id: &str, operation: &EvidenceOperation) -> Self {
        Self {
            event_type: "logical_activity_started".to_string(),
            turn_id: turn_id.to_string(),
            activity_id: activity_id(turn_id, operation),
            normalized_operation: normalized_operation(operation).to_string(),
            argument_hash: operation_argument_hash(operation),
            execution_status: LogicalActivityState::InProgress,
            contribution: EvidenceContribution::Empty,
            evidence_count: 0,
            source_domains: Vec::new(),
            duration_ms: 0,
            attempt_count: 0,
            retries: 0,
            duplicates_suppressed: 0,
            failure_reason: None,
            body_origin: None,
        }
    }

    pub(crate) fn completed(
        turn_id: &str,
        operation: &EvidenceOperation,
        completion: &LogicalActivityCompletion,
    ) -> Self {
        Self {
            event_type: "logical_activity_completed".to_string(),
            turn_id: turn_id.to_string(),
            activity_id: activity_id(turn_id, operation),
            normalized_operation: normalized_operation(operation).to_string(),
            argument_hash: operation_argument_hash(operation),
            execution_status: logical_status(&completion.execution),
            contribution: completion.contribution,
            evidence_count: completion.evidence_count,
            source_domains: completion.source_domains.clone(),
            duration_ms: completion.duration_ms,
            attempt_count: completion.attempt_count,
            retries: completion.attempt_count.saturating_sub(1),
            duplicates_suppressed: completion.duplicates_suppressed,
            failure_reason: normalized_failure(&completion.execution),
            body_origin: completion.body_origin,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogicalActivityCompletion {
    pub execution: ExecutionStatus,
    pub contribution: EvidenceContribution,
    pub evidence_count: u16,
    pub source_domains: Vec<String>,
    pub duration_ms: u64,
    pub attempt_count: u8,
    pub duplicates_suppressed: u8,
    pub body_origin: Option<BodyOrigin>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceOutcomeState {
    Verified,
    Conflict,
    Partial,
    Empty,
    Unavailable,
    Denied,
    VerificationShortfall,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EvidenceOutcomeKind {
    Mail,
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceOutcomeEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    pub turn_id: String,
    pub state: EvidenceOutcomeState,
    pub kind: EvidenceOutcomeKind,
    pub acquired: u16,
    pub requested: u16,
    pub source_count: u16,
    pub message: String,
}

impl EvidenceOutcomeEvent {
    pub(crate) fn from_validation(validation: &ValidationOutcome) -> Self {
        match validation {
            ValidationOutcome::Bundle(bundle) => outcome_from_bundle(bundle),
            ValidationOutcome::Recovery(recovery) => {
                let kind = if recovery.missing.iter().any(|shortfall| {
                    matches!(
                        shortfall.requirement,
                        super::EvidenceRequirement::DirectPage
                            | super::EvidenceRequirement::FetchedSources { .. }
                    )
                }) {
                    EvidenceOutcomeKind::Web
                } else {
                    EvidenceOutcomeKind::Mail
                };
                let (acquired, requested) =
                    relevant_counts(kind, &EvidenceCounts::default(), &recovery.requested);
                let state = match recovery.kind {
                    RecoveryKind::Empty | RecoveryKind::NoUsableEvidence => {
                        EvidenceOutcomeState::Empty
                    }
                    RecoveryKind::Unavailable => EvidenceOutcomeState::Unavailable,
                    RecoveryKind::Denied => EvidenceOutcomeState::Denied,
                    RecoveryKind::VerificationShortfall => {
                        EvidenceOutcomeState::VerificationShortfall
                    }
                    RecoveryKind::InvalidInput | RecoveryKind::Malformed => {
                        EvidenceOutcomeState::Unavailable
                    }
                };
                Self::new(
                    recovery.missing.first().map(|_| "").unwrap_or_default(),
                    state,
                    kind,
                    acquired,
                    requested,
                    0,
                )
            }
            ValidationOutcome::Clarification { .. } => Self::new(
                "",
                EvidenceOutcomeState::Partial,
                EvidenceOutcomeKind::Mail,
                0,
                0,
                0,
            ),
        }
    }

    pub(crate) fn with_turn_id(mut self, turn_id: &str) -> Self {
        self.turn_id = turn_id.to_string();
        self
    }

    pub(crate) fn with_canonical_answer(mut self, canonical: &CanonicalGroundedAnswer) -> Self {
        self.state = match canonical.outcome_status {
            CanonicalOutcomeStatus::Verified => EvidenceOutcomeState::Verified,
            CanonicalOutcomeStatus::Conflict => EvidenceOutcomeState::Conflict,
            CanonicalOutcomeStatus::Partial => EvidenceOutcomeState::Partial,
            CanonicalOutcomeStatus::VerificationShortfall => {
                EvidenceOutcomeState::VerificationShortfall
            }
        };
        self.message = outcome_message(
            self.state,
            self.kind,
            self.acquired,
            self.requested,
            self.source_count,
        );
        self
    }

    fn new(
        turn_id: &str,
        state: EvidenceOutcomeState,
        kind: EvidenceOutcomeKind,
        acquired: u16,
        requested: u16,
        source_count: u16,
    ) -> Self {
        Self {
            event_type: "evidence_outcome".to_string(),
            turn_id: turn_id.to_string(),
            state,
            kind,
            acquired,
            requested,
            source_count,
            message: outcome_message(state, kind, acquired, requested, source_count),
        }
    }
}

fn outcome_from_bundle(bundle: &EvidenceBundle) -> EvidenceOutcomeEvent {
    let kind = if is_web_intent(&bundle.intent) {
        EvidenceOutcomeKind::Web
    } else {
        EvidenceOutcomeKind::Mail
    };
    let (acquired, requested) = relevant_counts(kind, &bundle.acquired, &bundle.requested);
    let state = if kind == EvidenceOutcomeKind::Web && !bundle.conflicts.is_empty() {
        EvidenceOutcomeState::Conflict
    } else if bundle.completeness == Completeness::Complete {
        EvidenceOutcomeState::Verified
    } else {
        EvidenceOutcomeState::Partial
    };
    EvidenceOutcomeEvent::new(
        &bundle.turn_id,
        state,
        kind,
        acquired,
        requested,
        u16::from(bundle.acquired.web_sources),
    )
}

fn relevant_counts(
    kind: EvidenceOutcomeKind,
    acquired: &EvidenceCounts,
    requested: &EvidenceCounts,
) -> (u16, u16) {
    match kind {
        EvidenceOutcomeKind::Mail => {
            let body_request = requested.mail_bodies > 0;
            (
                u16::from(if body_request {
                    acquired.mail_bodies
                } else {
                    acquired.mail_headers
                }),
                u16::from(if body_request {
                    requested.mail_bodies
                } else {
                    requested.mail_headers
                }),
            )
        }
        EvidenceOutcomeKind::Web => (
            u16::from(acquired.web_sources),
            u16::from(requested.web_sources),
        ),
    }
}

fn is_web_intent(intent: &EvidenceIntent) -> bool {
    match intent {
        EvidenceIntent::WebDirectPage { .. } | EvidenceIntent::WebFact { .. } => true,
        EvidenceIntent::AnalyzeQuotedEvidence { intent } => is_web_intent(intent),
        _ => false,
    }
}

fn outcome_message(
    state: EvidenceOutcomeState,
    kind: EvidenceOutcomeKind,
    acquired: u16,
    requested: u16,
    source_count: u16,
) -> String {
    match (kind, state) {
        (EvidenceOutcomeKind::Mail, EvidenceOutcomeState::Verified) => {
            format!("Read {acquired} of {requested} emails")
        }
        (EvidenceOutcomeKind::Mail, EvidenceOutcomeState::Partial) => {
            format!("Read {acquired} of {requested} emails · partial")
        }
        (EvidenceOutcomeKind::Mail, EvidenceOutcomeState::Empty) => "No emails found".to_string(),
        (EvidenceOutcomeKind::Mail, EvidenceOutcomeState::Unavailable) => {
            "Mail unavailable".to_string()
        }
        (EvidenceOutcomeKind::Mail, EvidenceOutcomeState::Denied) => {
            "Mail access denied".to_string()
        }
        (_, EvidenceOutcomeState::VerificationShortfall) => "Couldn't verify sources".to_string(),
        (EvidenceOutcomeKind::Web, EvidenceOutcomeState::Verified) => {
            format!("Web verified · {source_count} sources")
        }
        (EvidenceOutcomeKind::Web, EvidenceOutcomeState::Conflict) => {
            format!("Web verified · {source_count} sources · conflict")
        }
        (EvidenceOutcomeKind::Web, EvidenceOutcomeState::Partial) => {
            format!("Web partially verified · {source_count} sources")
        }
        (EvidenceOutcomeKind::Web, EvidenceOutcomeState::Empty) => {
            "Couldn't verify sources".to_string()
        }
        (EvidenceOutcomeKind::Web, EvidenceOutcomeState::Unavailable) => {
            "Web unavailable".to_string()
        }
        (EvidenceOutcomeKind::Web, EvidenceOutcomeState::Denied) => "Web access denied".to_string(),
        (EvidenceOutcomeKind::Mail, EvidenceOutcomeState::Conflict) => {
            "Mail evidence conflict".to_string()
        }
    }
}

pub(crate) fn normalized_operation(operation: &EvidenceOperation) -> &'static str {
    match operation {
        EvidenceOperation::MailList { .. } => "mail.list",
        EvidenceOperation::MailSearch { .. } => "mail.search",
        EvidenceOperation::MailRead { .. } => "mail.read",
        EvidenceOperation::WebSearch { .. } => "web.search",
        EvidenceOperation::WebFetch { .. } => "web.fetch",
    }
}

pub(crate) fn operation_argument_hash(operation: &EvidenceOperation) -> String {
    hash(&format!(
        "bagent-evidence-operation-v1:{}",
        operation.key().as_str()
    ))
}

pub(crate) fn activity_id(turn_id: &str, operation: &EvidenceOperation) -> String {
    format!(
        "evidence:{}",
        hash(&format!("{turn_id}:{}", operation.key().as_str()))
    )
}

fn hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn logical_status(status: &ExecutionStatus) -> LogicalActivityState {
    match status {
        ExecutionStatus::Succeeded => LogicalActivityState::Succeeded,
        ExecutionStatus::Failed(_) => LogicalActivityState::Failed,
        ExecutionStatus::Denied => LogicalActivityState::Denied,
        ExecutionStatus::TimedOut => LogicalActivityState::TimedOut,
    }
}

pub(crate) fn normalized_failure(status: &ExecutionStatus) -> Option<String> {
    match status {
        ExecutionStatus::Succeeded => None,
        ExecutionStatus::Denied => Some("denied".to_string()),
        ExecutionStatus::TimedOut => Some("timed_out".to_string()),
        ExecutionStatus::Failed(failure) => Some(
            match failure {
                FailureCode::InvalidInput => "invalid_input",
                FailureCode::ConnectorUnavailable => "connector_unavailable",
                FailureCode::ConnectionReset => "connection_reset",
                FailureCode::RateLimited => "rate_limited",
                FailureCode::Http4xx(_) => "http_4xx",
                FailureCode::Http5xx(_) => "http_5xx",
                FailureCode::UnsupportedContentType => "unsupported_content_type",
                FailureCode::UnsafeDestination => "unsafe_destination",
                FailureCode::RedirectUnsafe => "redirect_unsafe",
                FailureCode::BodyTooLarge => "body_too_large",
                FailureCode::EmptyExtraction => "empty_extraction",
                FailureCode::ProviderChallenge => "provider_challenge",
                FailureCode::ParseFailure => "parse_failure",
                FailureCode::ModelUnavailable => "model_unavailable",
                FailureCode::ModelInvalidOutput => "model_invalid_output",
                FailureCode::AutomationFailed => "automation_failed",
                FailureCode::OtherNormalized => "other_normalized",
            }
            .to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{
        EvidenceBundle, EvidenceCounts, EvidenceRequirement, EvidenceShortfall, RecoveryOutcome,
        ShortfallReason, EVIDENCE_SCHEMA_VERSION,
    };

    fn mail_bundle(completeness: Completeness, acquired: u8) -> ValidationOutcome {
        ValidationOutcome::Bundle(Box::new(EvidenceBundle {
            version: EVIDENCE_SCHEMA_VERSION,
            turn_id: "turn-mail".to_string(),
            intent: EvidenceIntent::MailLatestContent {
                count: 3,
                requested_count: 3,
                unread_only: false,
            },
            completeness,
            requested: EvidenceCounts {
                mail_headers: 3,
                mail_bodies: 3,
                web_sources: 0,
            },
            acquired: EvidenceCounts {
                mail_headers: 3,
                mail_bodies: acquired,
                web_sources: 0,
            },
            missing: Vec::new(),
            mail: Vec::new(),
            web: Vec::new(),
            conflicts: Vec::new(),
            exclusions: Vec::new(),
            citation_allowlist: Vec::new(),
        }))
    }

    fn recovery(kind: RecoveryKind, reason: ShortfallReason) -> ValidationOutcome {
        ValidationOutcome::Recovery(RecoveryOutcome {
            kind,
            requested: EvidenceCounts {
                mail_headers: 3,
                mail_bodies: 3,
                web_sources: 0,
            },
            message: "private recovery prose".to_string(),
            missing: vec![EvidenceShortfall {
                requirement: EvidenceRequirement::MailBodies { count: 3 },
                missing_count: 3,
                reason,
            }],
            exclusions: Vec::new(),
        })
    }

    #[test]
    fn complete_partial_empty_unavailable_and_denied_mail_have_distinct_outcomes() {
        let complete =
            EvidenceOutcomeEvent::from_validation(&mail_bundle(Completeness::Complete, 3));
        assert_eq!(complete.state, EvidenceOutcomeState::Verified);
        assert_eq!(complete.message, "Read 3 of 3 emails");

        let partial = EvidenceOutcomeEvent::from_validation(&mail_bundle(Completeness::Partial, 2));
        assert_eq!(partial.state, EvidenceOutcomeState::Partial);
        assert_eq!(partial.message, "Read 2 of 3 emails · partial");

        for (kind, reason, expected) in [
            (
                RecoveryKind::Empty,
                ShortfallReason::Empty,
                EvidenceOutcomeState::Empty,
            ),
            (
                RecoveryKind::Unavailable,
                ShortfallReason::Unavailable,
                EvidenceOutcomeState::Unavailable,
            ),
            (
                RecoveryKind::Denied,
                ShortfallReason::Denied,
                EvidenceOutcomeState::Denied,
            ),
        ] {
            assert_eq!(
                EvidenceOutcomeEvent::from_validation(&recovery(kind, reason)).state,
                expected
            );
        }
    }

    #[test]
    fn web_verified_partial_and_verification_shortfall_are_terminal_shapes() {
        let make_bundle = |completeness| {
            ValidationOutcome::Bundle(Box::new(EvidenceBundle {
                version: EVIDENCE_SCHEMA_VERSION,
                turn_id: "turn-web".to_string(),
                intent: EvidenceIntent::WebFact {
                    query: "fixture".to_string(),
                    verification: super::super::VerificationLevel::Corroborated,
                },
                completeness,
                requested: EvidenceCounts {
                    mail_headers: 0,
                    mail_bodies: 0,
                    web_sources: 2,
                },
                acquired: EvidenceCounts {
                    mail_headers: 0,
                    mail_bodies: 0,
                    web_sources: if completeness == Completeness::Complete {
                        2
                    } else {
                        1
                    },
                },
                missing: Vec::new(),
                mail: Vec::new(),
                web: Vec::new(),
                conflicts: Vec::new(),
                exclusions: Vec::new(),
                citation_allowlist: Vec::new(),
            }))
        };
        assert_eq!(
            EvidenceOutcomeEvent::from_validation(&make_bundle(Completeness::Complete)).state,
            EvidenceOutcomeState::Verified
        );
        assert_eq!(
            EvidenceOutcomeEvent::from_validation(&make_bundle(Completeness::Partial)).state,
            EvidenceOutcomeState::Partial
        );

        let ValidationOutcome::Bundle(mut conflict_bundle) = make_bundle(Completeness::Complete)
        else {
            unreachable!();
        };
        conflict_bundle
            .conflicts
            .push(super::super::EvidenceConflict {
                evidence_ids: vec![
                    super::super::EvidenceId::new("web-1").unwrap(),
                    super::super::EvidenceId::new("web-2").unwrap(),
                ],
                description: "Figures differ".to_string(),
            });
        let conflict =
            EvidenceOutcomeEvent::from_validation(&ValidationOutcome::Bundle(conflict_bundle));
        assert_eq!(conflict.state, EvidenceOutcomeState::Conflict);
        assert_eq!(conflict.message, "Web verified · 2 sources · conflict");
        let canonical_shortfall = CanonicalGroundedAnswer {
            text: "Verification Shortfall: fewer than two claims.".to_string(),
            completeness: Completeness::Complete,
            outcome_status: CanonicalOutcomeStatus::VerificationShortfall,
            covered_evidence_ids: Vec::new(),
            citation_targets: Vec::new(),
            conflicts: Vec::new(),
            shortfalls: Vec::new(),
            source_identities: Vec::new(),
        };
        let canonical_outcome = conflict.with_canonical_answer(&canonical_shortfall);
        assert_eq!(
            canonical_outcome.state,
            EvidenceOutcomeState::VerificationShortfall
        );
        assert_eq!(canonical_outcome.message, "Couldn't verify sources");

        let shortfall = ValidationOutcome::Recovery(RecoveryOutcome {
            kind: RecoveryKind::VerificationShortfall,
            requested: EvidenceCounts {
                mail_headers: 0,
                mail_bodies: 0,
                web_sources: 2,
            },
            message: "private".to_string(),
            missing: vec![EvidenceShortfall {
                requirement: EvidenceRequirement::FetchedSources { count: 2 },
                missing_count: 2,
                reason: ShortfallReason::VerificationFailed,
            }],
            exclusions: Vec::new(),
        });
        let event = EvidenceOutcomeEvent::from_validation(&shortfall);
        assert_eq!(event.state, EvidenceOutcomeState::VerificationShortfall);
        assert_eq!(event.message, "Couldn't verify sources");
    }

    #[test]
    fn retry_and_duplicate_metadata_stay_on_the_original_logical_activity() {
        let operation = EvidenceOperation::MailRead {
            message_id: super::super::ValidatedMailId::new("fixture-id").unwrap(),
        };
        let completion = LogicalActivityCompletion {
            execution: ExecutionStatus::Succeeded,
            contribution: EvidenceContribution::Satisfied,
            evidence_count: 1,
            source_domains: Vec::new(),
            duration_ms: 42,
            attempt_count: 2,
            duplicates_suppressed: 1,
            body_origin: None,
        };
        let started = LogicalActivityEvent::started("turn", &operation);
        let completed = LogicalActivityEvent::completed("turn", &operation, &completion);
        assert_eq!(started.activity_id, completed.activity_id);
        assert_eq!(completed.retries, 1);
        assert_eq!(completed.duplicates_suppressed, 1);
    }

    #[test]
    fn successful_empty_contribution_never_increments_evidence_progress() {
        let operation = EvidenceOperation::WebFetch {
            candidate_id: super::super::CandidateId::new("candidate").unwrap(),
        };
        let completion = LogicalActivityCompletion {
            execution: ExecutionStatus::Succeeded,
            contribution: EvidenceContribution::Empty,
            evidence_count: 0,
            source_domains: Vec::new(),
            duration_ms: 1,
            attempt_count: 1,
            duplicates_suppressed: 0,
            body_origin: None,
        };
        let event = LogicalActivityEvent::completed("turn", &operation, &completion);
        assert_eq!(event.execution_status, LogicalActivityState::Succeeded);
        assert_eq!(event.contribution, EvidenceContribution::Empty);
        assert_eq!(event.evidence_count, 0);
    }

    #[test]
    fn mail_body_origin_is_structural_logical_activity_metadata() {
        let operation = EvidenceOperation::MailRead {
            message_id: super::super::ValidatedMailId::new("fixture-id").unwrap(),
        };
        let completion = LogicalActivityCompletion {
            execution: ExecutionStatus::Succeeded,
            contribution: EvidenceContribution::Satisfied,
            evidence_count: 1,
            source_domains: Vec::new(),
            duration_ms: 12,
            attempt_count: 1,
            duplicates_suppressed: 0,
            body_origin: Some(BodyOrigin::MailAutomation),
        };

        let event = LogicalActivityEvent::completed("turn", &operation, &completion);

        assert_eq!(event.body_origin, Some(BodyOrigin::MailAutomation));
        let serialized = serde_json::to_value(event).unwrap();
        assert_eq!(serialized["body_origin"], "mail_automation");
    }

    #[test]
    fn chat_and_automation_use_origin_independent_event_semantics() {
        let operation = EvidenceOperation::MailList {
            limit: 3,
            unread_only: false,
        };
        assert_eq!(
            LogicalActivityEvent::started("turn", &operation),
            LogicalActivityEvent::started("turn", &operation)
        );
    }

    #[test]
    fn every_required_phase_serializes_to_the_public_contract() {
        let phases = [
            EvidencePhase::FindingMail,
            EvidencePhase::Reading,
            EvidencePhase::Searching,
            EvidencePhase::Verifying,
            EvidencePhase::LoadingSynthesisModel,
            EvidencePhase::PreparingAnswer,
            EvidencePhase::Repairing,
            EvidencePhase::FallingBack,
            EvidencePhase::Validating,
            EvidencePhase::DeterministicRendering,
        ];
        let serialized = phases
            .into_iter()
            .map(|phase| {
                serde_json::to_value(EvidencePhaseEvent::acquisition(
                    "turn",
                    phase,
                    Some(0),
                    Some(1),
                ))
                .unwrap()["phase"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            serialized,
            [
                "finding_mail",
                "reading",
                "searching",
                "verifying",
                "loading_synthesis_model",
                "preparing_answer",
                "repairing",
                "falling_back",
                "validating",
                "deterministic_rendering",
            ]
        );
    }
}
