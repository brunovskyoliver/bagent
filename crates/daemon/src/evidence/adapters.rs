use super::{
    BodyState, CandidateId, EvidenceContribution, EvidenceId, EvidenceOperation, ExecutionStatus,
    FailureCode, MailBodyEvidence, MailHeaderEvidence, OperationResult, ProviderSet,
    ValidatedMailId, WebFetchEvidence, WebSearchResult,
};
use apple_mail_connector::{MailConnector, MailMessage, MailSearchFilter};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::time::Instant;

pub(crate) trait EvidenceClock {
    fn now(&self) -> DateTime<Utc>;
}

pub(crate) struct SystemEvidenceClock;

impl EvidenceClock for SystemEvidenceClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

#[async_trait]
pub(crate) trait MailEvidenceAdapter {
    async fn list(
        &mut self,
        limit: u8,
        unread_only: bool,
    ) -> OperationResult<Vec<MailHeaderEvidence>>;

    async fn search(
        &mut self,
        normalized_query: &str,
        limit: u8,
    ) -> OperationResult<Vec<MailHeaderEvidence>>;

    async fn read(&mut self, message_id: &ValidatedMailId) -> OperationResult<MailBodyEvidence>;
}

pub(crate) trait AppleMailBackend: Clone + Send + Sync + 'static {
    fn list_inbox(
        &self,
        limit: usize,
        unread_only: bool,
    ) -> Result<Vec<MailMessage>, AppleMailBackendError>;

    fn search_messages(
        &self,
        filter: &MailSearchFilter,
    ) -> Result<Vec<MailMessage>, AppleMailBackendError>;

    fn get_message(&self, rowid: i64) -> Result<Option<MailMessage>, AppleMailBackendError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppleMailBackendError {
    Unavailable,
    TimedOut,
    ConnectionReset,
}

impl AppleMailBackend for MailConnector {
    fn list_inbox(
        &self,
        limit: usize,
        unread_only: bool,
    ) -> Result<Vec<MailMessage>, AppleMailBackendError> {
        MailConnector::list_inbox(self, limit, unread_only).map_err(normalize_backend_error)
    }

    fn search_messages(
        &self,
        filter: &MailSearchFilter,
    ) -> Result<Vec<MailMessage>, AppleMailBackendError> {
        MailConnector::search_messages(self, filter).map_err(normalize_backend_error)
    }

    fn get_message(&self, rowid: i64) -> Result<Option<MailMessage>, AppleMailBackendError> {
        MailConnector::get_message(self, rowid).map_err(normalize_backend_error)
    }
}

fn normalize_backend_error(error: anyhow::Error) -> AppleMailBackendError {
    let normalized = error.to_string().to_ascii_lowercase();
    if normalized.contains("timed out")
        || normalized.contains("database is locked")
        || normalized.contains("database is busy")
    {
        AppleMailBackendError::TimedOut
    } else if normalized.contains("connection reset") {
        AppleMailBackendError::ConnectionReset
    } else {
        AppleMailBackendError::Unavailable
    }
}

pub(crate) struct AppleMailEvidenceAdapter<B = MailConnector> {
    backend: B,
}

impl AppleMailEvidenceAdapter<MailConnector> {
    pub(crate) fn new(connector: MailConnector) -> Self {
        Self { backend: connector }
    }
}

impl<B> AppleMailEvidenceAdapter<B> {
    #[cfg(test)]
    fn from_backend(backend: B) -> Self {
        Self { backend }
    }
}

#[async_trait]
impl<B: AppleMailBackend> MailEvidenceAdapter for AppleMailEvidenceAdapter<B> {
    async fn list(
        &mut self,
        limit: u8,
        unread_only: bool,
    ) -> OperationResult<Vec<MailHeaderEvidence>> {
        let operation = EvidenceOperation::MailList { limit, unread_only };
        let started = Instant::now();
        let backend = self.backend.clone();
        let result =
            tokio::task::spawn_blocking(move || backend.list_inbox(limit.into(), unread_only))
                .await;
        mail_headers_result(operation, started, result)
    }

