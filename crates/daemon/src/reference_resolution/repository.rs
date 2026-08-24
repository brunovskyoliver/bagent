use super::{
    artifacts::{self, ArtifactGraph, MentionRepresentation},
    confirmation::{self, ConfirmationDisposition, ConfirmationRequestKind},
    crypto::{AadBinding, CryptoCustody, CryptoFault},
};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use secrecy::ExposeSecret;
use std::{collections::BTreeSet, fmt, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

const HMAC_VERSION: u32 = 1;
const ENCRYPTION_VERSION: u32 = 1;
const MAX_DESCRIPTOR_BYTES: usize = 32 * 1024;
const MAX_STRUCTURAL_ID_BYTES: usize = 256;
const MAX_NORMALIZED_BYTES: usize = 8 * 1024;

pub(super) type SharedDatabase = Arc<Mutex<Connection>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RepositoryFault {
    InvalidInput,
    ForeignKeysDisabled,
    Unavailable,
    CorruptState,
    InvariantViolation,
    ConflictingRetry,
    Storage,
    Crypto,
    Clock,
    AlreadyConsumed,
}

impl From<CryptoFault> for RepositoryFault {
    fn from(fault: CryptoFault) -> Self {
        match fault {
            CryptoFault::KeyUnavailable | CryptoFault::KeyProvider => Self::Unavailable,
            CryptoFault::UnknownVersion | CryptoFault::MalformedCiphertext => Self::CorruptState,
            CryptoFault::AuthenticationFailed => Self::Crypto,
        }
    }
}

#[derive(Clone)]
pub(super) struct StagedMentionDescriptor {
    pub(super) mention_id: String,
    pub(super) referent_id: String,
    pub(super) normalized: String,
}

impl fmt::Debug for StagedMentionDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StagedMentionDescriptor")
            .field("mention_id", &"<redacted>")
            .field("referent_id", &"<redacted>")
            .field("normalized", &"<redacted>")
            .finish()
    }
}

pub(super) struct ResolveLedgerCommand {
    pub(super) turn_id: String,
    pub(super) session_id: String,
    pub(super) scope_id: String,
    pub(super) chat_session_id: Option<String>,
    pub(super) automation_id: Option<String>,
    pub(super) automation_run_id: Option<String>,
    pub(super) origin: &'static str,
    pub(super) input: Vec<u8>,
    pub(super) descriptors: Vec<StagedMentionDescriptor>,
    pub(super) now_ms: i64,
}

#[derive(Debug, Clone)]
pub(super) struct ResolutionSnapshot {
    pub(super) turn_id: String,
    pub(super) session_id: String,
    pub(super) session_seq: i64,
    pub(super) descriptors: Vec<StagedMentionDescriptor>,
    pub(super) candidates: Vec<super::types::LedgerCandidate>,
    pub(super) confirmation: ConfirmationDisposition,
}

pub(super) struct CandidateQuery {
    pub(super) session_id: String,
    pub(super) origin: &'static str,
    pub(super) current_seq: i64,
    pub(super) now_ms: i64,
}

pub(super) enum ResolutionDecision {
    KeepStaging,
    IssuePendingConfirmation(PendingConfirmationIssue),
    ConsumeConfirmation(ConfirmationConsumption),
    ConsumeConfirmationRequest(ConfirmationRequest),
}

pub(super) struct PendingConfirmationIssue {
    pub(super) confirmation_id: String,
    pub(super) mention_id: Option<String>,
    pub(super) referent_id: String,
    pub(super) provider_scope: &'static str,
    pub(super) sensitivity: &'static str,
    pub(super) proposal: Option<String>,
    pub(super) normalized: Vec<u8>,
    pub(super) normalization_version: u32,
    pub(super) compatibility_epoch: u32,
}

pub(super) struct ConfirmationRequest {
    pub(super) confirmation_id: String,
    pub(super) session_id: String,
    pub(super) execution_turn_id: String,
    pub(super) kind: ConfirmationRequestKind,
    pub(super) submitted: Vec<u8>,
}

pub(super) struct ConfirmationConsumption {
    pub(super) confirmation_id: String,
    pub(super) session_id: String,
    pub(super) initiating_turn_id: String,
    pub(super) referent_id: String,
    pub(super) provider_scope: &'static str,
    pub(super) sensitivity: &'static str,
    pub(super) normalization_version: u32,
    pub(super) compatibility_epoch: u32,
    pub(super) execution_turn_id: String,
    pub(super) action: ConfirmationAction,
}

pub(super) enum ConfirmationAction {
    Confirm {
        normalized: Vec<u8>,
        authorization_id: String,
        query_plan_hmac: [u8; 32],
        permit_nonce_hmac: [u8; 32],
        plan_version: u32,
        configuration_epoch: u32,
        process_epoch: u32,
        search_budget: u8,
        fetch_budget: u8,
        operations: Vec<AuthorizationOperationSpec>,
    },
    ConfirmReadOnly {
        normalized: Vec<u8>,
    },
    Edited,
    Invalidate,
}

pub(super) struct AuthorizationOperationSpec {
    pub(super) operation_ordinal: i64,
    pub(super) operation_hmac: [u8; 32],
    pub(super) operation_kind: &'static str,
    pub(super) provider: &'static str,
    pub(super) max_attempts: i64,
    pub(super) alternative_group: Option<u8>,
}

pub(super) type ResolutionDecisionFn =
    Box<dyn FnOnce(ResolutionSnapshot) -> Result<ResolutionDecision, RepositoryFault> + Send>;

pub(super) struct RecordCompletedCommand {
    pub(super) turn_id: String,
    pub(super) session_id: String,
    pub(super) input: Vec<u8>,
    pub(super) artifacts: ArtifactGraph,
    pub(super) now_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RecordDisposition {
    Recorded,
    IdenticalRetry,
}

pub(super) struct ReserveAttemptCommand {
    pub(super) authorization_id: String,
    pub(super) session_id: String,
    pub(super) execution_turn_id: String,
    pub(super) operation_ordinal: i64,
    pub(super) operation_hmac: [u8; 32],
    pub(super) provider: &'static str,
    pub(super) query_plan_hmac: [u8; 32],
    pub(super) permit_nonce_hmac: [u8; 32],
    pub(super) reservation_id: String,
    pub(super) now_ms: i64,
}

/// The V17 reservation command contains only structural bindings and keyed
/// digests. It never carries a query, URL, title, snippet, or provider body.
pub(super) struct ReserveAttemptV17Command {
    pub(super) authorization_id: String,
    pub(super) authorization_hmac: [u8; 32],
    pub(super) session_id: String,
    pub(super) initiating_turn_id: String,
    pub(super) execution_turn_id: String,
    pub(super) authorization_method: &'static str,
    pub(super) provider_scope: &'static str,
    pub(super) is_search: bool,
    pub(super) operation_slot: i64,
    pub(super) variant_id: Option<String>,
    pub(super) variant_hmac: Option<[u8; 32]>,
    pub(super) attempt_number: i64,
    pub(super) parent_reservation_id: Option<String>,
    pub(super) parent_reservation_hmac: Option<[u8; 32]>,
    pub(super) candidate_binding_id: Option<String>,
    pub(super) candidate_binding_hmac: Option<[u8; 32]>,
    pub(super) provider_hmac: [u8; 32],
    pub(super) operation_hmac: [u8; 32],
    pub(super) sealed_plan_hmac: [u8; 32],
    pub(super) permit_nonce_hmac: [u8; 32],
    pub(super) plan_version: i64,
    pub(super) schema_version: i64,
    pub(super) grammar_version: i64,
    pub(super) normalization_version: i64,
    pub(super) compatibility_epoch: i64,
    pub(super) configuration_epoch: i64,
    pub(super) process_epoch: i64,
    pub(super) reserved_searches: i64,
    pub(super) reserved_fetches: i64,
    pub(super) committed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReservationV17Readback {
    pub(super) authorization_id: String,
    pub(super) authorization_hmac: [u8; 32],
    pub(super) operation_slot: i64,
    pub(super) variant_id: Option<String>,
    pub(super) variant_hmac: Option<[u8; 32]>,
    pub(super) attempt_number: i64,
    pub(super) reservation_id: String,
    pub(super) parent_reservation_id: Option<String>,
    pub(super) parent_reservation_hmac: Option<[u8; 32]>,
    pub(super) candidate_binding_id: Option<String>,
    pub(super) candidate_binding_hmac: Option<[u8; 32]>,
    pub(super) provider_hmac: [u8; 32],
    pub(super) operation_hmac: [u8; 32],
    pub(super) sealed_plan_hmac: [u8; 32],
    pub(super) permit_nonce_hmac: [u8; 32],
    pub(super) committed_at_ms: i64,
    pub(super) reserved_searches: i64,
    pub(super) reserved_fetches: i64,
}

pub(super) struct SealCandidateV17Command {
    pub(super) authorization_id: String,
    pub(super) authorization_hmac: [u8; 32],
    pub(super) fetch_slot: i64,
    pub(super) parent_reservation_id: String,
    pub(super) parent_reservation_hmac: [u8; 32],
    pub(super) discovery_provider_hmac: [u8; 32],
    pub(super) normalized_url_hmac: [u8; 32],
    pub(super) source_identity_hmac: [u8; 32],
    pub(super) candidate_capability_hmac: [u8; 32],
    pub(super) retry_relationship_hmac: [u8; 32],
    pub(super) result_ordinal: i64,
    pub(super) binding_hmac: [u8; 32],
    pub(super) created_at_ms: i64,
    pub(super) expires_at_ms: i64,
    pub(super) schema_version: i64,
    pub(super) hmac_key_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CandidateBindingV17Readback {
    pub(super) candidate_binding_id: String,
    pub(super) binding_hmac: [u8; 32],
}

pub(super) struct StoreContinuationV17Command {
    pub(super) confirmation_id: String,
    pub(super) session_id: String,
    pub(super) initiating_turn_id: String,
    pub(super) continuation_ciphertext: Vec<u8>,
    pub(super) continuation_hmac: [u8; 32],
    pub(super) sealed_plan_hmac: [u8; 32],
    pub(super) referent_set_hmac: [u8; 32],
    pub(super) capability_version: i64,
    pub(super) schema_version: i64,
    pub(super) grammar_version: i64,
    pub(super) normalization_version: i64,
    pub(super) provider_scope: &'static str,
    pub(super) format_version: i64,
    pub(super) encryption_key_version: i64,
    pub(super) hmac_key_version: i64,
    pub(super) created_at_ms: i64,
    pub(super) expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ContinuationV17Readback {
    pub(super) continuation_ciphertext: Vec<u8>,
    pub(super) sealed_plan_hmac: [u8; 32],
    pub(super) referent_set_hmac: [u8; 32],
    pub(super) continuation_hmac: [u8; 32],
    pub(super) capability_version: i64,
    pub(super) schema_version: i64,
    pub(super) grammar_version: i64,
    pub(super) normalization_version: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReservationDisposition {
    Reserved { attempt_number: i64 },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PruneReport {
    pub(super) confirmations_expired: u64,
    pub(super) authorizations_terminalized: u64,
    pub(super) mentions_removed: u64,
    pub(super) query_tombstones_removed: u64,
    pub(super) confirmation_tombstones_removed: u64,
    pub(super) turns_removed: u64,
    pub(super) open_turns_removed: u64,
}

pub(super) trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0)
    }
}

#[async_trait]
pub(super) trait ReferenceRepository: Send + Sync {
    async fn transact_resolution(
        &self,
        command: ResolveLedgerCommand,
        decide: ResolutionDecisionFn,
    ) -> Result<ResolutionSnapshot, RepositoryFault>;

    async fn record_completed(
        &self,
        command: RecordCompletedCommand,
    ) -> Result<RecordDisposition, RepositoryFault>;

    async fn load_candidates(
        &self,
        query: CandidateQuery,
    ) -> Result<Vec<super::types::LedgerCandidate>, RepositoryFault>;

    async fn reserve_provider_attempt(
        &self,
        command: ReserveAttemptCommand,
    ) -> Result<ReservationDisposition, RepositoryFault>;

    async fn reserve_provider_attempt_v17(
        &self,
        command: ReserveAttemptV17Command,
    ) -> Result<ReservationV17Readback, RepositoryFault>;

    async fn seal_dynamic_candidate_v17(
        &self,
        command: SealCandidateV17Command,
    ) -> Result<CandidateBindingV17Readback, RepositoryFault>;

    async fn store_confirmation_continuation_v17(
        &self,
        command: StoreContinuationV17Command,
    ) -> Result<ContinuationV17Readback, RepositoryFault>;

    async fn load_confirmation_continuation_v17(
        &self,
        confirmation_id: String,
        session_id: String,
        initiating_turn_id: String,
    ) -> Result<ContinuationV17Readback, RepositoryFault>;

    async fn prune(&self, now_ms: i64) -> Result<PruneReport, RepositoryFault>;
}

pub(super) struct SqliteRepository {
    database: SharedDatabase,
    custody: Arc<CryptoCustody>,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for SqliteRepository {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SqliteRepository(<private>)")
    }
}

impl SqliteRepository {
    pub(super) fn new(database: SharedDatabase, custody: Arc<CryptoCustody>) -> Self {
        Self {
            database,
            custody,
            clock: Arc::new(SystemClock),
        }
    }

