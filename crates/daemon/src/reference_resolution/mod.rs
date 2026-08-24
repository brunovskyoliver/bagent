#![allow(dead_code)]
#![allow(unused_imports)]

mod artifacts;
mod confirmation;
mod crypto;
mod extraction;
mod mode;
mod query;
mod repository;
mod runtime;
mod types;

#[cfg(test)]
#[path = "contract_tests/mod.rs"]
mod contract;

pub(crate) use artifacts::CompletedTurnArtifacts;
pub(crate) use confirmation::{
    normalize_term, ConfirmationDisposition, ConfirmationRequestKind, COMPATIBILITY_EPOCH,
    CONFIRMATION_TTL_MS, NORMALIZATION_VERSION,
};
pub(crate) use extraction::{
    enumerate_spans, extract, resolve, validate_model_order, Extraction, ExtractionError,
    ResolutionTrace, StructuralDisposition, StructuralOutcome, MAX_MESSAGE_BYTES, MAX_SPANS,
    MAX_SPAN_BYTES,
};
pub(crate) use mode::{
    parse_resolver_mode, parse_resolver_mode_with_status, select_resolver_mode, ParsedResolverMode,
    ResolverMode, ResolverModeParseStatus, DEFAULT_RESOLVER_MODE, REFERENCE_RESOLVER_MODE_ENV,
};
pub(crate) use query::admit_provider_query;
pub(crate) use query::normalize_public_url_for_adapter;
#[cfg(test)]
pub(crate) use query::test_authorized_direct_fetch;
#[cfg(test)]
pub(crate) use query::{
    compose_comparison_query_for_test, compose_query_for_test, normalize_public_term_for_test,
    QueryFocus, QueryKind, QueryModifiers, QueryOperation, QueryReferentInput,
};
pub(crate) use runtime::{select_runtime, ResolverRuntime, RuntimeSelection};
pub(crate) use types::*;

use async_trait::async_trait;
use repository::ReferenceRepository;
use std::sync::Arc;
use tokio::sync::Mutex;

/// The only crate-visible behavioral seam for conversational reference
/// resolution. Construction and implementation details stay inside this
/// module.
#[async_trait]
pub(crate) trait ConversationalReferenceResolver: Send + Sync {
    async fn startup(&self) -> Result<(), ResolverFault> {
        Ok(())
    }

    async fn maintenance_prune(&self, _now_ms: i64) -> Result<(), ResolverFault> {
        Ok(())
    }

    async fn resolve_turn(
        &self,
        input: ResolveTurn,
    ) -> Result<ReferenceRoutingDecision, ResolverFault>;

    async fn record_completed_turn(
        &self,
        artifacts: CompletedTurnArtifacts,
    ) -> Result<(), ResolverFault>;

    async fn admit_provider_query(
        &self,
        permit: &ProviderQueryPermit,
        operation: ProviderOperation,
    ) -> ProviderQueryAuthorization;
}

#[derive(Default)]
struct InertResolver;

#[async_trait]
impl ConversationalReferenceResolver for InertResolver {
    async fn resolve_turn(
        &self,
        _input: ResolveTurn,
    ) -> Result<ReferenceRoutingDecision, ResolverFault> {
        Err(ResolverFault::Unavailable)
    }

    async fn record_completed_turn(
        &self,
        _artifacts: CompletedTurnArtifacts,
    ) -> Result<(), ResolverFault> {
        Ok(())
    }

    async fn admit_provider_query(
        &self,
        _permit: &ProviderQueryPermit,
        _operation: ProviderOperation,
    ) -> ProviderQueryAuthorization {
        ProviderQueryAuthorization::Denied {
            reason: AuthorizationDenial::ResolverUnavailable,
        }
    }
}

/// Return the contracts-only implementation. It performs no I/O, persistence,
/// configuration lookup, key access, diagnostics, ranking, or provider work.
pub(crate) fn production() -> Arc<dyn ConversationalReferenceResolver> {
    Arc::new(InertResolver)
}

struct PersistentResolver {
    repository: Arc<repository::SqliteRepository>,
}

#[async_trait]
impl ConversationalReferenceResolver for PersistentResolver {
    async fn startup(&self) -> Result<(), ResolverFault> {
        self.repository
            .readiness()
            .await
            .map_err(|_| ResolverFault::Unavailable)?;
        self.repository
            .prune_with_clock()
            .await
            .map_err(|_| ResolverFault::Unavailable)?;
        Ok(())
    }

    async fn maintenance_prune(&self, now_ms: i64) -> Result<(), ResolverFault> {
        self.repository
            .prune(now_ms)
            .await
            .map(|_| ())
            .map_err(|_| ResolverFault::Unavailable)
    }

