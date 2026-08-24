use std::collections::{HashMap, HashSet};

use super::{
    BodyState, CitationTarget, Completeness, EvidenceBundle, EvidenceContribution, EvidenceCounts,
    EvidenceExclusion, EvidenceIntent, EvidencePlan, EvidenceRequirement, EvidenceResults,
    EvidenceShortfall, ExecutionStatus, ExtractionStatus, FailureCode, MailBundleItem,
    MailHeaderEvidence, OperationResult, RecoveryKind, RecoveryOutcome, ShortfallReason,
    SourceAuthority, ValidationOutcome, WebBundleItem, EVIDENCE_SCHEMA_VERSION,
};

pub(crate) struct EvidenceValidator;

impl EvidenceValidator {
    pub(crate) fn validate(
        turn_id: &str,
        plan: &EvidencePlan,
        results: EvidenceResults,
    ) -> ValidationOutcome {
        let (intent, allow_instruction_analysis) = match &plan.intent {
            EvidenceIntent::AnalyzeQuotedEvidence { intent } => (intent.as_ref(), true),
            intent => (intent, false),
        };
        match intent {
            EvidenceIntent::MailLatestHeaders { count, .. } => {
                validate_mail_headers(turn_id, plan, results, *count)
            }
            EvidenceIntent::MailLatestContent {
                count,
                requested_count,
                ..
            } => validate_mail_content(
                turn_id,
                plan,
                results,
                *count,
                *requested_count,
                allow_instruction_analysis,
            ),
            EvidenceIntent::MailTargeted { needs_content, .. } => validate_targeted_mail(
                turn_id,
                plan,
                results,
                *needs_content,
                allow_instruction_analysis,
            ),
            EvidenceIntent::WebDirectPage { .. } => validate_web(
                turn_id,
                plan,
                intent,
                results,
                1,
                allow_instruction_analysis,
            ),
            EvidenceIntent::WebFact { verification, .. } => validate_web(
                turn_id,
                plan,
                intent,
                results,
                match verification {
                    super::VerificationLevel::SingleAuthoritative => 1,
                    super::VerificationLevel::Corroborated => 2,
                },
                allow_instruction_analysis,
            ),
            EvidenceIntent::AnalyzeQuotedEvidence { .. } => {
                unreachable!("quoted analysis intents are unwrapped before validation")
            }
        }
    }
}

fn validate_mail_headers(
    turn_id: &str,
    plan: &EvidencePlan,
    results: EvidenceResults,
    requested: u8,
) -> ValidationOutcome {
    let headers = distinct_headers(
        results
            .mail_list
            .iter()
            .filter(|result| usable_mail_headers(result))
            .filter_map(|result| result.value.as_ref())
            .flatten()
            .cloned(),
    );
    if headers.is_empty() {
        return mail_recovery(
            &results,
            EvidenceCounts {
                mail_headers: requested,
                ..Default::default()
            },
            mail_header_shortfalls(&results, requested, 0),
            Vec::new(),
        );
    }
    let acquired = usize_to_u8(headers.len()).min(requested);
    let missing = mail_header_shortfalls(&results, requested, acquired);
    bundle(
        turn_id,
        plan,
        ValidatedEvidence {
            requested: EvidenceCounts {
                mail_headers: requested,
                ..Default::default()
            },
            acquired: EvidenceCounts {
                mail_headers: acquired,
                ..Default::default()
            },
            missing,
            mail: headers
                .into_iter()
                .take(requested.into())
                .map(|header| mail_bundle_item(header, None))
                .collect(),
            web: Vec::new(),
            exclusions: Vec::new(),
            conflicts: results.conflicts,
        },
    )
}