    #[cfg(test)]
    pub(super) fn with_clock(
        database: SharedDatabase,
        custody: Arc<CryptoCustody>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            database,
            custody,
            clock,
        }
    }

    pub(super) async fn readiness(&self) -> Result<(), RepositoryFault> {
        let connection = self.database.lock().await;
        verify_connection(&connection)?;
        verify_schema_and_keys(&connection, &self.custody)?;
        let legacy_active: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM query_authorizations authorization
                 WHERE NOT EXISTS (
                     SELECT 1 FROM reference_confirmation_continuations_v17 continuation
                     WHERE continuation.sealed_plan_hmac = authorization.query_plan_hmac
                 )",
                [],
                |row| row.get(0),
            )
            .map_err(|_| RepositoryFault::CorruptState)?;
        if legacy_active != 0 {
            return Err(RepositoryFault::Unavailable);
        }
        Ok(())
    }

    pub(super) fn structural_hmac(
        &self,
        row_id: &str,
        session_id: &str,
        purpose: &str,
        turn_id: &str,
        value: &[u8],
    ) -> Result<[u8; 32], RepositoryFault> {
        self.custody
            .hmac(
                &AadBinding::new(row_id, session_id, purpose).with_turn(turn_id),
                1,
                value,
            )
            .map_err(Into::into)
    }

    pub(super) async fn prune_with_clock(&self) -> Result<PruneReport, RepositoryFault> {
        self.prune(self.clock.now_ms()).await
    }

    #[cfg(test)]
    pub(super) async fn database_for_test(&self) -> tokio::sync::MutexGuard<'_, Connection> {
        self.database.lock().await
    }

    async fn with_connection<T>(
        &self,
        operation: impl FnOnce(&mut Connection) -> Result<T, RepositoryFault>,
    ) -> Result<T, RepositoryFault> {
        let mut connection = self.database.lock().await;
        verify_connection(&connection)?;
        verify_schema_and_keys(&connection, &self.custody)?;
        operation(&mut connection)
    }
}

