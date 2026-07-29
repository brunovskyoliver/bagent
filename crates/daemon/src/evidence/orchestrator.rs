use std::collections::HashSet;

use async_trait::async_trait;
use serde_json::json;

use super::{
    AppleMailEvidenceAdapter, EvidenceContribution, EvidenceIntent, EvidenceOperation,
    EvidencePlanner, EvidenceRequest, EvidenceResults, ExecutionStatus, FailureCode,
    MailEvidenceAdapter, MailHeaderEvidence, OperationResult, ValidationOutcome,
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

    async fn record_execution(&mut self, _operation: &EvidenceOperation) {}
}

struct ExistingPolicyGate<'a> {
    state: &'a AppState,
    sink: &'a EventSink,
    origin: &'a ExecOrigin,
}

#[async_trait]
impl EvidenceOperationGate for ExistingPolicyGate<'_> {
    async fn admit(&mut self, operation: &EvidenceOperation) -> Admission {
        let args = operation_args(operation);
        let gate = Gate::new(&self.state.rules, self.origin);
        match gate.level("mail_inbox", &args, ToolKind::ReadOnly) {
            ApprovalLevel::Auto => Admission::Allowed,
            ApprovalLevel::Forbidden => Admission::Denied,
            ApprovalLevel::Ask => {
                let approved = request_tool_approval(
                    self.state,
                    self.sink,
                    self.origin,
                    "mail_inbox",
                    &self
                        .origin
                        .describe("Čítanie poštovej schránky (Apple Mail)"),
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

    async fn record_execution(&mut self, operation: &EvidenceOperation) {
        let tool = operation_tool_name(operation);
        let _ = self
            .sink
            .emit(json!({"type": "tool_call", "tool": tool}))
            .await;
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
}

pub(crate) async fn execute_evidence_turn(
    ctx: EvidenceContext<'_>,
    request: EvidenceRequest,
    intent: EvidenceIntent,
) -> Result<EvidenceTurnOutcome, EvidenceExecError> {
    if !is_mail_intent(&intent) {
        return Err(EvidenceExecError::UnsupportedIntent);
    }

    let plan = EvidencePlanner::plan(intent);
    let mut gate = ExistingPolicyGate {
        state: ctx.state,
        sink: ctx.sink,
        origin: ctx.origin,
    };
    let outcome = if let Some(connector) = ctx.state.mail.clone() {
        let mut adapter = AppleMailEvidenceAdapter::new(connector);
        execute_mail_plan(&mut adapter, &mut gate, &request.turn_id, &plan).await
    } else {
        execute_unavailable_mail_plan(&mut gate, &request.turn_id, &plan).await
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

pub(crate) async fn execute_unavailable_mail_plan<G: EvidenceOperationGate + Send>(
    gate: &mut G,
    turn_id: &str,
    plan: &super::EvidencePlan,
) -> EvidenceTurnOutcome {
    let operation = first_mail_operation(&plan.intent);
    let result = match gate.admit(&operation).await {
        Admission::Allowed => OperationResult::without_value(
            operation.key(),
            ExecutionStatus::Failed(FailureCode::ConnectorUnavailable),
            EvidenceContribution::Empty,
        ),
        Admission::Denied => denied_result(&operation),
    };
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
    let mut body_attempts = 0usize;
    for header in headers {
        if body_attempts >= usize::from(needs_bodies)
            || body_attempts >= usize::from(plan.budget.mail_body_attempts)
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
            results
                .mail_bodies
                .push(OperationResult::suppressed_duplicate(operation.key()));
            continue;
        }
        body_attempts += 1;
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
            usize::from(plan.budget.mail_body_attempts).saturating_sub(body_attempts);
        if result.retry_permitted(remaining_budget.min(usize::from(u8::MAX)) as u8) {
            match gate.admit(&operation).await {
                Admission::Allowed => {
                    gate.record_execution(&operation).await;
                    operations_executed += 1;
                    body_attempts += 1;
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
        results.mail_bodies.push(result);
    }

    EvidenceTurnOutcome {
        validation: super::EvidenceValidator::validate(turn_id, plan, results),
        operations_executed,
        approvals_denied,
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
        _ => json!({}),
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
        Completeness, EvidencePlanner, MailBodyEvidence, RecoveryKind, ValidatedMailId,
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

        assert_eq!(adapter.read_calls, 2);
        assert_eq!(outcome.operations_executed, 3);
        assert!(matches!(
            outcome.validation,
            ValidationOutcome::Bundle(bundle)
                if bundle.completeness == Completeness::Partial
                    && bundle.acquired.mail_bodies == 1
        ));
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.starts_with("gate:mail_read:"))
                .count(),
            2
        );
    }

    #[test]
    fn policy_arguments_match_the_real_mail_connector_shape() {
        let read = EvidenceOperation::MailRead {
            message_id: ValidatedMailId::new("42").unwrap(),
        };
        assert_eq!(operation_args(&read), json!({"rowid": 42}));
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
}
