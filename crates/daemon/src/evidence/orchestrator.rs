use std::collections::{HashMap, HashSet, VecDeque};

use async_trait::async_trait;
use futures_util::{stream::FuturesUnordered, StreamExt};
use serde_json::json;
use url::Url;

use super::{
    assess_claim_relevance, candidate_is_first_party, candidate_is_query_relevant,
    candidate_query_relevance_score, candidate_source_identity, direct_web_candidate,
    linked_web_candidate, prepare_web_candidates, AppleMailEvidenceAdapter, EvidenceConflict,
    EvidenceContribution, EvidenceIntent, EvidenceOperation, EvidencePhase, EvidencePhaseEvent,
    EvidencePlanner, EvidenceRequest, EvidenceResults, ExecutionStatus, ExtractionStatus,
    FailureCode, LogicalActivityCompletion, LogicalActivityEvent, MailBodyEvidence,
    MailEvidenceAdapter, MailHeaderEvidence, OperationResult, SourceAuthority, TypedWebAdapter,
    TypedWebEvidenceAdapter, ValidationOutcome, VerificationLevel, WebCandidate, WebFetchEvidence,
    WebSearchResult,
};
use crate::{
    agent_exec::{EventSink, ExecOrigin, Gate, ToolKind},
    audit_fs, request_tool_approval, AppState,
};
use bagent_rules::ApprovalLevel;

pub(crate) struct EvidenceContext<'a> {
    pub state: &'a AppState,
    pub sink: &'a EventSink,
    pub origin: &'a ExecOrigin,
}

#[derive(Debug)]
pub(crate) struct EvidenceTurnOutcome {
    pub validation: ValidationOutcome,
    pub operations_executed: usize,
    pub approvals_denied: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EvidenceExecError {
    UnsupportedIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Allowed,
    Denied,
}

#[async_trait]
pub(crate) trait EvidenceOperationGate {
    async fn admit(&mut self, operation: &EvidenceOperation) -> Admission;

    async fn admit_web_fetch(
        &mut self,
        operation: &EvidenceOperation,
        _validated_url: &Url,
    ) -> Admission {
        self.admit(operation).await
    }

    async fn record_execution(&mut self, _operation: &EvidenceOperation) {}

    async fn record_activity_started(&mut self, _operation: &EvidenceOperation) {}

    async fn record_completion(
        &mut self,
        _operation: &EvidenceOperation,
        _completion: LogicalActivityCompletion,
    ) {
    }

    async fn record_phase(
        &mut self,
        _phase: EvidencePhase,
        _completed: Option<u16>,
        _total: Option<u16>,
    ) {
    }

    async fn record_acquisition_diagnostic(&mut self, _event: serde_json::Value) {}
}

struct ExistingPolicyGate<'a> {
    state: &'a AppState,
    sink: &'a EventSink,
    origin: &'a ExecOrigin,
    turn_id: &'a str,
    started_activities: HashSet<super::OperationKey>,
    completed_activities: HashMap<super::OperationKey, LogicalActivityCompletion>,
    duplicate_suppressions: HashMap<super::OperationKey, u8>,
}

#[async_trait]
impl EvidenceOperationGate for ExistingPolicyGate<'_> {
    async fn record_acquisition_diagnostic(&mut self, mut event: serde_json::Value) {
        event["type"] = json!("evidence_acquisition_diagnostic");
        event["turn_id"] = json!(self.turn_id);
        let _ = self.sink.emit(event).await;
    }
    async fn admit(&mut self, operation: &EvidenceOperation) -> Admission {
        let args = operation_args(operation);
        let rule_name = match operation {
            EvidenceOperation::WebSearch { .. } => "web.search",
            EvidenceOperation::WebFetch { .. } => return Admission::Denied,
            _ => "mail_inbox",
        };
        self.admit_rule(rule_name, &args, operation).await
    }

    async fn admit_web_fetch(
        &mut self,
        operation: &EvidenceOperation,
        validated_url: &Url,
    ) -> Admission {
        self.admit_rule(
            "web.fetch",
            &json!({"url": validated_url.as_str()}),
            operation,
        )
        .await
    }

    async fn record_activity_started(&mut self, operation: &EvidenceOperation) {
        if self.started_activities.insert(operation.key()) {
            let _ = self
                .sink
                .emit(
                    serde_json::to_value(LogicalActivityEvent::started(self.turn_id, operation))
                        .expect("logical activity start is serializable"),
                )
                .await;
        }
    }

    async fn record_execution(&mut self, operation: &EvidenceOperation) {
        let tool = operation_tool_name(operation);
        audit_fs(
            &self.state.db,
            "tool_call",
            &json!({
                "tool": tool,
                "unattended": self.origin.unattended(),
                "orchestrated": "evidence",
            }),
        );
    }

    async fn record_completion(
        &mut self,
        operation: &EvidenceOperation,
        mut completion: LogicalActivityCompletion,
    ) {
        let key = operation.key();
        if completion.contribution == EvidenceContribution::Duplicate {
            let suppressed = self.duplicate_suppressions.entry(key.clone()).or_default();
            *suppressed = suppressed.saturating_add(completion.duplicates_suppressed.max(1));
            if let Some(original) = self.completed_activities.get(&key) {
                completion = original.clone();
            }
            completion.duplicates_suppressed = *suppressed;
        } else {
            completion.duplicates_suppressed = completion
                .duplicates_suppressed
                .saturating_add(*self.duplicate_suppressions.get(&key).unwrap_or(&0));
            self.completed_activities.insert(key, completion.clone());
        }
        let _ = self
            .sink
            .emit(
                serde_json::to_value(LogicalActivityEvent::completed(
                    self.turn_id,
                    operation,
                    &completion,
                ))
                .expect("logical activity completion is serializable"),
            )
            .await;
    }

    async fn record_phase(
        &mut self,
        phase: EvidencePhase,
        completed: Option<u16>,
        total: Option<u16>,
    ) {
        let _ = self
            .sink
            .emit(
                serde_json::to_value(EvidencePhaseEvent::acquisition(
                    self.turn_id,
                    phase,
                    completed,
                    total,
                ))
                .expect("evidence phase is serializable"),
            )
            .await;
    }
}

impl ExistingPolicyGate<'_> {
    async fn admit_rule(
        &self,
        rule_name: &str,
        args: &serde_json::Value,
        operation: &EvidenceOperation,
    ) -> Admission {
        let gate = Gate::new(&self.state.rules, self.origin);
        match gate.level(rule_name, args, ToolKind::ReadOnly) {
            ApprovalLevel::Auto => Admission::Allowed,
            ApprovalLevel::Forbidden => Admission::Denied,
            ApprovalLevel::Ask => {
                let description = match operation {
                    EvidenceOperation::WebSearch { .. } => "Web search",
                    EvidenceOperation::WebFetch { .. } => "Reading a selected web page",
                    _ => "Čítanie poštovej schránky (Apple Mail)",
                };
                let approved = request_tool_approval(
                    self.state,
                    self.sink,
                    self.origin,
                    rule_name,
                    &self.origin.describe(description),
                )
                .await;
                if approved {
                    Admission::Allowed
                } else {
                    Admission::Denied
                }
            }
        }
    }
}

pub(crate) async fn execute_evidence_turn(
    ctx: EvidenceContext<'_>,
    request: EvidenceRequest,
    intent: EvidenceIntent,
) -> Result<EvidenceTurnOutcome, EvidenceExecError> {
    let plan = EvidencePlanner::plan(intent);
    let mut gate = ExistingPolicyGate {
        state: ctx.state,
        sink: ctx.sink,
        origin: ctx.origin,
        turn_id: &request.turn_id,
        started_activities: HashSet::new(),
        completed_activities: HashMap::new(),
        duplicate_suppressions: HashMap::new(),
    };
    let outcome = if is_mail_intent(&plan.intent) {
        if let Some(connector) = ctx.state.mail.clone() {
            let mut adapter = AppleMailEvidenceAdapter::new(connector);
            execute_mail_plan(&mut adapter, &mut gate, &request.turn_id, &plan).await
        } else {
            execute_unavailable_mail_plan(&mut gate, &request.turn_id, &plan).await
        }
    } else if is_web_intent(&plan.intent) {
        let tavily_api_key = ctx.state.tavily_api_key.read().await.clone();
        execute_web_plan(
            TypedWebAdapter::production(tavily_api_key),
            &mut gate,
            &request.turn_id,
            &plan,
            "en",
        )
        .await
    } else {
        return Err(EvidenceExecError::UnsupportedIntent);
    };
    Ok(outcome)
}

fn is_mail_intent(intent: &EvidenceIntent) -> bool {
    match intent {
        EvidenceIntent::MailLatestHeaders { .. }
        | EvidenceIntent::MailLatestContent { .. }
        | EvidenceIntent::MailTargeted { .. } => true,
        EvidenceIntent::AnalyzeQuotedEvidence { intent } => is_mail_intent(intent),
        EvidenceIntent::WebDirectPage { .. } | EvidenceIntent::WebFact { .. } => false,
    }
}

fn is_web_intent(intent: &EvidenceIntent) -> bool {
    match intent {
        EvidenceIntent::WebDirectPage { .. } | EvidenceIntent::WebFact { .. } => true,
        EvidenceIntent::AnalyzeQuotedEvidence { intent } => is_web_intent(intent),
        _ => false,
    }
}