#[async_trait]
impl ReferenceRepository for SqliteRepository {
    async fn transact_resolution(
        &self,
        command: ResolveLedgerCommand,
        decide: ResolutionDecisionFn,
    ) -> Result<ResolutionSnapshot, RepositoryFault> {
        self.with_connection(|connection| {
            validate_turn_command(&command)?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| RepositoryFault::Storage)?;

            let next_seq = allocate_sequence(&transaction, &command)?;
            let input_binding =
                AadBinding::new(&command.turn_id, &command.session_id, "turn_input");
            let input_hmac = self.custody.hmac(&input_binding, 1, &command.input)?;
            transaction
                .execute(
                    "INSERT INTO reference_turns
                     (turn_id, session_id, chat_session_id, automation_run_id, session_seq,
                      origin, state, input_hmac, hmac_key_version, created_at_ms,
                      open_expires_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'open', ?7, ?8, ?9, ?10)",
                    params![
                        command.turn_id,
                        command.session_id,
                        command.chat_session_id,
                        command.automation_run_id,
                        next_seq,
                        command.origin,
                        input_hmac.as_slice(),
                        HMAC_VERSION as i64,
                        command.now_ms,
                        command.now_ms + 3_600_000_i64,
                    ],
                )
                .map_err(|_| RepositoryFault::Storage)?;

            let staged_descriptors = if command.origin == "automation" {
                Vec::new()
            } else {
                command.descriptors.clone()
            };
            let descriptor_bytes = encode_descriptors(&staged_descriptors)?;
            let staging_binding =
                AadBinding::new(&command.turn_id, &command.session_id, "staged_mentions");
            let staged_ciphertext = self.custody.encrypt(&staging_binding, &descriptor_bytes)?;
            let staged_hmac = self.custody.hmac(&staging_binding, 1, &descriptor_bytes)?;
            transaction
                .execute(
                    "INSERT INTO reference_turn_staging
                     (turn_id, staged_mentions_ciphertext, staged_mentions_hmac,
                      descriptor_version, encryption_key_version, hmac_key_version, created_at_ms)
                     VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6)",
                    params![
                        command.turn_id,
                        staged_ciphertext,
                        staged_hmac.as_slice(),
                        ENCRYPTION_VERSION as i64,
                        HMAC_VERSION as i64,
                        command.now_ms,
                    ],
                )
                .map_err(|_| RepositoryFault::Storage)?;

            let snapshot = ResolutionSnapshot {
                turn_id: command.turn_id.clone(),
                session_id: command.session_id.clone(),
                session_seq: next_seq,
                descriptors: staged_descriptors,
                candidates: load_candidates_in_transaction(
                    &transaction,
                    &self.custody,
                    &CandidateQuery {
                        session_id: command.session_id.clone(),
                        origin: command.origin,
                        current_seq: next_seq,
                        now_ms: command.now_ms,
                    },
                )?,
                confirmation: ConfirmationDisposition::Unchanged,
            };
            let mut snapshot = snapshot;
            match decide(snapshot.clone())? {
                ResolutionDecision::KeepStaging => {}
                ResolutionDecision::IssuePendingConfirmation(issue) => {
                    snapshot.confirmation =
                        issue_pending_confirmation(&transaction, &self.custody, &command, issue)?;
                }
                ResolutionDecision::ConsumeConfirmation(consumption) => {
                    snapshot.confirmation = match consume_confirmation(
                        &transaction,
                        &self.custody,
                        &command,
                        consumption,
                    ) {
                        Ok(disposition) => disposition,
                        Err(RepositoryFault::AlreadyConsumed) => {
                            transaction
                                .execute(
                                    "DELETE FROM reference_turns WHERE turn_id=?1",
                                    params![command.turn_id],
                                )
                                .map_err(|_| RepositoryFault::Storage)?;
                            ConfirmationDisposition::BlockedAlreadyConsumed
                        }
                        Err(fault) => return Err(fault),
                    };
                }
                ResolutionDecision::ConsumeConfirmationRequest(request) => {
                    snapshot.confirmation = consume_confirmation_request(
                        &transaction,
                        &self.custody,
                        &command,
                        request,
                    )?;
                    if matches!(
                        snapshot.confirmation,
                        ConfirmationDisposition::BlockedAlreadyConsumed
                            | ConfirmationDisposition::BlockedInteractiveAction
                    ) {
                        transaction
                            .execute(
                                "DELETE FROM reference_turns WHERE turn_id=?1",
                                params![command.turn_id],
                            )
                            .map_err(|_| RepositoryFault::Storage)?;
                    }
                }
            }
            transaction.commit().map_err(|_| RepositoryFault::Storage)?;
            Ok(snapshot)
        })
        .await
    }

    async fn reserve_provider_attempt_v17(
        &self,
        mut command: ReserveAttemptV17Command,
    ) -> Result<ReservationV17Readback, RepositoryFault> {
        self.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| RepositoryFault::Storage)?;
            // Sample trusted time only after the writer transaction begins.
            // The command timestamp is never trusted for expiry or readback.
            let committed_at_ms = self.clock.now_ms();
            if committed_at_ms < 0 {
                return Err(RepositoryFault::Clock);
            }

            let authorization = transaction
                .query_row(
                    "SELECT session_id, initiating_turn_id, execution_turn_id,
                            authorization_method, provider_scope, query_plan_hmac,
                            plan_version, schema_version, grammar_version,
                            normalization_version, compatibility_epoch,
                            configuration_epoch, process_epoch, expires_at_ms,
                            search_budget, fetch_budget, reserved_searches,
                            reserved_fetches
                     FROM query_authorizations WHERE authorization_id=?1",
                    params![command.authorization_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, Vec<u8>>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, i64>(8)?,
                            row.get::<_, i64>(9)?,
                            row.get::<_, i64>(10)?,
                            row.get::<_, i64>(11)?,
                            row.get::<_, i64>(12)?,
                            row.get::<_, i64>(13)?,
                            row.get::<_, i64>(14)?,
                            row.get::<_, i64>(15)?,
                            row.get::<_, i64>(16)?,
                            row.get::<_, i64>(17)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| RepositoryFault::Storage)?
                .ok_or(RepositoryFault::Unavailable)?;

            if authorization.0 != command.session_id
                || authorization.1 != command.initiating_turn_id
                || authorization.2 != command.execution_turn_id
                || authorization.3 != command.authorization_method
                || authorization.4 != command.provider_scope
                || !constant_time_equal(&authorization.5, &command.sealed_plan_hmac)
                || authorization.6 != command.plan_version
                || authorization.7 != command.schema_version
                || authorization.8 != command.grammar_version
                || authorization.9 != command.normalization_version
                || authorization.10 != command.compatibility_epoch
                || authorization.11 != command.configuration_epoch
                || authorization.12 != command.process_epoch
                || authorization.13 <= committed_at_ms
            {
                return Err(RepositoryFault::InvariantViolation);
            }
            if !constant_time_equal(
                &self.custody.hmac(
                    &AadBinding::new(
                        &command.authorization_id,
                        &command.session_id,
                        "authorization",
                    )
                    .with_turn(&command.execution_turn_id),
                    1,
                    command.authorization_id.as_bytes(),
                )?,
                &command.authorization_hmac,
            ) {
                return Err(RepositoryFault::InvariantViolation);
            }

            let is_candidate = command.candidate_binding_id.is_some();
            let is_variant = command.variant_id.is_some();
            if is_candidate {
                if command.is_search
                    || command.operation_slot < 2
                    || !matches!(command.attempt_number, 1 | 2)
                    || command.variant_id.is_some()
                {
                    return Err(RepositoryFault::InvariantViolation);
                }
                let candidate_id = command
                    .candidate_binding_id
                    .as_deref()
                    .ok_or(RepositoryFault::InvalidInput)?;
                let candidate_hmac = command
                    .candidate_binding_hmac
                    .ok_or(RepositoryFault::InvalidInput)?;
                let parent_hmac = command
                    .parent_reservation_hmac
                    .ok_or(RepositoryFault::InvalidInput)?;
                let candidate = transaction
                    .query_row(
                        "SELECT fetch_slot, parent_reservation_id,
                                parent_reservation_hmac, discovery_provider_hmac,
                                binding_hmac, state, expires_at_ms
                         FROM dynamic_candidate_bindings_v17
                         WHERE candidate_binding_id=?1",
                        params![candidate_id],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Vec<u8>>(2)?,
                                row.get::<_, Vec<u8>>(3)?,
                                row.get::<_, Vec<u8>>(4)?,
                                row.get::<_, String>(5)?,
                                row.get::<_, i64>(6)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|_| RepositoryFault::Storage)?
                    .ok_or(RepositoryFault::InvariantViolation)?;
                if command.operation_slot != 2 + candidate.0
                    || candidate.6 <= committed_at_ms
                    || candidate.1 != command.parent_reservation_id.as_deref().unwrap_or("")
                    || !constant_time_equal(&candidate.2, &parent_hmac)
                    || !constant_time_equal(&candidate.4, &candidate_hmac)
                    || !matches!(
                        (command.attempt_number, candidate.5.as_str()),
                        (1, "active") | (2, "spent")
                    )
                {
                    return Err(RepositoryFault::InvariantViolation);
                }

                // Candidate fetches are V17 operations. Recompute their
                // operation HMAC from the sealed provider identity, slot,
                // attempt, plan, and candidate binding; no V16 plaintext row
                // is needed or consulted.
                let provider_binding =
                    AadBinding::new(&command.authorization_id, &command.session_id, "provider")
                        .with_turn(&command.execution_turn_id);
                let operation_binding = AadBinding::new(
                    &command.authorization_id,
                    &command.session_id,
                    "operation_v17",
                )
                .with_turn(&command.execution_turn_id);
                let mut expected_operation = None;
                for provider in ["tavily", "duckduckgo", "wikipedia", "direct"] {
                    let provider_hmac =
                        self.custody
                            .hmac(&provider_binding, 1, provider.as_bytes())?;
                    if !constant_time_equal(&provider_hmac, &command.provider_hmac)
                        || !constant_time_equal(&provider_hmac, &candidate.3)
                    {
                        continue;
                    }
                    let mut input = Vec::new();
                    input.extend_from_slice(&command.operation_slot.to_be_bytes());
                    input.push(command.attempt_number as u8);
                    input.extend_from_slice(provider.as_bytes());
                    input.extend_from_slice(&command.sealed_plan_hmac);
                    input.extend_from_slice(&candidate_hmac);
                    expected_operation = Some(self.custody.hmac(&operation_binding, 1, &input)?);
                    break;
                }
                if expected_operation
                    .map(|value| constant_time_equal(&value, &command.operation_hmac))
                    != Some(true)
                {
                    return Err(RepositoryFault::InvariantViolation);
                }
            } else if is_variant {
                if !command.is_search || command.operation_slot != 1 || command.attempt_number != 2
                {
                    return Err(RepositoryFault::InvariantViolation);
                }
                let variant_id = command
                    .variant_id
                    .as_deref()
                    .ok_or(RepositoryFault::InvalidInput)?;
                let variant_hmac = command.variant_hmac.ok_or(RepositoryFault::InvalidInput)?;
                let variant = transaction
                    .query_row(
                        "SELECT variant_set_id, variant_hmac, operation_hmac,
                                provider_hmac, parent_search_operation_hmac,
                                attempt_number
                         FROM query_operation_variants_v17 WHERE variant_id=?1",
                        params![variant_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Vec<u8>>(1)?,
                                row.get::<_, Vec<u8>>(2)?,
                                row.get::<_, Vec<u8>>(3)?,
                                row.get::<_, Vec<u8>>(4)?,
                                row.get::<_, i64>(5)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|_| RepositoryFault::Storage)?
                    .ok_or(RepositoryFault::InvariantViolation)?;
                if !constant_time_equal(&variant.1, &variant_hmac)
                    || !constant_time_equal(&variant.2, &command.operation_hmac)
                    || !constant_time_equal(&variant.3, &command.provider_hmac)
                    || variant.5 != command.attempt_number
                {
                    return Err(RepositoryFault::InvariantViolation);
                }
                let updated = transaction
                    .execute(
                        "UPDATE query_operation_variant_sets_v17
                         SET state='spent', winner_variant_id=?2, updated_at_ms=?3
                         WHERE variant_set_id=?1 AND state='ready'",
                        params![variant.0, variant_id, committed_at_ms],
                    )
                    .map_err(|_| RepositoryFault::Storage)?;
                if updated != 1 {
                    return Err(RepositoryFault::AlreadyConsumed);
                }
            } else {
                let operation = transaction
                    .query_row(
                        "SELECT operation_kind, provider, max_attempts,
                                reserved_attempts, operation_hmac
                         FROM query_authorization_operations
                         WHERE authorization_id=?1 AND operation_ordinal=?2",
                        params![command.authorization_id, command.operation_slot],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, i64>(2)?,
                                row.get::<_, i64>(3)?,
                                row.get::<_, Vec<u8>>(4)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|_| RepositoryFault::Storage)?
                    .ok_or(RepositoryFault::InvariantViolation)?;
                let provider_binding = self.custody.hmac(
                    &AadBinding::new(&command.authorization_id, &command.session_id, "provider")
                        .with_turn(&command.execution_turn_id),
                    1,
                    operation.1.as_bytes(),
                )?;
                if operation.3 >= operation.2 {
                    return Err(RepositoryFault::AlreadyConsumed);
                }
                if !constant_time_equal(&operation.4, &command.operation_hmac)
                    || !constant_time_equal(&provider_binding, &command.provider_hmac)
                    || (operation.0 == "search") != command.is_search
                    || command.attempt_number != operation.3 + 1
                {
                    return Err(RepositoryFault::InvariantViolation);
                }
            }

            let next_searches = authorization.16 + i64::from(command.is_search);
            let next_fetches = authorization.17 + i64::from(!command.is_search);
            if (command.reserved_searches != 0 && command.reserved_searches != next_searches)
                || (command.reserved_fetches != 0 && command.reserved_fetches != next_fetches)
                || next_searches > authorization.14
                || next_fetches > authorization.15
            {
                return Err(RepositoryFault::InvariantViolation);
            }
            command.reserved_searches = next_searches;
            command.reserved_fetches = next_fetches;
            if let Some(parent_id) = command.parent_reservation_id.as_deref() {
                let parent_hmac = command
                    .parent_reservation_hmac
                    .ok_or(RepositoryFault::InvalidInput)?;
                let parent = transaction
                    .query_row(
                        "SELECT authorization_id, operation_slot, operation_hmac
                         FROM provider_attempt_reservations_v17
                         WHERE reservation_id=?1 AND state='committed'",
                        params![parent_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, Vec<u8>>(2)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|_| RepositoryFault::Storage)?
                    .ok_or(RepositoryFault::InvariantViolation)?;
                if parent.0 != command.authorization_id
                    || parent.1 != 0
                    || !constant_time_equal(&parent.2, &parent_hmac)
                {
                    return Err(RepositoryFault::InvariantViolation);
                }
            }
            if let Some(candidate_id) = command.candidate_binding_id.as_deref() {
                let candidate_hmac = command
                    .candidate_binding_hmac
                    .ok_or(RepositoryFault::InvalidInput)?;
                let candidate = transaction
                    .query_row(
                        "SELECT authorization_id, parent_reservation_id,
                                binding_hmac, state
                         FROM dynamic_candidate_bindings_v17
                         WHERE candidate_binding_id=?1",
                        params![candidate_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Vec<u8>>(2)?,
                                row.get::<_, String>(3)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(|_| RepositoryFault::Storage)?
                    .ok_or(RepositoryFault::InvariantViolation)?;
                if candidate.0 != command.authorization_id
                    || candidate.1 != command.parent_reservation_id.as_deref().unwrap_or("")
                    || !constant_time_equal(&candidate.2, &candidate_hmac)
                    || !matches!(candidate.3.as_str(), "active" | "spent")
                {
                    return Err(RepositoryFault::InvariantViolation);
                }
                transaction
                    .execute(
                        "UPDATE dynamic_candidate_bindings_v17 SET state='spent'
                         WHERE candidate_binding_id=?1 AND state IN ('active','spent')",
                        params![candidate_id],
                    )
                    .map_err(|_| RepositoryFault::Storage)?;
            }

            let already_reserved: Option<i64> =
                if let Some(variant_id) = command.variant_id.as_deref() {
                    transaction
                        .query_row(
                            "SELECT 1 FROM provider_attempt_reservations_v17
                         WHERE authorization_id=?1 AND operation_slot=?2
                           AND variant_id=?3 AND attempt_number=?4",
                            params![
                                command.authorization_id,
                                command.operation_slot,
                                variant_id,
                                command.attempt_number
                            ],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|_| RepositoryFault::Storage)?
                } else {
                    transaction
                        .query_row(
                            "SELECT 1 FROM provider_attempt_reservations_v17
                         WHERE authorization_id=?1 AND operation_slot=?2
                           AND variant_id IS NULL AND attempt_number=?3",
                            params![
                                command.authorization_id,
                                command.operation_slot,
                                command.attempt_number
                            ],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|_| RepositoryFault::Storage)?
                };
            if already_reserved.is_some() {
                return Err(RepositoryFault::AlreadyConsumed);
            }

            let reservation_id = Uuid::new_v4().to_string();
            transaction
                .execute(
                    "INSERT INTO provider_attempt_reservations_v17
                     (reservation_id, authorization_id, authorization_hmac,
                      operation_slot, variant_id, variant_hmac, attempt_number,
                      parent_reservation_id, parent_reservation_hmac,
                      candidate_binding_id, candidate_binding_hmac, provider_hmac,
                      operation_hmac, sealed_plan_hmac, permit_nonce_hmac,
                      committed_at_ms, reserved_searches, reserved_fetches, state)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                             ?12, ?13, ?14, ?15, ?16, ?17, ?18, 'committed')",
                    params![
                        reservation_id,
                        command.authorization_id,
                        command.authorization_hmac.as_slice(),
                        command.operation_slot,
                        command.variant_id,
                        command.variant_hmac.as_ref().map(|value| value.as_slice()),
                        command.attempt_number,
                        command.parent_reservation_id,
                        command
                            .parent_reservation_hmac
                            .as_ref()
                            .map(|value| value.as_slice()),
                        command.candidate_binding_id,
                        command
                            .candidate_binding_hmac
                            .as_ref()
                            .map(|value| value.as_slice()),
                        command.provider_hmac.as_slice(),
                        command.operation_hmac.as_slice(),
                        command.sealed_plan_hmac.as_slice(),
                        command.permit_nonce_hmac.as_slice(),
                        committed_at_ms,
                        command.reserved_searches,
                        command.reserved_fetches,
                    ],
                )
                .map_err(|_| RepositoryFault::Storage)?;

            if command.is_search {
                transaction
                    .execute(
                        "UPDATE query_authorizations
                         SET reserved_searches=?2 WHERE authorization_id=?1",
                        params![command.authorization_id, command.reserved_searches],
                    )
                    .map_err(|_| RepositoryFault::Storage)?;
            } else {
                transaction
                    .execute(
                        "UPDATE query_authorizations
                         SET reserved_fetches=?2 WHERE authorization_id=?1",
                        params![command.authorization_id, command.reserved_fetches],
                    )
                    .map_err(|_| RepositoryFault::Storage)?;
            }
            if !is_variant && !is_candidate {
                transaction
                    .execute(
                        "UPDATE query_authorization_operations
                         SET reserved_attempts=reserved_attempts+1
                         WHERE authorization_id=?1 AND operation_ordinal=?2",
                        params![command.authorization_id, command.operation_slot],
                    )
                    .map_err(|_| RepositoryFault::Storage)?;
            }
            let expected = ReservationV17Readback {
                authorization_id: command.authorization_id.clone(),
                authorization_hmac: command.authorization_hmac,
                operation_slot: command.operation_slot,
                variant_id: command.variant_id.clone(),
                variant_hmac: command.variant_hmac,
                attempt_number: command.attempt_number,
                reservation_id: reservation_id.clone(),
                parent_reservation_id: command.parent_reservation_id.clone(),
                parent_reservation_hmac: command.parent_reservation_hmac,
                candidate_binding_id: command.candidate_binding_id.clone(),
                candidate_binding_hmac: command.candidate_binding_hmac,
                provider_hmac: command.provider_hmac,
                operation_hmac: command.operation_hmac,
                sealed_plan_hmac: command.sealed_plan_hmac,
                permit_nonce_hmac: command.permit_nonce_hmac,
                committed_at_ms,
                reserved_searches: command.reserved_searches,
                reserved_fetches: command.reserved_fetches,
            };
            match transaction.commit() {
                Ok(()) => readback_v17(connection, &expected),
                Err(_) => readback_v17(connection, &expected).map_err(|_| RepositoryFault::Storage),
            }
        })
        .await
    }

    async fn seal_dynamic_candidate_v17(
        &self,
        command: SealCandidateV17Command,
    ) -> Result<CandidateBindingV17Readback, RepositoryFault> {
        self.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| RepositoryFault::Storage)?;
            let created_at_ms = self.clock.now_ms();
            let expires_at_ms = created_at_ms
                .checked_add(300_000)
                .ok_or(RepositoryFault::Clock)?;
            if created_at_ms < 0 {
                return Err(RepositoryFault::Clock);
            }
            let parent = transaction
                .query_row(
                    "SELECT authorization_id, operation_slot, operation_hmac
                     FROM provider_attempt_reservations_v17
                     WHERE reservation_id=?1 AND state='committed'",
                    params![command.parent_reservation_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| RepositoryFault::Storage)?
                .ok_or(RepositoryFault::InvariantViolation)?;
            if parent.0 != command.authorization_id
                || parent.1 != 0
                || !constant_time_equal(&parent.2, &command.parent_reservation_hmac)
            {
                return Err(RepositoryFault::InvariantViolation);
            }
            let authorization_exists: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM query_authorizations
                     WHERE authorization_id=?1 AND expires_at_ms > ?2",
                    params![command.authorization_id, created_at_ms],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| RepositoryFault::Storage)?;
            if authorization_exists.is_none() {
                return Err(RepositoryFault::InvariantViolation);
            }
            let candidate_binding_id = Uuid::new_v4().to_string();
            transaction
                .execute(
                    "INSERT INTO dynamic_candidate_bindings_v17
                     (candidate_binding_id, authorization_id, authorization_hmac,
                      fetch_slot, parent_reservation_id, parent_reservation_hmac,
                      discovery_provider_hmac, normalized_url_hmac,
                      source_identity_hmac, candidate_capability_hmac,
                      retry_relationship_hmac, result_ordinal, binding_hmac,
                      state, created_at_ms, expires_at_ms, schema_version,
                      hmac_key_version)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                             ?12, ?13, 'active', ?14, ?15, ?16, ?17)",
                    params![
                        candidate_binding_id,
                        command.authorization_id,
                        command.authorization_hmac.as_slice(),
                        command.fetch_slot,
                        command.parent_reservation_id,
                        command.parent_reservation_hmac.as_slice(),
                        command.discovery_provider_hmac.as_slice(),
                        command.normalized_url_hmac.as_slice(),
                        command.source_identity_hmac.as_slice(),
                        command.candidate_capability_hmac.as_slice(),
                        command.retry_relationship_hmac.as_slice(),
                        command.result_ordinal,
                        command.binding_hmac.as_slice(),
                        created_at_ms,
                        expires_at_ms,
                        command.schema_version,
                        command.hmac_key_version,
                    ],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            transaction.commit().map_err(|_| RepositoryFault::Storage)?;
            Ok(CandidateBindingV17Readback {
                candidate_binding_id,
                binding_hmac: command.binding_hmac,
            })
        })
        .await
    }

    async fn store_confirmation_continuation_v17(
        &self,
        command: StoreContinuationV17Command,
    ) -> Result<ContinuationV17Readback, RepositoryFault> {
        self.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| RepositoryFault::Storage)?;
            transaction
                .execute(
                    "INSERT INTO reference_confirmation_continuations_v17
                     (confirmation_id, session_id, initiating_turn_id,
                      continuation_ciphertext, continuation_hmac, sealed_plan_hmac,
                      referent_set_hmac, capability_version, schema_version,
                      grammar_version, normalization_version, provider_scope,
                      format_version, encryption_key_version, hmac_key_version,
                      created_at_ms, expires_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                             ?12, ?13, ?14, ?15, ?16, ?17)",
                    params![
                        command.confirmation_id,
                        command.session_id,
                        command.initiating_turn_id,
                        command.continuation_ciphertext.clone(),
                        command.continuation_hmac.as_slice(),
                        command.sealed_plan_hmac.as_slice(),
                        command.referent_set_hmac.as_slice(),
                        command.capability_version,
                        command.schema_version,
                        command.grammar_version,
                        command.normalization_version,
                        command.provider_scope,
                        command.format_version,
                        command.encryption_key_version,
                        command.hmac_key_version,
                        command.created_at_ms,
                        command.expires_at_ms,
                    ],
                )
                .map_err(|_| RepositoryFault::InvariantViolation)?;
            transaction.commit().map_err(|_| RepositoryFault::Storage)?;
            Ok(ContinuationV17Readback {
                continuation_ciphertext: command.continuation_ciphertext,
                sealed_plan_hmac: command.sealed_plan_hmac,
                referent_set_hmac: command.referent_set_hmac,
                continuation_hmac: command.continuation_hmac,
                capability_version: command.capability_version,
                schema_version: command.schema_version,
                grammar_version: command.grammar_version,
                normalization_version: command.normalization_version,
            })
        })
        .await
    }

    async fn load_confirmation_continuation_v17(
        &self,
        confirmation_id: String,
        session_id: String,
        initiating_turn_id: String,
    ) -> Result<ContinuationV17Readback, RepositoryFault> {
        self.with_connection(|connection| {
            connection
                .query_row(
                    "SELECT continuation_ciphertext, sealed_plan_hmac,
                            referent_set_hmac, continuation_hmac,
                            capability_version, schema_version, grammar_version,
                            normalization_version
                     FROM reference_confirmation_continuations_v17
                     WHERE confirmation_id=?1 AND session_id=?2
                       AND initiating_turn_id=?3",
                    params![confirmation_id, session_id, initiating_turn_id],
                    |row| {
                        let bytes = |index: usize| -> rusqlite::Result<[u8; 32]> {
                            let value = row.get::<_, Vec<u8>>(index)?;
                            value.try_into().map_err(|_| {
                                rusqlite::Error::InvalidColumnType(
                                    index,
                                    "hmac".into(),
                                    rusqlite::types::Type::Blob,
                                )
                            })
                        };
                        Ok(ContinuationV17Readback {
                            continuation_ciphertext: row.get(0)?,
                            sealed_plan_hmac: bytes(1)?,
                            referent_set_hmac: bytes(2)?,
                            continuation_hmac: bytes(3)?,
                            capability_version: row.get(4)?,
                            schema_version: row.get(5)?,
                            grammar_version: row.get(6)?,
                            normalization_version: row.get(7)?,
                        })
                    },
                )
                .optional()
                .map_err(|_| RepositoryFault::Storage)?
                .ok_or(RepositoryFault::Unavailable)
        })
        .await
    }

    async fn record_completed(
        &self,
        command: RecordCompletedCommand,
    ) -> Result<RecordDisposition, RepositoryFault> {
        self.with_connection(|connection| {
            if !artifacts::validate_uuid(&command.turn_id)
                || !bounded(&command.session_id, MAX_STRUCTURAL_ID_BYTES)
            {
                return Err(RepositoryFault::InvalidInput);
            }
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| RepositoryFault::Storage)?;
            let turn = transaction
                .query_row(
                    "SELECT session_id, state, input_hmac, artifact_hmac, completed_at_ms
                     FROM reference_turns WHERE turn_id=?1",
                    params![command.turn_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                            row.get::<_, Option<Vec<u8>>>(3)?,
                            row.get::<_, Option<i64>>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| RepositoryFault::Storage)?
                .ok_or(RepositoryFault::Unavailable)?;
            if turn.0 != command.session_id {
                return Err(RepositoryFault::InvariantViolation);
            }
            let input_binding =
                AadBinding::new(&command.turn_id, &command.session_id, "turn_input");
            self.custody
                .verify_hmac(&input_binding, 1, &command.input, &turn.2)
                .map_err(|_| RepositoryFault::ConflictingRetry)?;

            let artifact_hmac = graph_hmac(&self.custody, &command)?;
            if turn.1 == "completed" {
                if turn.3.as_deref() == Some(artifact_hmac.as_slice()) {
                    transaction.commit().map_err(|_| RepositoryFault::Storage)?;
                    return Ok(RecordDisposition::IdenticalRetry);
                }
                return Err(RepositoryFault::ConflictingRetry);
            }
            if turn.1 != "open" {
                return Err(RepositoryFault::InvariantViolation);
            }
            let staging = transaction
                .query_row(
                    "SELECT staged_mentions_ciphertext, staged_mentions_hmac
                     FROM reference_turn_staging WHERE turn_id=?1",
                    params![command.turn_id],
                    |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )
                .optional()
                .map_err(|_| RepositoryFault::Storage)?
                .ok_or(RepositoryFault::CorruptState)?;
            let staging_binding =
                AadBinding::new(&command.turn_id, &command.session_id, "staged_mentions");
            let plaintext = self.custody.decrypt(&staging_binding, &staging.0)?;
            self.custody
                .verify_hmac(&staging_binding, 1, plaintext.expose_secret(), &staging.1)
                .map_err(|_| RepositoryFault::CorruptState)?;
            let descriptors = decode_descriptors(plaintext.expose_secret())?;
            validate_staged_descriptors(&descriptors, &command.artifacts)?;

            insert_graph(&transaction, &self.custody, &command)?;
            terminalize_authorizations(&transaction, &command.turn_id, command.now_ms)?;
            transaction
                .execute(
                    "UPDATE reference_turns
                     SET state='completed', completion_code='completed',
                         producer_class=CASE WHEN EXISTS
                           (SELECT 1 FROM conversation_mentions WHERE turn_id=?1)
                           THEN 'resolver_user_input' ELSE 'no_mentions' END,
                         artifact_hmac=?2, completed_at_ms=?3
                     WHERE turn_id=?1 AND state='open'",
                    params![command.turn_id, artifact_hmac.as_slice(), command.now_ms],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            transaction
                .execute(
                    "DELETE FROM reference_turn_staging WHERE turn_id=?1",
                    params![command.turn_id],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            transaction.commit().map_err(|_| RepositoryFault::Storage)?;
            Ok(RecordDisposition::Recorded)
        })
        .await
    }

    async fn load_candidates(
        &self,
        query: CandidateQuery,
    ) -> Result<Vec<super::types::LedgerCandidate>, RepositoryFault> {
        self.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Deferred)
                .map_err(|_| RepositoryFault::Storage)?;
            let candidates = load_candidates_in_transaction(&transaction, &self.custody, &query)?;
            transaction
                .rollback()
                .map_err(|_| RepositoryFault::Storage)?;
            Ok(candidates)
        })
        .await
    }

    async fn reserve_provider_attempt(
        &self,
        command: ReserveAttemptCommand,
    ) -> Result<ReservationDisposition, RepositoryFault> {
        self.with_connection(|connection| {
            if !artifacts::validate_uuid(&command.authorization_id)
                || !artifacts::validate_uuid(&command.execution_turn_id)
                || !artifacts::validate_uuid(&command.reservation_id)
            {
                return Err(RepositoryFault::InvalidInput);
            }
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| RepositoryFault::Storage)?;
            let authorization = transaction
                .query_row(
                    "SELECT session_id, execution_turn_id, query_plan_hmac,
                            permit_nonce_hmac, expires_at_ms, search_budget, fetch_budget,
                            reserved_searches, reserved_fetches
                     FROM query_authorizations WHERE authorization_id=?1",
                    params![command.authorization_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                            row.get::<_, Vec<u8>>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, i64>(6)?,
                            row.get::<_, i64>(7)?,
                            row.get::<_, i64>(8)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| RepositoryFault::Storage)?
                .ok_or(RepositoryFault::Unavailable)?;
            if authorization.0 != command.session_id
                || authorization.1 != command.execution_turn_id
                || !constant_time_equal(&authorization.2, &command.query_plan_hmac)
                || !constant_time_equal(&authorization.3, &command.permit_nonce_hmac)
                || authorization.4 <= command.now_ms
            {
                return Err(RepositoryFault::InvariantViolation);
            }
            let operation = transaction
                .query_row(
                    "SELECT operation_kind, provider, max_attempts, reserved_attempts
                            , operation_hmac
                     FROM query_authorization_operations
                     WHERE authorization_id=?1 AND operation_ordinal=?2",
                    params![command.authorization_id, command.operation_ordinal],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, Vec<u8>>(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| RepositoryFault::Storage)?
                .ok_or(RepositoryFault::InvariantViolation)?;
            if operation.1 != command.provider
                || !constant_time_equal(&operation.4, &command.operation_hmac)
                || operation.3 >= operation.2
            {
                return Err(RepositoryFault::InvariantViolation);
            }
            let is_search = operation.0 == "search";
            if (is_search && authorization.7 >= authorization.5)
                || (!is_search && authorization.8 >= authorization.6)
            {
                return Err(RepositoryFault::InvariantViolation);
            }
            let attempt_number = operation.3 + 1;
            transaction
                .execute(
                    "INSERT INTO provider_attempt_reservations
                     (reservation_id, authorization_id, operation_ordinal, attempt_number, reserved_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        command.reservation_id,
                        command.authorization_id,
                        command.operation_ordinal,
                        attempt_number,
                        command.now_ms
                    ],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            transaction
                .execute(
                    "UPDATE query_authorization_operations
                     SET reserved_attempts=reserved_attempts+1
                     WHERE authorization_id=?1 AND operation_ordinal=?2",
                    params![command.authorization_id, command.operation_ordinal],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            let counter = if is_search { "reserved_searches" } else { "reserved_fetches" };
            let sql = format!(
                "UPDATE query_authorizations SET {counter}={counter}+1 WHERE authorization_id=?1"
            );
            transaction
                .execute(&sql, params![command.authorization_id])
                .map_err(|_| RepositoryFault::Storage)?;
            transaction.commit().map_err(|_| RepositoryFault::Storage)?;
            Ok(ReservationDisposition::Reserved { attempt_number })
        })
        .await
    }

    async fn prune(&self, now_ms: i64) -> Result<PruneReport, RepositoryFault> {
        self.with_connection(|connection| {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| RepositoryFault::Storage)?;
            let mut report = PruneReport::default();

            let confirmations = transaction
                .prepare(
                    "SELECT confirmation_id, session_id, initiating_turn_id, referent_id,
                            provider_scope, normalized_term_hmac, compatibility_epoch,
                            created_at_ms, hmac_key_version
                     FROM reference_confirmations WHERE expires_at_ms <= ?1",
                )
                .map_err(|_| RepositoryFault::Storage)?
                .query_map(params![now_ms], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                    ))
                })
                .map_err(|_| RepositoryFault::Storage)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| RepositoryFault::Storage)?;
            for confirmation in confirmations {
                transaction
                    .execute(
                        "INSERT INTO reference_confirmation_tombstones
                         (confirmation_id, session_id, initiating_turn_id, referent_id,
                          provider_scope, normalized_term_hmac, terminal_state,
                          compatibility_epoch, created_at_ms, terminal_at_ms,
                          delete_after_ms, hmac_key_version)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'expired', ?7, ?8, ?9, ?10, ?11)",
                        params![
                            confirmation.0,
                            confirmation.1,
                            confirmation.2,
                            confirmation.3,
                            confirmation.4,
                            confirmation.5,
                            confirmation.6,
                            confirmation.7,
                            now_ms,
                            now_ms + 86_400_000_i64,
                            confirmation.8
                        ],
                    )
                    .map_err(|_| RepositoryFault::Storage)?;
                transaction
                    .execute(
                        "DELETE FROM reference_confirmations WHERE confirmation_id=?1",
                        params![confirmation.0],
                    )
                    .map_err(|_| RepositoryFault::Storage)?;
                report.confirmations_expired += 1;
            }

            report.authorizations_terminalized =
                terminalize_expired_authorizations(&transaction, now_ms)?;

            let expired_mentions = transaction
                .execute(
                    "DELETE FROM conversation_mentions
                     WHERE expires_at_ms <= ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM reference_confirmations
                           WHERE reference_confirmations.mention_id = conversation_mentions.mention_id
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM query_authorizations
                           WHERE query_authorizations.mention_id = conversation_mentions.mention_id
                       )",
                    params![now_ms],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            report.mentions_removed = expired_mentions as u64;

            let query_tombstones = transaction
                .execute(
                    "DELETE FROM query_replay_tombstones
                     WHERE delete_after_ms <= ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM reference_confirmation_tombstones
                           WHERE reference_confirmation_tombstones.confirmation_id = query_replay_tombstones.confirmation_id
                       )",
                    params![now_ms],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            report.query_tombstones_removed = query_tombstones as u64;
            let confirmation_tombstones = transaction
                .execute(
                    "DELETE FROM reference_confirmation_tombstones
                     WHERE delete_after_ms <= ?1
                       AND NOT EXISTS (
                           SELECT 1 FROM query_replay_tombstones
                           WHERE query_replay_tombstones.confirmation_id = reference_confirmation_tombstones.confirmation_id
                       )",
                    params![now_ms],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            report.confirmation_tombstones_removed = confirmation_tombstones as u64;

            let turns = transaction
                .execute(
                    "DELETE FROM reference_turns
                     WHERE state='completed'
                       AND completed_at_ms + 86400000 <= ?1
                       AND NOT EXISTS (SELECT 1 FROM reference_confirmations WHERE initiating_turn_id=reference_turns.turn_id)
                       AND NOT EXISTS (SELECT 1 FROM query_authorizations WHERE execution_turn_id=reference_turns.turn_id)",
                    params![now_ms],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            report.turns_removed = turns as u64;
            let open_turns = transaction
                .execute(
                    "DELETE FROM reference_turns WHERE state='open' AND open_expires_at_ms <= ?1",
                    params![now_ms],
                )
                .map_err(|_| RepositoryFault::Storage)?;
            report.open_turns_removed = open_turns as u64;
            transaction.commit().map_err(|_| RepositoryFault::Storage)?;
            Ok(report)
        })
        .await
    }
}