    async fn resolve_turn(
        &self,
        input: ResolveTurn,
    ) -> Result<ReferenceRoutingDecision, ResolverFault> {
        let Some(envelope) = input.confirmation() else {
            return Err(ResolverFault::Unavailable);
        };
        let confirmation_id = envelope.challenge_id().as_uuid().to_string();
        let kind = match input.action() {
            RequestAction::Confirmation => ConfirmationRequestKind::Confirm,
            RequestAction::EditedUserText => ConfirmationRequestKind::Edit,
            RequestAction::UserText => return Err(ResolverFault::Unavailable),
        };
        if input.origin() == TurnOrigin::Automation {
            return Ok(ReferenceRoutingDecision::Confirmation(
                ConfirmationDisposition::BlockedInteractiveAction,
            ));
        }
        let turn_id = input.turn_id().as_uuid().to_string();
        let session_id = input.session_id().as_str().to_string();
        let submitted = confirmation_submitted_bytes(kind, envelope, input.current_input());
        let ledger_input = submitted.clone();
        let request_session_id = session_id.clone();
        let request_turn_id = turn_id.clone();
        let snapshot = self
            .repository
            .transact_resolution(
                repository::ResolveLedgerCommand {
                    turn_id: turn_id.clone(),
                    session_id: session_id.clone(),
                    scope_id: session_id.clone(),
                    chat_session_id: Some(session_id.clone()),
                    automation_id: None,
                    automation_run_id: None,
                    origin: "chat",
                    input: ledger_input,
                    descriptors: Vec::new(),
                    now_ms: now_ms(),
                },
                Box::new(move |_| {
                    Ok(repository::ResolutionDecision::ConsumeConfirmationRequest(
                        repository::ConfirmationRequest {
                            confirmation_id,
                            session_id: request_session_id,
                            execution_turn_id: request_turn_id,
                            kind,
                            submitted,
                        },
                    ))
                }),
            )
            .await
            .map_err(map_repository_fault)?;
        Ok(ReferenceRoutingDecision::Confirmation(
            snapshot.confirmation,
        ))
    }

    async fn record_completed_turn(
        &self,
        artifacts: CompletedTurnArtifacts,
    ) -> Result<(), ResolverFault> {
        let (turn_id, session_id, input, graph) = artifacts.into_record_parts();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .map_err(|_| ResolverFault::Unavailable)?;
        self.repository
            .record_completed(repository::RecordCompletedCommand {
                turn_id,
                session_id,
                input,
                artifacts: graph,
                now_ms,
            })
            .await
            .map(|_| ())
            .map_err(|fault| match fault {
                repository::RepositoryFault::ConflictingRetry => ResolverFault::ConflictingRetry,
                repository::RepositoryFault::InvariantViolation => {
                    ResolverFault::InvariantViolation
                }
                repository::RepositoryFault::InvalidInput => ResolverFault::InvalidInput,
                repository::RepositoryFault::Unavailable => ResolverFault::Unavailable,
                repository::RepositoryFault::CorruptState => ResolverFault::CorruptState,
                repository::RepositoryFault::ForeignKeysDisabled
                | repository::RepositoryFault::Storage
                | repository::RepositoryFault::Crypto
                | repository::RepositoryFault::Clock => ResolverFault::Unavailable,
                repository::RepositoryFault::AlreadyConsumed => ResolverFault::AlreadyConsumed,
            })
    }

    async fn admit_provider_query(
        &self,
        permit: &ProviderQueryPermit,
        operation: ProviderOperation,
    ) -> ProviderQueryAuthorization {
        query::admit_provider_query(permit, operation).await
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn confirmation_submitted_bytes(
    kind: ConfirmationRequestKind,
    envelope: &ConfirmationEnvelope,
    current_input: &UserAuthoredText,
) -> Vec<u8> {
    match kind {
        ConfirmationRequestKind::Confirm => envelope.proposed_term().as_bytes().to_vec(),
        ConfirmationRequestKind::Edit => current_input.as_str().as_bytes().to_vec(),
    }
}

#[cfg(test)]
mod ingress_tests {
    use super::*;

    #[test]
    fn confirmation_and_edit_read_from_their_distinct_byte_sources() {
        let envelope = ConfirmationEnvelope::new(ConfirmationId::new(), "Public term");
        let replacement = UserAuthoredText::new("replacement text");

        assert_eq!(
            confirmation_submitted_bytes(ConfirmationRequestKind::Confirm, &envelope, &replacement,),
            b"Public term"
        );
        assert_eq!(
            confirmation_submitted_bytes(ConfirmationRequestKind::Edit, &envelope, &replacement,),
            b"replacement text"
        );
    }
}

fn map_repository_fault(fault: repository::RepositoryFault) -> ResolverFault {
    match fault {
        repository::RepositoryFault::InvalidInput => ResolverFault::InvalidInput,
        repository::RepositoryFault::ForeignKeysDisabled
        | repository::RepositoryFault::Unavailable
        | repository::RepositoryFault::Storage
        | repository::RepositoryFault::Crypto
        | repository::RepositoryFault::Clock => ResolverFault::Unavailable,
        repository::RepositoryFault::CorruptState => ResolverFault::CorruptState,
        repository::RepositoryFault::InvariantViolation => ResolverFault::InvariantViolation,
        repository::RepositoryFault::ConflictingRetry => ResolverFault::ConflictingRetry,
        repository::RepositoryFault::AlreadyConsumed => ResolverFault::AlreadyConsumed,
    }
}

pub(crate) fn production_with_database(
    database: Arc<Mutex<rusqlite::Connection>>,
) -> Arc<dyn ConversationalReferenceResolver> {
    Arc::new(PersistentResolver {
        repository: Arc::new(repository::SqliteRepository::new(
            database,
            Arc::new(crypto::CryptoCustody::production()),
        )),
    })
}