pub(crate) async fn execute_web_plan<A, G>(
    adapter: A,
    gate: &mut G,
    turn_id: &str,
    plan: &super::EvidencePlan,
    lang: &str,
) -> EvidenceTurnOutcome
where
    A: TypedWebEvidenceAdapter,
    G: EvidenceOperationGate + Send,
{
    let intent = match &plan.intent {
        EvidenceIntent::AnalyzeQuotedEvidence { intent } => intent.as_ref(),
        intent => intent,
    };
    let mut results = EvidenceResults::default();
    let mut operations_executed = 0usize;
    let mut approvals_denied = 0usize;
    let mut candidates = match intent {
        EvidenceIntent::WebDirectPage { url } => vec![direct_web_candidate(url)],
        EvidenceIntent::WebFact { query, .. } => {
            let providers = super::web::web_provider_set(adapter.tavily_configured());
            let operation = EvidenceOperation::WebSearch {
                normalized_query: query.clone(),
                provider_set: providers.clone(),
            };
            gate.record_phase(EvidencePhase::Searching, Some(0), Some(1))
                .await;
            gate.record_activity_started(&operation).await;
            let mut search = match gate.admit(&operation).await {
                Admission::Allowed => {
                    gate.record_execution(&operation).await;
                    let result = adapter.search(query, lang, &providers).await;
                    operations_executed += usize::from(result.attempts);
                    result
                }
                Admission::Denied => {
                    approvals_denied += 1;
                    let result = denied_result(&operation);
                    gate.record_completion(&operation, completion_for_search(&result))
                        .await;
                    result
                }
            };
            if !matches!(search.execution, ExecutionStatus::Denied) {
                gate.record_completion(&operation, completion_for_search(&search))
                    .await;
            }
            gate.record_phase(
                EvidencePhase::Searching,
                Some(u16::from(
                    matches!(
                        search.contribution,
                        EvidenceContribution::Satisfied | EvidenceContribution::Partial
                    ) && search
                        .value
                        .as_ref()
                        .is_some_and(|value| !value.candidates.is_empty()),
                )),
                Some(1),
            )
            .await;
            let mut candidates = search
                .value
                .as_ref()
                .map(|value| value.candidates.clone())
                .unwrap_or_default();
            prepare_web_candidates(query, &mut candidates);
            if let Some(value) = search.value.as_mut() {
                value.candidates = candidates.clone();
            }
            results.web_searches.push(search);
            if let Some(search) = results.web_searches.last() {
                record_search_diagnostics(
                    gate,
                    search,
                    candidates.len(),
                    1,
                    plan.budget.web_search_attempts,
                )
                .await;
            }
            record_candidate_diagnostics(gate, query, &candidates, plan.budget.web_fetch_attempts)
                .await;
            if plan.budget.web_search_attempts > 1
                && search_needs_diversification(intent, query, &candidates)
            {
                let diversified_query = diversified_search_query(intent, query, &candidates);
                let operation = EvidenceOperation::WebSearch {
                    normalized_query: diversified_query.clone(),
                    provider_set: providers.clone(),
                };
                gate.record_activity_started(&operation).await;
                let mut diversified = match gate.admit(&operation).await {
                    Admission::Allowed => {
                        gate.record_execution(&operation).await;
                        let result = adapter.search(&diversified_query, lang, &providers).await;
                        operations_executed += usize::from(result.attempts);
                        result
                    }
                    Admission::Denied => {
                        approvals_denied += 1;
                        denied_result(&operation)
                    }
                };
                gate.record_completion(&operation, completion_for_search(&diversified))
                    .await;
                if let Some(value) = diversified.value.as_mut() {
                    candidates.append(&mut value.candidates);
                }
                prepare_web_candidates(query, &mut candidates);
                results.web_searches.push(diversified);
                if let Some(search) = results.web_searches.last() {
                    record_search_diagnostics(
                        gate,
                        search,
                        candidates.len(),
                        2,
                        plan.budget.web_search_attempts,
                    )
                    .await;
                }
                record_candidate_diagnostics(
                    gate,
                    query,
                    &candidates,
                    plan.budget.web_fetch_attempts,
                )
                .await;
            }
            candidates
        }
        _ => {
            return EvidenceTurnOutcome {
                validation: super::EvidenceValidator::validate(turn_id, plan, results),
                operations_executed,
                approvals_denied,
            };
        }
    };
    let query = match intent {
        EvidenceIntent::WebFact { query, .. } => query.as_str(),
        EvidenceIntent::WebDirectPage { .. } => "",
        _ => "",
    };
    prepare_web_candidates(query, &mut candidates);
    let mut queue = VecDeque::from(candidates);
    let mut seen_operations = HashSet::new();
    let mut attempted_source_identities = HashSet::new();
    let mut attempts_used = 0u8;
    let mut exploration_rounds = 0u8;
    let concurrency = plan.budget.max_parallel_fetches.clamp(1, 2);

    loop {
        if web_contract_satisfied(intent, &results.web_fetches)
            || attempts_used >= plan.budget.web_fetch_attempts
        {
            break;
        }
        if queue.is_empty() {
            if exploration_rounds >= plan.budget.optional_exploration_rounds {
                break;
            }
            exploration_rounds += 1;
            let mut links = results
                .web_fetches
                .iter()
                .filter_map(|result| result.value.as_ref())
                .flat_map(|evidence| evidence.links.iter())
                .enumerate()
                .map(|(index, reference)| {
                    linked_web_candidate(
                        reference,
                        index.saturating_add(1).min(usize::from(u16::MAX)) as u16,
                    )
                })
                .collect::<Vec<_>>();
            prepare_web_candidates(query, &mut links);
            queue.extend(links);
            if queue.is_empty() {
                break;
            }
        }

        let mut inflight = FuturesUnordered::new();
        let mut scheduled = Vec::new();
        while inflight.len() < usize::from(concurrency)
            && attempts_used < plan.budget.web_fetch_attempts
        {
            let unseen_position = queue.iter().position(|candidate| {
                !attempted_source_identities.contains(&candidate_source_identity(candidate))
            });
            let candidate = if let Some(position) = unseen_position {
                queue.remove(position).expect("queued candidate position")
            } else if inflight.is_empty() {
                let Some(candidate) = queue.pop_front() else {
                    break;
                };
                candidate
            } else {
                break;
            };
            if !attempted_source_identities.insert(candidate_source_identity(&candidate))
                && matches!(
                    intent,
                    EvidenceIntent::WebFact {
                        verification: VerificationLevel::Corroborated,
                        ..
                    }
                )
                && !inflight.is_empty()
            {
                queue.push_back(candidate);
                break;
            }
            let operation = EvidenceOperation::WebFetch {
                candidate_id: candidate.candidate_id.clone(),
            };
            if !seen_operations.insert(operation.key()) {
                let duplicate = OperationResult::suppressed_duplicate(operation.key());
                gate.record_completion(&operation, completion_for_fetch(&duplicate))
                    .await;
                results.web_fetches.push(duplicate);
                continue;
            }
            gate.record_activity_started(&operation).await;
            gate.record_phase(
                EvidencePhase::Verifying,
                Some(usable_web_source_count(&results.web_fetches)),
                Some(required_web_source_count(intent)),
            )
            .await;
            match gate
                .admit_web_fetch(&operation, &candidate.requested_url)
                .await
            {
                Admission::Allowed => {
                    gate.record_acquisition_diagnostic(json!({
                        "status": "fetch_scheduled",
                        "source_identity": candidate_source_identity(&candidate).as_str(),
                        "rank": candidate.rank,
                        "authority": if candidate_is_first_party(query, &candidate) { "first_party" } else { "other" },
                        "fetch_attempts_used": attempts_used,
                        "fetch_attempt_budget": plan.budget.web_fetch_attempts,
                    })).await;
                    gate.record_execution(&operation).await;
                    attempts_used += 1;
                    operations_executed += 1;
                    scheduled.push(candidate.candidate_id.clone());
                    let task_adapter = adapter.clone();
                    inflight.push(async move {
                        let result = task_adapter.fetch(&candidate).await;
                        (candidate, result)
                    });
                }
                Admission::Denied => {
                    approvals_denied += 1;
                    let result = denied_result(&operation);
                    gate.record_completion(&operation, completion_for_fetch(&result))
                        .await;
                    results.web_fetches.push(result);
                }
            }
        }
        if inflight.is_empty() {
            continue;
        }
        let mut completed = HashMap::new();
        while let Some((candidate, mut result)) = inflight.next().await {
            apply_ranked_authority(query, &candidate, &mut result);
            apply_passage_selection(intent, &mut result);
            if result.execution.retryable()
                && result.attempts < 2
                && attempts_used < plan.budget.web_fetch_attempts
            {
                let operation = EvidenceOperation::WebFetch {
                    candidate_id: candidate.candidate_id.clone(),
                };
                match gate
                    .admit_web_fetch(&operation, &candidate.requested_url)
                    .await
                {
                    Admission::Allowed => {
                        gate.record_execution(&operation).await;
                        attempts_used += 1;
                        operations_executed += 1;
                        let retry = adapter.fetch(&candidate).await;
                        merge_retry(&mut result, retry);
                        apply_ranked_authority(query, &candidate, &mut result);
                        apply_passage_selection(intent, &mut result);
                    }
                    Admission::Denied => {
                        approvals_denied += 1;
                    }
                }
            }
            completed.insert(candidate.candidate_id, result);
        }
        for candidate_id in scheduled {
            if let Some(mut result) = completed.remove(&candidate_id) {
                if let Some(evidence) = result.value.as_ref() {
                    let duplicate_source = results.web_fetches.iter().any(|prior| {
                        prior.value.as_ref().is_some_and(|prior| {
                            prior.final_url == evidence.final_url
                                || prior.source_identity == evidence.source_identity
                        })
                    });
                    if duplicate_source {
                        result.contribution = EvidenceContribution::Duplicate;
                    }
                }
                let operation = EvidenceOperation::WebFetch {
                    candidate_id: candidate_id.clone(),
                };
                gate.record_completion(&operation, completion_for_fetch(&result))
                    .await;
                let (
                    source_identity,
                    authority,
                    relevance_score,
                    extraction_result,
                    rejection_reason,
                ) = fetch_diagnostic_fields(&result, query);
                gate.record_acquisition_diagnostic(json!({
                    "status": "fetch_completed",
                    "source_identity": source_identity,
                    "authority": authority,
                    "relevance_score": relevance_score,
                    "extraction_result": extraction_result,
                    "rejection_reason": rejection_reason,
                    "fetch_attempts_used": attempts_used,
                    "fetch_attempt_budget": plan.budget.web_fetch_attempts,
                }))
                .await;
                if let Some(evidence) = result.value.as_ref() {
                    let mut discovered = evidence
                        .links
                        .iter()
                        .enumerate()
                        .map(|(index, reference)| {
                            linked_web_candidate(
                                reference,
                                index.saturating_add(1).min(usize::from(u16::MAX)) as u16,
                            )
                        })
                        .collect::<Vec<_>>();
                    discovered.retain(|candidate| candidate_is_query_relevant(query, candidate));
                    prepare_web_candidates(query, &mut discovered);
                    for candidate in discovered.into_iter().rev() {
                        if !seen_operations.contains(
                            &EvidenceOperation::WebFetch {
                                candidate_id: candidate.candidate_id.clone(),
                            }
                            .key(),
                        ) {
                            queue.push_front(candidate);
                        }
                    }
                }
                results.web_fetches.push(result);
                gate.record_phase(
                    EvidencePhase::Verifying,
                    Some(usable_web_source_count(&results.web_fetches)),
                    Some(required_web_source_count(intent)),
                )
                .await;
            }
        }
    }
    if matches!(
        intent,
        EvidenceIntent::WebFact {
            verification: VerificationLevel::Corroborated,
            ..
        }
    ) {
        results.conflicts = detect_web_conflicts(&results.web_fetches);
    }

    EvidenceTurnOutcome {
        validation: super::EvidenceValidator::validate(turn_id, plan, results),
        operations_executed,
        approvals_denied,
    }
}