fn readback_v17(
    connection: &Connection,
    expected: &ReservationV17Readback,
) -> Result<ReservationV17Readback, RepositoryFault> {
    let actual = connection
        .query_row(
            "SELECT authorization_id, authorization_hmac, operation_slot,
                    variant_id, variant_hmac, attempt_number, reservation_id,
                    parent_reservation_id, parent_reservation_hmac,
                    candidate_binding_id, candidate_binding_hmac, provider_hmac,
                    operation_hmac, sealed_plan_hmac, permit_nonce_hmac,
                    committed_at_ms, reserved_searches, reserved_fetches
             FROM provider_attempt_reservations_v17 WHERE reservation_id=?1",
            params![expected.reservation_id],
            |row| {
                let bytes = |index: usize| -> rusqlite::Result<[u8; 32]> {
                    let value = row.get::<_, Vec<u8>>(index)?;
                    value.try_into().map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            index,
                            "hmac".into(),
                            rusqlite::types::Type::Blob,
                        )
                    })
                };
                let optional_bytes = |index: usize| -> rusqlite::Result<Option<[u8; 32]>> {
                    row.get::<_, Option<Vec<u8>>>(index)?
                        .map_or(Ok(None), |value| {
                            value.try_into().map(Some).map_err(|_| {
                                rusqlite::Error::InvalidColumnType(
                                    index,
                                    "hmac".into(),
                                    rusqlite::types::Type::Blob,
                                )
                            })
                        })
                };
                Ok(ReservationV17Readback {
                    authorization_id: row.get(0)?,
                    authorization_hmac: bytes(1)?,
                    operation_slot: row.get(2)?,
                    variant_id: row.get(3)?,
                    variant_hmac: optional_bytes(4)?,
                    attempt_number: row.get(5)?,
                    reservation_id: row.get(6)?,
                    parent_reservation_id: row.get(7)?,
                    parent_reservation_hmac: optional_bytes(8)?,
                    candidate_binding_id: row.get(9)?,
                    candidate_binding_hmac: optional_bytes(10)?,
                    provider_hmac: bytes(11)?,
                    operation_hmac: bytes(12)?,
                    sealed_plan_hmac: bytes(13)?,
                    permit_nonce_hmac: bytes(14)?,
                    committed_at_ms: row.get(15)?,
                    reserved_searches: row.get(16)?,
                    reserved_fetches: row.get(17)?,
                })
            },
        )
        .optional()
        .map_err(|_| RepositoryFault::Storage)?
        .ok_or(RepositoryFault::Unavailable)?;
    if actual != *expected {
        return Err(RepositoryFault::InvariantViolation);
    }
    Ok(actual)
}