    async fn search(
        &mut self,
        normalized_query: &str,
        limit: u8,
    ) -> OperationResult<Vec<MailHeaderEvidence>> {
        let operation = EvidenceOperation::MailSearch {
            normalized_query: normalized_query.to_string(),
            limit,
        };
        if normalized_query.trim().is_empty() {
            return failed_result(
                operation,
                FailureCode::InvalidInput,
                EvidenceContribution::Empty,
                0,
            );
        }
        let started = Instant::now();
        let backend = self.backend.clone();
        let filter = MailSearchFilter {
            limit: limit.into(),
            keywords: normalized_query
                .split_whitespace()
                .map(str::to_string)
                .collect(),
            ..Default::default()
        };
        let result = tokio::task::spawn_blocking(move || backend.search_messages(&filter)).await;
        mail_headers_result(operation, started, result)
    }

    async fn read(&mut self, message_id: &ValidatedMailId) -> OperationResult<MailBodyEvidence> {
        let operation = EvidenceOperation::MailRead {
            message_id: message_id.clone(),
        };
        let Ok(rowid) = message_id.as_str().parse::<i64>() else {
            return failed_result(
                operation,
                FailureCode::InvalidInput,
                EvidenceContribution::Empty,
                0,
            );
        };
        if rowid <= 0 {
            return failed_result(
                operation,
                FailureCode::InvalidInput,
                EvidenceContribution::Empty,
                0,
            );
        }

        let started = Instant::now();
        let backend = self.backend.clone();
        let result = tokio::task::spawn_blocking(move || backend.get_message(rowid)).await;
        let duration_ms = elapsed_ms(started);
        match result {
            Ok(Ok(Some(message))) => mail_body_result(operation, message, duration_ms),
            Ok(Ok(None)) => {
                let body = MailBodyEvidence {
                    evidence_id: opaque_evidence_id("mail-body", rowid),
                    header_id: opaque_evidence_id("mail-header", rowid),
                    body: String::new(),
                    body_state: BodyState::UnavailableLocally,
                };
                value_result(operation, body, EvidenceContribution::Empty, duration_ms)
            }
            Ok(Err(error)) => backend_error_result(operation, error, duration_ms),
            Err(_) => failed_result(
                operation,
                FailureCode::OtherNormalized,
                EvidenceContribution::Empty,
                duration_ms,
            ),
        }
    }
}

fn mail_headers_result(
    operation: EvidenceOperation,
    started: Instant,
    result: Result<Result<Vec<MailMessage>, AppleMailBackendError>, tokio::task::JoinError>,
) -> OperationResult<Vec<MailHeaderEvidence>> {
    let duration_ms = elapsed_ms(started);
    match result {
        Ok(Ok(messages)) => {
            let mut headers = Vec::with_capacity(messages.len());
            let mut invalid_items = 0u8;
            for message in messages {
                match mail_header_evidence(message) {
                    Ok(header) => headers.push(header),
                    Err(FailureCode::ParseFailure) => {
                        invalid_items = invalid_items.saturating_add(1)
                    }
                    Err(_) => invalid_items = invalid_items.saturating_add(1),
                }
            }
            if invalid_items > 0 {
                OperationResult {
                    key: operation.key(),
                    attempts: 1,
                    execution: ExecutionStatus::Failed(FailureCode::ParseFailure),
                    contribution: if headers.is_empty() {
                        EvidenceContribution::Empty
                    } else {
                        EvidenceContribution::Partial
                    },
                    value: (!headers.is_empty()).then_some(headers),
                    duration_ms,
                    invalid_items,
                }
            } else {
                let contribution = if headers.is_empty() {
                    EvidenceContribution::Empty
                } else {
                    EvidenceContribution::Satisfied
                };
                value_result(operation, headers, contribution, duration_ms)
            }
        }
        Ok(Err(error)) => backend_error_result(operation, error, duration_ms),
        Err(_) => failed_result(
            operation,
            FailureCode::OtherNormalized,
            EvidenceContribution::Empty,
            duration_ms,
        ),
    }
}