async fn record_search_diagnostics<G: EvidenceOperationGate + Send>(
    gate: &mut G,
    search: &OperationResult<WebSearchResult>,
    candidate_count: usize,
    search_attempts_used: u8,
    search_attempt_budget: u8,
) {
    if let Some(value) = &search.value {
        for provider in &value.providers {
            gate.record_acquisition_diagnostic(json!({
                "status": "search_completed",
                "provider": format!("{:?}", provider.provider).to_ascii_lowercase(),
                "provider_status": format!("{:?}", provider.status).to_ascii_lowercase(),
                "candidate_count": candidate_count,
                "duration_ms": provider.duration_ms,
                "search_attempts_used": search_attempts_used,
                "search_attempt_budget": search_attempt_budget,
            }))
            .await;
        }
    }
}

async fn record_candidate_diagnostics<G: EvidenceOperationGate + Send>(
    gate: &mut G,
    query: &str,
    candidates: &[WebCandidate],
    fetch_attempt_budget: u8,
) {
    let mut seen = HashSet::new();
    for candidate in candidates {
        let identity = candidate_source_identity(candidate);
        let duplicate_source = !seen.insert(identity.clone());
        gate.record_acquisition_diagnostic(json!({
            "status": "candidate_ranked",
            "source_identity": identity.as_str(),
            "rank": candidate.rank,
            "authority": if candidate_is_first_party(query, candidate) { "first_party" } else { "other" },
            "relevance_score": candidate_query_relevance_score(query, candidate),
            "rejection_reason": duplicate_source.then_some("duplicate_source_deferred"),
            "fetch_attempts_used": 0,
            "fetch_attempt_budget": fetch_attempt_budget,
        })).await;
    }
}

fn fetch_diagnostic_fields(
    result: &OperationResult<WebFetchEvidence>,
    query: &str,
) -> (
    Option<String>,
    &'static str,
    u16,
    &'static str,
    Option<String>,
) {
    let Some(evidence) = result.value.as_ref() else {
        return (
            None,
            "unknown",
            0,
            "no_evidence",
            Some(format!("{:?}", result.execution).to_ascii_lowercase()),
        );
    };
    let relevance = evidence
        .passages
        .iter()
        .map(|passage| assess_claim_relevance(query, &passage.text).query_coverage_basis_points)
        .max()
        .unwrap_or_default();
    let rejection = evidence
        .quality
        .low_quality_reason
        .map(|reason| format!("{reason:?}").to_ascii_lowercase());
    let authority = match evidence.authority {
        SourceAuthority::FirstParty => "first_party",
        SourceAuthority::AuthoritativeReference => "authoritative_reference",
        SourceAuthority::Other => "other",
    };
    let extraction = match evidence.extraction {
        ExtractionStatus::Readable => "readable",
        ExtractionStatus::ReadableTruncated => "readable_truncated",
        ExtractionStatus::Empty => "empty",
        ExtractionStatus::Unsupported => "unsupported",
    };
    (
        Some(evidence.source_identity.as_str().to_string()),
        authority,
        relevance,
        extraction,
        rejection,
    )
}

fn search_needs_diversification(
    intent: &EvidenceIntent,
    query: &str,
    candidates: &[WebCandidate],
) -> bool {
    match intent {
        EvidenceIntent::WebFact {
            verification: VerificationLevel::SingleAuthoritative,
            ..
        } => !candidates
            .iter()
            .any(|candidate| candidate_is_first_party(query, candidate)),
        EvidenceIntent::WebFact {
            verification: VerificationLevel::Corroborated,
            ..
        } => {
            candidates
                .iter()
                .map(candidate_source_identity)
                .collect::<HashSet<_>>()
                .len()
                < 2
        }
        _ => false,
    }
}

fn diversified_search_query(
    intent: &EvidenceIntent,
    query: &str,
    candidates: &[WebCandidate],
) -> String {
    let exclusion = candidates
        .first()
        .map(candidate_source_identity)
        .map(|identity| format!(" -site:{}", identity.as_str()))
        .unwrap_or_default();
    match intent {
        EvidenceIntent::WebFact {
            verification: VerificationLevel::SingleAuthoritative,
            ..
        } => {
            format!("{query} official{exclusion}")
        }
        EvidenceIntent::WebFact {
            verification: VerificationLevel::Corroborated,
            ..
        } => {
            let normalized = query.to_ascii_lowercase();
            let terminology = if ["population", "rate", "percent", "number", "statistics"]
                .iter()
                .any(|term| normalized.contains(term))
            {
                " official statistics"
            } else {
                " official"
            };
            format!("{query}{terminology}{exclusion}")
        }
        _ => query.to_string(),
    }
}

fn detect_web_conflicts(results: &[OperationResult<WebFetchEvidence>]) -> Vec<EvidenceConflict> {
    let usable = results
        .iter()
        .filter(|result| matches!(result.execution, ExecutionStatus::Succeeded))
        .filter_map(|result| result.value.as_ref())
        .filter(|evidence| !evidence.passages.is_empty())
        .collect::<Vec<_>>();
    for (left_index, left) in usable.iter().enumerate() {
        for right in usable.iter().skip(left_index + 1) {
            if left.source_identity == right.source_identity {
                continue;
            }
            let left_text = left
                .passages
                .iter()
                .map(|passage| passage.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            let right_text = right
                .passages
                .iter()
                .map(|passage| passage.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            let left_scalars = scalar_claim_tokens(&left_text);
            let right_scalars = scalar_claim_tokens(&right_text);
            let numeric_conflict = !left_scalars.is_empty()
                && !right_scalars.is_empty()
                && left_scalars.is_disjoint(&right_scalars);
            let negation_conflict = contains_negation(&left_text) != contains_negation(&right_text);
            if numeric_conflict || negation_conflict {
                return vec![EvidenceConflict {
                    evidence_ids: vec![left.evidence_id.clone(), right.evidence_id.clone()],
                    description: "Independent fetched sources contain unresolved differing claims."
                        .to_string(),
                }];
            }
        }
    }
    Vec::new()
}

fn scalar_claim_tokens(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|token| {
            token
                .trim_matches(|character: char| {
                    !character.is_ascii_digit() && !matches!(character, '.' | ',' | '%' | '$' | '€')
                })
                .to_string()
        })
        .filter(|token| token.chars().any(|character| character.is_ascii_digit()))
        .collect()
}

fn contains_negation(text: &str) -> bool {
    [" no ", " not ", " never ", "false", "cannot", "can't"]
        .iter()
        .any(|term| text.contains(term))
}

fn apply_ranked_authority(
    query: &str,
    candidate: &WebCandidate,
    result: &mut OperationResult<WebFetchEvidence>,
) {
    if !query.is_empty() {
        if let Some(evidence) = result.value.as_mut() {
            let mut final_candidate = candidate.clone();
            final_candidate.requested_url = evidence.final_url.clone();
            if !candidate_is_first_party(query, &final_candidate) {
                return;
            }
            evidence.authority = SourceAuthority::FirstParty;
        }
    }
}

fn apply_passage_selection(
    intent: &EvidenceIntent,
    result: &mut OperationResult<WebFetchEvidence>,
) {
    let Some(evidence) = result.value.as_mut() else {
        return;
    };
    if !matches!(result.execution, ExecutionStatus::Succeeded) {
        return;
    }
    if matches!(
        evidence.quality.low_quality_reason,
        Some(
            super::ExtractionLowQualityReason::TooLittleUsefulText
                | super::ExtractionLowQualityReason::MostlyBoilerplate
        )
    ) {
        evidence.passages.clear();
        result.contribution = EvidenceContribution::Irrelevant;
        return;
    }
    match intent {
        EvidenceIntent::WebDirectPage { .. } => {
            select_direct_page_passages(evidence);
        }
        EvidenceIntent::WebFact { query, .. } => {
            rank_fact_passages(query, evidence);
            if evidence.passages.is_empty() {
                result.contribution = EvidenceContribution::Irrelevant;
            }
        }
        _ => {}
    }
}

fn select_direct_page_passages(evidence: &mut WebFetchEvidence) {
    if evidence.passages.len() <= 6 {
        return;
    }
    let mut selected = evidence
        .passages
        .iter()
        .enumerate()
        .filter(|(_, passage)| {
            let length = passage.text.chars().count();
            (3..=120).contains(&length)
                && !passage
                    .text
                    .chars()
                    .any(|character| character.is_ascii_digit())
                && !passage.text.contains(['.', '!', '?'])
        })
        .take(2)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut descriptive = evidence
        .passages
        .iter()
        .enumerate()
        .filter(|(index, _)| !selected.contains(index))
        .map(|(index, passage)| {
            let words = passage
                .text
                .split_whitespace()
                .count()
                .min(usize::from(u16::MAX)) as u16;
            let sentences = passage
                .text
                .matches(['.', '!', '?'])
                .count()
                .min(usize::from(u8::MAX)) as u8;
            let numbers = passage
                .text
                .split_whitespace()
                .filter(|word| word.chars().any(|character| character.is_ascii_digit()))
                .count()
                .min(usize::from(u8::MAX)) as u8;
            (index, sentences, words, numbers)
        })
        .collect::<Vec<_>>();
    descriptive.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| left.3.cmp(&right.3))
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    selected.extend(
        descriptive
            .into_iter()
            .take(6usize.saturating_sub(selected.len()))
            .map(|(index, _, _, _)| index),
    );
    evidence.passages = selected
        .into_iter()
        .map(|index| evidence.passages[index].clone())
        .collect();
}

