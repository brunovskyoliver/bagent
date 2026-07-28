use super::{
    CandidateId, EvidenceOperation, MailBodyEvidence, MailHeaderEvidence, OperationResult,
    ProviderSet, ValidatedMailId, WebFetchEvidence, WebSearchResult,
};
use chrono::{DateTime, Utc};

pub(crate) trait EvidenceClock {
    fn now(&self) -> DateTime<Utc>;
}

pub(crate) struct SystemEvidenceClock;

impl EvidenceClock for SystemEvidenceClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

pub(crate) trait MailEvidenceAdapter {
    fn list(&mut self, limit: u8, unread_only: bool) -> OperationResult<Vec<MailHeaderEvidence>>;

    fn search(
        &mut self,
        normalized_query: &str,
        limit: u8,
    ) -> OperationResult<Vec<MailHeaderEvidence>>;

    fn read(&mut self, message_id: &ValidatedMailId) -> OperationResult<MailBodyEvidence>;
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
        let results = super::fixtures::three_readable_messages();
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
impl MailEvidenceAdapter for FakeMailAdapter {
    fn list(&mut self, limit: u8, unread_only: bool) -> OperationResult<Vec<MailHeaderEvidence>> {
        let operation = EvidenceOperation::MailList { limit, unread_only };
        self.operations.push(operation.clone());
        OperationResult::succeeded(
            operation.key(),
            self.headers.iter().take(limit.into()).cloned().collect(),
        )
    }

    fn search(
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

    fn read(&mut self, message_id: &ValidatedMailId) -> OperationResult<MailBodyEvidence> {
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