fn mail_header_evidence(message: MailMessage) -> Result<MailHeaderEvidence, FailureCode> {
    if message.rowid <= 0 {
        return Err(FailureCode::ParseFailure);
    }
    let received_at =
        DateTime::from_timestamp(message.received_at, 0).ok_or(FailureCode::ParseFailure)?;
    Ok(MailHeaderEvidence {
        evidence_id: opaque_evidence_id("mail-header", message.rowid),
        connector_id: ValidatedMailId::new(message.rowid.to_string())
            .map_err(|_| FailureCode::ParseFailure)?,
        sender: message.sender,
        subject: message.subject,
        received_at,
    })
}

fn mail_body_result(
    operation: EvidenceOperation,
    message: MailMessage,
    duration_ms: u64,
) -> OperationResult<MailBodyEvidence> {
    if message.rowid <= 0 {
        return failed_result(
            operation,
            FailureCode::ParseFailure,
            EvidenceContribution::Empty,
            duration_ms,
        );
    }
    let (body, body_state, contribution) = match message.body {
        Some(body) if !body.trim().is_empty() => (
            body.chars().take(4_000).collect(),
            BodyState::Readable,
            EvidenceContribution::Satisfied,
        ),
        Some(_) if message.body_available => {
            (String::new(), BodyState::Empty, EvidenceContribution::Empty)
        }
        _ => (
            String::new(),
            BodyState::UnavailableLocally,
            EvidenceContribution::Empty,
        ),
    };
    value_result(
        operation,
        MailBodyEvidence {
            evidence_id: opaque_evidence_id("mail-body", message.rowid),
            header_id: opaque_evidence_id("mail-header", message.rowid),
            body,
            body_state,
        },
        contribution,
        duration_ms,
    )
}

fn opaque_evidence_id(kind: &str, rowid: i64) -> EvidenceId {
    let digest = Sha256::digest(format!("{kind}:{rowid}").as_bytes());
    EvidenceId::new(format!("{kind}-{}", hex_prefix(&digest, 16)))
        .expect("hashed evidence identifiers are valid")
}