fn rank_fact_passages(query: &str, evidence: &mut WebFetchEvidence) {
    let mut contextual_headings = evidence
        .passages
        .iter()
        .filter(|passage| {
            passage.text.chars().count() <= 120
                && !passage
                    .text
                    .chars()
                    .any(|character| character.is_ascii_digit())
                && !passage.text.contains(['.', '!', '?'])
        })
        .map(|passage| {
            (
                passage.text.clone(),
                assess_claim_relevance(query, &passage.text).query_coverage_basis_points,
            )
        })
        .filter(|(_, coverage)| *coverage > 0)
        .collect::<Vec<_>>();
    contextual_headings.sort_by(|left, right| right.1.cmp(&left.1));
    for passage in &mut evidence.passages {
        if !passage
            .text
            .chars()
            .any(|character| character.is_ascii_digit())
            || assess_claim_relevance(query, &passage.text).eligible
        {
            continue;
        }
        for (heading, _) in &contextual_headings {
            let contextual = format!("{heading}: {}", passage.text);
            if contextual.chars().count() <= 1_200
                && assess_claim_relevance(query, &contextual).eligible
            {
                passage.text = contextual;
                break;
            }
        }
    }

    let mut ranked = evidence
        .passages
        .drain(..)
        .enumerate()
        .map(|(index, passage)| {
            let relevance = assess_claim_relevance(query, &passage.text);
            (passage, index, relevance)
        })
        .collect::<Vec<_>>();
    let max_coverage = ranked
        .iter()
        .map(|(_, _, relevance)| relevance.query_coverage_basis_points)
        .max()
        .unwrap_or_default();
    evidence.quality.query_coverage_basis_points = max_coverage;
    if max_coverage == 0 {
        evidence.quality.low_quality_reason =
            Some(super::ExtractionLowQualityReason::LowQueryCoverage);
        return;
    }
    if !ranked.iter().any(|(_, _, relevance)| relevance.eligible) {
        evidence.quality.low_quality_reason =
            Some(super::ExtractionLowQualityReason::NoClaimRelevantPassage);
        return;
    }

    evidence.quality.low_quality_reason = None;
    ranked.retain(|(_, _, relevance)| relevance.query_coverage_basis_points > 0);
    ranked.sort_by(|left, right| {
        right
            .2
            .query_coverage_basis_points
            .cmp(&left.2.query_coverage_basis_points)
            .then_with(|| {
                right
                    .2
                    .numeric_or_date_relevant
                    .cmp(&left.2.numeric_or_date_relevant)
            })
            .then_with(|| left.1.cmp(&right.1))
    });
    evidence.passages = ranked
        .into_iter()
        .take(6)
        .map(|(passage, _, _)| passage)
        .collect();
}

fn merge_retry(
    first: &mut OperationResult<WebFetchEvidence>,
    retry: OperationResult<WebFetchEvidence>,
) {
    first.attempts = first.attempts.saturating_add(retry.attempts);
    first.duration_ms = first.duration_ms.saturating_add(retry.duration_ms);
    first.execution = retry.execution;
    first.contribution = retry.contribution;
    first.value = retry.value;
    first.invalid_items = first.invalid_items.saturating_add(retry.invalid_items);
}

fn web_contract_satisfied(
    intent: &EvidenceIntent,
    results: &[OperationResult<WebFetchEvidence>],
) -> bool {
    let usable = results
        .iter()
        .filter(|result| matches!(result.execution, ExecutionStatus::Succeeded))
        .filter_map(|result| result.value.as_ref())
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
        .collect::<Vec<_>>();
    match intent {
        EvidenceIntent::WebDirectPage { .. } => !usable.is_empty(),
        EvidenceIntent::WebFact {
            verification: VerificationLevel::SingleAuthoritative,
            ..
        } => usable
            .iter()
            .any(|evidence| evidence.authority == SourceAuthority::FirstParty),
        EvidenceIntent::WebFact {
            verification: VerificationLevel::Corroborated,
            ..
        } => {
            usable
                .iter()
                .map(|evidence| &evidence.source_identity)
                .collect::<HashSet<_>>()
                .len()
                >= 2
        }
        _ => false,
    }
}

pub(crate) async fn execute_unavailable_mail_plan<G: EvidenceOperationGate + Send>(
    gate: &mut G,
    turn_id: &str,
    plan: &super::EvidencePlan,
) -> EvidenceTurnOutcome {
    let operation = first_mail_operation(&plan.intent);
    gate.record_phase(EvidencePhase::FindingMail, Some(0), Some(1))
        .await;
    gate.record_activity_started(&operation).await;
    let result = match gate.admit(&operation).await {
        Admission::Allowed => OperationResult::without_value(
            operation.key(),
            ExecutionStatus::Failed(FailureCode::ConnectorUnavailable),
            EvidenceContribution::Empty,
        ),
        Admission::Denied => denied_result(&operation),
    };
    gate.record_completion(&operation, completion_for_headers(&result))
        .await;
    let approvals_denied = usize::from(result.execution == ExecutionStatus::Denied);
    let mut results = EvidenceResults::default();
    push_list_or_search(&operation, result, &mut results);
    EvidenceTurnOutcome {
        validation: super::EvidenceValidator::validate(turn_id, plan, results),
        operations_executed: 0,
        approvals_denied,
    }
}

pub(crate) async fn execute_mail_plan<A, G>(
    adapter: &mut A,
    gate: &mut G,
    turn_id: &str,
    plan: &super::EvidencePlan,
) -> EvidenceTurnOutcome
where
    A: MailEvidenceAdapter + Send,
    G: EvidenceOperationGate + Send,
{
    let mut results = EvidenceResults::default();
    let mut operations_executed = 0;
    let mut approvals_denied = 0;
    let intent = match &plan.intent {
        EvidenceIntent::AnalyzeQuotedEvidence { intent } => intent.as_ref(),
        intent => intent,
    };

    let first = first_mail_operation(intent);
    gate.record_phase(EvidencePhase::FindingMail, Some(0), Some(1))
        .await;
    gate.record_activity_started(&first).await;
    let first_result = match gate.admit(&first).await {
        Admission::Allowed => {
            operations_executed += 1;
            gate.record_execution(&first).await;
            execute_adapter_operation(adapter, &first).await
        }
        Admission::Denied => {
            approvals_denied += 1;
            denied_headers_result(&first)
        }
    };
    gate.record_completion(&first, completion_for_headers(&first_result))
        .await;
    gate.record_phase(
        EvidencePhase::FindingMail,
        Some(u16::from(
            first_result.execution == ExecutionStatus::Succeeded
                && !first_result.value.as_ref().is_none_or(Vec::is_empty),
        )),
        Some(1),
    )
    .await;
    let headers = first_result.value.clone().unwrap_or_default();
    push_list_or_search(&first, first_result, &mut results);

    let needs_bodies = match intent {
        EvidenceIntent::MailLatestContent { count, .. } => *count,
        EvidenceIntent::MailTargeted {
            needs_content: true,
            ..
        } if headers.len() == 1 => 1,
        _ => 0,
    };
    let mut distinct_ids = HashSet::new();
    let mut distinct_reads_started = 0usize;
    let mut attempts_used = 0usize;
    for header in headers {
        if distinct_reads_started >= usize::from(needs_bodies)
            || attempts_used >= usize::from(plan.budget.mail_body_attempts)
        {
            break;
        }
        let operation = EvidenceOperation::MailRead {
            message_id: header.connector_id,
        };
        let EvidenceOperation::MailRead { message_id } = &operation else {
            unreachable!()
        };
        if !distinct_ids.insert(message_id.clone()) {
            let duplicate = OperationResult::suppressed_duplicate(operation.key());
            gate.record_completion(&operation, completion_for_body(&duplicate))
                .await;
            results.mail_bodies.push(duplicate);
            continue;
        }
        distinct_reads_started += 1;
        gate.record_activity_started(&operation).await;
        gate.record_phase(
            EvidencePhase::Reading,
            Some(satisfied_mail_body_count(&results.mail_bodies)),
            Some(u16::from(needs_bodies)),
        )
        .await;
        attempts_used += 1;
        let mut result = match gate.admit(&operation).await {
            Admission::Allowed => {
                operations_executed += 1;
                gate.record_execution(&operation).await;
                adapter
                    .read(match &operation {
                        EvidenceOperation::MailRead { message_id } => message_id,
                        _ => unreachable!(),
                    })
                    .await
            }
            Admission::Denied => {
                approvals_denied += 1;
                denied_result(&operation)
            }
        };
        let remaining_budget =
            usize::from(plan.budget.mail_body_attempts).saturating_sub(attempts_used);
        if result.retry_permitted(remaining_budget.min(usize::from(u8::MAX)) as u8) {
            match gate.admit(&operation).await {
                Admission::Allowed => {
                    gate.record_execution(&operation).await;
                    operations_executed += 1;
                    attempts_used += 1;
                    let prior_attempts = result.attempts;
                    let prior_duration = result.duration_ms;
                    let mut retry = adapter
                        .read(match &operation {
                            EvidenceOperation::MailRead { message_id } => message_id,
                            _ => unreachable!(),
                        })
                        .await;
                    retry.attempts = prior_attempts.saturating_add(retry.attempts);
                    retry.duration_ms = prior_duration.saturating_add(retry.duration_ms);
                    result = retry;
                }
                Admission::Denied => {
                    approvals_denied += 1;
                    let attempts = result.attempts;
                    result = denied_result(&operation);
                    result.attempts = attempts;
                }
            }
        }
        gate.record_completion(&operation, completion_for_body(&result))
            .await;
        results.mail_bodies.push(result);
        gate.record_phase(
            EvidencePhase::Reading,
            Some(satisfied_mail_body_count(&results.mail_bodies)),
            Some(u16::from(needs_bodies)),
        )
        .await;
    }

    EvidenceTurnOutcome {
        validation: super::EvidenceValidator::validate(turn_id, plan, results),
        operations_executed,
        approvals_denied,
    }
}

