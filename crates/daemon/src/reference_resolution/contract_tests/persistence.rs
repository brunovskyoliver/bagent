use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeSet;
use tempfile::NamedTempFile;

#[test]
fn crypto_red_evidence_requires_authenticated_format_and_custody() {
    use crate::reference_resolution::crypto::{AadBinding, CryptoCustody, FakeKeyProvider};

    let custody = CryptoCustody::with_provider(FakeKeyProvider::deterministic());
    let binding = AadBinding::new("synthetic-row", "synthetic-session", "synthetic-field");
    let ciphertext = custody
        .encrypt(&binding, b"SYNTHETIC_MENTION_SENTINEL_01A1")
        .expect("synthetic encryption");
    assert!(!ciphertext
        .windows(b"SYNTHETIC_MENTION_SENTINEL_01A1".len())
        .any(|window| window == b"SYNTHETIC_MENTION_SENTINEL_01A1"));
    assert_eq!(ciphertext[0], 1);
    assert_eq!(u32::from_be_bytes(ciphertext[1..5].try_into().unwrap()), 1);
    assert!(custody.decrypt(&binding, &ciphertext).is_ok());
}

#[test]
fn crypto_rejects_tamper_wrong_key_aad_swaps_and_unknown_versions() {
    use crate::reference_resolution::crypto::{
        AadBinding, CryptoCustody, CryptoFault, FakeKeyProvider, FakeNonceProvider,
    };
    use secrecy::ExposeSecret;

    let custody = CryptoCustody::with_provider(FakeKeyProvider::deterministic());
    let binding = AadBinding::new("synthetic-row", "synthetic-session", "synthetic-field")
        .with_turn("synthetic-turn");
    let other_binding = AadBinding::new(
        "synthetic-other-row",
        "synthetic-session",
        "synthetic-field",
    )
    .with_turn("synthetic-turn");
    let mut ciphertext = custody
        .encrypt(&binding, b"SYNTHETIC_PRIVATE_MAIL_SENTINEL_7F4A")
        .unwrap();

    ciphertext[5 + 24] ^= 0x80;
    assert!(matches!(
        custody.decrypt(&binding, &ciphertext),
        Err(CryptoFault::AuthenticationFailed)
    ));

    let ciphertext = custody
        .encrypt(&binding, b"SYNTHETIC_QUERY_SENTINEL_8B2C")
        .unwrap();
    assert!(matches!(
        custody.decrypt(&other_binding, &ciphertext),
        Err(CryptoFault::AuthenticationFailed)
    ));

    let wrong_key =
        CryptoCustody::with_provider(FakeKeyProvider::from_keys([0x33; 32], [0x44; 32]));
    assert!(matches!(
        wrong_key.decrypt(&binding, &ciphertext),
        Err(CryptoFault::AuthenticationFailed)
    ));

    let mut unknown_format = ciphertext.clone();
    unknown_format[0] = 99;
    assert!(matches!(
        custody.decrypt(&binding, &unknown_format),
        Err(CryptoFault::UnknownVersion)
    ));
    let mut unknown_key = ciphertext;
    unknown_key[1..5].copy_from_slice(&99_u32.to_be_bytes());
    assert!(matches!(
        custody.decrypt(&binding, &unknown_key),
        Err(CryptoFault::UnknownVersion)
    ));

    let digest = custody
        .hmac(&binding, 1, b"synthetic normalized term")
        .unwrap();
    assert!(custody
        .verify_hmac(&binding, 1, b"synthetic normalized term", &digest)
        .is_ok());
    assert!(matches!(
        custody.verify_hmac(&binding, 1, b"different normalized term", &digest),
        Err(CryptoFault::AuthenticationFailed)
    ));
    let plaintext = custody
        .decrypt(&binding, &custody.encrypt(&binding, b"synthetic").unwrap())
        .unwrap();
    assert_eq!(plaintext.expose_secret(), b"synthetic");

    let deterministic_a = CryptoCustody::with_providers(
        FakeKeyProvider::deterministic(),
        FakeNonceProvider::new(0x5a),
    );
    let deterministic_b = CryptoCustody::with_providers(
        FakeKeyProvider::deterministic(),
        FakeNonceProvider::new(0x5a),
    );
    assert_eq!(
        deterministic_a
            .encrypt(&binding, b"synthetic deterministic bytes")
            .unwrap(),
        deterministic_b
            .encrypt(&binding, b"synthetic deterministic bytes")
            .unwrap()
    );
}