fn hex_prefix(bytes: &[u8], count: usize) -> String {
    bytes
        .iter()
        .take(count)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

fn value_result<T>(
    operation: EvidenceOperation,
    value: T,
    contribution: EvidenceContribution,
    duration_ms: u64,
) -> OperationResult<T> {
    OperationResult {
        key: operation.key(),
        attempts: 1,
        execution: ExecutionStatus::Succeeded,
        contribution,
        value: Some(value),
        duration_ms,
        invalid_items: 0,
    }
}

fn failed_result<T>(
    operation: EvidenceOperation,
    failure: FailureCode,
    contribution: EvidenceContribution,
    duration_ms: u64,
) -> OperationResult<T> {
    OperationResult {
        key: operation.key(),
        attempts: 1,
        execution: ExecutionStatus::Failed(failure),
        contribution,
        value: None,
        duration_ms,
        invalid_items: 0,
    }
}

fn backend_error_result<T>(
    operation: EvidenceOperation,
    error: AppleMailBackendError,
    duration_ms: u64,
) -> OperationResult<T> {
    match error {
        AppleMailBackendError::Unavailable => failed_result(
            operation,
            FailureCode::ConnectorUnavailable,
            EvidenceContribution::Empty,
            duration_ms,
        ),
        AppleMailBackendError::TimedOut => OperationResult {
            key: operation.key(),
            attempts: 1,
            execution: ExecutionStatus::TimedOut,
            contribution: EvidenceContribution::Empty,
            value: None,
            duration_ms,
            invalid_items: 0,
        },
        AppleMailBackendError::ConnectionReset => failed_result(
            operation,
            FailureCode::ConnectionReset,
            EvidenceContribution::Empty,
            duration_ms,
        ),
    }
}

pub(crate) trait WebEvidenceAdapter {
    fn search(
        &mut self,
        normalized_query: &str,
        provider_set: &ProviderSet,
    ) -> OperationResult<WebSearchResult>;

    fn fetch(&mut self, candidate_id: &CandidateId) -> OperationResult<WebFetchEvidence>;
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct FakeMailAdapter {
    headers: Vec<MailHeaderEvidence>,
    bodies: std::collections::HashMap<ValidatedMailId, MailBodyEvidence>,
    operations: Vec<EvidenceOperation>,
}

#[cfg(test)]
impl FakeMailAdapter {
    pub(crate) fn with_three_readable_messages() -> Self {
        Self::from_results(super::fixtures::three_readable_messages())
    }

    pub(crate) fn with_duplicate_identifier() -> Self {
        Self::from_results(super::fixtures::duplicate_mail_identifier())
    }

    fn from_results(results: super::EvidenceResults) -> Self {
        let headers = results.mail_list[0].value.clone().unwrap_or_default();
        let bodies = results
            .mail_bodies
            .into_iter()
            .filter_map(|result| result.value)
            .filter_map(|body| {
                let header = headers
                    .iter()
                    .find(|header| header.evidence_id == body.header_id)?;
                Some((header.connector_id.clone(), body))
            })
            .collect();
        Self {
            headers,
            bodies,
            operations: Vec::new(),
        }
    }

    pub(crate) fn operations(&self) -> &[EvidenceOperation] {
        &self.operations
    }
}

#[cfg(test)]
#[async_trait]
impl MailEvidenceAdapter for FakeMailAdapter {
    async fn list(
        &mut self,
        limit: u8,
        unread_only: bool,
    ) -> OperationResult<Vec<MailHeaderEvidence>> {
        let operation = EvidenceOperation::MailList { limit, unread_only };
        self.operations.push(operation.clone());
        OperationResult::succeeded(
            operation.key(),
            self.headers.iter().take(limit.into()).cloned().collect(),
        )
    }

    async fn search(
        &mut self,
        normalized_query: &str,
        limit: u8,
    ) -> OperationResult<Vec<MailHeaderEvidence>> {
        let operation = EvidenceOperation::MailSearch {
            normalized_query: normalized_query.to_string(),
            limit,
        };
        self.operations.push(operation.clone());
        let query = normalized_query.to_lowercase();
        let matches = self
            .headers
            .iter()
            .filter(|header| {
                header.sender.to_lowercase().contains(&query)
                    || header.subject.to_lowercase().contains(&query)
            })
            .take(limit.into())
            .cloned()
            .collect();
        OperationResult::succeeded(operation.key(), matches)
    }

    async fn read(&mut self, message_id: &ValidatedMailId) -> OperationResult<MailBodyEvidence> {
        let operation = EvidenceOperation::MailRead {
            message_id: message_id.clone(),
        };
        self.operations.push(operation.clone());
        match self.bodies.get(message_id).cloned() {
            Some(body) => OperationResult::succeeded(operation.key(), body),
            None => OperationResult::without_value(
                operation.key(),
                super::ExecutionStatus::Failed(super::FailureCode::InvalidInput),
                super::EvidenceContribution::Empty,
            ),
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct FakeWebAdapter {
    pub searches: std::collections::VecDeque<OperationResult<WebSearchResult>>,
    pub fetches: std::collections::HashMap<CandidateId, OperationResult<WebFetchEvidence>>,
    operations: Vec<EvidenceOperation>,
}

#[cfg(test)]
impl FakeWebAdapter {
    pub(crate) fn operations(&self) -> &[EvidenceOperation] {
        &self.operations
    }
}

#[cfg(test)]
impl WebEvidenceAdapter for FakeWebAdapter {
    fn search(
        &mut self,
        normalized_query: &str,
        provider_set: &ProviderSet,
    ) -> OperationResult<WebSearchResult> {
        let operation = EvidenceOperation::WebSearch {
            normalized_query: normalized_query.to_string(),
            provider_set: provider_set.clone(),
        };
        self.operations.push(operation.clone());
        self.searches.pop_front().unwrap_or_else(|| {
            OperationResult::without_value(
                operation.key(),
                super::ExecutionStatus::Failed(super::FailureCode::ConnectorUnavailable),
                super::EvidenceContribution::Empty,
            )
        })
    }

    fn fetch(&mut self, candidate_id: &CandidateId) -> OperationResult<WebFetchEvidence> {
        let operation = EvidenceOperation::WebFetch {
            candidate_id: candidate_id.clone(),
        };
        self.operations.push(operation.clone());
        self.fetches.remove(candidate_id).unwrap_or_else(|| {
            OperationResult::without_value(
                operation.key(),
                super::ExecutionStatus::Failed(super::FailureCode::InvalidInput),
                super::EvidenceContribution::Empty,
            )
        })
    }
}

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct FakeEvidenceClock {
    now: DateTime<Utc>,
}

#[cfg(test)]
impl FakeEvidenceClock {
    pub(crate) fn new(now: DateTime<Utc>) -> Self {
        Self { now }
    }

    pub(crate) fn advance(&mut self, duration: chrono::Duration) {
        self.now += duration;
    }
}

#[cfg(test)]
impl EvidenceClock for FakeEvidenceClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }
}

#[cfg(test)]
mod apple_mail_adapter_tests {
    use super::*;
    use apple_mail_connector::{MailMessage, MailSearchFilter};
    use std::collections::HashMap;

    #[derive(Clone)]
    struct StubAppleMailBackend {
        listed: Result<Vec<MailMessage>, AppleMailBackendError>,
        searched: Result<Vec<MailMessage>, AppleMailBackendError>,
        messages: HashMap<i64, Result<Option<MailMessage>, AppleMailBackendError>>,
    }

    impl Default for StubAppleMailBackend {
        fn default() -> Self {
            Self {
                listed: Ok(Vec::new()),
                searched: Ok(Vec::new()),
                messages: HashMap::new(),
            }
        }
    }

    impl AppleMailBackend for StubAppleMailBackend {
        fn list_inbox(
            &self,
            _limit: usize,
            _unread_only: bool,
        ) -> Result<Vec<MailMessage>, AppleMailBackendError> {
            self.listed.clone()
        }

        fn search_messages(
            &self,
            _filter: &MailSearchFilter,
        ) -> Result<Vec<MailMessage>, AppleMailBackendError> {
            self.searched.clone()
        }

        fn get_message(&self, rowid: i64) -> Result<Option<MailMessage>, AppleMailBackendError> {
            self.messages.get(&rowid).cloned().unwrap_or(Ok(None))
        }
    }

    fn message(rowid: i64, body: Option<&str>, body_available: bool) -> MailMessage {
        MailMessage {
            rowid,
            subject: "Quarterly update".into(),
            sender: "alice@example.com".into(),
            sender_display: Some("Alice".into()),
            recipient: Some("oliver@example.com".into()),
            received_at: 1_753_698_600,
            is_read: false,
            mailbox_url: "imap://example/INBOX".into(),
            body: body.map(str::to_string),
            body_available,
            language: Some("en".into()),
            attachments: Vec::new(),
            message_id: None,
        }
    }

    #[tokio::test]
    async fn real_adapter_keeps_empty_unavailable_malformed_and_readable_results_distinct() {
        let empty_backend = StubAppleMailBackend {
            listed: Ok(Vec::new()),
            searched: Ok(Vec::new()),
            ..Default::default()
        };
        let mut empty = AppleMailEvidenceAdapter::from_backend(empty_backend);
        let empty_result = empty.list(3, false).await;
        assert_eq!(empty_result.execution, ExecutionStatus::Succeeded);
        assert_eq!(empty_result.contribution, EvidenceContribution::Empty);
        assert_eq!(empty_result.value, Some(Vec::new()));

        let unavailable_backend = StubAppleMailBackend {
            listed: Err(AppleMailBackendError::Unavailable),
            searched: Ok(Vec::new()),
            ..Default::default()
        };
        let mut unavailable = AppleMailEvidenceAdapter::from_backend(unavailable_backend);
        let unavailable_result = unavailable.list(3, false).await;
        assert_eq!(
            unavailable_result.execution,
            ExecutionStatus::Failed(FailureCode::ConnectorUnavailable)
        );
        assert_eq!(unavailable_result.value, None);

        let timed_out_backend = StubAppleMailBackend {
            listed: Err(AppleMailBackendError::TimedOut),
            searched: Ok(Vec::new()),
            ..Default::default()
        };
        let mut timed_out = AppleMailEvidenceAdapter::from_backend(timed_out_backend);
        let timed_out_result = timed_out.list(3, false).await;
        assert_eq!(timed_out_result.execution, ExecutionStatus::TimedOut);

        let malformed_backend = StubAppleMailBackend {
            listed: Ok(vec![message(0, None, false)]),
            searched: Ok(Vec::new()),
            ..Default::default()
        };
        let mut malformed = AppleMailEvidenceAdapter::from_backend(malformed_backend);
        let malformed_result = malformed.list(3, false).await;
        assert_eq!(
            malformed_result.execution,
            ExecutionStatus::Failed(FailureCode::ParseFailure)
        );
        let malformed_plan =
            super::super::EvidencePlanner::plan(super::super::EvidenceIntent::MailLatestHeaders {
                count: 3,
                unread_only: false,
            });
        assert!(matches!(
            super::super::EvidenceValidator::validate(
                "turn-malformed",
                &malformed_plan,
                super::super::EvidenceResults {
                    mail_list: vec![malformed_result],
                    ..Default::default()
                },
            ),
            super::super::ValidationOutcome::Recovery(recovery)
                if recovery.kind == super::super::RecoveryKind::Malformed
        ));

        let readable_mixed_message = message(42, Some("Readable"), true);
        let mut mixed_messages = HashMap::new();
        mixed_messages.insert(42, Ok(Some(readable_mixed_message.clone())));
        let mixed_backend = StubAppleMailBackend {
            listed: Ok(vec![readable_mixed_message, message(0, None, false)]),
            searched: Ok(Vec::new()),
            messages: mixed_messages,
        };
        let mut mixed = AppleMailEvidenceAdapter::from_backend(mixed_backend);
        let mixed_result = mixed.list(3, false).await;
        assert_eq!(
            mixed_result.execution,
            ExecutionStatus::Failed(FailureCode::ParseFailure)
        );
        assert_eq!(mixed_result.contribution, EvidenceContribution::Partial);
        assert_eq!(mixed_result.invalid_items, 1);
        assert_eq!(mixed_result.value.as_ref().map(Vec::len), Some(1));
        let mixed_header = mixed_result.value.as_ref().unwrap()[0].clone();
        let mixed_body = mixed.read(&mixed_header.connector_id).await;
        let mixed_plan =
            super::super::EvidencePlanner::plan(super::super::EvidenceIntent::MailLatestContent {
                count: 3,
                requested_count: 3,
                unread_only: false,
            });
        let mixed_validation = super::super::EvidenceValidator::validate(
            "turn-mixed-malformed",
            &mixed_plan,
            super::super::EvidenceResults {
                mail_list: vec![mixed_result],
                mail_bodies: vec![mixed_body],
                ..Default::default()
            },
        );
        assert!(matches!(
            mixed_validation,
            super::super::ValidationOutcome::Bundle(bundle)
                if bundle.completeness == super::super::Completeness::Partial
                    && bundle.acquired.mail_bodies == 1
                    && bundle.missing.iter().any(|missing| {
                        missing.reason == super::super::ShortfallReason::Malformed
                            && missing.missing_count == 1
                    })
                    && bundle.missing.iter().any(|missing| {
                        missing.reason == super::super::ShortfallReason::Empty
                            && missing.missing_count == 1
                    })
        ));

        let readable_message = message(42, Some("A real locally cached body."), true);
        let mut messages = HashMap::new();
        messages.insert(42, Ok(Some(readable_message.clone())));
        let readable_backend = StubAppleMailBackend {
            listed: Ok(vec![readable_message]),
            searched: Ok(Vec::new()),
            messages,
        };
        let mut readable = AppleMailEvidenceAdapter::from_backend(readable_backend);
        let headers = readable.list(1, false).await.value.unwrap();
        let body = readable.read(&headers[0].connector_id).await.value.unwrap();
        assert_eq!(body.body_state, BodyState::Readable);
        assert_eq!(body.body, "A real locally cached body.");
    }

    #[tokio::test]
    async fn empty_search_query_is_invalid_input_not_empty_evidence() {
        let mut adapter = AppleMailEvidenceAdapter::from_backend(StubAppleMailBackend::default());

        let result = adapter.search("   ", 10).await;

        assert_eq!(
            result.execution,
            ExecutionStatus::Failed(FailureCode::InvalidInput)
        );
        assert_eq!(result.contribution, EvidenceContribution::Empty);
        assert_eq!(result.value, None);
    }

    #[tokio::test]
    async fn real_adapter_distinguishes_unavailable_and_empty_bodies() {
        let unavailable_message = message(51, None, false);
        let empty_message = message(52, Some("   "), true);
        let mut messages = HashMap::new();
        messages.insert(51, Ok(Some(unavailable_message.clone())));
        messages.insert(52, Ok(Some(empty_message.clone())));
        let backend = StubAppleMailBackend {
            listed: Ok(vec![unavailable_message, empty_message]),
            searched: Ok(Vec::new()),
            messages,
        };
        let mut adapter = AppleMailEvidenceAdapter::from_backend(backend);
        let headers = adapter.list(2, false).await.value.unwrap();

        let unavailable = adapter.read(&headers[0].connector_id).await.value.unwrap();
        let empty = adapter.read(&headers[1].connector_id).await.value.unwrap();

        assert_eq!(unavailable.body_state, BodyState::UnavailableLocally);
        assert_eq!(empty.body_state, BodyState::Empty);
    }

    #[tokio::test]
    async fn synthesis_safe_typed_payload_never_contains_raw_mail_rowids() {
        let readable_message = message(987_654_321, Some("Grounded body."), true);
        let mut messages = HashMap::new();
        messages.insert(987_654_321, Ok(Some(readable_message.clone())));
        let backend = StubAppleMailBackend {
            listed: Ok(vec![readable_message]),
            searched: Ok(Vec::new()),
            messages,
        };
        let mut adapter = AppleMailEvidenceAdapter::from_backend(backend);
        let header_result = adapter.list(1, false).await;
        let header = header_result.value.as_ref().unwrap()[0].clone();
        let body_result = adapter.read(&header.connector_id).await;
        let plan =
            super::super::EvidencePlanner::plan(super::super::EvidenceIntent::MailLatestContent {
                count: 1,
                requested_count: 1,
                unread_only: false,
            });
        let outcome = super::super::EvidenceValidator::validate(
            "turn-safe",
            &plan,
            super::super::EvidenceResults {
                mail_list: vec![header_result],
                mail_bodies: vec![body_result],
                ..Default::default()
            },
        );
        let super::super::ValidationOutcome::Bundle(bundle) = outcome else {
            panic!("readable adapter output should validate");
        };

        let serialized = serde_json::to_string(&bundle).unwrap();
        assert!(!serialized.contains("987654321"));
        assert!(!serialized.contains("connector_id"));
        assert!(serialized.contains("Grounded body."));
    }
}