fn load_candidates_in_transaction(
    transaction: &Transaction<'_>,
    custody: &CryptoCustody,
    query: &CandidateQuery,
) -> Result<Vec<super::types::LedgerCandidate>, RepositoryFault> {
    use super::types::{
        EntityKind, LedgerCandidate, LedgerProvenance, MentionSensitivity, MentionVisibility,
    };

    if query.origin != "chat" || query.current_seq <= 1 || query.now_ms < 0 {
        return Ok(Vec::new());
    }
    let mut statement = transaction
        .prepare(
            "SELECT m.mention_id, m.referent_id, m.entity_kind, m.text_kind,
                    m.provenance, m.producer, m.visibility, m.sensitivity,
                    m.turn_id, t.session_seq, m.created_at_ms, m.expires_at_ms,
                    m.public_display_ciphertext, m.public_normalized_ciphertext,
                    m.normalized_term_hmac
             FROM conversation_mentions m
             JOIN reference_turns t ON t.turn_id = m.turn_id
             WHERE m.session_id = ?1
               AND t.session_id = ?1
               AND t.origin = 'chat'
               AND t.state = 'completed'
               AND t.session_seq BETWEEN ?2 - 10 AND ?2 - 1
               AND m.created_at_ms <= ?3
               AND m.expires_at_ms > ?3
             ORDER BY t.session_seq ASC, m.mention_id ASC",
        )
        .map_err(|_| RepositoryFault::Storage)?;
    let rows = statement
        .query_map(
            params![query.session_id, query.current_seq, query.now_ms],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, Option<Vec<u8>>>(12)?,
                    row.get::<_, Option<Vec<u8>>>(13)?,
                    row.get::<_, Option<Vec<u8>>>(14)?,
                ))
            },
        )
        .map_err(|_| RepositoryFault::Storage)?;
    let mut candidates = Vec::new();
    for row in rows {
        let (
            mention_id,
            referent_id,
            entity_kind,
            text_kind,
            provenance,
            producer,
            visibility,
            sensitivity,
            turn_id,
            introduced_sequence,
            created_at_ms,
            expires_at_ms,
            display_ciphertext,
            normalized_ciphertext,
            normalized_hmac,
        ) = row.map_err(|_| RepositoryFault::Storage)?;
        let mention_uuid =
            Uuid::parse_str(&mention_id).map_err(|_| RepositoryFault::CorruptState)?;
        let mention_id = super::MentionId::from_uuid(mention_uuid);
        let entity_kind =
            EntityKind::from_str(&entity_kind).ok_or(RepositoryFault::CorruptState)?;
        let provenance = match (provenance.as_str(), producer.as_str()) {
            ("user_authored", "resolver_user_input") => LedgerProvenance::PriorUser,
            ("web_evidence", "canonical_web") => LedgerProvenance::CanonicalWeb,
            ("web_evidence", "accepted_polish") => LedgerProvenance::AcceptedPolish,
            ("assistant_authored", _) => LedgerProvenance::Assistant,
            ("mail_evidence", _) => LedgerProvenance::Mail,
            ("attachment_evidence", _) => LedgerProvenance::Attachment,
            ("unknown", _) => LedgerProvenance::Unknown,
            _ => return Err(RepositoryFault::CorruptState),
        };
        let visibility = match visibility.as_str() {
            "provider_safe" => MentionVisibility::ProviderSafe,
            "local_only" => MentionVisibility::LocalOnly,
            "confirmation_only" => MentionVisibility::ConfirmationOnly,
            "unknown" => MentionVisibility::Unknown,
            _ => return Err(RepositoryFault::CorruptState),
        };
        let sensitivity = match sensitivity.as_str() {
            "public" => MentionSensitivity::Public,
            "private" => MentionSensitivity::Private,
            "sensitive" => MentionSensitivity::Sensitive,
            "unknown" => MentionSensitivity::Unknown,
            _ => return Err(RepositoryFault::CorruptState),
        };
        let canonical_mapping_intact = match provenance {
            LedgerProvenance::CanonicalWeb => mapping_is_intact(
                transaction,
                custody,
                &mention_id,
                &turn_id,
                &query.session_id,
            )?,
            LedgerProvenance::AcceptedPolish => {
                let parent = transaction
                    .query_row(
                        "SELECT d.parent_mention_id, parent.turn_id
                         FROM mention_derivations d
                         JOIN conversation_mentions parent
                           ON parent.mention_id=d.parent_mention_id
                         WHERE d.derived_mention_id=?1
                           AND d.derivation_kind='accepted_polish_of'",
                        params![mention_id.as_uuid().to_string()],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()
                    .map_err(|_| RepositoryFault::Storage)?;
                parent.is_some_and(|(parent, parent_turn_id)| {
                    let Ok(parent_id) = Uuid::parse_str(&parent) else {
                        return false;
                    };
                    mapping_is_intact(
                        transaction,
                        custody,
                        &super::MentionId::from_uuid(parent_id),
                        &parent_turn_id,
                        &query.session_id,
                    )
                    .unwrap_or(false)
                })
            }
            _ => true,
        };
        if matches!(
            provenance,
            LedgerProvenance::CanonicalWeb | LedgerProvenance::AcceptedPolish
        ) && !canonical_mapping_intact
        {
            continue;
        }
        let (display, normalized) = if text_kind == "public_visible" {
            let display_ciphertext = display_ciphertext.ok_or(RepositoryFault::CorruptState)?;
            let normalized_ciphertext =
                normalized_ciphertext.ok_or(RepositoryFault::CorruptState)?;
            let normalized_hmac = normalized_hmac.ok_or(RepositoryFault::CorruptState)?;
            let display_binding = AadBinding::new(
                &mention_id.as_uuid().to_string(),
                &query.session_id,
                "public_display",
            )
            .with_turn(&turn_id)
            .with_referent(&referent_id);
            let normalized_binding = AadBinding::new(
                &mention_id.as_uuid().to_string(),
                &query.session_id,
                "public_normalized",
            )
            .with_turn(&turn_id)
            .with_referent(&referent_id);
            let display = custody
                .decrypt(&display_binding, &display_ciphertext)
                .map_err(RepositoryFault::from)?
                .expose_secret()
                .to_vec();
            let normalized = custody
                .decrypt(&normalized_binding, &normalized_ciphertext)
                .map_err(RepositoryFault::from)?;
            custody
                .verify_hmac(
                    &AadBinding::new(
                        &mention_id.as_uuid().to_string(),
                        &query.session_id,
                        "mention_term",
                    )
                    .with_turn(&turn_id)
                    .with_referent(&referent_id),
                    1,
                    normalized.expose_secret(),
                    &normalized_hmac,
                )
                .map_err(RepositoryFault::from)?;
            (
                Some(String::from_utf8(display).map_err(|_| RepositoryFault::CorruptState)?),
                Some(
                    String::from_utf8(normalized.expose_secret().to_vec())
                        .map_err(|_| RepositoryFault::CorruptState)?,
                ),
            )
        } else {
            // Restricted and opaque representations remain opaque.  In
            // particular, safe-looking source text is never recovered here.
            (None, None)
        };
        let age_turns = query
            .current_seq
            .saturating_sub(introduced_sequence)
            .min(u8::MAX as i64) as u8;
        let age_minutes = query
            .now_ms
            .saturating_sub(created_at_ms)
            .max(0)
            .div_euclid(60_000)
            .min(u32::MAX as i64) as u32;
        candidates.push(LedgerCandidate {
            mention_id,
            referent_id,
            entity_kind,
            display,
            normalized,
            provenance,
            visibility,
            sensitivity,
            introduced_sequence,
            created_at_ms,
            expires_at_ms,
            age_turns,
            age_minutes,
            canonical_mapping_intact,
        });
    }
    Ok(candidates)
}