fn completion_for_headers(
    result: &OperationResult<Vec<MailHeaderEvidence>>,
) -> LogicalActivityCompletion {
    LogicalActivityCompletion {
        execution: result.execution.clone(),
        contribution: result.contribution,
        evidence_count: contribution_count(
            result.contribution,
            result.value.as_ref().map_or(0, Vec::len),
        ),
        source_domains: Vec::new(),
        duration_ms: result.duration_ms,
        attempt_count: result.attempts,
        duplicates_suppressed: u8::from(result.contribution == EvidenceContribution::Duplicate),
        body_origin: None,
    }
}

fn completion_for_body(result: &OperationResult<MailBodyEvidence>) -> LogicalActivityCompletion {
    LogicalActivityCompletion {
        execution: result.execution.clone(),
        contribution: result.contribution,
        evidence_count: contribution_count(
            result.contribution,
            usize::from(result.value.is_some()),
        ),
        source_domains: Vec::new(),
        duration_ms: result.duration_ms,
        attempt_count: result.attempts,
        duplicates_suppressed: u8::from(result.contribution == EvidenceContribution::Duplicate),
        body_origin: result.value.as_ref().map(|body| body.body_origin),
    }
}

fn completion_for_search(result: &OperationResult<WebSearchResult>) -> LogicalActivityCompletion {
    LogicalActivityCompletion {
        execution: result.execution.clone(),
        contribution: result.contribution,
        // Search candidates are discovery inputs. Only fetched, validated
        // sources contribute evidence progress.
        evidence_count: 0,
        source_domains: Vec::new(),
        duration_ms: result.duration_ms,
        attempt_count: result.attempts,
        duplicates_suppressed: u8::from(result.contribution == EvidenceContribution::Duplicate),
        body_origin: None,
    }
}

fn completion_for_fetch(result: &OperationResult<WebFetchEvidence>) -> LogicalActivityCompletion {
    let source_domains = result
        .value
        .as_ref()
        .and_then(|value| value.final_url.host_str())
        .map(|domain| vec![domain.to_string()])
        .unwrap_or_default();
    LogicalActivityCompletion {
        execution: result.execution.clone(),
        contribution: result.contribution,
        evidence_count: contribution_count(
            result.contribution,
            usize::from(result.value.is_some()),
        ),
        source_domains,
        duration_ms: result.duration_ms,
        attempt_count: result.attempts,
        duplicates_suppressed: u8::from(result.contribution == EvidenceContribution::Duplicate),
        body_origin: None,
    }
}

fn contribution_count(contribution: EvidenceContribution, count: usize) -> u16 {
    if matches!(
        contribution,
        EvidenceContribution::Satisfied | EvidenceContribution::Partial
    ) {
        count.min(usize::from(u16::MAX)) as u16
    } else {
        0
    }
}

fn satisfied_mail_body_count(results: &[OperationResult<MailBodyEvidence>]) -> u16 {
    results
        .iter()
        .map(|result| completion_for_body(result).evidence_count)
        .sum()
}

fn usable_web_source_count(results: &[OperationResult<WebFetchEvidence>]) -> u16 {
    results
        .iter()
        .filter(|result| {
            matches!(
                result.contribution,
                EvidenceContribution::Satisfied | EvidenceContribution::Partial
            )
        })
        .filter_map(|result| result.value.as_ref())
        .map(|evidence| &evidence.source_identity)
        .collect::<HashSet<_>>()
        .len()
        .min(usize::from(u16::MAX)) as u16
}

fn required_web_source_count(intent: &EvidenceIntent) -> u16 {
    match intent {
        EvidenceIntent::WebFact {
            verification: VerificationLevel::Corroborated,
            ..
        } => 2,
        _ => 1,
    }
}

async fn execute_adapter_operation<A: MailEvidenceAdapter + Send>(
    adapter: &mut A,
    operation: &EvidenceOperation,
) -> OperationResult<Vec<MailHeaderEvidence>> {
    match operation {
        EvidenceOperation::MailList { limit, unread_only } => {
            adapter.list(*limit, *unread_only).await
        }
        EvidenceOperation::MailSearch {
            normalized_query,
            limit,
        } => adapter.search(normalized_query, *limit).await,
        _ => unreachable!("first Mail operation is a list or search"),
    }
}

fn first_mail_operation(intent: &EvidenceIntent) -> EvidenceOperation {
    match intent {
        EvidenceIntent::MailLatestHeaders { count, unread_only }
        | EvidenceIntent::MailLatestContent {
            count, unread_only, ..
        } => EvidenceOperation::MailList {
            limit: *count,
            unread_only: *unread_only,
        },
        EvidenceIntent::MailTargeted { query, .. } => EvidenceOperation::MailSearch {
            normalized_query: query.clone(),
            limit: 10,
        },
        EvidenceIntent::AnalyzeQuotedEvidence { intent } => first_mail_operation(intent),
        _ => unreachable!("Stage 2 executes only Mail evidence"),
    }
}

fn operation_args(operation: &EvidenceOperation) -> serde_json::Value {
    match operation {
        EvidenceOperation::MailList { limit, unread_only } => {
            json!({"limit": limit, "unread_only": unread_only})
        }
        EvidenceOperation::MailSearch {
            normalized_query,
            limit,
        } => json!({"query": normalized_query, "limit": limit}),
        EvidenceOperation::MailRead { message_id } => {
            let rowid = message_id.as_str().parse::<i64>().ok();
            json!({"rowid": rowid})
        }
        EvidenceOperation::WebSearch {
            normalized_query, ..
        } => json!({"query": normalized_query}),
        EvidenceOperation::WebFetch { .. } => json!({}),
    }
}

fn operation_tool_name(operation: &EvidenceOperation) -> &'static str {
    match operation {
        EvidenceOperation::MailList { .. } => "mail_list_inbox",
        EvidenceOperation::MailSearch { .. } => "mail_search",
        EvidenceOperation::MailRead { .. } => "mail_read",
        EvidenceOperation::WebSearch { .. } => "web_search",
        EvidenceOperation::WebFetch { .. } => "web_fetch",
    }
}

fn denied_headers_result(
    operation: &EvidenceOperation,
) -> OperationResult<Vec<MailHeaderEvidence>> {
    denied_result(operation)
}

fn denied_result<T>(operation: &EvidenceOperation) -> OperationResult<T> {
    OperationResult::without_value(
        operation.key(),
        ExecutionStatus::Denied,
        EvidenceContribution::Empty,
    )
}