#[test]
fn missing_key_with_existing_rows_fails_closed_without_replacement() {
    use crate::reference_resolution::crypto::{CryptoCustody, CryptoFault, FakeKeyProvider};
    use std::collections::BTreeSet;

    let custody = CryptoCustody::with_provider(FakeKeyProvider::missing());
    assert_eq!(
        custody.ensure_for_database(&BTreeSet::from([(1_u32, 1_u32)])),
        Err(CryptoFault::KeyUnavailable)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn repository_readiness_never_replaces_a_missing_key_for_persisted_rows() {
    use crate::reference_resolution::crypto::{CryptoCustody, FakeKeyProvider};
    use crate::reference_resolution::repository::SqliteRepository;

    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    connection
        .execute(
            "INSERT INTO reference_turns
             (turn_id, session_id, chat_session_id, session_seq, origin, state,
              input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
             VALUES ('20202020-2020-4020-8020-202020202020', '1', '1', 1, 'chat',
                     'open', zeroblob(32), 1, 1000, 3601000)",
            [],
        )
        .unwrap();
    let ciphertext = vec![1_u8, 2, 3, 4, 5];
    connection
        .execute(
            "INSERT INTO reference_turn_staging
             (turn_id, staged_mentions_ciphertext, staged_mentions_hmac,
              descriptor_version, encryption_key_version, hmac_key_version, created_at_ms)
             VALUES ('20202020-2020-4020-8020-202020202020', ?1, zeroblob(32), 1, 1, 1, 1000)",
            rusqlite::params![ciphertext],
        )
        .unwrap();
    let database = std::sync::Arc::new(tokio::sync::Mutex::new(connection));
    let repository = SqliteRepository::new(
        std::sync::Arc::clone(&database),
        std::sync::Arc::new(CryptoCustody::with_provider(FakeKeyProvider::missing())),
    );
    assert_eq!(
        repository.readiness().await,
        Err(crate::reference_resolution::repository::RepositoryFault::Unavailable)
    );
    let database = database.lock().await;
    let retained: Vec<u8> = database
        .query_row(
            "SELECT staged_mentions_ciphertext FROM reference_turn_staging",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(retained, ciphertext);
}

fn insert_synthetic_session(connection: &Connection, id: &str) {
    connection
        .execute(
            "INSERT INTO sessions (id, started_at) VALUES (?1, ?2)",
            rusqlite::params![id, "2026-08-19T10:00:00Z"],
        )
        .expect("insert synthetic resolver parent session");
}

fn repository_for(
    connection: Connection,
) -> std::sync::Arc<crate::reference_resolution::repository::SqliteRepository> {
    use crate::reference_resolution::crypto::{CryptoCustody, FakeKeyProvider};
    use crate::reference_resolution::repository::SqliteRepository;

    let database = std::sync::Arc::new(tokio::sync::Mutex::new(connection));
    std::sync::Arc::new(SqliteRepository::new(
        database,
        std::sync::Arc::new(CryptoCustody::with_provider(
            FakeKeyProvider::deterministic(),
        )),
    ))
}

fn public_artifact(
    turn_id: &str,
    session_id: &str,
    display: &str,
) -> crate::reference_resolution::artifacts::MentionArtifact {
    use crate::reference_resolution::artifacts::{MentionArtifact, MentionRepresentation};

    MentionArtifact {
        mention_id: "11111111-1111-4111-8111-111111111111".into(),
        referent_id: "synthetic-referent".into(),
        turn_id: turn_id.into(),
        session_id: session_id.into(),
        canonical_parent_mention_id: None,
        entity_kind: "product".into(),
        provenance: "user_authored".into(),
        assistant_lineage: None,
        producer: "resolver_user_input".into(),
        visibility: "provider_safe".into(),
        sensitivity: "public".into(),
        direct_user: true,
        untrusted_evidence: false,
        origin_ref_hmac: None,
        mail_body_origin: None,
        representation: MentionRepresentation::PublicVisible {
            display: display.into(),
            normalized: "synthetic public term".into(),
        },
        created_at_ms: 1_000,
        expires_at_ms: 1_801_000,
        hmac_key_version: 1,
        encryption_key_version: Some(1),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn transact_resolution_encrypts_bounded_staging_and_allocates_sequences() {
    use crate::reference_resolution::repository::{
        ReferenceRepository, ResolutionDecision, ResolveLedgerCommand, StagedMentionDescriptor,
    };

    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    let repository = repository_for(connection);
    let command = |turn_id: &str| ResolveLedgerCommand {
        turn_id: turn_id.into(),
        session_id: "1".into(),
        scope_id: "synthetic-chat-scope".into(),
        chat_session_id: Some("1".into()),
        automation_id: None,
        automation_run_id: None,
        origin: "chat",
        input: b"SYNTHETIC_CONVERSATION_SENTINEL_03C3".to_vec(),
        descriptors: vec![StagedMentionDescriptor {
            mention_id: "22222222-2222-4222-8222-222222222222".into(),
            referent_id: "synthetic-referent".into(),
            normalized: "SYNTHETIC_MENTION_SENTINEL_01A1".into(),
        }],
        now_ms: 1_000,
    };

    let first = repository
        .transact_resolution(
            command("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            Box::new(|snapshot| {
                assert_eq!(snapshot.session_seq, 1);
                Ok(ResolutionDecision::KeepStaging)
            }),
        )
        .await
        .expect("first resolution transaction");
    let second = repository
        .transact_resolution(
            command("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),
            Box::new(|snapshot| {
                assert_eq!(snapshot.session_seq, 2);
                Ok(ResolutionDecision::KeepStaging)
            }),
        )
        .await
        .expect("second resolution transaction");
    assert_eq!(first.session_seq, 1);
    assert_eq!(second.session_seq, 2);

    let database = repository_database(&repository).await;
    let bytes = database_bytes(&database);
    assert!(!bytes
        .windows(b"SYNTHETIC_CONVERSATION_SENTINEL_03C3".len())
        .any(|window| window == b"SYNTHETIC_CONVERSATION_SENTINEL_03C3"));
    assert!(!bytes
        .windows(b"SYNTHETIC_MENTION_SENTINEL_01A1".len())
        .any(|window| window == b"SYNTHETIC_MENTION_SENTINEL_01A1"));
}

#[tokio::test(flavor = "current_thread")]
async fn automation_resolution_snapshots_are_always_empty() {
    use crate::reference_resolution::repository::{
        ReferenceRepository, ResolutionDecision, ResolveLedgerCommand, StagedMentionDescriptor,
    };

    let connection = fresh_connection();
    connection
        .execute(
            "INSERT INTO automations
             (id, name, prompt, enabled, timezone, schedule_json, created_at, updated_at)
             VALUES ('synthetic-automation', 'synthetic', 'synthetic', 1, 'UTC', '{}', 'now', 'now')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO automation_runs
             (id, automation_id, scheduled_for, status, created_at)
             VALUES ('synthetic-run', 'synthetic-automation', 'now', 'running', 'now')",
            [],
        )
        .unwrap();
    let repository = repository_for(connection);
    let snapshot = repository
        .transact_resolution(
            ResolveLedgerCommand {
                turn_id: "abababab-abab-4aba-8aba-abababababab".into(),
                session_id: "synthetic-automation-session".into(),
                scope_id: "synthetic-automation-scope".into(),
                chat_session_id: None,
                automation_id: Some("synthetic-automation".into()),
                automation_run_id: Some("synthetic-run".into()),
                origin: "automation",
                input: b"synthetic automation input".to_vec(),
                descriptors: vec![StagedMentionDescriptor {
                    mention_id: "cdcdcdcd-cdcd-4cdc-8cdc-cdcdcdcdcdcd".into(),
                    referent_id: "synthetic-referent".into(),
                    normalized: "synthetic normalized".into(),
                }],
                now_ms: 1_000,
            },
            Box::new(|snapshot| {
                assert!(snapshot.descriptors.is_empty());
                Ok(ResolutionDecision::KeepStaging)
            }),
        )
        .await
        .unwrap();
    assert!(snapshot.descriptors.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn typed_candidate_query_enforces_chat_window_and_keeps_automation_empty() {
    use crate::reference_resolution::artifacts::ArtifactGraph;
    use crate::reference_resolution::repository::{
        CandidateQuery, RecordCompletedCommand, RecordDisposition, ReferenceRepository,
        ResolutionDecision, ResolveLedgerCommand,
    };
    use crate::reference_resolution::LedgerProvenance;

    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    let repository = repository_for(connection);
    let turn_id = "12121212-1212-4121-8121-121212121212";
    repository
        .transact_resolution(
            ResolveLedgerCommand {
                turn_id: turn_id.into(),
                session_id: "1".into(),
                scope_id: "synthetic-candidate-scope".into(),
                chat_session_id: Some("1".into()),
                automation_id: None,
                automation_run_id: None,
                origin: "chat",
                input: b"synthetic candidate introduction".to_vec(),
                descriptors: vec![],
                now_ms: 1_000,
            },
            Box::new(|snapshot| {
                assert_eq!(snapshot.candidates.len(), 0);
                Ok(ResolutionDecision::KeepStaging)
            }),
        )
        .await
        .unwrap();
    let mut graph = ArtifactGraph::default();
    graph
        .mentions
        .push(public_artifact(turn_id, "1", "synthetic public term"));
    assert_eq!(
        repository
            .record_completed(RecordCompletedCommand {
                turn_id: turn_id.into(),
                session_id: "1".into(),
                input: b"synthetic candidate introduction".to_vec(),
                artifacts: graph,
                now_ms: 2_000,
            })
            .await
            .unwrap(),
        RecordDisposition::Recorded
    );

    let candidates = repository
        .load_candidates(CandidateQuery {
            session_id: "1".into(),
            origin: "chat",
            current_seq: 2,
            now_ms: 2_000,
        })
        .await
        .unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].entity_kind,
        crate::reference_resolution::EntityKind::Product
    );
    assert_eq!(candidates[0].provenance, LedgerProvenance::PriorUser);
    assert_eq!(
        candidates[0].normalized.as_deref(),
        Some("synthetic public term")
    );
    assert_eq!(candidates[0].age_turns, 1);
    assert_eq!(candidates[0].age_minutes, 0);

    assert!(repository
        .load_candidates(CandidateQuery {
            session_id: "1".into(),
            origin: "automation",
            current_seq: 2,
            now_ms: 2_000,
        })
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn confirmation_consumption_is_atomic_one_use_and_ciphertext_free_after_commit() {
    use crate::reference_resolution::crypto::{AadBinding, CryptoCustody, FakeKeyProvider};
    use crate::reference_resolution::repository::{
        AuthorizationOperationSpec, ConfirmationAction, ConfirmationConsumption,
        ReferenceRepository, RepositoryFault, ResolutionDecision, ResolveLedgerCommand,
    };

    let initiating_turn_id = "23232323-2323-4232-8232-232323232323";
    let execution_turn_id = "24242424-2424-4242-8242-242424242424";
    let confirmation_id = "25252525-2525-4252-8252-252525252525";
    let authorization_id = "26262626-2626-4262-8262-262626262626";
    let custody = CryptoCustody::with_provider(FakeKeyProvider::deterministic());
    let proposal_binding = AadBinding::new(confirmation_id, "1", "confirmation_proposal")
        .with_turn(initiating_turn_id)
        .with_referent("synthetic-referent");
    let term_binding = AadBinding::new(confirmation_id, "1", "confirmation_term")
        .with_turn(initiating_turn_id)
        .with_referent("synthetic-referent");
    let proposal = custody
        .encrypt(&proposal_binding, b"SYNTHETIC_PROPOSAL_SENTINEL_6E6E")
        .unwrap();
    let term_hmac = custody
        .hmac(&term_binding, 1, b"synthetic normalized term")
        .unwrap();
    let repository = repository_for(fresh_connection());
    {
        let database = repository_database(&repository).await;
        insert_synthetic_session(&database, "1");
        database
            .execute(
                "INSERT INTO reference_turns
                 (turn_id, session_id, chat_session_id, session_seq, origin, state,
                  input_hmac, hmac_key_version, completion_code, producer_class,
                  artifact_hmac, created_at_ms, completed_at_ms, open_expires_at_ms)
                 VALUES (?1, '1', '1', 1, 'chat', 'completed', zeroblob(32), 1,
                         'completed', 'no_mentions', zeroblob(32), 1000, 1000, 3601000)",
                rusqlite::params![initiating_turn_id],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO reference_session_sequences
                 (scope_id, session_id, origin, chat_session_id, next_seq,
                  created_at_ms, updated_at_ms)
                 VALUES ('synthetic-confirmation-scope', '1', 'chat', '1', 2, 1000, 1000)",
                [],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO reference_confirmations
                 (confirmation_id, session_id, initiating_turn_id, referent_id,
                  provider_scope, sensitivity, proposal_ciphertext, normalized_term_hmac,
                  normalization_version, compatibility_epoch, created_at_ms, expires_at_ms,
                  encryption_key_version, hmac_key_version)
                 VALUES (?1, '1', ?2, 'synthetic-referent', 'web_search_fetch', 'public',
                         ?3, ?4, 1, 1, 1000, 301000, 1, 1)",
                rusqlite::params![
                    confirmation_id,
                    initiating_turn_id,
                    proposal,
                    term_hmac.as_slice()
                ],
            )
            .unwrap();
    }

    let make_consumption = || ConfirmationConsumption {
        confirmation_id: confirmation_id.into(),
        session_id: "1".into(),
        initiating_turn_id: initiating_turn_id.into(),
        referent_id: "synthetic-referent".into(),
        provider_scope: "web_search_fetch",
        sensitivity: "public",
        normalization_version: 1,
        compatibility_epoch: 1,
        execution_turn_id: execution_turn_id.into(),
        action: ConfirmationAction::Confirm {
            normalized: b"synthetic normalized term".to_vec(),
            authorization_id: authorization_id.into(),
            query_plan_hmac: [0x31; 32],
            permit_nonce_hmac: [0x32; 32],
            plan_version: 1,
            configuration_epoch: 1,
            process_epoch: 1,
            search_budget: 1,
            fetch_budget: 0,
            operations: vec![AuthorizationOperationSpec {
                operation_ordinal: 0,
                operation_hmac: [0x33; 32],
                operation_kind: "search",
                provider: "tavily",
                max_attempts: 1,
                alternative_group: None,
            }],
        },
    };
    let first_consumption = make_consumption();
    repository
        .transact_resolution(
            ResolveLedgerCommand {
                turn_id: execution_turn_id.into(),
                session_id: "1".into(),
                scope_id: "synthetic-confirmation-scope".into(),
                chat_session_id: Some("1".into()),
                automation_id: None,
                automation_run_id: None,
                origin: "chat",
                input: b"synthetic confirmation turn".to_vec(),
                descriptors: vec![],
                now_ms: 2_000,
            },
            Box::new(move |_| Ok(ResolutionDecision::ConsumeConfirmation(first_consumption))),
        )
        .await
        .unwrap();

    let database = repository_database(&repository).await;
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM reference_confirmations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        0
    );
    assert_eq!(
        database
            .query_row(
                "SELECT terminal_state FROM reference_confirmation_tombstones",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "consumed"
    );
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM query_authorizations", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM query_authorization_operations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    let bytes = database_bytes(&database);
    assert!(!bytes
        .windows(b"SYNTHETIC_PROPOSAL_SENTINEL_6E6E".len())
        .any(|window| window == b"SYNTHETIC_PROPOSAL_SENTINEL_6E6E"));
    drop(database);

    let mut replay_consumption = make_consumption();
    replay_consumption.execution_turn_id = "27272727-2727-4272-8272-272727272727".into();
    let replay = repository
        .transact_resolution(
            ResolveLedgerCommand {
                turn_id: "27272727-2727-4272-8272-272727272727".into(),
                session_id: "1".into(),
                scope_id: "synthetic-confirmation-scope".into(),
                chat_session_id: Some("1".into()),
                automation_id: None,
                automation_run_id: None,
                origin: "chat",
                input: b"synthetic replay turn".to_vec(),
                descriptors: vec![],
                now_ms: 2_001,
            },
            Box::new(move |_| Ok(ResolutionDecision::ConsumeConfirmation(replay_consumption))),
        )
        .await;
    assert!(
        !matches!(replay, Err(RepositoryFault::InvariantViolation)),
        "confirmation replay must be a closed already-consumed result, not a generic repository fault"
    );
    let database = repository_database(&repository).await;
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM reference_turns", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        database
            .query_row("SELECT COUNT(*) FROM query_authorizations", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
}

#[tokio::test(flavor = "current_thread")]
async fn edited_confirmation_tombstone_binds_the_editing_execution_turn() {
    use crate::reference_resolution::crypto::{AadBinding, CryptoCustody, FakeKeyProvider};
    use crate::reference_resolution::repository::{
        ConfirmationAction, ConfirmationConsumption, ReferenceRepository, ResolutionDecision,
        ResolveLedgerCommand,
    };

    let initiating_turn_id = "28282828-2828-4282-8282-282828282828";
    let execution_turn_id = "29292929-2929-4292-8292-292929292929";
    let confirmation_id = "30303030-3030-4303-8303-303030303030";
    let custody = CryptoCustody::with_provider(FakeKeyProvider::deterministic());
    let proposal_binding = AadBinding::new(confirmation_id, "1", "confirmation_proposal")
        .with_turn(initiating_turn_id)
        .with_referent("synthetic-referent");
    let term_binding = AadBinding::new(confirmation_id, "1", "confirmation_term")
        .with_turn(initiating_turn_id)
        .with_referent("synthetic-referent");
    let proposal = custody
        .encrypt(&proposal_binding, b"SYNTHETIC_EDIT_PROPOSAL_SENTINEL_11E1")
        .unwrap();
    let term_hmac = custody
        .hmac(&term_binding, 1, b"synthetic normalized term")
        .unwrap();
    let repository = repository_for(fresh_connection());
    {
        let database = repository_database(&repository).await;
        insert_synthetic_session(&database, "1");
        database
            .execute(
                "INSERT INTO reference_turns
                 (turn_id, session_id, chat_session_id, session_seq, origin, state,
                  input_hmac, hmac_key_version, completion_code, producer_class,
                  artifact_hmac, created_at_ms, completed_at_ms, open_expires_at_ms)
                 VALUES (?1, '1', '1', 1, 'chat', 'completed', zeroblob(32), 1,
                         'completed', 'no_mentions', zeroblob(32), 1000, 1000, 3601000)",
                rusqlite::params![initiating_turn_id],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO reference_session_sequences
                 (scope_id, session_id, origin, chat_session_id, next_seq,
                  created_at_ms, updated_at_ms)
                 VALUES ('synthetic-edit-scope', '1', 'chat', '1', 2, 1000, 1000)",
                [],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO reference_confirmations
                 (confirmation_id, session_id, initiating_turn_id, referent_id,
                  provider_scope, sensitivity, proposal_ciphertext, normalized_term_hmac,
                  normalization_version, compatibility_epoch, created_at_ms, expires_at_ms,
                  encryption_key_version, hmac_key_version)
                 VALUES (?1, '1', ?2, 'synthetic-referent', 'web_search_fetch', 'public',
                         ?3, ?4, 1, 1, 1000, 301000, 1, 1)",
                rusqlite::params![
                    confirmation_id,
                    initiating_turn_id,
                    proposal,
                    term_hmac.as_slice()
                ],
            )
            .unwrap();
    }

    repository
        .transact_resolution(
            ResolveLedgerCommand {
                turn_id: execution_turn_id.into(),
                session_id: "1".into(),
                scope_id: "synthetic-edit-scope".into(),
                chat_session_id: Some("1".into()),
                automation_id: None,
                automation_run_id: None,
                origin: "chat",
                input: b"synthetic edit envelope".to_vec(),
                descriptors: vec![],
                now_ms: 2_000,
            },
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmation(
                    ConfirmationConsumption {
                        confirmation_id: confirmation_id.into(),
                        session_id: "1".into(),
                        initiating_turn_id: initiating_turn_id.into(),
                        referent_id: "synthetic-referent".into(),
                        provider_scope: "web_search_fetch",
                        sensitivity: "public",
                        normalization_version: 1,
                        compatibility_epoch: 1,
                        execution_turn_id: execution_turn_id.into(),
                        action: ConfirmationAction::Edited,
                    },
                ))
            }),
        )
        .await
        .unwrap();

    let database = repository_database(&repository).await;
    let bound_execution_turn: Option<String> = database
        .query_row(
            "SELECT execution_turn_id FROM reference_confirmation_tombstones
             WHERE confirmation_id=?1",
            rusqlite::params![confirmation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(bound_execution_turn.as_deref(), Some(execution_turn_id));
}

#[tokio::test(flavor = "current_thread")]
async fn completed_turn_graph_is_atomic_and_retries_are_typed() {
    use crate::reference_resolution::artifacts::ArtifactGraph;
    use crate::reference_resolution::repository::{
        RecordCompletedCommand, RecordDisposition, ReferenceRepository, RepositoryFault,
        ResolutionDecision, ResolveLedgerCommand, StagedMentionDescriptor,
    };

    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    let repository = repository_for(connection);
    let turn_id = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    repository
        .transact_resolution(
            ResolveLedgerCommand {
                turn_id: turn_id.into(),
                session_id: "1".into(),
                scope_id: "synthetic-chat-scope".into(),
                chat_session_id: Some("1".into()),
                automation_id: None,
                automation_run_id: None,
                origin: "chat",
                input: b"synthetic current input".to_vec(),
                descriptors: vec![StagedMentionDescriptor {
                    mention_id: "11111111-1111-4111-8111-111111111111".into(),
                    referent_id: "synthetic-referent".into(),
                    normalized: "synthetic public term".into(),
                }],
                now_ms: 1_000,
            },
            Box::new(|_| Ok(ResolutionDecision::KeepStaging)),
        )
        .await
        .unwrap();

    let mut graph = ArtifactGraph::default();
    graph.mentions.push(public_artifact(
        turn_id,
        "1",
        "SYNTHETIC_MENTION_SENTINEL_01A1",
    ));
    let command = || RecordCompletedCommand {
        turn_id: turn_id.into(),
        session_id: "1".into(),
        input: b"synthetic current input".to_vec(),
        artifacts: graph.clone(),
        now_ms: 2_000,
    };
    assert_eq!(
        repository.record_completed(command()).await.unwrap(),
        RecordDisposition::Recorded
    );
    assert_eq!(
        repository.record_completed(command()).await.unwrap(),
        RecordDisposition::IdenticalRetry
    );
    let mut conflict = graph;
    conflict.mentions[0].representation =
        crate::reference_resolution::artifacts::MentionRepresentation::PublicVisible {
            display: "SYNTHETIC_CONFLICTING_ARTIFACT_04D4".into(),
            normalized: "synthetic public term".into(),
        };
    let conflicting = repository
        .record_completed(RecordCompletedCommand {
            turn_id: turn_id.into(),
            session_id: "1".into(),
            input: b"synthetic current input".to_vec(),
            artifacts: conflict,
            now_ms: 2_000,
        })
        .await;
    assert_eq!(conflicting, Err(RepositoryFault::ConflictingRetry));
}

#[tokio::test(flavor = "current_thread")]
async fn encrypted_staging_survives_temporary_file_reopen_with_stable_test_keys() {
    use crate::reference_resolution::artifacts::ArtifactGraph;
    use crate::reference_resolution::crypto::{CryptoCustody, FakeKeyProvider};
    use crate::reference_resolution::repository::{
        RecordCompletedCommand, ReferenceRepository, ResolutionDecision, ResolveLedgerCommand,
        StagedMentionDescriptor,
    };

    let file = NamedTempFile::new().unwrap();
    let turn_id = "38383838-3838-4383-8383-383838383838";
    {
        let mut connection = Connection::open(file.path()).unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .unwrap();
        crate::embedded::migrations::runner()
            .run(&mut connection)
            .unwrap();
        insert_synthetic_session(&connection, "1");
        let repository = repository_for(connection);
        repository
            .transact_resolution(
                ResolveLedgerCommand {
                    turn_id: turn_id.into(),
                    session_id: "1".into(),
                    scope_id: "synthetic-reopen-scope".into(),
                    chat_session_id: Some("1".into()),
                    automation_id: None,
                    automation_run_id: None,
                    origin: "chat",
                    input: b"SYNTHETIC_DB_INPUT_SENTINEL_5A5A".to_vec(),
                    descriptors: vec![StagedMentionDescriptor {
                        mention_id: "11111111-1111-4111-8111-111111111111".into(),
                        referent_id: "synthetic-referent".into(),
                        normalized: "synthetic public term".into(),
                    }],
                    now_ms: 1000,
                },
                Box::new(|_| Ok(ResolutionDecision::KeepStaging)),
            )
            .await
            .unwrap();
        drop(repository);
    }

    let reopened = Connection::open(file.path()).unwrap();
    reopened.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    let repository = std::sync::Arc::new(
        crate::reference_resolution::repository::SqliteRepository::new(
            std::sync::Arc::new(tokio::sync::Mutex::new(reopened)),
            std::sync::Arc::new(CryptoCustody::with_provider(
                FakeKeyProvider::deterministic(),
            )),
        ),
    );
    let mut graph = ArtifactGraph::default();
    graph.mentions.push(public_artifact(
        turn_id,
        "1",
        "SYNTHETIC_DB_DISPLAY_SENTINEL_5B5B",
    ));
    assert_eq!(
        repository
            .record_completed(RecordCompletedCommand {
                turn_id: turn_id.into(),
                session_id: "1".into(),
                input: b"SYNTHETIC_DB_INPUT_SENTINEL_5A5A".to_vec(),
                artifacts: graph,
                now_ms: 2000,
            })
            .await
            .unwrap(),
        crate::reference_resolution::repository::RecordDisposition::Recorded
    );
    for path in [
        file.path().to_path_buf(),
        std::path::PathBuf::from(format!("{}-wal", file.path().display())),
        std::path::PathBuf::from(format!("{}-shm", file.path().display())),
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            for sentinel in [
                b"SYNTHETIC_DB_INPUT_SENTINEL_5A5A".as_slice(),
                b"SYNTHETIC_DB_DISPLAY_SENTINEL_5B5B".as_slice(),
            ] {
                assert!(!bytes
                    .windows(sentinel.len())
                    .any(|window| window == sentinel));
            }
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn injected_graph_failure_rolls_back_every_mention_and_anchor_write() {
    use crate::reference_resolution::artifacts::{
        AnchorArtifact, AnchorKind, ArtifactGraph, DisplayClass,
    };
    use crate::reference_resolution::repository::{
        RecordCompletedCommand, ReferenceRepository, RepositoryFault, ResolutionDecision,
        ResolveLedgerCommand, StagedMentionDescriptor,
    };

    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    let repository = repository_for(connection);
    let turn_id = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
    repository
        .transact_resolution(
            ResolveLedgerCommand {
                turn_id: turn_id.into(),
                session_id: "1".into(),
                scope_id: "synthetic-failure-scope".into(),
                chat_session_id: Some("1".into()),
                automation_id: None,
                automation_run_id: None,
                origin: "chat",
                input: b"synthetic input".to_vec(),
                descriptors: vec![StagedMentionDescriptor {
                    mention_id: "11111111-1111-4111-8111-111111111111".into(),
                    referent_id: "synthetic-referent".into(),
                    normalized: "synthetic public term".into(),
                }],
                now_ms: 1_000,
            },
            Box::new(|_| Ok(ResolutionDecision::KeepStaging)),
        )
        .await
        .unwrap();
    let mut graph = ArtifactGraph::default();
    graph
        .mentions
        .push(public_artifact(turn_id, "1", "synthetic display"));
    graph.anchors.push(AnchorArtifact {
        anchor_id: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".into(),
        mention_id: "99999999-9999-4999-8999-999999999999".into(),
        turn_id: turn_id.into(),
        kind: AnchorKind::Visible,
        display_class: DisplayClass::UserInput,
        ordinal: 0,
        start_utf8: Some(0),
        end_utf8: Some(4),
        visible_span_hmac: Some([0x55; 32]),
        opaque_anchor_hmac: None,
        hmac_key_version: 1,
        created_at_ms: 1_000,
    });
    let result = repository
        .record_completed(RecordCompletedCommand {
            turn_id: turn_id.into(),
            session_id: "1".into(),
            input: b"synthetic input".to_vec(),
            artifacts: graph,
            now_ms: 2_000,
        })
        .await;
    assert_eq!(result, Err(RepositoryFault::InvariantViolation));

    let database = repository_database(&repository).await;
    let mention_count: i64 = database
        .query_row("SELECT COUNT(*) FROM conversation_mentions", [], |row| {
            row.get(0)
        })
        .unwrap();
    let staging_count: i64 = database
        .query_row("SELECT COUNT(*) FROM reference_turn_staging", [], |row| {
            row.get(0)
        })
        .unwrap();
    let state: String = database
        .query_row(
            "SELECT state FROM reference_turns WHERE turn_id=?1",
            rusqlite::params![turn_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mention_count, 0);
    assert_eq!(staging_count, 1);
    assert_eq!(state, "open");
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_provider_reservations_spend_one_bounded_attempt() {
    use crate::reference_resolution::repository::{
        ReferenceRepository, ReservationDisposition, ReserveAttemptCommand,
    };

    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    let repository = repository_for(connection);
    let turn_id = "ffffffff-ffff-4fff-8fff-ffffffffffff";
    let other_turn_id = "abababab-abab-4aba-8aba-abababababab";
    {
        let database = repository_database(&repository).await;
        for id in [turn_id, other_turn_id] {
            database
                .execute(
                    "INSERT INTO reference_turns
                     (turn_id, session_id, chat_session_id, session_seq, origin, state,
                      input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
                     VALUES (?1, '1', '1', ?2, 'chat', 'open', zeroblob(32), 1, 1000, 3601000)",
                    rusqlite::params![id, if id == turn_id { 1_i64 } else { 2_i64 }],
                )
                .unwrap();
        }
        database
            .execute(
                "INSERT INTO query_authorizations
                 (authorization_id, session_id, initiating_turn_id, execution_turn_id,
                  referent_id, authorization_method, provider_scope, query_plan_hmac,
                  permit_nonce_hmac, plan_version, hmac_key_version, compatibility_epoch,
                  configuration_epoch, process_epoch, search_budget, fetch_budget,
                  issued_at_ms, expires_at_ms)
                 VALUES ('12121212-1212-4121-8121-121212121212', '1', ?1, ?2,
                         'synthetic-referent', 'current_user', 'web_search_fetch',
                         zeroblob(32), zeroblob(32), 1, 1, 1, 1, 1, 1, 0, 1000, 301000)",
                rusqlite::params![turn_id, turn_id],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO query_authorization_operations
                 (authorization_id, operation_ordinal, operation_hmac, operation_kind,
                  provider, max_attempts)
                 VALUES ('12121212-1212-4121-8121-121212121212', 0, zeroblob(32), 'search', 'tavily', 1)",
                [],
            )
            .unwrap();
    }
    let make_command = |reservation_id: &str| ReserveAttemptCommand {
        authorization_id: "12121212-1212-4121-8121-121212121212".into(),
        session_id: "1".into(),
        execution_turn_id: turn_id.into(),
        operation_ordinal: 0,
        operation_hmac: [0; 32],
        provider: "tavily",
        query_plan_hmac: [0; 32],
        permit_nonce_hmac: [0; 32],
        reservation_id: reservation_id.into(),
        now_ms: 2_000,
    };
    let (first, second) = tokio::join!(
        repository.reserve_provider_attempt(make_command("13131313-1313-4131-8131-131313131313")),
        repository.reserve_provider_attempt(make_command("14141414-1414-4141-8141-141414141414")),
    );
    let winners = [first, second]
        .into_iter()
        .filter(|result| matches!(result, Ok(ReservationDisposition::Reserved { .. })))
        .count();
    assert_eq!(winners, 1);
    let database = repository_database(&repository).await;
    let reservations: i64 = database
        .query_row(
            "SELECT COUNT(*) FROM provider_attempt_reservations",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let spent: i64 = database
        .query_row(
            "SELECT reserved_searches FROM query_authorizations
             WHERE authorization_id='12121212-1212-4121-8121-121212121212'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(reservations, 1);
    assert_eq!(spent, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn prune_reconciles_expiry_retention_and_reopen_safe_cleanup() {
    use crate::reference_resolution::repository::ReferenceRepository;

    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    let repository = repository_for(connection);
    let turn_id = "15151515-1515-4151-8151-151515151515";
    let open_turn_id = "16161616-1616-4161-8161-161616161616";
    let mention_id = "17171717-1717-4171-8171-171717171717";
    let confirmation_id = "18181818-1818-4181-8181-181818181818";
    let authorization_id = "19191919-1919-4191-8191-191919191919";
    {
        let database = repository_database(&repository).await;
        database
            .execute(
                "INSERT INTO reference_session_sequences
                 (scope_id, session_id, origin, chat_session_id, next_seq,
                  created_at_ms, updated_at_ms)
                 VALUES ('synthetic-retention-scope', '1', 'chat', '1', 3, 1000, 1000)",
                [],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO reference_turns
                 (turn_id, session_id, chat_session_id, session_seq, origin, state,
                  input_hmac, hmac_key_version, completion_code, producer_class,
                  artifact_hmac, created_at_ms, completed_at_ms, open_expires_at_ms)
                 VALUES (?1, '1', '1', 1, 'chat', 'completed', zeroblob(32), 1,
                         'completed', 'resolver_user_input', zeroblob(32), 1000, 1000, 3601000)",
                rusqlite::params![turn_id],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO reference_turns
                 (turn_id, session_id, chat_session_id, session_seq, origin, state,
                  input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
                 VALUES (?1, '1', '1', 2, 'chat', 'open', zeroblob(32), 1, 1000, 3601000)",
                rusqlite::params![open_turn_id],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO conversation_mentions
                 (mention_id, referent_id, turn_id, session_id, entity_kind, text_kind,
                  provenance, producer, visibility, sensitivity, direct_user,
                  untrusted_evidence, public_display_ciphertext,
                  public_normalized_ciphertext, normalized_term_hmac, created_at_ms,
                  expires_at_ms, hmac_key_version, encryption_key_version)
                 VALUES (?1, 'synthetic-referent', ?2, '1', 'product', 'public_visible',
                         'user_authored', 'resolver_user_input', 'provider_safe', 'public',
                         1, 0, X'01', X'02', zeroblob(32), 1000, 1801000, 1, 1)",
                rusqlite::params![mention_id, turn_id],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO reference_confirmations
                 (confirmation_id, session_id, initiating_turn_id, mention_id, referent_id,
                  provider_scope, sensitivity, proposal_ciphertext, normalized_term_hmac,
                  normalization_version, compatibility_epoch, created_at_ms, expires_at_ms,
                  encryption_key_version, hmac_key_version)
                 VALUES (?1, '1', ?2, ?3, 'synthetic-referent', 'web_search_fetch', 'public',
                         X'03', zeroblob(32), 1, 1, 1000, 301000, 1, 1)",
                rusqlite::params![confirmation_id, turn_id, mention_id],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO query_authorizations
                 (authorization_id, session_id, initiating_turn_id, execution_turn_id,
                  referent_id, authorization_method, provider_scope, query_plan_hmac,
                  permit_nonce_hmac, plan_version, hmac_key_version, compatibility_epoch,
                  configuration_epoch, process_epoch, search_budget, fetch_budget,
                  issued_at_ms, expires_at_ms)
                 VALUES (?1, '1', ?2, ?2, 'synthetic-referent', 'current_user',
                         'web_search_fetch', zeroblob(32), zeroblob(32), 1, 1, 1, 1, 1,
                         1, 0, 1000, 301000)",
                rusqlite::params![authorization_id, turn_id],
            )
            .unwrap();
        database
            .execute(
                "INSERT INTO query_authorization_operations
                 (authorization_id, operation_ordinal, operation_hmac, operation_kind,
                  provider, max_attempts)
                 VALUES (?1, 0, zeroblob(32), 'search', 'tavily', 1)",
                rusqlite::params![authorization_id],
            )
            .unwrap();
    }

    let before_five_minutes = repository.prune(300_999).await.unwrap();
    assert_eq!(before_five_minutes.confirmations_expired, 0);
    assert_eq!(before_five_minutes.authorizations_terminalized, 0);
    {
        let database = repository_database(&repository).await;
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM reference_confirmations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM query_authorizations", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );
    }

    let at_five_minutes = repository.prune(301_000).await.unwrap();
    assert_eq!(at_five_minutes.confirmations_expired, 1);
    assert_eq!(at_five_minutes.authorizations_terminalized, 1);
    {
        let database = repository_database(&repository).await;
        let state: String = database
            .query_row(
                "SELECT terminal_state FROM reference_confirmation_tombstones WHERE confirmation_id=?1",
                rusqlite::params![confirmation_id],
                |row| row.get(0),
            )
            .unwrap();
        let replay_state: String = database
            .query_row(
                "SELECT terminal_state FROM query_replay_tombstones WHERE authorization_id=?1",
                rusqlite::params![authorization_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "expired");
        assert_eq!(replay_state, "expired");
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM reference_confirmations", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM query_authorizations", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM conversation_mentions", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );
    }

    let at_mention_expiry = repository.prune(1_801_000).await.unwrap();
    assert_eq!(at_mention_expiry.mentions_removed, 1);
    {
        let database = repository_database(&repository).await;
        assert_eq!(
            database
                .query_row("SELECT COUNT(*) FROM conversation_mentions", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            0
        );
    }

    let before_open_expiry = repository.prune(3_600_999).await.unwrap();
    assert_eq!(before_open_expiry.open_turns_removed, 0);
    let at_open_expiry = repository.prune(3_601_000).await.unwrap();
    assert_eq!(at_open_expiry.open_turns_removed, 1);

    let before_turn_retention = repository.prune(86_400_999).await.unwrap();
    assert_eq!(before_turn_retention.turns_removed, 0);
    let at_turn_retention = repository.prune(86_401_000).await.unwrap();
    assert_eq!(at_turn_retention.turns_removed, 1);

    let before_tombstone_expiry = repository.prune(86_700_999).await.unwrap();
    assert_eq!(before_tombstone_expiry.query_tombstones_removed, 0);
    assert_eq!(before_tombstone_expiry.confirmation_tombstones_removed, 0);
    let at_tombstone_expiry = repository.prune(86_701_000).await.unwrap();
    assert_eq!(at_tombstone_expiry.query_tombstones_removed, 1);
    assert_eq!(at_tombstone_expiry.confirmation_tombstones_removed, 1);
    assert_eq!(at_tombstone_expiry.turns_removed, 0);

    let second_cleanup = repository.prune(86_701_000).await.unwrap();
    assert_eq!(second_cleanup, Default::default());
    let database = repository_database(&repository).await;
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM reference_session_sequences",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert!(
        database
            .query_row(
                "SELECT COUNT(*) FROM reference_turns WHERE turn_id=?1",
                rusqlite::params![turn_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 0
    );
}

async fn repository_database(
    repository: &std::sync::Arc<crate::reference_resolution::repository::SqliteRepository>,
) -> tokio::sync::MutexGuard<'_, Connection> {
    repository.database_for_test().await
}

pub(super) fn database_bytes(connection: &Connection) -> Vec<u8> {
    let mut bytes = Vec::new();
    for table in [
        "reference_turn_staging",
        "conversation_mentions",
        "reference_confirmations",
        "reference_confirmation_tombstones",
    ] {
        let mut statement = connection
            .prepare(&format!("SELECT * FROM {table}"))
            .unwrap();
        let mut rows = statement.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for index in 0..row.as_ref().column_count() {
                if let Ok(value) = row.get_ref(index) {
                    match value {
                        rusqlite::types::ValueRef::Text(value)
                        | rusqlite::types::ValueRef::Blob(value) => bytes.extend_from_slice(value),
                        _ => {}
                    }
                }
            }
        }
    }
    bytes
}

const RESOLVER_TABLES: &[&str] = &[
    "reference_session_sequences",
    "reference_turns",
    "reference_turn_staging",
    "conversation_mentions",
    "mention_derivations",
    "mention_anchors",
    "mention_web_mappings",
    "reference_confirmations",
    "reference_confirmation_tombstones",
    "query_authorizations",
    "query_authorization_operations",
    "provider_attempt_reservations",
    "provider_attempt_reservations_v17",
    "query_replay_tombstones",
    "query_operation_variant_sets_v17",
    "query_operation_variants_v17",
    "reference_confirmation_continuations_v17",
];

fn fresh_connection() -> Connection {
    let mut connection = Connection::open_in_memory().expect("synthetic SQLite connection");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys before migrations");
    crate::embedded::migrations::runner()
        .run(&mut connection)
        .expect("fresh V1 through latest migration");
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("read foreign-key pragma");
    assert_eq!(foreign_keys, 1);
    connection
}

#[test]
fn fresh_migration_creates_the_complete_resolver_schema_without_backfill() {
    let connection = fresh_connection();
    let version: i64 = connection
        .query_row(
            "SELECT MAX(version) FROM refinery_schema_history",
            [],
            |row| row.get(0),
        )
        .expect("read migration ceiling");
    assert_eq!(version, 18);

    let actual: BTreeSet<String> = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'reference_%'
             UNION ALL SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'conversation_%'
             UNION ALL SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'mention_%'
             UNION ALL SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'query_%'
             UNION ALL SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'provider_%'",
        )
        .expect("list resolver tables")
        .query_map([], |row| row.get(0))
        .expect("read resolver table names")
        .collect::<rusqlite::Result<_>>()
        .expect("collect resolver table names");
    assert_eq!(
        actual,
        RESOLVER_TABLES.iter().map(|name| (*name).into()).collect()
    );

    for table in RESOLVER_TABLES {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("count resolver rows");
        assert_eq!(count, 0, "fresh database must not backfill {table}");
    }
}

#[test]
fn v14_upgrade_applies_v15_and_v16_without_resolver_backfill() {
    let mut connection = Connection::open_in_memory().expect("synthetic SQLite connection");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys before migrations");
    crate::embedded::migrations::runner()
        .set_target(refinery::Target::Version(14))
        .run(&mut connection)
        .expect("migrate synthetic database to V14");

    connection
        .execute(
            "INSERT INTO sessions (id, started_at) VALUES (?1, ?2)",
            rusqlite::params![1_i64, 1_724_060_400_i64],
        )
        .expect("insert synthetic historical session");

    crate::embedded::migrations::runner()
        .run(&mut connection)
        .expect("upgrade synthetic V14 database");

    let version: i64 = connection
        .query_row(
            "SELECT MAX(version) FROM refinery_schema_history",
            [],
            |row| row.get(0),
        )
        .expect("read upgraded migration ceiling");
    assert_eq!(version, 18);
    let resolver_rows: i64 = connection
        .query_row(
            "SELECT (SELECT COUNT(*) FROM reference_turns) +
                    (SELECT COUNT(*) FROM conversation_mentions)",
            [],
            |row| row.get(0),
        )
        .expect("count resolver rows after upgrade");
    assert_eq!(resolver_rows, 0);
}

#[test]
fn v15_migration_set_refuses_a_v16_database_without_mutation() {
    let mut connection = fresh_connection();
    let before_history: Vec<(i64, String)> = connection
        .prepare("SELECT version, name FROM refinery_schema_history ORDER BY version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let before_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'reference_%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let current_migrations = crate::embedded::migrations::runner()
        .get_migrations()
        .clone();
    let old_migrations = current_migrations[..15].to_vec();
    let refused = refinery::Runner::new(&old_migrations).run(&mut connection);
    assert!(refused.is_err());
    let after_history: Vec<(i64, String)> = connection
        .prepare("SELECT version, name FROM refinery_schema_history ORDER BY version")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let after_tables: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'reference_%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after_history, before_history);
    assert_eq!(after_tables, before_tables);
}

#[test]
fn migrated_database_reopens_with_foreign_keys_enabled() {
    let file = NamedTempFile::new().expect("temporary SQLite database");
    {
        let mut connection = Connection::open(file.path()).expect("open temporary database");
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable foreign keys");
        crate::embedded::migrations::runner()
            .run(&mut connection)
            .expect("migrate temporary database");
    }

    let connection = Connection::open_with_flags(
        file.path(),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .expect("reopen temporary database");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys after reopen");
    let version: i64 = connection
        .query_row(
            "SELECT MAX(version) FROM refinery_schema_history",
            [],
            |row| row.get(0),
        )
        .expect("read reopened migration ceiling");
    assert_eq!(version, 18);
    let foreign_keys: i64 = connection
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .expect("read reopened foreign-key pragma");
    assert_eq!(foreign_keys, 1);
}

#[test]
fn session_and_automation_parent_deletes_cascade_resolver_rows() {
    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    connection
        .execute(
            "INSERT INTO reference_session_sequences
             (scope_id, session_id, origin, chat_session_id, next_seq,
              created_at_ms, updated_at_ms)
             VALUES ('synthetic-chat-cascade', '1', 'chat', '1', 2, 1000, 1000)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO reference_turns
             (turn_id, session_id, chat_session_id, session_seq, origin, state,
              input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
             VALUES ('28282828-2828-4282-8282-282828282828', '1', '1', 1, 'chat',
                     'open', zeroblob(32), 1, 1000, 3601000)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO reference_turn_staging
             (turn_id, staged_mentions_ciphertext, staged_mentions_hmac,
              descriptor_version, encryption_key_version, hmac_key_version, created_at_ms)
             VALUES ('28282828-2828-4282-8282-282828282828', X'01', zeroblob(32), 1, 1, 1, 1000)",
            [],
        )
        .unwrap();
    connection
        .execute("DELETE FROM sessions WHERE id='1'", [])
        .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM reference_turns", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM reference_session_sequences",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );

    connection
        .execute(
            "INSERT INTO automations
             (id, name, prompt, enabled, timezone, schedule_json, created_at, updated_at)
             VALUES ('synthetic-automation', 'synthetic', 'synthetic', 1, 'UTC', '{}', 'now', 'now')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO automation_runs
             (id, automation_id, scheduled_for, status, created_at)
             VALUES ('synthetic-run', 'synthetic-automation', 'now', 'running', 'now')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO reference_session_sequences
             (scope_id, session_id, origin, automation_id, next_seq,
              created_at_ms, updated_at_ms)
             VALUES ('synthetic-automation-cascade', 'synthetic-run', 'automation',
                     'synthetic-automation', 2, 1000, 1000)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO reference_turns
             (turn_id, session_id, automation_run_id, session_seq, origin, state,
              input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
             VALUES ('29292929-2929-4292-8292-292929292929', 'synthetic-run',
                     'synthetic-run', 1, 'automation', 'open', zeroblob(32), 1, 1000, 3601000)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "DELETE FROM automations WHERE id='synthetic-automation'",
            [],
        )
        .unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM reference_turns", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM reference_session_sequences",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
}

#[test]
fn migration_installs_required_indexes_triggers_and_foreign_keys() {
    let connection = fresh_connection();
    let expected_indexes = [
        "reference_turns_session_state_seq",
        "reference_turns_open_expiry",
        "reference_turns_created",
        "conversation_mentions_session_eligibility",
        "conversation_mentions_referent",
        "conversation_mentions_expiry",
        "conversation_mentions_parent",
        "mention_derivations_parent",
        "mention_derivations_derived",
        "mention_anchors_mention",
        "mention_anchors_turn_ordinal",
        "mention_web_mappings_mention",
        "mention_web_mappings_evidence",
        "mention_web_mappings_source",
        "mention_web_mappings_url",
        "reference_confirmations_session_expiry",
        "reference_confirmations_referent",
        "reference_confirmation_tombstones_retention",
        "query_authorizations_execution_turn",
        "query_authorizations_expiry",
        "query_authorizations_nonce",
        "query_authorization_operations_hmac",
        "query_authorization_operations_alternative",
        "provider_attempt_reservations_operation",
        "query_replay_tombstones_retention",
        "query_replay_tombstones_nonce",
        "reference_confirmation_continuations_session_expiry_v17",
        "reference_confirmation_continuations_plan_v17",
        "query_operation_variants_set_v17",
        "query_operation_variants_hmac_v17",
        "provider_attempt_reservations_v17_authorization",
        "provider_attempt_reservations_v17_operation",
        "provider_attempt_reservations_v17_candidate",
        "provider_attempt_reservations_v17_committed",
        "provider_attempt_reservations_v17_primary_attempt",
        "dynamic_candidate_bindings_v17_authorization",
        "dynamic_candidate_bindings_v17_parent",
        "dynamic_candidate_bindings_v17_binding",
        "dynamic_candidate_bindings_v17_expiry",
    ];
    let expected_triggers = [
        "conversation_mentions_immutable",
        "conversation_mentions_parent_binding",
        "mention_derivations_validate",
        "mention_derivations_immutable",
        "mention_anchors_validate",
        "mention_anchors_immutable",
        "mention_web_mappings_validate",
        "mention_web_mappings_immutable",
        "reference_confirmations_immutable",
        "reference_confirmation_tombstones_immutable",
        "query_authorizations_immutable",
        "query_authorization_operations_validate",
        "query_replay_tombstones_immutable",
        "reference_confirmation_continuations_validate_v17",
        "reference_confirmation_continuations_immutable_v17",
        "query_operation_variant_sets_ready_v17",
        "query_operation_variant_sets_immutable_v17",
        "provider_attempt_reservations_validate_v17",
        "provider_attempt_reservations_immutable_v17",
        "dynamic_candidate_bindings_validate_v17",
        "dynamic_candidate_bindings_immutable_v17",
    ];
    for name in expected_indexes {
        let found: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                rusqlite::params![name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "missing required index {name}");
    }
    for name in expected_triggers {
        let found: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='trigger' AND name=?1",
                rusqlite::params![name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(found, 1, "missing required trigger {name}");
    }
    let foreign_key_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_foreign_key_list('reference_confirmations')
             WHERE \"table\"='conversation_mentions' AND on_delete='RESTRICT'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(foreign_key_count, 1);
}

#[test]
fn schema_rejects_malformed_ids_hmacs_expiries_budgets_and_representations() {
    let connection = fresh_connection();
    insert_synthetic_session(&connection, "1");
    let malformed_turn = connection.execute(
        "INSERT INTO reference_turns
         (turn_id, session_id, chat_session_id, session_seq, origin, state,
          input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
         VALUES ('NOT-A-UUID', '1', '1', 1, 'chat', 'open', zeroblob(32), 1, 1000, 3601000)",
        [],
    );
    assert!(malformed_turn.is_err());

    let malformed_hmac = connection.execute(
        "INSERT INTO reference_turns
         (turn_id, session_id, chat_session_id, session_seq, origin, state,
          input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
         VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', '1', '1', 1, 'chat', 'open', zeroblob(31), 1, 1000, 3601000)",
        [],
    );
    assert!(malformed_hmac.is_err());

    let malformed_expiry = connection.execute(
        "INSERT INTO reference_turns
         (turn_id, session_id, chat_session_id, session_seq, origin, state,
          input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
         VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', '1', '1', 1, 'chat', 'open', zeroblob(32), 1, 1000, 3600001)",
        [],
    );
    assert!(malformed_expiry.is_err());

    let canonical_turn = "30303030-3030-4030-8030-303030303030";
    connection
        .execute(
            "INSERT INTO reference_turns
             (turn_id, session_id, chat_session_id, session_seq, origin, state,
              input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
             VALUES (?1, '1', '1', 1, 'chat', 'open', zeroblob(32), 1, 1000, 3601000)",
            rusqlite::params![canonical_turn],
        )
        .unwrap();
    let malformed_grouping = connection.execute(
        "INSERT INTO reference_turns
         (turn_id, session_id, chat_session_id, session_seq, origin, state,
          input_hmac, hmac_key_version, created_at_ms, open_expires_at_ms)
         VALUES ('aaaaaaa--aaa-4aaa-8aaa-aaaaaaaaaaaa', '1', '1', 2, 'chat',
                 'open', zeroblob(32), 1, 1000, 3601000)",
        [],
    );
    assert!(malformed_grouping.is_err());

    let invalid_budget = connection.execute(
        "INSERT INTO query_authorizations
         (authorization_id, session_id, initiating_turn_id, execution_turn_id,
          referent_id, authorization_method, provider_scope, query_plan_hmac,
          permit_nonce_hmac, plan_version, hmac_key_version, compatibility_epoch,
          configuration_epoch, process_epoch, search_budget, fetch_budget,
          issued_at_ms, expires_at_ms)
         VALUES ('31313131-3131-4131-8131-313131313131', '1', ?1, ?1,
                 'synthetic-referent', 'current_user', 'web_search_fetch',
                 zeroblob(32), zeroblob(32), 1, 1, 1, 1, 1, 3, 0, 1000, 301000)",
        rusqlite::params![canonical_turn],
    );
    assert!(invalid_budget.is_err());

    let invalid_representation = connection.execute(
        "INSERT INTO conversation_mentions
         (mention_id, referent_id, turn_id, session_id, entity_kind, text_kind,
          provenance, producer, visibility, sensitivity, direct_user,
          untrusted_evidence, created_at_ms, expires_at_ms, hmac_key_version)
         VALUES ('32323232-3232-4232-8232-323232323232', 'synthetic-referent', ?1,
                 '1', 'product', 'public_visible', 'user_authored',
                 'resolver_user_input', 'provider_safe', 'public', 1, 0, 1000, 1801000, 1)",
        rusqlite::params![canonical_turn],
    );
    assert!(invalid_representation.is_err());

    for (mention_id, fingerprint) in [
        ("33333333-3333-4333-8333-333333333333", 0x33_u8),
        ("34343434-3434-4343-8343-343434343434", 0x34_u8),
    ] {
        connection
            .execute(
                "INSERT INTO conversation_mentions
                 (mention_id, referent_id, turn_id, session_id, entity_kind, text_kind,
                  provenance, producer, visibility, sensitivity, direct_user,
                  untrusted_evidence, opaque_fingerprint, created_at_ms, expires_at_ms,
                  hmac_key_version)
                 VALUES (?1, 'synthetic-referent', ?2, '1', 'product', 'opaque',
                         'unknown', 'legacy_assistant', 'local_only', 'unknown', 0, 1,
                         ?3, 1000, 1801000, 1)",
                rusqlite::params![mention_id, canonical_turn, vec![fingerprint; 32]],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO mention_derivations
             (derivation_id, derived_mention_id, parent_mention_id,
              derivation_kind, parent_ordinal, created_at_ms)
             VALUES ('35353535-3535-4353-8353-353535353535',
                     '34343434-3434-4343-8343-343434343434',
                     '33333333-3333-4333-8333-333333333333',
                     'exact_structured_repeat_of', 0, 1000)",
            [],
        )
        .unwrap();
    let cycle = connection.execute(
        "INSERT INTO mention_derivations
         (derivation_id, derived_mention_id, parent_mention_id,
          derivation_kind, parent_ordinal, created_at_ms)
         VALUES ('36363636-3636-4363-8363-363636363636',
                 '33333333-3333-4333-8333-333333333333',
                 '34343434-3434-4343-8343-343434343434',
                 'exact_structured_repeat_of', 0, 1000)",
        [],
    );
    assert!(cycle.is_err());

    connection
        .execute(
            "INSERT INTO reference_confirmations
             (confirmation_id, session_id, initiating_turn_id, mention_id,
              referent_id, provider_scope, sensitivity, proposal_ciphertext,
              normalized_term_hmac, normalization_version, compatibility_epoch,
              created_at_ms, expires_at_ms, encryption_key_version, hmac_key_version)
             VALUES ('37373737-3737-4373-8373-373737373737', '1', ?1, ?2,
                     'synthetic-referent', 'web_search_fetch', 'public', X'01',
                     zeroblob(32), 1, 1, 1000, 301000, 1, 1)",
            rusqlite::params![canonical_turn, "33333333-3333-4333-8333-333333333333"],
        )
        .unwrap();
    assert!(connection
        .execute(
            "DELETE FROM conversation_mentions
             WHERE mention_id='33333333-3333-4333-8333-333333333333'",
            [],
        )
        .is_err());
}