fn mapping_is_intact(
    transaction: &Transaction<'_>,
    custody: &CryptoCustody,
    mention_id: &super::MentionId,
    turn_id: &str,
    session_id: &str,
) -> Result<bool, RepositoryFault> {
    let mapping = transaction
        .query_row(
            "SELECT m.mapping_id, m.canonical_anchor_id,
                    m.source_identity_ciphertext, m.source_identity_hmac,
                    m.public_url_ciphertext, m.public_url_hmac
             FROM mention_web_mappings m
             JOIN mention_anchors a ON a.anchor_id=m.canonical_anchor_id
             WHERE m.mention_id=?1 AND a.mention_id=?1
               AND a.anchor_kind='visible' AND a.display_class='canonical'",
            params![mention_id.as_uuid().to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|_| RepositoryFault::Storage)?;
    let Some((mapping_id, _anchor_id, source_ciphertext, source_hmac, url_ciphertext, url_hmac)) =
        mapping
    else {
        return Ok(false);
    };
    let source_binding = AadBinding::new(&mapping_id, session_id, "source_identity")
        .with_turn(turn_id)
        .with_referent(&mention_id.as_uuid().to_string());
    let url_binding = AadBinding::new(&mapping_id, session_id, "public_url")
        .with_turn(turn_id)
        .with_referent(&mention_id.as_uuid().to_string());
    let source = custody
        .decrypt(&source_binding, &source_ciphertext)
        .map_err(RepositoryFault::from)?;
    let url = custody
        .decrypt(&url_binding, &url_ciphertext)
        .map_err(RepositoryFault::from)?;
    custody
        .verify_hmac(&source_binding, 1, source.expose_secret(), &source_hmac)
        .map_err(RepositoryFault::from)?;
    custody
        .verify_hmac(&url_binding, 1, url.expose_secret(), &url_hmac)
        .map_err(RepositoryFault::from)?;
    Ok(true)
}

fn verify_connection(connection: &Connection) -> Result<(), RepositoryFault> {
    let enabled: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .map_err(|_| RepositoryFault::Unavailable)?;
    if enabled != 1 {
        return Err(RepositoryFault::ForeignKeysDisabled);
    }
    Ok(())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    let mut difference = u8::from(left.len() != right.len());
    let width = left.len().max(right.len());
    for index in 0..width {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= left_byte ^ right_byte;
    }
    difference == 0
}

fn verify_schema_and_keys(
    connection: &Connection,
    custody: &CryptoCustody,
) -> Result<(), RepositoryFault> {
    let version: i64 = connection
        .query_row(
            "SELECT MAX(version) FROM refinery_schema_history",
            [],
            |row| row.get(0),
        )
        .map_err(|_| RepositoryFault::Unavailable)?;
    if version < 16 {
        return Err(RepositoryFault::Unavailable);
    }
    let versions = persisted_key_versions(connection)?;
    custody.ensure_for_database(&versions)?;
    Ok(())
}

fn validate_turn_command(command: &ResolveLedgerCommand) -> Result<(), RepositoryFault> {
    if !artifacts::validate_uuid(&command.turn_id)
        || !bounded(&command.session_id, MAX_STRUCTURAL_ID_BYTES)
        || !bounded(&command.scope_id, MAX_STRUCTURAL_ID_BYTES)
        || command.input.is_empty()
        || command.input.len() > 64 * 1024
        || command.now_ms < 0
        || !matches!(command.origin, "chat" | "automation")
    {
        return Err(RepositoryFault::InvalidInput);
    }
    if command
        .chat_session_id
        .as_deref()
        .is_some_and(|value| !bounded(value, MAX_STRUCTURAL_ID_BYTES))
        || command
            .automation_id
            .as_deref()
            .is_some_and(|value| !bounded(value, MAX_STRUCTURAL_ID_BYTES))
        || command
            .automation_run_id
            .as_deref()
            .is_some_and(|value| !bounded(value, MAX_STRUCTURAL_ID_BYTES))
    {
        return Err(RepositoryFault::InvalidInput);
    }
    if command.origin == "chat"
        && (command.chat_session_id.as_deref() != Some(command.session_id.as_str())
            || command.automation_id.is_some()
            || command.automation_run_id.is_some())
    {
        return Err(RepositoryFault::InvalidInput);
    }
    if command.origin == "automation"
        && (command.chat_session_id.is_some()
            || command.automation_id.is_none()
            || command.automation_run_id.is_none())
    {
        return Err(RepositoryFault::InvalidInput);
    }
    Ok(())
}

fn allocate_sequence(
    transaction: &Transaction<'_>,
    command: &ResolveLedgerCommand,
) -> Result<i64, RepositoryFault> {
    transaction
        .execute(
            "INSERT OR IGNORE INTO reference_session_sequences
             (scope_id, session_id, origin, chat_session_id, automation_id,
              next_seq, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?6)",
            params![
                command.scope_id,
                command.session_id,
                command.origin,
                command.chat_session_id,
                command.automation_id,
                command.now_ms
            ],
        )
        .map_err(|_| RepositoryFault::Storage)?;
    let next: i64 = transaction
        .query_row(
            "SELECT next_seq FROM reference_session_sequences WHERE scope_id=?1",
            params![command.scope_id],
            |row| row.get(0),
        )
        .map_err(|_| RepositoryFault::Storage)?;
    transaction
        .execute(
            "UPDATE reference_session_sequences SET next_seq=?2, updated_at_ms=?3 WHERE scope_id=?1",
            params![command.scope_id, next + 1, command.now_ms],
        )
        .map_err(|_| RepositoryFault::Storage)?;
    Ok(next)
}

fn encode_descriptors(descriptors: &[StagedMentionDescriptor]) -> Result<Vec<u8>, RepositoryFault> {
    if descriptors.len() > 1024 {
        return Err(RepositoryFault::InvalidInput);
    }
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&(descriptors.len() as u32).to_be_bytes());
    for descriptor in descriptors {
        if !artifacts::validate_uuid(&descriptor.mention_id)
            || !bounded(&descriptor.referent_id, MAX_STRUCTURAL_ID_BYTES)
            || !bounded(&descriptor.normalized, MAX_NORMALIZED_BYTES)
        {
            return Err(RepositoryFault::InvalidInput);
        }
        push_string(&mut encoded, &descriptor.mention_id)?;
        push_string(&mut encoded, &descriptor.referent_id)?;
        push_string(&mut encoded, &descriptor.normalized)?;
    }
    if encoded.len() > MAX_DESCRIPTOR_BYTES {
        return Err(RepositoryFault::InvalidInput);
    }
    Ok(encoded)
}

fn decode_descriptors(encoded: &[u8]) -> Result<Vec<StagedMentionDescriptor>, RepositoryFault> {
    if encoded.len() < 4 || encoded.len() > MAX_DESCRIPTOR_BYTES {
        return Err(RepositoryFault::CorruptState);
    }
    let mut offset = 0;
    let count = read_u32(encoded, &mut offset)? as usize;
    if count > 1024 {
        return Err(RepositoryFault::CorruptState);
    }
    let mut descriptors = Vec::with_capacity(count);
    for _ in 0..count {
        let mention_id = read_string(encoded, &mut offset)?;
        let referent_id = read_string(encoded, &mut offset)?;
        let normalized = read_string(encoded, &mut offset)?;
        if !artifacts::validate_uuid(&mention_id)
            || !bounded(&referent_id, MAX_STRUCTURAL_ID_BYTES)
            || !bounded(&normalized, MAX_NORMALIZED_BYTES)
        {
            return Err(RepositoryFault::CorruptState);
        }
        descriptors.push(StagedMentionDescriptor {
            mention_id,
            referent_id,
            normalized,
        });
    }
    if offset != encoded.len() {
        return Err(RepositoryFault::CorruptState);
    }
    Ok(descriptors)
}

fn validate_staged_descriptors(
    descriptors: &[StagedMentionDescriptor],
    graph: &ArtifactGraph,
) -> Result<(), RepositoryFault> {
    for descriptor in descriptors {
        let mention = graph
            .mentions
            .iter()
            .find(|mention| mention.mention_id == descriptor.mention_id)
            .ok_or(RepositoryFault::InvariantViolation)?;
        if mention.referent_id != descriptor.referent_id
            || !matches!(
                &mention.representation,
                MentionRepresentation::PublicVisible { normalized, .. }
                    if normalized == &descriptor.normalized
            )
        {
            return Err(RepositoryFault::InvariantViolation);
        }
    }
    Ok(())
}

fn push_string(buffer: &mut Vec<u8>, value: &str) -> Result<(), RepositoryFault> {
    if value.len() > u16::MAX as usize {
        return Err(RepositoryFault::InvalidInput);
    }
    buffer.extend_from_slice(&(value.len() as u16).to_be_bytes());
    buffer.extend_from_slice(value.as_bytes());
    Ok(())
}

fn bounded(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum
}

fn read_u32(encoded: &[u8], offset: &mut usize) -> Result<u32, RepositoryFault> {
    if encoded.len().saturating_sub(*offset) < 4 {
        return Err(RepositoryFault::CorruptState);
    }
    let value = u32::from_be_bytes(
        encoded[*offset..*offset + 4]
            .try_into()
            .map_err(|_| RepositoryFault::CorruptState)?,
    );
    *offset += 4;
    Ok(value)
}

fn read_string(encoded: &[u8], offset: &mut usize) -> Result<String, RepositoryFault> {
    if encoded.len().saturating_sub(*offset) < 2 {
        return Err(RepositoryFault::CorruptState);
    }
    let length = u16::from_be_bytes(
        encoded[*offset..*offset + 2]
            .try_into()
            .map_err(|_| RepositoryFault::CorruptState)?,
    ) as usize;
    *offset += 2;
    if encoded.len().saturating_sub(*offset) < length {
        return Err(RepositoryFault::CorruptState);
    }
    let value = std::str::from_utf8(&encoded[*offset..*offset + length])
        .map_err(|_| RepositoryFault::CorruptState)?
        .to_owned();
    *offset += length;
    Ok(value)
}