fn push_list_or_search(
    operation: &EvidenceOperation,
    result: OperationResult<Vec<MailHeaderEvidence>>,
    results: &mut EvidenceResults,
) {
    match operation {
        EvidenceOperation::MailList { .. } => results.mail_list.push(result),
        EvidenceOperation::MailSearch { .. } => results.mail_search.push(result),
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{
        Completeness, EvidencePlanner, MailBodyEvidence, ProviderSet, RecoveryKind,
        ValidatedMailId, WebProvider, WebSearchResult,
    };
    use std::sync::{Arc, Mutex};

    struct RecordingAdapter {
        inner: crate::evidence::FakeMailAdapter,
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl MailEvidenceAdapter for RecordingAdapter {
        async fn list(
            &mut self,
            limit: u8,
            unread_only: bool,
        ) -> OperationResult<Vec<MailHeaderEvidence>> {
            let operation = EvidenceOperation::MailList { limit, unread_only };
            self.log
                .lock()
                .unwrap()
                .push(format!("execute:{}", operation.key().as_str()));
            self.inner.list(limit, unread_only).await
        }

        async fn search(
            &mut self,
            normalized_query: &str,
            limit: u8,
        ) -> OperationResult<Vec<MailHeaderEvidence>> {
            let operation = EvidenceOperation::MailSearch {
                normalized_query: normalized_query.into(),
                limit,
            };
            self.log
                .lock()
                .unwrap()
                .push(format!("execute:{}", operation.key().as_str()));
            self.inner.search(normalized_query, limit).await
        }

        async fn read(
            &mut self,
            message_id: &ValidatedMailId,
        ) -> OperationResult<MailBodyEvidence> {
            let operation = EvidenceOperation::MailRead {
                message_id: message_id.clone(),
            };
            self.log
                .lock()
                .unwrap()
                .push(format!("execute:{}", operation.key().as_str()));
            self.inner.read(message_id).await
        }
    }

    struct RecordingGate {
        log: Arc<Mutex<Vec<String>>>,
        deny_read_number: Option<usize>,
        reads_seen: usize,
    }

    struct EventRecordingGate {
        events: Vec<serde_json::Value>,
        started: HashSet<super::super::OperationKey>,
        admission: Admission,
    }

    #[async_trait]
    impl EvidenceOperationGate for EventRecordingGate {
        async fn admit(&mut self, _operation: &EvidenceOperation) -> Admission {
            self.admission
        }

        async fn record_activity_started(&mut self, operation: &EvidenceOperation) {
            if self.started.insert(operation.key()) {
                self.events.push(
                    serde_json::to_value(LogicalActivityEvent::started("turn-events", operation))
                        .unwrap(),
                );
            }
        }

        async fn record_completion(
            &mut self,
            operation: &EvidenceOperation,
            completion: LogicalActivityCompletion,
        ) {
            self.events.push(
                serde_json::to_value(LogicalActivityEvent::completed(
                    "turn-events",
                    operation,
                    &completion,
                ))
                .unwrap(),
            );
        }

        async fn record_phase(
            &mut self,
            phase: EvidencePhase,
            completed: Option<u16>,
            total: Option<u16>,
        ) {
            self.events.push(
                serde_json::to_value(EvidencePhaseEvent::acquisition(
                    "turn-events",
                    phase,
                    completed,
                    total,
                ))
                .unwrap(),
            );
        }
    }

    impl EventRecordingGate {
        fn new() -> Self {
            Self {
                events: Vec::new(),
                started: HashSet::new(),
                admission: Admission::Allowed,
            }
        }

        fn denying() -> Self {
            Self {
                admission: Admission::Denied,
                ..Self::new()
            }
        }
    }

    #[async_trait]
    impl EvidenceOperationGate for RecordingGate {
        async fn admit(&mut self, operation: &EvidenceOperation) -> Admission {
            self.log
                .lock()
                .unwrap()
                .push(format!("gate:{}", operation.key().as_str()));
            if matches!(operation, EvidenceOperation::MailRead { .. }) {
                self.reads_seen += 1;
                if self.deny_read_number == Some(self.reads_seen) {
                    return Admission::Denied;
                }
            }
            Admission::Allowed
        }
    }

    #[tokio::test]
    async fn orchestrator_gates_immediately_before_each_mail_execution() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut adapter = RecordingAdapter {
            inner: crate::evidence::FakeMailAdapter::with_three_readable_messages(),
            log: log.clone(),
        };
        let mut gate = RecordingGate {
            log: log.clone(),
            deny_read_number: None,
            reads_seen: 0,
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestContent {
            count: 3,
            requested_count: 3,
            unread_only: false,
        });

        let outcome = execute_mail_plan(&mut adapter, &mut gate, "turn-1", &plan).await;

        assert!(matches!(
            outcome.validation,
            ValidationOutcome::Bundle(bundle)
                if bundle.completeness == Completeness::Complete
                    && bundle.acquired.mail_bodies == 3
        ));
        let entries = log.lock().unwrap().clone();
        assert_eq!(entries.len(), 8);
        for pair in entries.chunks_exact(2) {
            assert!(pair[0].starts_with("gate:"));
            assert_eq!(
                pair[0].strip_prefix("gate:"),
                pair[1].strip_prefix("execute:")
            );
        }
    }

    #[tokio::test]
    async fn complete_mail_events_report_evidence_progress_not_call_count() {
        let mut adapter = crate::evidence::FakeMailAdapter::with_three_readable_messages();
        let mut gate = EventRecordingGate::new();
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestContent {
            count: 3,
            requested_count: 3,
            unread_only: false,
        });

        let outcome = execute_mail_plan(&mut adapter, &mut gate, "turn-events", &plan).await;

        assert!(matches!(
            outcome.validation,
            ValidationOutcome::Bundle(bundle) if bundle.acquired.mail_bodies == 3
        ));
        let final_reading = gate
            .events
            .iter()
            .rev()
            .find(|event| event["phase"] == "reading")
            .unwrap();
        assert_eq!(final_reading["completed"], 3);
        assert_eq!(final_reading["total"], 3);
        assert_eq!(
            gate.events
                .iter()
                .filter(|event| event["type"] == "logical_activity_started")
                .count(),
            4
        );
        assert_eq!(
            gate.events
                .iter()
                .filter(|event| {
                    event["type"] == "logical_activity_completed"
                        && event["evidence_count"].as_u64().unwrap_or_default() > 0
                })
                .count(),
            4
        );
    }

    #[tokio::test]
    async fn denied_operation_has_a_correlated_started_and_completed_activity() {
        let mut adapter = crate::evidence::FakeMailAdapter::with_three_readable_messages();
        let mut gate = EventRecordingGate::denying();
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestHeaders {
            count: 3,
            unread_only: false,
        });

        let _ = execute_mail_plan(&mut adapter, &mut gate, "turn-events", &plan).await;

        let activities = gate
            .events
            .iter()
            .filter(|event| {
                matches!(
                    event["type"].as_str(),
                    Some("logical_activity_started" | "logical_activity_completed")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(activities.len(), 2);
        assert_eq!(activities[0]["activity_id"], activities[1]["activity_id"]);
        assert_eq!(activities[1]["execution_status"], "denied");
        assert_eq!(activities[1]["evidence_count"], 0);
    }

    #[tokio::test]
    async fn duplicate_suppression_is_grouped_and_does_not_increase_read_progress() {
        let mut adapter = crate::evidence::FakeMailAdapter::with_duplicate_identifier();
        let mut gate = EventRecordingGate::new();
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestContent {
            count: 3,
            requested_count: 3,
            unread_only: false,
        });

        let _ = execute_mail_plan(&mut adapter, &mut gate, "turn-events", &plan).await;

        let duplicate = gate
            .events
            .iter()
            .find(|event| event["duplicates_suppressed"] == 1)
            .unwrap();
        assert_eq!(duplicate["evidence_count"], 0);
        let final_reading = gate
            .events
            .iter()
            .rev()
            .find(|event| event["phase"] == "reading")
            .unwrap();
        assert_eq!(final_reading["completed"], 2);
        assert_eq!(final_reading["total"], 3);
    }

    #[tokio::test]
    async fn transient_retry_emits_one_started_activity_and_one_grouped_completion() {
        struct RetryOnce {
            inner: crate::evidence::FakeMailAdapter,
            failed: bool,
        }
        #[async_trait]
        impl MailEvidenceAdapter for RetryOnce {
            async fn list(
                &mut self,
                limit: u8,
                unread_only: bool,
            ) -> OperationResult<Vec<MailHeaderEvidence>> {
                self.inner.list(limit, unread_only).await
            }

            async fn search(
                &mut self,
                query: &str,
                limit: u8,
            ) -> OperationResult<Vec<MailHeaderEvidence>> {
                self.inner.search(query, limit).await
            }

            async fn read(
                &mut self,
                message_id: &ValidatedMailId,
            ) -> OperationResult<MailBodyEvidence> {
                if !self.failed {
                    self.failed = true;
                    return OperationResult::without_value(
                        EvidenceOperation::MailRead {
                            message_id: message_id.clone(),
                        }
                        .key(),
                        ExecutionStatus::TimedOut,
                        EvidenceContribution::Empty,
                    );
                }
                self.inner.read(message_id).await
            }
        }

        let mut adapter = RetryOnce {
            inner: crate::evidence::FakeMailAdapter::with_three_readable_messages(),
            failed: false,
        };
        let mut gate = EventRecordingGate::new();
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestContent {
            count: 2,
            requested_count: 2,
            unread_only: false,
        });

        let _ = execute_mail_plan(&mut adapter, &mut gate, "turn-events", &plan).await;

        let read_completion = gate
            .events
            .iter()
            .find(|event| {
                event["type"] == "logical_activity_completed"
                    && event["normalized_operation"] == "mail.read"
                    && event["retries"] == 1
            })
            .unwrap();
        let retried_activity_id = &read_completion["activity_id"];
        let read_starts = gate
            .events
            .iter()
            .filter(|event| {
                event["type"] == "logical_activity_started"
                    && event["activity_id"] == *retried_activity_id
            })
            .count();
        assert_eq!(read_starts, 1);
        assert_eq!(read_completion["attempt_count"], 2);
        assert_eq!(read_completion["retries"], 1);
        assert_eq!(read_completion["evidence_count"], 1);
    }

    #[tokio::test]
    async fn denied_read_is_not_executed_and_remains_a_distinct_partial_shortfall() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut adapter = RecordingAdapter {
            inner: crate::evidence::FakeMailAdapter::with_three_readable_messages(),
            log: log.clone(),
        };
        let mut gate = RecordingGate {
            log: log.clone(),
            deny_read_number: Some(2),
            reads_seen: 0,
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestContent {
            count: 3,
            requested_count: 3,
            unread_only: false,
        });

        let outcome = execute_mail_plan(&mut adapter, &mut gate, "turn-2", &plan).await;

        assert_eq!(outcome.approvals_denied, 1);
        assert_eq!(outcome.operations_executed, 3);
        assert!(matches!(
            outcome.validation,
            ValidationOutcome::Bundle(bundle)
                if bundle.completeness == Completeness::Partial
                    && bundle.acquired.mail_bodies == 2
        ));
        let entries = log.lock().unwrap().clone();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.starts_with("gate:mail_read:"))
                .count(),
            3
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.starts_with("execute:mail_read:"))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn denied_list_never_executes_the_adapter_and_returns_denied_recovery() {
        struct DenyAllGate;
        #[async_trait]
        impl EvidenceOperationGate for DenyAllGate {
            async fn admit(&mut self, _operation: &EvidenceOperation) -> Admission {
                Admission::Denied
            }
        }

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut adapter = RecordingAdapter {
            inner: crate::evidence::FakeMailAdapter::with_three_readable_messages(),
            log: log.clone(),
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestHeaders {
            count: 3,
            unread_only: false,
        });
        let outcome = execute_mail_plan(&mut adapter, &mut DenyAllGate, "turn-3", &plan).await;

        assert!(log.lock().unwrap().is_empty());
        assert_eq!(outcome.operations_executed, 0);
        assert_eq!(outcome.approvals_denied, 1);
        assert!(matches!(
            outcome.validation,
            ValidationOutcome::Recovery(recovery) if recovery.kind == RecoveryKind::Denied
        ));
    }

    #[tokio::test]
    async fn duplicate_identifiers_are_typed_and_suppressed_without_gate_or_budget_use() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut adapter = RecordingAdapter {
            inner: crate::evidence::FakeMailAdapter::with_duplicate_identifier(),
            log: log.clone(),
        };
        let mut gate = RecordingGate {
            log: log.clone(),
            deny_read_number: None,
            reads_seen: 0,
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestContent {
            count: 3,
            requested_count: 3,
            unread_only: false,
        });

        let outcome = execute_mail_plan(&mut adapter, &mut gate, "turn-duplicate", &plan).await;

        assert_eq!(outcome.operations_executed, 3);
        assert!(matches!(
            outcome.validation,
            ValidationOutcome::Bundle(bundle)
                if bundle.completeness == Completeness::Partial
                    && bundle.acquired.mail_bodies == 2
                    && bundle.missing.iter().any(|missing| {
                        missing.reason == crate::evidence::ShortfallReason::Duplicate
                            && missing.missing_count == 1
                    })
        ));
        let entries = log.lock().unwrap();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.starts_with("gate:mail_read:"))
                .count(),
            2
        );
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.starts_with("execute:mail_read:"))
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn transient_read_retries_once_with_a_fresh_gate_and_consumes_body_budget() {
        struct TransientOnceAdapter {
            inner: crate::evidence::FakeMailAdapter,
            read_calls: usize,
        }

        #[async_trait]
        impl MailEvidenceAdapter for TransientOnceAdapter {
            async fn list(
                &mut self,
                limit: u8,
                unread_only: bool,
            ) -> OperationResult<Vec<MailHeaderEvidence>> {
                self.inner.list(limit, unread_only).await
            }

            async fn search(
                &mut self,
                normalized_query: &str,
                limit: u8,
            ) -> OperationResult<Vec<MailHeaderEvidence>> {
                self.inner.search(normalized_query, limit).await
            }

            async fn read(
                &mut self,
                message_id: &ValidatedMailId,
            ) -> OperationResult<MailBodyEvidence> {
                self.read_calls += 1;
                if self.read_calls == 1 {
                    return OperationResult::without_value(
                        EvidenceOperation::MailRead {
                            message_id: message_id.clone(),
                        }
                        .key(),
                        ExecutionStatus::TimedOut,
                        EvidenceContribution::Empty,
                    );
                }
                self.inner.read(message_id).await
            }
        }

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut adapter = TransientOnceAdapter {
            inner: crate::evidence::FakeMailAdapter::with_three_readable_messages(),
            read_calls: 0,
        };
        let mut gate = RecordingGate {
            log: log.clone(),
            deny_read_number: None,
            reads_seen: 0,
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::MailLatestContent {
            count: 2,
            requested_count: 2,
            unread_only: false,
        });

        let outcome = execute_mail_plan(&mut adapter, &mut gate, "turn-retry", &plan).await;

        assert_eq!(adapter.read_calls, 3);
        assert_eq!(outcome.operations_executed, 4);
        assert!(matches!(
            outcome.validation,
            ValidationOutcome::Bundle(bundle)
                if bundle.completeness == Completeness::Complete
                    && bundle.acquired.mail_bodies == 2
        ));
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.starts_with("gate:mail_read:"))
                .count(),
            3
        );
    }

    #[test]
    fn policy_arguments_match_the_real_mail_connector_shape() {
        let read = EvidenceOperation::MailRead {
            message_id: ValidatedMailId::new("42").unwrap(),
        };
        assert_eq!(operation_args(&read), json!({"rowid": 42}));
        let search = EvidenceOperation::WebSearch {
            normalized_query: "current fact".into(),
            provider_set: ProviderSet(vec![WebProvider::Wikipedia, WebProvider::DuckDuckGo]),
        };
        assert_eq!(operation_args(&search), json!({"query": "current fact"}));
    }

    #[test]
    fn quoted_web_intent_is_not_admitted_to_the_stage_two_mail_executor() {
        let intent = EvidenceIntent::AnalyzeQuotedEvidence {
            intent: Box::new(EvidenceIntent::WebFact {
                query: "current fact".into(),
                verification: crate::evidence::VerificationLevel::SingleAuthoritative,
            }),
        };
        assert!(!is_mail_intent(&intent));
    }

    #[derive(Clone)]
    struct ScriptedWebAdapter {
        search: OperationResult<WebSearchResult>,
        fetches: Arc<Mutex<HashMap<String, VecDeque<OperationResult<WebFetchEvidence>>>>>,
        calls: Arc<Mutex<Vec<String>>>,
        active: Arc<std::sync::atomic::AtomicUsize>,
        max_active: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl TypedWebEvidenceAdapter for ScriptedWebAdapter {
        async fn search(
            &self,
            _query: &str,
            _lang: &str,
            _providers: &ProviderSet,
        ) -> OperationResult<WebSearchResult> {
            self.calls.lock().unwrap().push("search".into());
            self.search.clone()
        }

        async fn fetch(&self, candidate: &WebCandidate) -> OperationResult<WebFetchEvidence> {
            use std::sync::atomic::Ordering;
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.calls
                .lock()
                .unwrap()
                .push(format!("fetch:{}", candidate.candidate_id.as_str()));
            self.fetches
                .lock()
                .unwrap()
                .get_mut(candidate.candidate_id.as_str())
                .and_then(VecDeque::pop_front)
                .expect("scripted fetch result")
        }
    }

    struct WebRecordingGate {
        log: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl EvidenceOperationGate for WebRecordingGate {
        async fn admit(&mut self, operation: &EvidenceOperation) -> Admission {
            self.log
                .lock()
                .unwrap()
                .push(format!("gate:{}", operation.key().as_str()));
            Admission::Allowed
        }

        async fn admit_web_fetch(
            &mut self,
            operation: &EvidenceOperation,
            _validated_url: &Url,
        ) -> Admission {
            self.admit(operation).await
        }

        async fn record_execution(&mut self, operation: &EvidenceOperation) {
            self.log
                .lock()
                .unwrap()
                .push(format!("execute:{}", operation.key().as_str()));
        }
    }

    fn web_candidate(url: &str, rank: u16) -> WebCandidate {
        let url = Url::parse(url).unwrap();
        let mut candidate = direct_web_candidate(&url);
        candidate.provider = WebProvider::DuckDuckGo;
        candidate.rank = rank;
        candidate.title = format!("Acme Official Website authoritative fact {rank}");
        candidate
    }

    fn web_search_result(candidates: Vec<WebCandidate>) -> OperationResult<WebSearchResult> {
        OperationResult::succeeded(
            EvidenceOperation::WebSearch {
                normalized_query: "Acme fact online".into(),
                provider_set: ProviderSet(vec![WebProvider::Wikipedia, WebProvider::DuckDuckGo]),
            }
            .key(),
            WebSearchResult {
                providers: Vec::new(),
                candidates,
            },
        )
    }

    fn readable_fetch(
        candidate: &WebCandidate,
        final_url: &str,
        source: &str,
        text: &str,
    ) -> OperationResult<WebFetchEvidence> {
        OperationResult::succeeded(
            EvidenceOperation::WebFetch {
                candidate_id: candidate.candidate_id.clone(),
            }
            .key(),
            WebFetchEvidence {
                evidence_id: super::super::EvidenceId::new(format!("evidence-{}", candidate.rank))
                    .unwrap(),
                candidate_id: candidate.candidate_id.clone(),
                requested_url: candidate.requested_url.clone(),
                final_url: Url::parse(final_url).unwrap(),
                redirect_chain: vec![Url::parse(final_url).unwrap()],
                http_status: 200,
                content_type: "text/html".into(),
                bytes_read: text.len() as u64,
                characters_extracted: text.chars().count() as u64,
                extraction: ExtractionStatus::Readable,
                quality: super::super::ExtractionQuality {
                    useful_text_length: text.chars().count() as u64,
                    ..Default::default()
                },
                authority: SourceAuthority::Other,
                source_identity: super::super::SourceIdentity::new(source).unwrap(),
                passages: vec![super::super::EvidencePassage {
                    passage_id: super::super::EvidenceId::new(format!(
                        "passage-{}",
                        candidate.rank
                    ))
                    .unwrap(),
                    text: text.into(),
                    truncated: false,
                }],
                links: Vec::new(),
            },
        )
    }

    fn readable_passages(
        candidate: &WebCandidate,
        final_url: &str,
        source: &str,
        passages: &[&str],
    ) -> OperationResult<WebFetchEvidence> {
        let mut result = readable_fetch(candidate, final_url, source, &passages.join(" "));
        let evidence = result.value.as_mut().expect("readable evidence");
        evidence.passages = passages
            .iter()
            .enumerate()
            .map(|(index, text)| super::super::EvidencePassage {
                passage_id: super::super::EvidenceId::new(format!(
                    "passage-{}-{}",
                    candidate.rank, index
                ))
                .unwrap(),
                text: (*text).to_string(),
                truncated: false,
            })
            .collect();
        result
    }

    fn scripted_web_adapter(
        candidates: Vec<WebCandidate>,
        scripts: Vec<Vec<OperationResult<WebFetchEvidence>>>,
    ) -> ScriptedWebAdapter {
        let mut fetches = HashMap::new();
        for (candidate, results) in candidates.iter().zip(scripts) {
            fetches
                .entry(candidate.candidate_id.as_str().to_string())
                .or_insert_with(|| VecDeque::from(results));
        }
        ScriptedWebAdapter {
            search: web_search_result(candidates),
            fetches: Arc::new(Mutex::new(fetches)),
            calls: Arc::new(Mutex::new(Vec::new())),
            active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            max_active: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }

    #[tokio::test]
    async fn web_fact_is_gated_immediately_and_stops_on_ranked_first_party_evidence() {
        let first_party = web_candidate("https://acme.com/fact", 2);
        let reference = web_candidate("https://en.wikipedia.org/wiki/Acme", 1);
        let reference_failure = OperationResult::without_value(
            EvidenceOperation::WebFetch {
                candidate_id: reference.candidate_id.clone(),
            }
            .key(),
            ExecutionStatus::Failed(FailureCode::Http4xx(404)),
            EvidenceContribution::Empty,
        );
        let adapter = scripted_web_adapter(
            vec![reference, first_party.clone()],
            vec![
                vec![reference_failure],
                vec![readable_fetch(
                    &first_party,
                    "https://acme.com/final",
                    "acme.com",
                    "Acme reports the fact is 42.",
                )],
            ],
        );
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut gate = WebRecordingGate { log: log.clone() };
        let plan = EvidencePlanner::plan(EvidenceIntent::WebFact {
            query: "Acme fact online".into(),
            verification: VerificationLevel::SingleAuthoritative,
        });

        let outcome = execute_web_plan(adapter, &mut gate, "turn-web", &plan, "en").await;

        let ValidationOutcome::Bundle(bundle) = outcome.validation else {
            panic!("first-party fetched evidence should satisfy the fact");
        };
        assert_eq!(bundle.acquired.web_sources, 1);
        assert_eq!(
            bundle.citation_allowlist[0].url.as_str(),
            "https://acme.com/final"
        );
        let log = log.lock().unwrap();
        assert!(log[0].starts_with("gate:web_search:"));
        assert!(log[1].starts_with("execute:web_search:"));
        assert!(log[2].starts_with("gate:web_fetch:"));
        assert!(log[3].starts_with("execute:web_fetch:"));
    }

    #[tokio::test]
    async fn corroborated_web_enforces_independence_retry_budget_duplicates_and_concurrency() {
        use std::sync::atomic::Ordering;

        let candidates = (1..=6)
            .map(|rank| web_candidate(&format!("https://site{rank}.example/fact"), rank))
            .collect::<Vec<_>>();
        let transient = OperationResult::without_value(
            EvidenceOperation::WebFetch {
                candidate_id: candidates[0].candidate_id.clone(),
            }
            .key(),
            ExecutionStatus::TimedOut,
            EvidenceContribution::Empty,
        );
        let scripts = candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                if index == 0 {
                    vec![
                        transient.clone(),
                        readable_fetch(
                            candidate,
                            candidate.requested_url.as_str(),
                            "same-source.example",
                            "The example value is 41.",
                        ),
                    ]
                } else {
                    vec![readable_fetch(
                        candidate,
                        candidate.requested_url.as_str(),
                        if index == 1 {
                            "same-source.example"
                        } else {
                            "independent.example"
                        },
                        if index == 2 {
                            "The example value is 42."
                        } else {
                            "The example value is 41."
                        },
                    )]
                }
            })
            .collect::<Vec<_>>();
        let adapter = scripted_web_adapter(candidates, scripts);
        let observed = adapter.clone();
        let mut gate = WebRecordingGate {
            log: Arc::new(Mutex::new(Vec::new())),
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::WebFact {
            query: "compare current example values".into(),
            verification: VerificationLevel::Corroborated,
        });

        let outcome = execute_web_plan(adapter, &mut gate, "turn-corroborated", &plan, "en").await;

        let ValidationOutcome::Bundle(bundle) = outcome.validation else {
            panic!("two independent fetched sources should produce a bundle");
        };
        assert_eq!(outcome.operations_executed, 6);
        assert_eq!(bundle.acquired.web_sources, 2);
        assert_eq!(bundle.conflicts.len(), 1);
        assert!(observed.max_active.load(Ordering::SeqCst) <= 2);
        assert_eq!(
            observed
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.starts_with("fetch:"))
                .count(),
            5
        );
    }

    #[tokio::test]
    async fn canonical_duplicate_web_operations_are_suppressed_before_gate_and_fetch() {
        let first = web_candidate("https://acme.com/fact?utm_source=one", 1);
        let second = web_candidate("https://acme.com/fact#top", 2);
        assert_eq!(first.candidate_id, second.candidate_id);
        let adapter = scripted_web_adapter(
            vec![first.clone(), second],
            vec![
                vec![readable_fetch(
                    &first,
                    "https://acme.com/fact",
                    "acme.com",
                    "Acme fact is 42.",
                )],
                Vec::new(),
            ],
        );
        let observed = adapter.clone();
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut gate = WebRecordingGate { log: log.clone() };
        let plan = EvidencePlanner::plan(EvidenceIntent::WebFact {
            query: "Acme fact online".into(),
            verification: VerificationLevel::SingleAuthoritative,
        });

        let outcome = execute_web_plan(adapter, &mut gate, "turn-duplicate", &plan, "en").await;

        assert!(matches!(outcome.validation, ValidationOutcome::Bundle(_)));
        assert_eq!(
            observed
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.starts_with("fetch:"))
                .count(),
            1
        );
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.starts_with("gate:web_fetch:"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn web_fact_retains_query_relevant_numeric_passages_instead_of_page_beginning() {
        let candidate = web_candidate("https://bratislava.sk/population", 1);
        let adapter = scripted_web_adapter(
            vec![candidate.clone()],
            vec![vec![readable_passages(
                &candidate,
                candidate.requested_url.as_str(),
                "bratislava.sk",
                &[
                    "Home Services City office Contact Sitemap",
                    "Bratislava has a long history and many cultural institutions.",
                    "Population data",
                    "Bratislava had 475,503 residents as of 31 December 2024.",
                    "Related links and archived reports",
                ],
            )]],
        );
        let mut gate = WebRecordingGate {
            log: Arc::new(Mutex::new(Vec::new())),
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::WebFact {
            query: "What is the current population of Bratislava?".into(),
            verification: VerificationLevel::SingleAuthoritative,
        });

        let outcome = execute_web_plan(adapter, &mut gate, "turn-population", &plan, "en").await;

        let ValidationOutcome::Bundle(bundle) = outcome.validation else {
            panic!("claim-relevant numeric evidence should be eligible");
        };
        let evidence = &bundle.web[0].evidence;
        assert!(evidence.passages[0].text.contains("475,503"));
        assert!(evidence
            .passages
            .iter()
            .all(|passage| !passage.text.contains("Home Services")));
        assert!(evidence.quality.query_coverage_basis_points > 0);
        assert_eq!(evidence.quality.low_quality_reason, None);
    }

    #[tokio::test]
    async fn irrelevant_nonempty_web_text_becomes_verification_shortfall() {
        let candidate = web_candidate("https://bratislava.sk/population", 1);
        let adapter = scripted_web_adapter(
            vec![candidate.clone()],
            vec![vec![readable_passages(
                &candidate,
                candidate.requested_url.as_str(),
                "bratislava.sk",
                &["Home Products Careers Privacy Sign in Contact us"],
            )]],
        );
        let mut gate = WebRecordingGate {
            log: Arc::new(Mutex::new(Vec::new())),
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::WebFact {
            query: "What is the current population of Bratislava?".into(),
            verification: VerificationLevel::SingleAuthoritative,
        });

        let outcome = execute_web_plan(adapter, &mut gate, "turn-low-quality", &plan, "en").await;

        let ValidationOutcome::Recovery(recovery) = outcome.validation else {
            panic!("irrelevant navigation must not become Fetched Evidence");
        };
        assert_eq!(
            recovery.kind,
            super::super::RecoveryKind::VerificationShortfall
        );
        assert!(recovery.message.starts_with("Verification Shortfall:"));
        assert!(!recovery.message.contains("Home Products"));
        assert!(recovery
            .message
            .contains("[bratislava.sk](https://bratislava.sk/population)"));
    }

    #[tokio::test]
    async fn current_numeric_fact_rejects_an_unrelated_number_about_the_right_subject() {
        let candidate = web_candidate("https://bratislava.sk/history", 1);
        let adapter = scripted_web_adapter(
            vec![candidate.clone()],
            vec![vec![readable_passages(
                &candidate,
                candidate.requested_url.as_str(),
                "bratislava.sk",
                &["Bratislava was first mentioned in writing in 907."],
            )]],
        );
        let mut gate = WebRecordingGate {
            log: Arc::new(Mutex::new(Vec::new())),
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::WebFact {
            query: "What is the current population of Bratislava?".into(),
            verification: VerificationLevel::SingleAuthoritative,
        });

        let outcome = execute_web_plan(adapter, &mut gate, "turn-wrong-number", &plan, "en").await;

        assert!(matches!(outcome.validation, ValidationOutcome::Recovery(_)));
    }

    #[tokio::test]
    async fn direct_page_selection_prefers_descriptive_content_beyond_front_matter() {
        let url = "https://report.example/about";
        let candidate = web_candidate(url, 1);
        let adapter = scripted_web_adapter(
            vec![candidate.clone()],
            vec![vec![readable_passages(
                &candidate,
                url,
                "report.example",
                &[
                    "Organization profile",
                    "Published 2026",
                    "Reference 12345",
                    "Edition 7",
                    "Document 42",
                    "Updated 2026",
                    "Record 88",
                    "This organization provides community services and publishes practical guidance for residents.",
                ],
            )]],
        );
        let mut gate = WebRecordingGate {
            log: Arc::new(Mutex::new(Vec::new())),
        };
        let plan = EvidencePlanner::plan(EvidenceIntent::WebDirectPage {
            url: Url::parse(url).unwrap(),
        });

        let outcome =
            execute_web_plan(adapter, &mut gate, "turn-direct-description", &plan, "en").await;

        let ValidationOutcome::Bundle(bundle) = outcome.validation else {
            panic!("descriptive direct page should be eligible");
        };
        assert_eq!(
            bundle.web[0].evidence.passages[0].text,
            "Organization profile"
        );
        assert!(bundle.web[0].evidence.passages[1]
            .text
            .contains("provides community services"));
    }
}
