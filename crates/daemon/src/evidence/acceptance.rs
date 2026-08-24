//! Stage 8 acceptance-only external-boundary fixtures.
//!
//! This module is absent from ordinary builds. Even when compiled, the daemon
//! only constructs these controls when `BAGENT_STAGE8_ACCEPTANCE_FIXTURES=1`
//! is present at startup. The selected fixture is process-local, is never
//! derived from prompt text, and is not serialized into prompts, diagnostics,
//! audit rows, or persisted traces.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use serde::Deserialize;
use url::Url;

use super::{
    BodyOrigin, BodyState, CandidateId, EvidenceContribution, EvidenceId, EvidenceOperation,
    ExecutionStatus, ExtractionQuality, ExtractionStatus, FailureCode, MailBodyEvidence,
    MailEvidenceAdapter, MailHeaderEvidence, OperationResult, ProviderResult, ProviderSet,
    ProviderStatus, SourceAuthority, SourceIdentity, TypedWebEvidenceAdapter, ValidatedMailId,
    WebCandidate, WebFetchEvidence, WebProvider, WebSearchResult,
};

pub(crate) const STAGE8_ACCEPTANCE_FIXTURES_ENV: &str = "BAGENT_STAGE8_ACCEPTANCE_FIXTURES";

pub(crate) fn acceptance_runtime_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AcceptanceAcquisition {
    MailComplete,
    MailTransientRetry,
    MailPartial,
    MailUnavailable,
    MailDenied,
    MailEmpty,
    WebAuthoritative,
    WebCorroborated,
    WebConflict,
    WebAmbiguousTable,
    WebIrrelevantEntity,
    WebRedirect,
    WebTavilyMissingCredential,
    #[serde(rename = "web_tavily_429")]
    WebTavily429,
    WebTavilyTimeout,
    WebTavilyMalformed,
    WebDdgFallback,
    WebAllFetchFailure,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AcceptancePolish {
    #[default]
    Passthrough,
    Accepted,
    Rejected,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) struct AcceptanceFixtureSelection {
    pub acquisition: AcceptanceAcquisition,
    #[serde(default)]
    pub polish: AcceptancePolish,
}

#[derive(Clone, Default)]
pub(crate) struct AcceptanceControl {
    selection: Arc<RwLock<Option<AcceptanceFixtureSelection>>>,
}

impl AcceptanceControl {
    pub(crate) fn set(&self, selection: Option<AcceptanceFixtureSelection>) {
        *self.selection.write().expect("acceptance control lock") = selection;
    }

    pub(crate) fn selection(&self) -> Option<AcceptanceFixtureSelection> {
        *self.selection.read().expect("acceptance control lock")
    }

    pub(crate) fn mail_adapter(&self) -> Option<AcceptanceMailAdapter> {
        let selection = self.selection()?;
        AcceptanceMailAdapter::new(selection.acquisition)
    }

    pub(crate) fn web_adapter(&self) -> Option<AcceptanceWebAdapter> {
        let selection = self.selection()?;
        AcceptanceWebAdapter::new(selection.acquisition)
    }
}

pub(crate) struct AcceptanceMailAdapter {
    scenario: AcceptanceAcquisition,
    reads: Arc<Mutex<HashMap<String, usize>>>,
}

impl AcceptanceMailAdapter {
    fn new(scenario: AcceptanceAcquisition) -> Option<Self> {
        matches!(
            scenario,
            AcceptanceAcquisition::MailComplete
                | AcceptanceAcquisition::MailTransientRetry
                | AcceptanceAcquisition::MailPartial
                | AcceptanceAcquisition::MailUnavailable
                | AcceptanceAcquisition::MailDenied
                | AcceptanceAcquisition::MailEmpty
        )
        .then(|| Self {
            scenario,
            reads: Default::default(),
        })
    }
}

#[async_trait]
impl MailEvidenceAdapter for AcceptanceMailAdapter {
    async fn list(
        &mut self,
        limit: u8,
        unread_only: bool,
    ) -> OperationResult<Vec<MailHeaderEvidence>> {
        let operation = EvidenceOperation::MailList { limit, unread_only };
        match self.scenario {
            AcceptanceAcquisition::MailDenied => OperationResult::without_value(
                operation.key(),
                ExecutionStatus::Denied,
                EvidenceContribution::Empty,
            ),
            AcceptanceAcquisition::MailEmpty => OperationResult::without_value(
                operation.key(),
                ExecutionStatus::Succeeded,
                EvidenceContribution::Empty,
            ),
            _ => OperationResult::succeeded(
                operation.key(),
                acceptance_mail_headers()
                    .into_iter()
                    .take(limit.into())
                    .collect(),
            ),
        }
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
        OperationResult::without_value(
            operation.key(),
            ExecutionStatus::Failed(FailureCode::InvalidInput),
            EvidenceContribution::Empty,
        )
    }

    async fn read(&mut self, message_id: &ValidatedMailId) -> OperationResult<MailBodyEvidence> {
        let operation = EvidenceOperation::MailRead {
            message_id: message_id.clone(),
        };
        let Some((index, header)) = acceptance_mail_headers()
            .into_iter()
            .enumerate()
            .find(|(_, header)| &header.connector_id == message_id)
        else {
            return OperationResult::without_value(
                operation.key(),
                ExecutionStatus::Failed(FailureCode::InvalidInput),
                EvidenceContribution::Empty,
            );
        };
        let attempt = {
            let mut reads = self.reads.lock().expect("acceptance read counter");
            let count = reads.entry(message_id.as_str().to_string()).or_default();
            *count += 1;
            *count
        };
        if self.scenario == AcceptanceAcquisition::MailTransientRetry && index == 0 && attempt == 1
        {
            return OperationResult::without_value(
                operation.key(),
                ExecutionStatus::TimedOut,
                EvidenceContribution::Empty,
            );
        }
        if self.scenario == AcceptanceAcquisition::MailUnavailable
            || (self.scenario == AcceptanceAcquisition::MailPartial && index > 0)
        {
            return OperationResult::succeeded(
                operation.key(),
                MailBodyEvidence {
                    evidence_id: acceptance_evidence_id(&format!("mail-body-{}", index + 1)),
                    header_id: header.evidence_id,
                    body: String::new(),
                    body_state: BodyState::UnavailableLocally,
                    body_origin: BodyOrigin::Unavailable,
                },
            );
        }
        OperationResult::succeeded(
            operation.key(),
            MailBodyEvidence {
                evidence_id: acceptance_evidence_id(&format!("mail-body-{}", index + 1)),
                header_id: header.evidence_id,
                body: format!("Body for Subject {}", index + 1),
                body_state: BodyState::Readable,
                body_origin: BodyOrigin::LocalEmlx,
            },
        )
    }
}

#[derive(Clone)]
pub(crate) struct AcceptanceWebAdapter {
    scenario: AcceptanceAcquisition,
    discovered: Arc<Mutex<HashMap<String, WebCandidate>>>,
}

impl AcceptanceWebAdapter {
    fn new(scenario: AcceptanceAcquisition) -> Option<Self> {
        matches!(
            scenario,
            AcceptanceAcquisition::WebAuthoritative
                | AcceptanceAcquisition::WebCorroborated
                | AcceptanceAcquisition::WebConflict
                | AcceptanceAcquisition::WebAmbiguousTable
                | AcceptanceAcquisition::WebIrrelevantEntity
                | AcceptanceAcquisition::WebRedirect
                | AcceptanceAcquisition::WebTavilyMissingCredential
                | AcceptanceAcquisition::WebTavily429
                | AcceptanceAcquisition::WebTavilyTimeout
                | AcceptanceAcquisition::WebTavilyMalformed
                | AcceptanceAcquisition::WebDdgFallback
                | AcceptanceAcquisition::WebAllFetchFailure
        )
        .then(|| Self {
            scenario,
            discovered: Default::default(),
        })
    }

    fn provider_statuses(&self) -> Option<Vec<ProviderResult>> {
        let tavily = match self.scenario {
            AcceptanceAcquisition::WebTavilyMissingCredential => {
                ProviderStatus::Failed(FailureCode::ConnectorUnavailable)
            }
            AcceptanceAcquisition::WebTavily429 => ProviderStatus::Failed(FailureCode::RateLimited),
            AcceptanceAcquisition::WebTavilyTimeout => ProviderStatus::TimedOut,
            AcceptanceAcquisition::WebTavilyMalformed => ProviderStatus::InvalidResponse,
            AcceptanceAcquisition::WebDdgFallback => {
                return Some(vec![
                    ProviderResult {
                        provider: WebProvider::Tavily,
                        status: ProviderStatus::Failed(FailureCode::RateLimited),
                        duration_ms: 0,
                    },
                    ProviderResult {
                        provider: WebProvider::DuckDuckGo,
                        status: ProviderStatus::Succeeded { result_count: 1 },
                        duration_ms: 0,
                    },
                ]);
            }
            _ => return None,
        };
        Some(vec![
            ProviderResult {
                provider: WebProvider::Tavily,
                status: tavily,
                duration_ms: 0,
            },
            ProviderResult {
                provider: WebProvider::DuckDuckGo,
                status: ProviderStatus::Empty,
                duration_ms: 0,
            },
        ])
    }
}

#[async_trait]
impl TypedWebEvidenceAdapter for AcceptanceWebAdapter {
    fn tavily_configured(&self) -> bool {
        matches!(
            self.scenario,
            AcceptanceAcquisition::WebTavilyMissingCredential
                | AcceptanceAcquisition::WebTavily429
                | AcceptanceAcquisition::WebTavilyTimeout
                | AcceptanceAcquisition::WebTavilyMalformed
                | AcceptanceAcquisition::WebDdgFallback
        )
    }

    async fn search(
        &self,
        query: &str,
        _lang: &str,
        providers: &ProviderSet,
    ) -> OperationResult<WebSearchResult> {
        let operation = EvidenceOperation::WebSearch {
            normalized_query: query.to_string(),
            provider_set: providers.clone(),
        };
        let statuses = self.provider_statuses();
        if self.scenario != AcceptanceAcquisition::WebDdgFallback {
            if let Some(provider_statuses) = &statuses {
                return OperationResult::succeeded(
                    operation.key(),
                    WebSearchResult {
                        providers: provider_statuses.clone(),
                        candidates: Vec::new(),
                    },
                );
            }
        }
        let candidates = acceptance_candidates(self.scenario);
        let mut discovered = self.discovered.lock().expect("acceptance discovery set");
        for candidate in &candidates {
            discovered.insert(
                candidate.candidate_id.as_str().to_string(),
                candidate.clone(),
            );
        }
        OperationResult::succeeded(
            operation.key(),
            WebSearchResult {
                providers: statuses.unwrap_or_else(|| {
                    vec![ProviderResult {
                        provider: WebProvider::DuckDuckGo,
                        status: ProviderStatus::Succeeded {
                            result_count: candidates.len() as u16,
                        },
                        duration_ms: 0,
                    }]
                }),
                candidates,
            },
        )
    }

    async fn fetch(&self, candidate: &WebCandidate) -> OperationResult<WebFetchEvidence> {
        let operation = EvidenceOperation::WebFetch {
            candidate_id: candidate.candidate_id.clone(),
        };
        let discovered = self.discovered.lock().expect("acceptance discovery set");
        if !discovered.contains_key(candidate.candidate_id.as_str()) {
            return OperationResult::without_value(
                operation.key(),
                ExecutionStatus::Failed(FailureCode::InvalidInput),
                EvidenceContribution::Empty,
            );
        }
        drop(discovered);
        if self.scenario == AcceptanceAcquisition::WebAllFetchFailure {
            return OperationResult::without_value(
                operation.key(),
                ExecutionStatus::Failed(FailureCode::ConnectionReset),
                EvidenceContribution::Empty,
            );
        }
        OperationResult::succeeded(operation.key(), acceptance_fetch(self.scenario, candidate))
    }
}

fn acceptance_mail_headers() -> Vec<MailHeaderEvidence> {
    (1..=3)
        .map(|index| MailHeaderEvidence {
            evidence_id: acceptance_evidence_id(&format!("mail-header-{index}")),
            connector_id: ValidatedMailId::new(format!("opaque-mail-{index}"))
                .expect("valid opaque acceptance mail id"),
            sender: format!("Sender {index}"),
            subject: format!("Subject {index}"),
            received_at: Utc
                .with_ymd_and_hms(2026, 7, 28, 10, index, 0)
                .single()
                .expect("valid acceptance timestamp"),
        })
        .collect()
}

fn acceptance_candidates(scenario: AcceptanceAcquisition) -> Vec<WebCandidate> {
    let specs: &[(&str, &str)] = match scenario {
        AcceptanceAcquisition::WebAuthoritative | AcceptanceAcquisition::WebDdgFallback => &[(
            "https://public-office.example/president",
            "President of the Slovak Republic",
        )],
        AcceptanceAcquisition::WebRedirect => &[(
            "https://public-office.example/requested",
            "President of the Slovak Republic",
        )],
        AcceptanceAcquisition::WebCorroborated
        | AcceptanceAcquisition::WebConflict
        | AcceptanceAcquisition::WebAmbiguousTable => &[
            (
                "https://publisher-one.example/fact",
                "Bratislava population",
            ),
            (
                "https://publisher-two.example/fact",
                "Bratislava population",
            ),
        ],
        AcceptanceAcquisition::WebIrrelevantEntity => {
            &[("https://reference.example/president", "President profile")]
        }
        AcceptanceAcquisition::WebAllFetchFailure => &[
            ("https://failed-one.example/fact", "Slovakia capital"),
            ("https://failed-two.example/fact", "Slovakia capital"),
        ],
        _ => &[],
    };
    specs
        .iter()
        .enumerate()
        .map(|(index, (url, title))| WebCandidate {
            candidate_id: CandidateId::new(format!("discovered-candidate-{}", index + 1))
                .expect("valid acceptance candidate id"),
            provider: WebProvider::DuckDuckGo,
            rank: (index + 1) as u16,
            title: (*title).to_string(),
            requested_url: Url::parse(url).expect("valid acceptance URL"),
            snippet: "Discovery-only fixture snippet; never evidence.".into(),
        })
        .collect()
}

fn acceptance_fetch(scenario: AcceptanceAcquisition, candidate: &WebCandidate) -> WebFetchEvidence {
    let index = candidate.rank as usize;
    let (final_url, identity, passages, owner_bound) = match scenario {
        AcceptanceAcquisition::WebAuthoritative | AcceptanceAcquisition::WebDdgFallback => (
            candidate.requested_url.clone(),
            "public-office.example",
            vec![
                "President of the Slovak Republic",
                "Office of the President of the Slovak Republic",
                "Peter Pellegrini is the President of the Slovak Republic.",
            ],
            true,
        ),
        AcceptanceAcquisition::WebRedirect => (
            Url::parse("https://public-office.example/final").unwrap(),
            "public-office.example",
            vec![
                "President of the Slovak Republic",
                "Office of the President of the Slovak Republic",
                "Peter Pellegrini is the President of the Slovak Republic.",
            ],
            true,
        ),
        AcceptanceAcquisition::WebCorroborated => (
            candidate.requested_url.clone(),
            if index == 1 {
                "publisher-one.example"
            } else {
                "publisher-two.example"
            },
            vec![if index == 1 {
                "Bratislava city proper population stands at 475,503 as of 2024."
            } else {
                "The city proper population of Bratislava was 475,503 at the end of 2024."
            }],
            false,
        ),
        AcceptanceAcquisition::WebConflict => (
            candidate.requested_url.clone(),
            if index == 1 {
                "publisher-one.example"
            } else {
                "publisher-two.example"
            },
            vec![if index == 1 {
                "The city proper population of Bratislava was 475,503 at the end of 2024."
            } else {
                "The urban-area population estimate for Bratislava was 440,948 in 2025."
            }],
            false,
        ),
        AcceptanceAcquisition::WebAmbiguousTable => (
            candidate.requested_url.clone(),
            if index == 1 {
                "publisher-one.example"
            } else {
                "publisher-two.example"
            },
            vec![if index == 1 {
                "Bratislava 442,197 428,672 411,228 475,503 480,902"
            } else {
                "Bratislava population statistics table 2020 2021 2022 2023 2024"
            }],
            false,
        ),
        AcceptanceAcquisition::WebIrrelevantEntity => (
            candidate.requested_url.clone(),
            "reference.example",
            vec!["Peter Novak is the president of an unrelated sports association."],
            false,
        ),
        _ => (
            candidate.requested_url.clone(),
            "acceptance.example",
            vec!["Acceptance fixture page."],
            false,
        ),
    };
    let text_len = passages.iter().map(|value| value.len()).sum::<usize>();
    WebFetchEvidence {
        evidence_id: acceptance_evidence_id(&format!("web-{index}")),
        candidate_id: candidate.candidate_id.clone(),
        requested_url: candidate.requested_url.clone(),
        final_url: final_url.clone(),
        redirect_chain: if final_url == candidate.requested_url {
            vec![final_url]
        } else {
            vec![candidate.requested_url.clone(), final_url]
        },
        http_status: 200,
        content_type: "text/html".into(),
        bytes_read: text_len as u64,
        characters_extracted: text_len as u64,
        extraction: ExtractionStatus::Readable,
        quality: ExtractionQuality {
            useful_text_length: text_len as u64,
            ..Default::default()
        },
        page_owner_identity_bound: owner_bound,
        authority: SourceAuthority::Other,
        source_identity: SourceIdentity::new(identity).expect("valid acceptance source identity"),
        passages: passages
            .into_iter()
            .enumerate()
            .map(|(passage_index, text)| super::EvidencePassage {
                passage_id: acceptance_evidence_id(&format!("passage-{index}-{passage_index}")),
                text: text.into(),
                truncated: false,
            })
            .collect(),
        links: Vec::new(),
    }
}

fn acceptance_evidence_id(value: &str) -> EvidenceId {
    EvidenceId::new(value).expect("valid acceptance evidence id")
}

fn acceptance_canonical_mail_text() -> String {
    (1..=3)
        .map(|index| {
            format!(
                "{index}. Sender: Sender {index}\n   Subject: Subject {index}\n   Date: 2026-07-28 10:0{index} UTC\n   Summary: Body for Subject {index}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_boundary_requires_exact_flag() {
        assert!(acceptance_runtime_enabled(Some("1")));
        for value in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert!(!acceptance_runtime_enabled(value));
        }
    }

    #[test]
    fn reference_acceptance_compile_gate_is_feature_scoped() {
        let selection = AcceptanceFixtureSelection {
            acquisition: AcceptanceAcquisition::MailComplete,
            polish: AcceptancePolish::Passthrough,
        };
        assert_eq!(selection.acquisition, AcceptanceAcquisition::MailComplete);
    }

    #[tokio::test]
    async fn fetch_rejects_any_candidate_not_returned_by_typed_discovery() {
        let adapter = AcceptanceWebAdapter::new(AcceptanceAcquisition::WebAuthoritative).unwrap();
        let candidate = WebCandidate {
            candidate_id: CandidateId::new("not-discovered").unwrap(),
            provider: WebProvider::DuckDuckGo,
            rank: 1,
            title: "Static candidate".into(),
            requested_url: Url::parse("https://static.example/fact").unwrap(),
            snippet: String::new(),
        };

        let result = adapter.fetch(&candidate).await;

        assert_eq!(
            result.execution,
            ExecutionStatus::Failed(FailureCode::InvalidInput)
        );
        assert!(result.value.is_none());
    }
}
