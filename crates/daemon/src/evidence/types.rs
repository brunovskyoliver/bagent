use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use url::Url;

pub(crate) const EVIDENCE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceRequest {
    pub version: u16,
    pub turn_id: String,
    pub session_id: String,
    #[serde(skip_serializing)]
    pub original_text: String,
    pub origin: EvidenceOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EvidenceOrigin {
    Chat,
    Automation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EvidenceIntent {
    MailLatestHeaders {
        count: u8,
        unread_only: bool,
    },
    MailLatestContent {
        count: u8,
        requested_count: u8,
        unread_only: bool,
    },
    MailTargeted {
        query: String,
        needs_content: bool,
    },
    WebDirectPage {
        url: Url,
    },
    WebFact {
        query: String,
        verification: VerificationLevel,
    },
    AnalyzeQuotedEvidence {
        intent: Box<EvidenceIntent>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum VerificationLevel {
    SingleAuthoritative,
    Corroborated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Classification {
    Recognized(EvidenceIntent),
    NeedsClarification {
        prompt: String,
        alternatives: Vec<IntentSummary>,
    },
    NotEvidenceIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IntentSummary {
    pub label: String,
    pub scope: EvidenceScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EvidenceScope {
    MailHeaders,
    MailContent,
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidencePlan {
    pub version: u16,
    pub intent: EvidenceIntent,
    pub requirements: Vec<EvidenceRequirement>,
    pub budget: EvidenceBudget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EvidenceRequirement {
    MailHeaders { count: u8 },
    MailBodies { count: u8 },
    TargetedMail { needs_content: bool },
    DirectPage,
    FetchedSources { count: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceBudget {
    pub mail_list_attempts: u8,
    pub mail_body_attempts: u8,
    pub web_search_attempts: u8,
    pub web_fetch_attempts: u8,
    pub max_parallel_fetches: u8,
    pub optional_exploration_rounds: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) struct OperationKey(String);

impl OperationKey {
    pub(crate) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EvidenceOperation {
    MailList {
        limit: u8,
        unread_only: bool,
    },
    MailSearch {
        normalized_query: String,
        limit: u8,
    },
    MailRead {
        message_id: ValidatedMailId,
    },
    WebSearch {
        normalized_query: String,
        provider_set: ProviderSet,
    },
    WebFetch {
        candidate_id: CandidateId,
    },
}

impl EvidenceOperation {
    pub(crate) fn key(&self) -> OperationKey {
        let value = match self {
            Self::MailList { limit, unread_only } => {
                format!("mail_list:{limit}:{unread_only}")
            }
            Self::MailSearch {
                normalized_query,
                limit,
            } => format!(
                "mail_search:{limit}:{}",
                normalize_key_part(normalized_query)
            ),
            Self::MailRead { message_id } => format!("mail_read:{}", message_id.as_str()),
            Self::WebSearch {
                normalized_query,
                provider_set,
            } => format!(
                "web_search:{}:{}",
                provider_set.key_part(),
                normalize_key_part(normalized_query)
            ),
            Self::WebFetch { candidate_id } => {
                format!("web_fetch:{}", candidate_id.as_str())
            }
        };
        OperationKey::new(value)
    }
}

fn normalize_key_part(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

macro_rules! validated_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        pub(crate) struct $name(String);

        impl $name {
            pub(crate) fn new(value: impl Into<String>) -> Result<Self, FailureCode> {
                let value = value.into();
                if value.trim().is_empty() || value.chars().any(char::is_control) {
                    Err(FailureCode::InvalidInput)
                } else {
                    Ok(Self(value))
                }
            }

            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(|_| serde::de::Error::custom("invalid evidence id"))
            }
        }
    };
}

validated_id!(ValidatedMailId);
validated_id!(CandidateId);
validated_id!(EvidenceId);
validated_id!(SourceIdentity);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderSet(pub Vec<WebProvider>);

impl ProviderSet {
    fn key_part(&self) -> String {
        let mut providers = self.0.iter().map(WebProvider::as_str).collect::<Vec<_>>();
        providers.sort_unstable();
        providers.dedup();
        providers.join("+")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum WebProvider {
    DuckDuckGo,
    Wikipedia,
    Direct,
}

impl WebProvider {
    fn as_str(&self) -> &'static str {
        match self {
            Self::DuckDuckGo => "duckduckgo",
            Self::Wikipedia => "wikipedia",
            Self::Direct => "direct",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ExecutionStatus {
    Succeeded,
    Failed(FailureCode),
    Denied,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FailureCode {
    InvalidInput,
    ConnectorUnavailable,
    ConnectionReset,
    RateLimited,
    Http4xx(u16),
    Http5xx(u16),
    UnsupportedContentType,
    UnsafeDestination,
    RedirectUnsafe,
    BodyTooLarge,
    EmptyExtraction,
    ProviderChallenge,
    ParseFailure,
    ModelUnavailable,
    ModelInvalidOutput,
    OtherNormalized,
}

impl FailureCode {
    pub(crate) fn retryable(&self) -> bool {
        matches!(
            self,
            Self::ConnectionReset | Self::RateLimited | Self::Http5xx(_)
        )
    }
}

impl ExecutionStatus {
    pub(crate) fn retryable(&self) -> bool {
        matches!(self, Self::TimedOut)
            || matches!(self, Self::Failed(failure) if failure.retryable())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EvidenceContribution {
    Satisfied,
    Partial,
    Empty,
    Duplicate,
    Irrelevant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OperationResult<T> {
    pub key: OperationKey,
    pub attempts: u8,
    pub execution: ExecutionStatus,
    pub contribution: EvidenceContribution,
    pub value: Option<T>,
    pub duration_ms: u64,
}

impl<T> OperationResult<T> {
    pub(crate) fn succeeded(key: OperationKey, value: T) -> Self {
        Self {
            key,
            attempts: 1,
            execution: ExecutionStatus::Succeeded,
            contribution: EvidenceContribution::Satisfied,
            value: Some(value),
            duration_ms: 0,
        }
    }

    pub(crate) fn without_value(
        key: OperationKey,
        execution: ExecutionStatus,
        contribution: EvidenceContribution,
    ) -> Self {
        Self {
            key,
            attempts: 1,
            execution,
            contribution,
            value: None,
            duration_ms: 0,
        }
    }

    pub(crate) fn retry_permitted(&self, remaining_global_budget: u8) -> bool {
        remaining_global_budget > 0 && self.attempts < 2 && self.execution.retryable()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MailHeaderEvidence {
    pub evidence_id: EvidenceId,
    pub connector_id: ValidatedMailId,
    pub sender: String,
    pub subject: String,
    pub received_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MailBodyEvidence {
    pub evidence_id: EvidenceId,
    pub header_id: EvidenceId,
    pub body: String,
    pub body_state: BodyState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum BodyState {
    Readable,
    UnavailableLocally,
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WebSearchResult {
    pub providers: Vec<ProviderResult>,
    pub candidates: Vec<WebCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderResult {
    pub provider: WebProvider,
    pub status: ProviderStatus,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ProviderStatus {
    Succeeded { result_count: u16 },
    Empty,
    Challenged,
    Failed(FailureCode),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WebCandidate {
    pub candidate_id: CandidateId,
    pub provider: WebProvider,
    pub rank: u16,
    pub title: String,
    pub requested_url: Url,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WebFetchEvidence {
    pub evidence_id: EvidenceId,
    pub candidate_id: CandidateId,
    pub requested_url: Url,
    pub final_url: Url,
    pub redirect_chain: Vec<Url>,
    pub http_status: u16,
    pub content_type: String,
    pub bytes_read: u64,
    pub characters_extracted: u64,
    pub extraction: ExtractionStatus,
    pub authority: SourceAuthority,
    pub source_identity: SourceIdentity,
    pub passages: Vec<EvidencePassage>,
    pub links: Vec<ValidatedReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SourceAuthority {
    FirstParty,
    AuthoritativeReference,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ExtractionStatus {
    Readable,
    ReadableTruncated,
    Empty,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidencePassage {
    pub passage_id: EvidenceId,
    pub text: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ValidatedReference {
    pub url: Url,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EvidenceResults {
    pub mail_list: Vec<OperationResult<Vec<MailHeaderEvidence>>>,
    pub mail_search: Vec<OperationResult<Vec<MailHeaderEvidence>>>,
    pub mail_bodies: Vec<OperationResult<MailBodyEvidence>>,
    pub web_searches: Vec<OperationResult<WebSearchResult>>,
    pub web_fetches: Vec<OperationResult<WebFetchEvidence>>,
    pub conflicts: Vec<EvidenceConflict>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceBundle {
    pub version: u16,
    pub turn_id: String,
    pub intent: EvidenceIntent,
    pub completeness: Completeness,
    pub requested: EvidenceCounts,
    pub acquired: EvidenceCounts,
    pub missing: Vec<EvidenceShortfall>,
    pub mail: Vec<MailBundleItem>,
    pub web: Vec<WebBundleItem>,
    pub conflicts: Vec<EvidenceConflict>,
    pub exclusions: Vec<EvidenceExclusion>,
    pub citation_allowlist: Vec<CitationTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Completeness {
    Complete,
    Partial,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceCounts {
    pub mail_headers: u8,
    pub mail_bodies: u8,
    pub web_sources: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceShortfall {
    pub requirement: EvidenceRequirement,
    pub missing_count: u8,
    pub reason: ShortfallReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ShortfallReason {
    Empty,
    Denied,
    Unavailable,
    BodyUnavailable,
    Duplicate,
    VerificationFailed,
    Ambiguous,
    BatchLimit,
    ExcludedAsInstruction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MailBundleItem {
    pub evidence_id: EvidenceId,
    pub sender: String,
    pub subject: String,
    pub received_at: DateTime<Utc>,
    pub body: Option<String>,
    pub body_state: Option<BodyState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WebBundleItem {
    pub evidence: WebFetchEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceConflict {
    pub evidence_ids: Vec<EvidenceId>,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EvidenceExclusion {
    pub evidence_id: EvidenceId,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CitationTarget {
    pub evidence_id: EvidenceId,
    pub url: Url,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValidationOutcome {
    Bundle(Box<EvidenceBundle>),
    Recovery(RecoveryOutcome),
    Clarification {
        headers: Vec<MailHeaderEvidence>,
        prompt: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RecoveryOutcome {
    pub kind: RecoveryKind,
    pub requested: EvidenceCounts,
    pub message: String,
    pub missing: Vec<EvidenceShortfall>,
    pub exclusions: Vec<EvidenceExclusion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryKind {
    Empty,
    Unavailable,
    Denied,
    VerificationShortfall,
    NoUsableEvidence,
}