fn validate_mail_content(
    turn_id: &str,
    plan: &EvidencePlan,
    results: EvidenceResults,
    batch_count: u8,
    requested_count: u8,
    allow_instruction_analysis: bool,
) -> ValidationOutcome {
    let headers = distinct_headers(
        results
            .mail_list
            .iter()
            .filter(|result| usable_mail_headers(result))
            .filter_map(|result| result.value.as_ref())
            .flatten()
            .cloned(),
    );
    let header_by_evidence = headers
        .iter()
        .cloned()
        .map(|header| (header.evidence_id.clone(), header))
        .collect::<HashMap<_, _>>();
    let mut seen_bodies = HashSet::new();
    let readable = results
        .mail_bodies
        .iter()
        .filter(|result| matches!(result.execution, ExecutionStatus::Succeeded))
        .filter_map(|result| result.value.as_ref())
        .filter(|body| body.body_state == BodyState::Readable && !body.body.trim().is_empty())
        .filter(|body| header_by_evidence.contains_key(&body.header_id))
        .filter(|body| seen_bodies.insert(body.header_id.clone()))
        .cloned()
        .collect::<Vec<_>>();
    let (excluded, readable): (Vec<_>, Vec<_>) = readable
        .into_iter()
        .partition(|body| !allow_instruction_analysis && contains_instruction_like(&body.body));
    let exclusions = excluded
        .iter()
        .map(|body| EvidenceExclusion {
            evidence_id: body.evidence_id.clone(),
            reason: "instruction-like content excluded from ordinary synthesis".to_string(),
        })
        .collect::<Vec<_>>();
    let missing = mail_body_shortfalls(
        &results,
        requested_count,
        usize_to_u8(readable.len()),
        usize_to_u8(excluded.len()),
        batch_count,
        invalid_mail_header_count(&results),
    );

    if readable.is_empty() {
        return mail_recovery(
            &results,
            EvidenceCounts {
                mail_headers: requested_count,
                mail_bodies: requested_count,
                ..Default::default()
            },
            missing,
            exclusions,
        );
    }
    let acquired_bodies = usize_to_u8(readable.len()).min(batch_count);
    let acquired_headers = usize_to_u8(headers.len()).min(batch_count);
    let mail = readable
        .into_iter()
        .take(batch_count.into())
        .filter_map(|body| {
            header_by_evidence
                .get(&body.header_id)
                .cloned()
                .map(|header| mail_bundle_item(header, Some(body)))
        })
        .collect();
    bundle(
        turn_id,
        plan,
        ValidatedEvidence {
            requested: EvidenceCounts {
                mail_headers: requested_count,
                mail_bodies: requested_count,
                ..Default::default()
            },
            acquired: EvidenceCounts {
                mail_headers: acquired_headers,
                mail_bodies: acquired_bodies,
                ..Default::default()
            },
            missing,
            mail,
            web: Vec::new(),
            exclusions,
            conflicts: results.conflicts,
        },
    )
}

fn validate_targeted_mail(
    turn_id: &str,
    plan: &EvidencePlan,
    results: EvidenceResults,
    needs_content: bool,
    allow_instruction_analysis: bool,
) -> ValidationOutcome {
    let headers = distinct_headers(
        results
            .mail_search
            .iter()
            .filter(|result| usable_mail_headers(result))
            .filter_map(|result| result.value.as_ref())
            .flatten()
            .cloned(),
    );
    if headers.len() > 1 {
        return ValidationOutcome::Clarification {
            headers,
            prompt: "Multiple messages match. Which one should I use?".to_string(),
        };
    }
    if headers.is_empty() {
        return mail_recovery(
            &results,
            EvidenceCounts {
                mail_headers: 1,
                mail_bodies: u8::from(needs_content),
                ..Default::default()
            },
            shortfall(
                EvidenceRequirement::TargetedMail { needs_content },
                1,
                ShortfallReason::Empty,
            ),
            Vec::new(),
        );
    }
    if needs_content {
        return validate_mail_content(
            turn_id,
            plan,
            EvidenceResults {
                mail_list: results.mail_search,
                mail_bodies: results.mail_bodies,
                conflicts: results.conflicts,
                ..Default::default()
            },
            1,
            1,
            allow_instruction_analysis,
        );
    }
    bundle(
        turn_id,
        plan,
        ValidatedEvidence {
            requested: EvidenceCounts {
                mail_headers: 1,
                ..Default::default()
            },
            acquired: EvidenceCounts {
                mail_headers: 1,
                ..Default::default()
            },
            missing: Vec::new(),
            mail: vec![mail_bundle_item(
                headers.into_iter().next().expect("one targeted header"),
                None,
            )],
            web: Vec::new(),
            exclusions: Vec::new(),
            conflicts: results.conflicts,
        },
    )
}