fn graph_hmac(
    custody: &CryptoCustody,
    command: &RecordCompletedCommand,
) -> Result<[u8; 32], RepositoryFault> {
    let mut encoded = Vec::new();
    for part in command.artifacts.canonical_parts() {
        encoded.extend_from_slice(&(part.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&part);
    }
    let binding = AadBinding::new(&command.turn_id, &command.session_id, "artifact_graph");
    custody.hmac(&binding, 1, &encoded).map_err(Into::into)
}

fn insert_graph(
    transaction: &Transaction<'_>,
    custody: &CryptoCustody,
    command: &RecordCompletedCommand,
) -> Result<(), RepositoryFault> {
    let mut normalized_digests = std::collections::HashMap::new();
    for mention in &command.artifacts.mentions {
        if !artifacts::validate_uuid(&mention.mention_id)
            || !artifacts::validate_uuid(&mention.turn_id)
            || mention
                .canonical_parent_mention_id
                .as_deref()
                .is_some_and(|id| !artifacts::validate_uuid(id))
            || mention.session_id != command.session_id
            || mention.turn_id != command.turn_id
            || !bounded(&mention.referent_id, MAX_STRUCTURAL_ID_BYTES)
            || mention.created_at_ms < 0
            || mention.expires_at_ms != mention.created_at_ms + 1_800_000
        {
            return Err(RepositoryFault::InvalidInput);
        }
        let binding = AadBinding::new(&mention.mention_id, &mention.session_id, "mention_term")
            .with_turn(&mention.turn_id)
            .with_referent(&mention.referent_id);
        let (display_ciphertext, normalized_ciphertext, normalized_hmac, restricted_hmac, opaque) =
            match &mention.representation {
                MentionRepresentation::PublicVisible {
                    display,
                    normalized,
                } => {
                    if !bounded(display, MAX_NORMALIZED_BYTES)
                        || !bounded(normalized, MAX_NORMALIZED_BYTES)
                    {
                        return Err(RepositoryFault::InvalidInput);
                    }
                    let display_ciphertext = custody.encrypt(
                        &AadBinding::new(
                            &mention.mention_id,
                            &mention.session_id,
                            "public_display",
                        )
                        .with_turn(&mention.turn_id)
                        .with_referent(&mention.referent_id),
                        display.as_bytes(),
                    )?;
                    let normalized_ciphertext = custody.encrypt(
                        &AadBinding::new(
                            &mention.mention_id,
                            &mention.session_id,
                            "public_normalized",
                        )
                        .with_turn(&mention.turn_id)
                        .with_referent(&mention.referent_id),
                        normalized.as_bytes(),
                    )?;
                    let digest = custody.hmac(&binding, 1, normalized.as_bytes())?;
                    normalized_digests.insert(mention.mention_id.clone(), digest);
                    (
                        Some(display_ciphertext),
                        Some(normalized_ciphertext),
                        Some(digest.to_vec()),
                        None,
                        None,
                    )
                }
                MentionRepresentation::Restricted { span_hmac } => {
                    (None, None, None, Some(span_hmac.to_vec()), None)
                }
                MentionRepresentation::Opaque { fingerprint } => {
                    (None, None, None, None, Some(fingerprint.to_vec()))
                }
            };
        transaction
            .execute(
                "INSERT INTO conversation_mentions
                (mention_id, referent_id, turn_id, session_id, canonical_parent_mention_id,
                  entity_kind,
                  text_kind, provenance, assistant_lineage, producer, visibility,
                  sensitivity, direct_user, untrusted_evidence, origin_ref_hmac,
                  mail_body_origin, public_display_ciphertext, public_normalized_ciphertext,
                  normalized_term_hmac, restricted_span_hmac, opaque_fingerprint,
                  created_at_ms, expires_at_ms, hmac_key_version, encryption_key_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                         ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
                params![
                    mention.mention_id,
                    mention.referent_id,
                    mention.turn_id,
                    mention.session_id,
                    mention.canonical_parent_mention_id,
                    mention.entity_kind,
                    mention.representation.kind().as_str(),
                    mention.provenance,
                    mention.assistant_lineage,
                    mention.producer,
                    mention.visibility,
                    mention.sensitivity,
                    i64::from(mention.direct_user),
                    i64::from(mention.untrusted_evidence),
                    mention
                        .origin_ref_hmac
                        .as_ref()
                        .map(|value| value.as_slice()),
                    mention.mail_body_origin,
                    display_ciphertext,
                    normalized_ciphertext,
                    normalized_hmac,
                    restricted_hmac,
                    opaque,
                    mention.created_at_ms,
                    mention.expires_at_ms,
                    mention.hmac_key_version as i64,
                    mention.encryption_key_version.map(i64::from),
                ],
            )
            .map_err(|_| RepositoryFault::InvariantViolation)?;
    }

    for derivation in &command.artifacts.derivations {
        if !artifacts::validate_uuid(&derivation.derivation_id)
            || !artifacts::validate_uuid(&derivation.derived_mention_id)
            || !artifacts::validate_uuid(&derivation.parent_mention_id)
            || derivation.parent_ordinal < 0
            || derivation.created_at_ms < 0
        {
            return Err(RepositoryFault::InvalidInput);
        }
        transaction
            .execute(
                "INSERT INTO mention_derivations
                 (derivation_id, derived_mention_id, parent_mention_id,
                  derivation_kind, parent_ordinal, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    derivation.derivation_id,
                    derivation.derived_mention_id,
                    derivation.parent_mention_id,
                    derivation.kind.as_str(),
                    derivation.parent_ordinal,
                    derivation.created_at_ms,
                ],
            )
            .map_err(|_| RepositoryFault::InvariantViolation)?;
    }

    for anchor in &command.artifacts.anchors {
        if !artifacts::validate_uuid(&anchor.anchor_id)
            || !artifacts::validate_uuid(&anchor.mention_id)
            || !artifacts::validate_uuid(&anchor.turn_id)
            || anchor.ordinal < 0
            || anchor.created_at_ms < 0
        {
            return Err(RepositoryFault::InvalidInput);
        }
        transaction
            .execute(
                "INSERT INTO mention_anchors
                 (anchor_id, mention_id, turn_id, anchor_kind, display_class,
                  ordinal, start_utf8, end_utf8, visible_span_hmac,
                  opaque_anchor_hmac, hmac_key_version, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    anchor.anchor_id,
                    anchor.mention_id,
                    anchor.turn_id,
                    anchor.kind.as_str(),
                    anchor.display_class.as_str(),
                    anchor.ordinal,
                    anchor.start_utf8,
                    anchor.end_utf8,
                    anchor
                        .visible_span_hmac
                        .as_ref()
                        .map(|value| value.as_slice()),
                    anchor
                        .opaque_anchor_hmac
                        .as_ref()
                        .map(|value| value.as_slice()),
                    anchor.hmac_key_version as i64,
                    anchor.created_at_ms,
                ],
            )
            .map_err(|_| RepositoryFault::InvariantViolation)?;
    }

    for mapping in &command.artifacts.web_mappings {
        if !artifacts::validate_uuid(&mapping.mapping_id)
            || !artifacts::validate_uuid(&mapping.mention_id)
            || !artifacts::validate_uuid(&mapping.canonical_anchor_id)
            || mapping.source_ordinal < 0
            || mapping.validated_at_ms < 0
            || mapping.encryption_key_version == 0
            || mapping.hmac_key_version == 0
            || !bounded(&mapping.source_identity, MAX_STRUCTURAL_ID_BYTES)
            || !bounded(&mapping.public_url, MAX_NORMALIZED_BYTES)
        {
            return Err(RepositoryFault::InvalidInput);
        }
        let source_binding =
            AadBinding::new(&mapping.mapping_id, &command.session_id, "source_identity")
                .with_turn(&command.turn_id)
                .with_referent(&mapping.mention_id);
        let expected_source_hmac =
            custody.hmac(&source_binding, 1, mapping.source_identity.as_bytes())?;
        let url_binding = AadBinding::new(&mapping.mapping_id, &command.session_id, "public_url")
            .with_turn(&command.turn_id)
            .with_referent(&mapping.mention_id);
        let expected_url_hmac = custody.hmac(&url_binding, 1, mapping.public_url.as_bytes())?;
        if expected_source_hmac != mapping.source_identity_hmac
            || expected_url_hmac != mapping.public_url_hmac
        {
            return Err(RepositoryFault::InvariantViolation);
        }
        let source_ciphertext =
            custody.encrypt(&source_binding, mapping.source_identity.as_bytes())?;
        let url_ciphertext = custody.encrypt(&url_binding, mapping.public_url.as_bytes())?;
        transaction
            .execute(
                "INSERT INTO mention_web_mappings
                 (mapping_id, mention_id, canonical_anchor_id, source_ordinal,
                  evidence_id_hmac, source_identity_ciphertext, source_identity_hmac,
                  public_url_ciphertext, public_url_hmac, authority,
                  network_policy_version, validated_at_ms, encryption_key_version,
                  hmac_key_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![
                    mapping.mapping_id,
                    mapping.mention_id,
                    mapping.canonical_anchor_id,
                    mapping.source_ordinal,
                    mapping.evidence_id_hmac.as_slice(),
                    source_ciphertext,
                    mapping.source_identity_hmac.as_slice(),
                    url_ciphertext,
                    mapping.public_url_hmac.as_slice(),
                    mapping.authority,
                    mapping.network_policy_version as i64,
                    mapping.validated_at_ms,
                    mapping.encryption_key_version as i64,
                    mapping.hmac_key_version as i64,
                ],
            )
            .map_err(|_| RepositoryFault::InvariantViolation)?;
    }
    let _ = normalized_digests;
    Ok(())
}

type PendingConfirmationRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
);

fn issue_pending_confirmation(
    transaction: &Transaction<'_>,
    custody: &CryptoCustody,
    turn: &ResolveLedgerCommand,
    issue: PendingConfirmationIssue,
) -> Result<ConfirmationDisposition, RepositoryFault> {
    if !artifacts::validate_uuid(&issue.confirmation_id)
        || issue
            .mention_id
            .as_deref()
            .is_some_and(|id| !artifacts::validate_uuid(id))
        || !bounded(&issue.referent_id, MAX_STRUCTURAL_ID_BYTES)
        || !matches!(
            issue.provider_scope,
            "web_search_fetch" | "direct_public_fetch"
        )
        || issue.normalization_version == 0
        || issue.compatibility_epoch == 0
    {
        return Err(RepositoryFault::InvalidInput);
    }
    if turn.origin != "chat" {
        return Ok(ConfirmationDisposition::BlockedInteractiveAction);
    }
    if issue.sensitivity != "public" {
        return Ok(ConfirmationDisposition::Unchanged);
    }
    if let Some(mention_id) = issue.mention_id.as_deref() {
        let mention = transaction
            .query_row(
                "SELECT session_id, referent_id, sensitivity, visibility
                 FROM conversation_mentions WHERE mention_id=?1",
                params![mention_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| RepositoryFault::Storage)?;
        if !mention.is_some_and(|(session_id, referent_id, sensitivity, visibility)| {
            session_id == turn.session_id
                && referent_id == issue.referent_id
                && sensitivity == "public"
                && matches!(visibility.as_str(), "provider_safe" | "confirmation_only")
        }) {
            return Ok(ConfirmationDisposition::Unchanged);
        }
    }
    let Some(proposal) = issue.proposal else {
        return Ok(ConfirmationDisposition::Unchanged);
    };
    if proposal.trim().is_empty()
        || proposal.len() > MAX_NORMALIZED_BYTES
        || proposal.chars().any(char::is_control)
        || !confirmation::is_safe_public_proposal(&proposal)
    {
        return Ok(ConfirmationDisposition::Unchanged);
    }
    let normalized = confirmation::normalize_term(&proposal);
    if normalized.is_empty() || normalized != issue.normalized {
        return Err(RepositoryFault::InvalidInput);
    }
    let expires_at_ms = turn
        .now_ms
        .checked_add(confirmation::CONFIRMATION_TTL_MS)
        .ok_or(RepositoryFault::InvalidInput)?;
    let proposal_binding = AadBinding::new(
        &issue.confirmation_id,
        &turn.session_id,
        "confirmation_proposal",
    )
    .with_turn(&turn.turn_id)
    .with_referent(&issue.referent_id);
    let term_binding = AadBinding::new(
        &issue.confirmation_id,
        &turn.session_id,
        "confirmation_term",
    )
    .with_turn(&turn.turn_id)
    .with_referent(&issue.referent_id);
    let proposal_ciphertext = custody.encrypt(&proposal_binding, proposal.as_bytes())?;
    let normalized_term_hmac =
        custody.hmac(&term_binding, issue.normalization_version, &normalized)?;
    transaction
        .execute(
            "INSERT INTO reference_confirmations
             (confirmation_id, session_id, initiating_turn_id, mention_id, referent_id,
              provider_scope, sensitivity, proposal_ciphertext, normalized_term_hmac,
              normalization_version, compatibility_epoch, created_at_ms, expires_at_ms,
              encryption_key_version, hmac_key_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                issue.confirmation_id,
                turn.session_id,
                turn.turn_id,
                issue.mention_id,
                issue.referent_id,
                issue.provider_scope,
                issue.sensitivity,
                proposal_ciphertext,
                normalized_term_hmac.as_slice(),
                issue.normalization_version as i64,
                issue.compatibility_epoch as i64,
                turn.now_ms,
                expires_at_ms,
                ENCRYPTION_VERSION as i64,
                HMAC_VERSION as i64,
            ],
        )
        .map_err(|_| RepositoryFault::InvariantViolation)?;
    Ok(ConfirmationDisposition::Pending {
        confirmation_id: issue.confirmation_id,
        proposal,
        expires_at_ms,
    })
}

fn consume_confirmation_request(
    transaction: &Transaction<'_>,
    custody: &CryptoCustody,
    turn: &ResolveLedgerCommand,
    request: ConfirmationRequest,
) -> Result<ConfirmationDisposition, RepositoryFault> {
    if turn.origin != "chat" {
        return Ok(ConfirmationDisposition::BlockedInteractiveAction);
    }
    if request.session_id != turn.session_id
        || request.execution_turn_id != turn.turn_id
        || !artifacts::validate_uuid(&request.confirmation_id)
        || !artifacts::validate_uuid(&request.execution_turn_id)
        || request.submitted != turn.input
    {
        return Err(RepositoryFault::InvalidInput);
    }
    let pending = transaction
        .query_row(
            "SELECT session_id, initiating_turn_id, referent_id, provider_scope,
                    sensitivity, normalization_version, compatibility_epoch
             FROM reference_confirmations WHERE confirmation_id=?1",
            params![request.confirmation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| RepositoryFault::Storage)?;
    let Some((
        session_id,
        initiating_turn_id,
        referent_id,
        provider_scope,
        sensitivity,
        normalization_version,
        compatibility_epoch,
    )) = pending
    else {
        let tombstone = transaction
            .query_row(
                "SELECT session_id, terminal_state FROM reference_confirmation_tombstones
                 WHERE confirmation_id=?1",
                params![request.confirmation_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| RepositoryFault::Storage)?;
        return match tombstone {
            Some((session_id, _)) if session_id == request.session_id => {
                Ok(ConfirmationDisposition::BlockedAlreadyConsumed)
            }
            Some(_) => Err(RepositoryFault::InvariantViolation),
            None => Err(RepositoryFault::Unavailable),
        };
    };
    let action = match request.kind {
        ConfirmationRequestKind::Confirm => ConfirmationAction::ConfirmReadOnly {
            normalized: confirmation::normalize_term(
                std::str::from_utf8(&request.submitted)
                    .map_err(|_| RepositoryFault::InvalidInput)?,
            ),
        },
        ConfirmationRequestKind::Edit => ConfirmationAction::Edited,
    };
    consume_confirmation(
        transaction,
        custody,
        turn,
        ConfirmationConsumption {
            confirmation_id: request.confirmation_id,
            session_id,
            initiating_turn_id,
            referent_id,
            provider_scope: match provider_scope.as_str() {
                "web_search_fetch" => "web_search_fetch",
                "direct_public_fetch" => "direct_public_fetch",
                _ => return Err(RepositoryFault::CorruptState),
            },
            sensitivity: match sensitivity.as_str() {
                "public" => "public",
                "private" => "private",
                "sensitive" => "sensitive",
                _ => return Err(RepositoryFault::CorruptState),
            },
            normalization_version: u32::try_from(normalization_version)
                .map_err(|_| RepositoryFault::CorruptState)?,
            compatibility_epoch: u32::try_from(compatibility_epoch)
                .map_err(|_| RepositoryFault::CorruptState)?,
            execution_turn_id: turn.turn_id.clone(),
            action,
        },
    )
}

fn consume_confirmation(
    transaction: &Transaction<'_>,
    custody: &CryptoCustody,
    turn: &ResolveLedgerCommand,
    consumption: ConfirmationConsumption,
) -> Result<ConfirmationDisposition, RepositoryFault> {
    if !artifacts::validate_uuid(&consumption.confirmation_id)
        || !artifacts::validate_uuid(&consumption.initiating_turn_id)
        || !artifacts::validate_uuid(&consumption.execution_turn_id)
        || consumption.execution_turn_id != turn.turn_id
        || consumption.session_id != turn.session_id
        || consumption.normalization_version == 0
        || consumption.compatibility_epoch == 0
        || !matches!(
            consumption.provider_scope,
            "web_search_fetch" | "direct_public_fetch"
        )
        || !matches!(consumption.sensitivity, "public" | "private" | "sensitive")
    {
        return Err(RepositoryFault::InvalidInput);
    }
    let pending = transaction
        .query_row(
            "SELECT confirmation_id, session_id, initiating_turn_id, mention_id,
                    referent_id, provider_scope, sensitivity, normalized_term_hmac,
                    proposal_ciphertext, normalization_version, compatibility_epoch,
                    created_at_ms, expires_at_ms, encryption_key_version,
                    hmac_key_version
             FROM reference_confirmations WHERE confirmation_id=?1",
            params![consumption.confirmation_id],
            read_pending_confirmation,
        )
        .optional()
        .map_err(|_| RepositoryFault::Storage)?
        .ok_or_else(|| {
            let tombstone = transaction
                .query_row(
                    "SELECT session_id FROM reference_confirmation_tombstones
                     WHERE confirmation_id=?1",
                    params![consumption.confirmation_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .ok()
                .flatten();
            if tombstone.as_deref() == Some(consumption.session_id.as_str()) {
                RepositoryFault::AlreadyConsumed
            } else {
                RepositoryFault::InvariantViolation
            }
        })?;
    if pending.1 != consumption.session_id
        || pending.2 != consumption.initiating_turn_id
        || pending.4 != consumption.referent_id
        || pending.5 != consumption.provider_scope
        || pending.6 != consumption.sensitivity
        || pending.9 != i64::from(consumption.normalization_version)
        || pending.10 != i64::from(consumption.compatibility_epoch)
        || pending.13 != ENCRYPTION_VERSION as i64
        || pending.14 != HMAC_VERSION as i64
    {
        return Err(RepositoryFault::InvariantViolation);
    }
    if pending.9 != i64::from(confirmation::NORMALIZATION_VERSION)
        || pending.10 != i64::from(confirmation::COMPATIBILITY_EPOCH)
    {
        return Err(RepositoryFault::InvariantViolation);
    }

    let proposal_binding = AadBinding::new(&pending.0, &pending.1, "confirmation_proposal")
        .with_turn(&pending.2)
        .with_referent(&pending.4);
    let _proposal = custody.decrypt(&proposal_binding, &pending.8)?;
    let term_binding = AadBinding::new(&pending.0, &pending.1, "confirmation_term")
        .with_turn(&pending.2)
        .with_referent(&pending.4);

    let (terminal_state, authorize) = match consumption.action {
        ConfirmationAction::Edited => ("edited", None),
        ConfirmationAction::Invalidate => ("invalidated", None),
        ConfirmationAction::ConfirmReadOnly { normalized } => {
            if pending.12 <= turn.now_ms {
                ("expired", None)
            } else {
                match custody.verify_hmac(
                    &term_binding,
                    consumption.normalization_version,
                    &normalized,
                    &pending.7,
                ) {
                    Ok(()) => ("consumed", None),
                    Err(CryptoFault::AuthenticationFailed) => ("term_mismatch", None),
                    Err(fault) => return Err(fault.into()),
                }
            }
        }
        ConfirmationAction::Confirm {
            normalized,
            authorization_id,
            query_plan_hmac,
            permit_nonce_hmac,
            plan_version,
            configuration_epoch,
            process_epoch,
            search_budget,
            fetch_budget,
            operations,
        } => {
            if pending.12 <= turn.now_ms {
                ("expired", None)
            } else {
                match custody.verify_hmac(
                    &term_binding,
                    consumption.normalization_version,
                    &normalized,
                    &pending.7,
                ) {
                    Ok(()) => {
                        validate_authorization_plan(
                            &authorization_id,
                            consumption.provider_scope,
                            search_budget,
                            fetch_budget,
                            &operations,
                        )?;
                        (
                            "consumed",
                            Some((
                                authorization_id,
                                query_plan_hmac,
                                permit_nonce_hmac,
                                plan_version,
                                configuration_epoch,
                                process_epoch,
                                search_budget,
                                fetch_budget,
                                operations,
                            )),
                        )
                    }
                    Err(CryptoFault::AuthenticationFailed) => ("term_mismatch", None),
                    Err(fault) => return Err(fault.into()),
                }
            }
        }
    };

    transaction
        .execute(
            "INSERT INTO reference_confirmation_tombstones
             (confirmation_id, session_id, initiating_turn_id, execution_turn_id,
              referent_id, provider_scope, normalized_term_hmac, terminal_state,
              compatibility_epoch, created_at_ms, terminal_at_ms, delete_after_ms,
              hmac_key_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10 + 86400000, ?11)",
            params![
                pending.0,
                pending.1,
                pending.2,
                Some(consumption.execution_turn_id.as_str()),
                pending.4,
                pending.5,
                pending.7,
                terminal_state,
                pending.10,
                turn.now_ms,
                pending.14,
            ],
        )
        .map_err(|_| RepositoryFault::InvariantViolation)?;

    if let Some((
        authorization_id,
        query_plan_hmac,
        permit_nonce_hmac,
        plan_version,
        configuration_epoch,
        process_epoch,
        search_budget,
        fetch_budget,
        operations,
    )) = authorize
    {
        transaction
            .execute(
                "INSERT INTO query_authorizations
                 (authorization_id, session_id, initiating_turn_id, execution_turn_id,
                  referent_id, mention_id, confirmation_id, authorization_method,
                  provider_scope, query_plan_hmac, permit_nonce_hmac, plan_version,
                  normalization_version, hmac_key_version, compatibility_epoch,
                  configuration_epoch, process_epoch, search_budget, fetch_budget,
                  issued_at_ms, expires_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'confirmed', ?8, ?9, ?10,
                         ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?19 + 300000)",
                params![
                    authorization_id,
                    pending.1,
                    pending.2,
                    consumption.execution_turn_id,
                    pending.4,
                    pending.3,
                    pending.0,
                    pending.5,
                    query_plan_hmac.as_slice(),
                    permit_nonce_hmac.as_slice(),
                    plan_version as i64,
                    pending.9,
                    pending.14,
                    pending.10,
                    configuration_epoch as i64,
                    process_epoch as i64,
                    search_budget as i64,
                    fetch_budget as i64,
                    turn.now_ms,
                ],
            )
            .map_err(|_| RepositoryFault::InvariantViolation)?;
        for operation in operations {
            let alternative_group = operation
                .alternative_group
                .map(|group| format!("alternative-{group}"));
            transaction
                .execute(
                    "INSERT INTO query_authorization_operations
                     (authorization_id, operation_ordinal, operation_hmac,
                      operation_kind, provider, max_attempts, alternative_group)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        authorization_id,
                        operation.operation_ordinal,
                        operation.operation_hmac.as_slice(),
                        operation.operation_kind,
                        operation.provider,
                        operation.max_attempts,
                        alternative_group,
                    ],
                )
                .map_err(|_| RepositoryFault::InvariantViolation)?;
        }
    }

    transaction
        .execute(
            "DELETE FROM reference_confirmations WHERE confirmation_id=?1",
            params![pending.0],
        )
        .map_err(|_| RepositoryFault::Storage)?;
    Ok(match terminal_state {
        "consumed" => ConfirmationDisposition::Confirmed {
            referent_id: pending.4,
            mention_id: pending.3,
            provider_scope: pending.5,
        },
        "edited" => ConfirmationDisposition::EditAccepted {
            replacement: turn.input.clone(),
        },
        "term_mismatch" => ConfirmationDisposition::BlockedTermMismatch,
        "expired" => ConfirmationDisposition::BlockedExpired,
        "invalidated" => ConfirmationDisposition::BlockedInvalidationFailure,
        _ => ConfirmationDisposition::Unavailable,
    })
}

fn read_pending_confirmation(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingConfirmationRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
    ))
}

fn validate_authorization_plan(
    authorization_id: &str,
    provider_scope: &str,
    search_budget: u8,
    fetch_budget: u8,
    operations: &[AuthorizationOperationSpec],
) -> Result<(), RepositoryFault> {
    if !artifacts::validate_uuid(authorization_id)
        || search_budget > 2
        || fetch_budget > 5
        || operations.is_empty()
    {
        return Err(RepositoryFault::InvalidInput);
    }
    let mut ordinals = BTreeSet::new();
    let mut operation_hmacs = BTreeSet::new();
    let mut alternatives = BTreeSet::new();
    let mut has_first_search = false;
    let mut has_second_search = false;
    for operation in operations {
        if operation.operation_ordinal < 0
            || !ordinals.insert(operation.operation_ordinal)
            || !operation_hmacs.insert(operation.operation_hmac)
            || !matches!(operation.operation_kind, "search" | "fetch")
            || !matches!(
                operation.provider,
                "tavily" | "duckduckgo" | "wikipedia" | "direct"
            )
            || !matches!(operation.max_attempts, 1 | 2)
            || (provider_scope == "direct_public_fetch" && operation.provider != "direct")
            || (provider_scope == "web_search_fetch" && operation.provider == "direct")
            || (operation.operation_kind == "fetch" && operation.alternative_group.is_some())
        {
            return Err(RepositoryFault::InvalidInput);
        }
        if operation.operation_kind == "search" && operation.operation_ordinal == 0 {
            has_first_search = true;
        }
        if operation.operation_kind == "search" && operation.operation_ordinal == 1 {
            has_second_search = true;
        }
        if let Some(group) = operation.alternative_group {
            if operation.operation_ordinal != 1 || !alternatives.insert(group) {
                return Err(RepositoryFault::InvalidInput);
            }
        }
    }
    if alternatives.len() > 1
        || (!alternatives.is_empty() && (!has_first_search || !has_second_search))
    {
        return Err(RepositoryFault::InvalidInput);
    }
    Ok(())
}

type AuthorizationRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
);

fn terminalize_authorizations(
    transaction: &Transaction<'_>,
    execution_turn_id: &str,
    now_ms: i64,
) -> Result<(), RepositoryFault> {
    let authorizations = load_authorizations_for_turn(transaction, execution_turn_id)?;
    for authorization in authorizations {
        write_replay_tombstone(transaction, &authorization, now_ms, "completed")?;
    }
    Ok(())
}

fn terminalize_expired_authorizations(
    transaction: &Transaction<'_>,
    now_ms: i64,
) -> Result<u64, RepositoryFault> {
    let authorizations = transaction
        .prepare(
            "SELECT authorization_id, session_id, initiating_turn_id, execution_turn_id,
                    referent_id, confirmation_id, authorization_method, provider_scope,
                    query_plan_hmac, permit_nonce_hmac, reserved_searches, reserved_fetches,
                    hmac_key_version, compatibility_epoch, configuration_epoch, process_epoch
             FROM query_authorizations WHERE expires_at_ms <= ?1",
        )
        .map_err(|_| RepositoryFault::Storage)?
        .query_map(params![now_ms], read_authorization_row)
        .map_err(|_| RepositoryFault::Storage)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| RepositoryFault::Storage)?;
    for authorization in &authorizations {
        write_replay_tombstone(transaction, authorization, now_ms, "expired")?;
    }
    Ok(authorizations.len() as u64)
}

fn load_authorizations_for_turn(
    transaction: &Transaction<'_>,
    execution_turn_id: &str,
) -> Result<Vec<AuthorizationRow>, RepositoryFault> {
    transaction
        .prepare(
            "SELECT authorization_id, session_id, initiating_turn_id, execution_turn_id,
                    referent_id, confirmation_id, authorization_method, provider_scope,
                    query_plan_hmac, permit_nonce_hmac, reserved_searches, reserved_fetches,
                    hmac_key_version, compatibility_epoch, configuration_epoch, process_epoch
             FROM query_authorizations WHERE execution_turn_id=?1",
        )
        .map_err(|_| RepositoryFault::Storage)?
        .query_map(params![execution_turn_id], read_authorization_row)
        .map_err(|_| RepositoryFault::Storage)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|_| RepositoryFault::Storage)
}

fn read_authorization_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuthorizationRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
    ))
}

fn write_replay_tombstone(
    transaction: &Transaction<'_>,
    authorization: &AuthorizationRow,
    now_ms: i64,
    terminal_state: &str,
) -> Result<(), RepositoryFault> {
    transaction
        .execute(
            "INSERT INTO query_replay_tombstones
             (authorization_id, session_id, initiating_turn_id, execution_turn_id,
              referent_id, confirmation_id, authorization_method, provider_scope,
              query_plan_hmac, permit_nonce_hmac, final_reserved_searches,
              final_reserved_fetches, hmac_key_version, compatibility_epoch,
              configuration_epoch, process_epoch, terminal_state, terminal_at_ms,
              delete_after_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                authorization.0,
                authorization.1,
                authorization.2,
                authorization.3,
                authorization.4,
                authorization.5,
                authorization.6,
                authorization.7,
                authorization.8,
                authorization.9,
                authorization.10,
                authorization.11,
                authorization.12,
                authorization.13,
                authorization.14,
                authorization.15,
                terminal_state,
                now_ms,
                now_ms + 86_400_000_i64,
            ],
        )
        .map_err(|_| RepositoryFault::Storage)?;
    transaction
        .execute(
            "DELETE FROM query_authorizations WHERE authorization_id=?1",
            params![authorization.0],
        )
        .map_err(|_| RepositoryFault::Storage)?;
    Ok(())
}

fn persisted_key_versions(
    connection: &Connection,
) -> Result<BTreeSet<(u32, u32)>, RepositoryFault> {
    let mut versions = BTreeSet::new();
    for (table, encryption_column, hmac_column) in [
        ("reference_turns", None, Some("hmac_key_version")),
        (
            "reference_turn_staging",
            Some("encryption_key_version"),
            Some("hmac_key_version"),
        ),
        (
            "conversation_mentions",
            Some("encryption_key_version"),
            Some("hmac_key_version"),
        ),
        (
            "mention_web_mappings",
            Some("encryption_key_version"),
            Some("hmac_key_version"),
        ),
        ("mention_anchors", None, Some("hmac_key_version")),
        (
            "reference_confirmations",
            Some("encryption_key_version"),
            Some("hmac_key_version"),
        ),
        (
            "reference_confirmation_tombstones",
            None,
            Some("hmac_key_version"),
        ),
        ("query_authorizations", None, Some("hmac_key_version")),
        ("query_replay_tombstones", None, Some("hmac_key_version")),
    ] {
        let sql = match (encryption_column, hmac_column) {
            (Some(encryption), Some(hmac)) => {
                format!("SELECT COALESCE({encryption}, 1), {hmac} FROM {table}")
            }
            (None, Some(hmac)) => format!("SELECT 1, {hmac} FROM {table}"),
            _ => continue,
        };
        let mut statement = connection
            .prepare(&sql)
            .map_err(|_| RepositoryFault::Storage)?;
        let rows = statement
            .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .map_err(|_| RepositoryFault::Storage)?;
        for row in rows {
            let (encryption, hmac) = row.map_err(|_| RepositoryFault::Storage)?;
            if encryption <= 0 || hmac <= 0 {
                return Err(RepositoryFault::CorruptState);
            }
            versions.insert((encryption as u32, hmac as u32));
        }
    }
    Ok(versions)
}