fn validate_web(
    turn_id: &str,
    plan: &EvidencePlan,
    intent: &EvidenceIntent,
    results: EvidenceResults,
    requested_sources: u8,
    allow_instruction_analysis: bool,
) -> ValidationOutcome {
    let searched_candidates = results
        .web_searches
        .iter()
        .filter(|result| matches!(result.execution, ExecutionStatus::Succeeded))
        .filter_map(|result| result.value.as_ref())
        .flat_map(|search| search.candidates.iter())
        .map(|candidate| {
            (
                candidate.candidate_id.clone(),
                candidate.requested_url.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let validated_exploration_urls = results
        .web_fetches
        .iter()
        .filter_map(|result| result.value.as_ref())
        .flat_map(|evidence| evidence.links.iter())
        .map(|reference| reference.url.clone())
        .collect::<HashSet<_>>();
    let mut seen_sources = HashSet::new();
    let mut exclusions = Vec::new();
    let fetched = results
        .web_fetches
        .iter()
        .filter(|result| matches!(result.execution, ExecutionStatus::Succeeded))
        .filter(|result| {
            matches!(
                result.contribution,
                EvidenceContribution::Satisfied | EvidenceContribution::Partial
            )
        })
        .filter_map(|result| result.value.as_ref())
        .filter(|evidence| match intent {
            EvidenceIntent::WebDirectPage { url } => evidence.requested_url == *url,
            EvidenceIntent::WebFact { verification, .. } => {
                (searched_candidates
                    .get(&evidence.candidate_id)
                    .is_some_and(|url| *url == evidence.requested_url)
                    || validated_exploration_urls.contains(&evidence.requested_url))
                    && (matches!(verification, super::VerificationLevel::Corroborated)
                        || evidence.authority == SourceAuthority::FirstParty)
            }
            _ => false,
        })
        .filter(|evidence| {
            matches!(
                evidence.extraction,
                ExtractionStatus::Readable | ExtractionStatus::ReadableTruncated
            ) && evidence.quality.low_quality_reason.is_none()
                && evidence
                    .passages
                    .iter()
                    .any(|passage| !passage.text.trim().is_empty())
        })
        .filter_map(|evidence| {
            let mut sanitized = evidence.clone();
            sanitized.passages.retain(|passage| {
                let excluded =
                    !allow_instruction_analysis && contains_instruction_like(&passage.text);
                if excluded {
                    exclusions.push(EvidenceExclusion {
                        evidence_id: passage.passage_id.clone(),
                        reason: "instruction-like content excluded from ordinary synthesis"
                            .to_string(),
                    });
                }
                !excluded
            });
            (!sanitized.passages.is_empty()).then_some(sanitized)
        })
        .filter(|evidence| {
            evidence.final_url.host_str().is_some()
                && seen_sources.insert(evidence.source_identity.clone())
        })
        .collect::<Vec<_>>();
    if fetched.is_empty() {
        let fetched_links = results
            .web_fetches
            .iter()
            .filter(|result| matches!(result.execution, ExecutionStatus::Succeeded))
            .filter_map(|result| result.value.as_ref())
            .filter(|evidence| match intent {
                EvidenceIntent::WebDirectPage { url } => evidence.requested_url == *url,
                EvidenceIntent::WebFact { .. } => {
                    searched_candidates
                        .get(&evidence.candidate_id)
                        .is_some_and(|url| *url == evidence.requested_url)
                        || validated_exploration_urls.contains(&evidence.requested_url)
                }
                _ => false,
            })
            .filter(|evidence| evidence.final_url.host_str().is_some())
            .map(|evidence| evidence.final_url.clone())
            .collect::<HashSet<_>>();
        let source_suffix = if fetched_links.is_empty() {
            String::new()
        } else {
            let mut links = fetched_links
                .into_iter()
                .map(|url| format!("[{}]({url})", url.host_str().expect("filtered source host")))
                .collect::<Vec<_>>();
            links.sort();
            format!(" Sources fetched but not usable: {}.", links.join(", "))
        };
        return ValidationOutcome::Recovery(RecoveryOutcome {
            kind: RecoveryKind::VerificationShortfall,
            requested: EvidenceCounts {
                web_sources: requested_sources,
                ..Default::default()
            },
            message: format!(
                "Verification Shortfall: I couldn't verify this request from fetched page evidence.{source_suffix} You can retry or provide a direct authoritative URL."
            ),
            missing: shortfall(
                EvidenceRequirement::FetchedSources {
                    count: requested_sources,
                },
                requested_sources,
                ShortfallReason::VerificationFailed,
            ),
            exclusions,
        });
    }
    let acquired = usize_to_u8(fetched.len()).min(requested_sources);
    let missing = shortfall(
        EvidenceRequirement::FetchedSources {
            count: requested_sources,
        },
        requested_sources.saturating_sub(acquired),
        ShortfallReason::VerificationFailed,
    );
    let citation_allowlist = fetched
        .iter()
        .take(requested_sources.into())
        .map(|evidence| CitationTarget {
            evidence_id: evidence.evidence_id.clone(),
            url: evidence.final_url.clone(),
        })
        .collect();
    let web = fetched
        .into_iter()
        .take(requested_sources.into())
        .map(|evidence| WebBundleItem { evidence })
        .collect();
    let mut outcome = bundle(
        turn_id,
        plan,
        ValidatedEvidence {
            requested: EvidenceCounts {
                web_sources: requested_sources,
                ..Default::default()
            },
            acquired: EvidenceCounts {
                web_sources: acquired,
                ..Default::default()
            },
            missing,
            mail: Vec::new(),
            web,
            exclusions,
            conflicts: results.conflicts,
        },
    );
    if let ValidationOutcome::Bundle(bundle) = &mut outcome {
        bundle.citation_allowlist = citation_allowlist;
    }
    outcome
}

fn distinct_headers(
    headers: impl IntoIterator<Item = MailHeaderEvidence>,
) -> Vec<MailHeaderEvidence> {
    let mut seen = HashSet::new();
    headers
        .into_iter()
        .filter(|header| seen.insert(header.connector_id.clone()))
        .collect()
}

fn usable_mail_headers(result: &OperationResult<Vec<MailHeaderEvidence>>) -> bool {
    matches!(result.execution, ExecutionStatus::Succeeded)
        || matches!(
            result.execution,
            ExecutionStatus::Failed(FailureCode::ParseFailure)
        ) && result.contribution == EvidenceContribution::Partial
}

fn invalid_mail_header_count(results: &EvidenceResults) -> u8 {
    results
        .mail_list
        .iter()
        .chain(results.mail_search.iter())
        .map(|result| result.invalid_items)
        .fold(0u8, u8::saturating_add)
}

fn mail_header_shortfalls(
    results: &EvidenceResults,
    requested_count: u8,
    acquired_count: u8,
) -> Vec<EvidenceShortfall> {
    let requirement = EvidenceRequirement::MailHeaders {
        count: requested_count,
    };
    let mut remaining = requested_count.saturating_sub(acquired_count);
    let malformed = invalid_mail_header_count(results).min(remaining);
    let mut missing = Vec::new();
    if malformed > 0 {
        missing.push(EvidenceShortfall {
            requirement: requirement.clone(),
            missing_count: malformed,
            reason: ShortfallReason::Malformed,
        });
        remaining = remaining.saturating_sub(malformed);
    }
    if remaining > 0 {
        missing.push(EvidenceShortfall {
            requirement,
            missing_count: remaining,
            reason: ShortfallReason::Empty,
        });
    }
    missing
}

fn mail_recovery(
    results: &EvidenceResults,
    requested: EvidenceCounts,
    missing: Vec<EvidenceShortfall>,
    exclusions: Vec<EvidenceExclusion>,
) -> ValidationOutcome {
    let executions = results
        .mail_list
        .iter()
        .map(|result| &result.execution)
        .chain(results.mail_search.iter().map(|result| &result.execution))
        .chain(results.mail_bodies.iter().map(|result| &result.execution))
        .collect::<Vec<_>>();
    let kind = if executions
        .iter()
        .any(|status| matches!(status, ExecutionStatus::Denied))
    {
        RecoveryKind::Denied
    } else if executions
        .iter()
        .any(|status| matches!(status, ExecutionStatus::Failed(FailureCode::InvalidInput)))
    {
        RecoveryKind::InvalidInput
    } else if executions
        .iter()
        .any(|status| matches!(status, ExecutionStatus::Failed(FailureCode::ParseFailure)))
    {
        RecoveryKind::Malformed
    } else if executions.iter().any(|status| {
        matches!(
            status,
            ExecutionStatus::Failed(_) | ExecutionStatus::TimedOut
        )
    }) || results.mail_bodies.iter().any(|result| {
        result
            .value
            .as_ref()
            .is_some_and(|body| body.body_state == BodyState::UnavailableLocally)
    }) {
        RecoveryKind::Unavailable
    } else if results.mail_bodies.iter().any(|result| {
        result
            .value
            .as_ref()
            .is_some_and(|body| body.body_state == BodyState::Empty)
    }) || !exclusions.is_empty()
    {
        RecoveryKind::NoUsableEvidence
    } else {
        RecoveryKind::Empty
    };
    let message = match kind {
        RecoveryKind::Empty => {
            "The Mail query succeeded but returned no matching evidence. You can broaden the request or try again later."
        }
        RecoveryKind::InvalidInput => {
            "The Mail request was invalid and was not executed. Check the search terms or message selection and try again."
        }
        RecoveryKind::Malformed => {
            "Mail returned malformed message metadata that could not be used safely. Refresh Mail and try again."
        }
        RecoveryKind::Denied => {
            "Mail access was denied. You can approve access and retry when you are ready."
        }
        RecoveryKind::Unavailable => {
            "Mail or the requested message body is currently unavailable. You can retry after Mail finishes syncing."
        }
        RecoveryKind::NoUsableEvidence => {
            "Mail returned no content safe for ordinary synthesis. You can explicitly ask me to analyze the instructions as quoted data."
        }
        RecoveryKind::VerificationShortfall => unreachable!(),
    };
    ValidationOutcome::Recovery(RecoveryOutcome {
        kind,
        requested,
        message: message.to_string(),
        missing,
        exclusions,
    })
}

fn mail_body_shortfalls(
    results: &EvidenceResults,
    requested_count: u8,
    acquired_count: u8,
    excluded_count: u8,
    batch_count: u8,
    malformed_count: u8,
) -> Vec<EvidenceShortfall> {
    let requirement = EvidenceRequirement::MailBodies {
        count: requested_count,
    };
    let mut remaining = requested_count.saturating_sub(acquired_count);
    let mut missing = Vec::new();
    let categories = [
        (
            requested_count.saturating_sub(batch_count),
            ShortfallReason::BatchLimit,
        ),
        (malformed_count, ShortfallReason::Malformed),
        (
            usize_to_u8(
                results
                    .mail_bodies
                    .iter()
                    .filter(|result| result.execution == ExecutionStatus::Denied)
                    .count(),
            ),
            ShortfallReason::Denied,
        ),
        (
            usize_to_u8(
                results
                    .mail_bodies
                    .iter()
                    .filter_map(|result| result.value.as_ref())
                    .filter(|body| body.body_state == BodyState::UnavailableLocally)
                    .count(),
            ),
            ShortfallReason::BodyUnavailable,
        ),
        (
            usize_to_u8(
                results
                    .mail_bodies
                    .iter()
                    .filter_map(|result| result.value.as_ref())
                    .filter(|body| {
                        body.body_state == BodyState::Empty
                            || (body.body_state == BodyState::Readable
                                && body.body.trim().is_empty())
                    })
                    .count(),
            ),
            ShortfallReason::Empty,
        ),
        (
            usize_to_u8(
                results
                    .mail_bodies
                    .iter()
                    .filter(|result| {
                        result.contribution == EvidenceContribution::Duplicate
                            && result.attempts == 0
                    })
                    .count(),
            ),
            ShortfallReason::Duplicate,
        ),
        (excluded_count, ShortfallReason::ExcludedAsInstruction),
    ];
    for (count, reason) in categories {
        let count = count.min(remaining);
        if count > 0 {
            missing.push(EvidenceShortfall {
                requirement: requirement.clone(),
                missing_count: count,
                reason,
            });
            remaining = remaining.saturating_sub(count);
        }
    }
    if remaining > 0 {
        missing.push(EvidenceShortfall {
            requirement,
            missing_count: remaining,
            reason: ShortfallReason::Empty,
        });
    }
    missing
}

fn shortfall(
    requirement: EvidenceRequirement,
    missing_count: u8,
    reason: ShortfallReason,
) -> Vec<EvidenceShortfall> {
    if missing_count == 0 {
        Vec::new()
    } else {
        vec![EvidenceShortfall {
            requirement,
            missing_count,
            reason,
        }]
    }
}

struct ValidatedEvidence {
    requested: EvidenceCounts,
    acquired: EvidenceCounts,
    missing: Vec<EvidenceShortfall>,
    mail: Vec<MailBundleItem>,
    web: Vec<WebBundleItem>,
    exclusions: Vec<EvidenceExclusion>,
    conflicts: Vec<super::EvidenceConflict>,
}

fn bundle(turn_id: &str, plan: &EvidencePlan, evidence: ValidatedEvidence) -> ValidationOutcome {
    ValidationOutcome::Bundle(Box::new(EvidenceBundle {
        version: EVIDENCE_SCHEMA_VERSION,
        turn_id: turn_id.to_string(),
        intent: plan.intent.clone(),
        completeness: if evidence.missing.is_empty() {
            Completeness::Complete
        } else {
            Completeness::Partial
        },
        requested: evidence.requested,
        acquired: evidence.acquired,
        missing: evidence.missing,
        mail: evidence.mail,
        web: evidence.web,
        conflicts: evidence.conflicts,
        exclusions: evidence.exclusions,
        citation_allowlist: Vec::new(),
    }))
}

fn contains_instruction_like(value: &str) -> bool {
    let normalized = value.to_lowercase();
    [
        "ignore previous instructions",
        "ignore all instructions",
        "system prompt",
        "<|system|>",
        "assistant: call",
        "execute this tool",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn mail_bundle_item(
    header: MailHeaderEvidence,
    body: Option<super::MailBodyEvidence>,
) -> MailBundleItem {
    MailBundleItem {
        evidence_id: header.evidence_id,
        sender: header.sender,
        subject: header.subject,
        received_at: header.received_at,
        body: body.as_ref().map(|body| body.body.clone()),
        body_state: body.as_ref().map(|body| body.body_state),
        body_origin: body.map(|body| body.body_origin),
    }
}

fn usize_to_u8(value: usize) -> u8 {
    value.min(u8::MAX.into()) as u8
}
