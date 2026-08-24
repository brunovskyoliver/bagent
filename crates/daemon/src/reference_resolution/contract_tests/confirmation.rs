use rusqlite::{Connection, OpenFlags};
use tempfile::NamedTempFile;

use super::persistence::database_bytes;

fn fresh_connection() -> Connection {
    let mut connection = Connection::open_with_flags(
        ":memory:",
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .expect("open synthetic confirmation database");
    connection
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys before migrations");
    crate::embedded::migrations::runner()
        .run(&mut connection)
        .expect("apply synthetic confirmation migrations");
    connection
}

fn insert_session(connection: &Connection) {
    connection
        .execute(
            "INSERT INTO sessions (id, started_at) VALUES ('1', '2026-08-19T10:00:00Z')",
            [],
        )
        .unwrap();
}

fn repository(
    connection: Connection,
) -> std::sync::Arc<crate::reference_resolution::repository::SqliteRepository> {
    use crate::reference_resolution::crypto::{CryptoCustody, FakeKeyProvider};
    use crate::reference_resolution::repository::SqliteRepository;

    std::sync::Arc::new(SqliteRepository::new(
        std::sync::Arc::new(tokio::sync::Mutex::new(connection)),
        std::sync::Arc::new(CryptoCustody::with_provider(FakeKeyProvider::deterministic())),
    ))
}

async fn seed_pending(
    repository: &std::sync::Arc<crate::reference_resolution::repository::SqliteRepository>,
    confirmation_id: &str,
    initiating_turn_id: &str,
) {
    use crate::reference_resolution::crypto::{AadBinding, CryptoCustody, FakeKeyProvider};
    use crate::reference_resolution::repository::{
        PendingConfirmationIssue, ReferenceRepository, ResolveLedgerCommand, ResolutionDecision,
    };

    let database = repository.database_for_test().await;
    insert_session(&database);
    drop(database);
    let proposal = "Synthetic Public Term";
    let custody = CryptoCustody::with_provider(FakeKeyProvider::deterministic());
    let normalized = crate::reference_resolution::normalize_term(proposal);
    let issue = PendingConfirmationIssue {
        confirmation_id: confirmation_id.into(),
        mention_id: None,
        referent_id: "synthetic-referent".into(),
        provider_scope: "web_search_fetch",
        sensitivity: "public",
        proposal: Some(proposal.into()),
        normalized,
        normalization_version: 1,
        compatibility_epoch: 1,
    };
    repository
        .transact_resolution(
            ResolveLedgerCommand {
                turn_id: initiating_turn_id.into(),
                session_id: "1".into(),
                scope_id: "synthetic-confirmation-fixture".into(),
                chat_session_id: Some("1".into()),
                automation_id: None,
                automation_run_id: None,
                origin: "chat",
                input: b"synthetic initiating request".to_vec(),
                descriptors: vec![],
                now_ms: 1_000,
            },
            Box::new(move |_| Ok(ResolutionDecision::IssuePendingConfirmation(issue))),
        )
        .await
        .expect("issue synthetic pending confirmation");

    // Keep the producer-side custody in this fixture visibly independent from
    // the repository implementation; the proposal is never inserted as text.
    let binding = AadBinding::new("unused", "unused", "unused");
    let _ = custody.hmac(&binding, 1, b"fixture-control");
    let database = repository.database_for_test().await;
    assert_eq!(
        database
            .query_row(
                "SELECT expires_at_ms FROM reference_confirmations
                 WHERE confirmation_id=?1",
                rusqlite::params![confirmation_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        301_000
    );
    assert!(!database_bytes(&database)
        .windows(proposal.len())
        .any(|window| window == proposal.as_bytes()));
}

fn request_command(
    turn_id: &str,
    input: &[u8],
    now_ms: i64,
) -> crate::reference_resolution::repository::ResolveLedgerCommand {
    crate::reference_resolution::repository::ResolveLedgerCommand {
        turn_id: turn_id.into(),
        session_id: "1".into(),
        scope_id: "synthetic-confirmation-fixture".into(),
        chat_session_id: Some("1".into()),
        automation_id: None,
        automation_run_id: None,
        origin: "chat",
        input: input.to_vec(),
        descriptors: vec![],
        now_ms,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn confirmation_edit_replacement_public() {
    use crate::reference_resolution::confirmation::{
        terminal_event_sequence, ConfirmationDisposition,
    };
    use crate::reference_resolution::confirmation::ConfirmationRequestKind;
    use crate::reference_resolution::repository::{
        ConfirmationRequest, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    seed_pending(
        &repository,
        "31313131-3131-4313-8313-313131313131",
        "32323232-3232-4323-8323-323232323232",
    )
    .await;
    let replacement = b"Look up Aster Nova 12 online.";
    let execution_turn_id = "33333333-3333-4333-8333-333333333333";
    let snapshot = repository
        .transact_resolution(
            request_command(execution_turn_id, replacement, 2_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "31313131-3131-4313-8313-313131313131".into(),
                        session_id: "1".into(),
                        execution_turn_id: execution_turn_id.into(),
                        kind: ConfirmationRequestKind::Edit,
                        submitted: replacement.to_vec(),
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert!(matches!(
        snapshot.confirmation,
        ConfirmationDisposition::EditAccepted { replacement: ref bytes }
            if bytes.as_slice() == replacement
    ));
    assert_eq!(
        terminal_event_sequence(&snapshot.confirmation),
        vec!["reference_resolution(edit_accepted,proceeding)"]
    );
    let database = repository.database_for_test().await;
    assert_eq!(database.query_row("SELECT COUNT(*) FROM reference_confirmations", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    let (state, bound_turn): (String, Option<String>) = database
        .query_row(
            "SELECT terminal_state, execution_turn_id
             FROM reference_confirmation_tombstones WHERE confirmation_id='31313131-3131-4313-8313-313131313131'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "edited");
    assert_eq!(bound_turn.as_deref(), Some(execution_turn_id));
    assert_eq!(database.query_row("SELECT COUNT(*) FROM conversation_mentions", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn confirmation_edit_replacement_blocked() {
    use crate::reference_resolution::confirmation::{
        terminal_event_sequence, ConfirmationDisposition,
    };
    use crate::reference_resolution::confirmation::ConfirmationRequestKind;
    use crate::reference_resolution::repository::{
        ConfirmationRequest, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    seed_pending(
        &repository,
        "34343434-3434-4343-8343-343434343434",
        "35353535-3535-4353-8353-353535353535",
    )
    .await;
    let replacement = b"Look up that private account online.";
    let execution_turn_id = "36363636-3636-4363-8363-363636363636";
    let snapshot = repository
        .transact_resolution(
            request_command(execution_turn_id, replacement, 2_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "34343434-3434-4343-8343-343434343434".into(),
                        session_id: "1".into(),
                        execution_turn_id: execution_turn_id.into(),
                        kind: ConfirmationRequestKind::Edit,
                        submitted: replacement.to_vec(),
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert!(matches!(
        snapshot.confirmation,
        ConfirmationDisposition::EditAccepted { replacement: ref bytes }
            if bytes.as_slice() == replacement
    ));
    assert_eq!(
        terminal_event_sequence(&snapshot.confirmation),
        vec!["reference_resolution(edit_accepted,proceeding)"]
    );
    let database = repository.database_for_test().await;
    assert_eq!(database.query_row("SELECT COUNT(*) FROM conversation_mentions", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert!(!database_bytes(&database).windows(b"SYNTHETIC_PRIVATE_REPLACEMENT".len()).any(|window| window == b"SYNTHETIC_PRIVATE_REPLACEMENT"));
}

#[tokio::test(flavor = "current_thread")]
async fn confirmation_invalidation_failure() {
    use crate::reference_resolution::confirmation::{
        terminal_event_sequence, ConfirmationDisposition,
    };
    use crate::reference_resolution::repository::{
        ConfirmationAction, ConfirmationConsumption, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    seed_pending(
        &repository,
        "37373737-3737-4373-8373-373737373737",
        "38383838-3838-4383-8383-383838383838",
    )
    .await;
    let execution_turn_id = "39393939-3939-4393-8393-393939393939";
    let snapshot = repository
        .transact_resolution(
            request_command(execution_turn_id, b"SYNTHETIC_REPLACEMENT_SENTINEL_4D4D", 2_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmation(
                    ConfirmationConsumption {
                        confirmation_id: "37373737-3737-4373-8373-373737373737".into(),
                        session_id: "1".into(),
                        initiating_turn_id: "38383838-3838-4383-8383-383838383838".into(),
                        referent_id: "synthetic-referent".into(),
                        provider_scope: "web_search_fetch",
                        sensitivity: "public",
                        normalization_version: 1,
                        compatibility_epoch: 1,
                        execution_turn_id: execution_turn_id.into(),
                        action: ConfirmationAction::Invalidate,
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert!(matches!(
        snapshot.confirmation,
        ConfirmationDisposition::BlockedInvalidationFailure
    ));
    assert_eq!(
        terminal_event_sequence(&snapshot.confirmation),
        vec!["reference_resolution(invalidation_failed,blocked)", "done"]
    );
    let database = repository.database_for_test().await;
    assert_eq!(database.query_row("SELECT terminal_state FROM reference_confirmation_tombstones", [], |row| row.get::<_, String>(0)).unwrap(), "invalidated");
    assert_eq!(database.query_row("SELECT COUNT(*) FROM conversation_mentions", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn confirmation_term_mismatch() {
    use crate::reference_resolution::confirmation::{
        terminal_event_sequence, ConfirmationDisposition,
    };
    use crate::reference_resolution::confirmation::ConfirmationRequestKind;
    use crate::reference_resolution::repository::{
        ConfirmationRequest, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    seed_pending(
        &repository,
        "40404040-4040-4404-8404-404040404040",
        "41414141-4141-4414-8414-414141414141",
    )
    .await;
    let execution_turn_id = "42424242-4242-4424-8424-424242424242";
    let snapshot = repository
        .transact_resolution(
            request_command(execution_turn_id, b"Different term", 2_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "40404040-4040-4404-8404-404040404040".into(),
                        session_id: "1".into(),
                        execution_turn_id: execution_turn_id.into(),
                        kind: ConfirmationRequestKind::Confirm,
                        submitted: b"Different term".to_vec(),
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.confirmation, ConfirmationDisposition::BlockedTermMismatch);
    assert_eq!(
        terminal_event_sequence(&snapshot.confirmation),
        vec!["reference_resolution(term_mismatch,blocked)", "done"]
    );
    let database = repository.database_for_test().await;
    assert_eq!(database.query_row("SELECT terminal_state FROM reference_confirmation_tombstones", [], |row| row.get::<_, String>(0)).unwrap(), "term_mismatch");
    assert_eq!(database.query_row("SELECT COUNT(*) FROM query_authorizations", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(database.query_row("SELECT COUNT(*) FROM conversation_mentions", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn confirmation_term_mismatch_replay() {
    use crate::reference_resolution::confirmation::{
        terminal_event_sequence, ConfirmationDisposition,
    };
    use crate::reference_resolution::confirmation::ConfirmationRequestKind;
    use crate::reference_resolution::repository::{
        ConfirmationRequest, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    seed_pending(
        &repository,
        "43434343-4343-4434-8434-434343434343",
        "44444444-4444-4444-8444-444444444444",
    )
    .await;
    let first_turn = "45454545-4545-4454-8454-454545454545";
    let first = repository
        .transact_resolution(
            request_command(first_turn, b"Different term", 2_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "43434343-4343-4434-8434-434343434343".into(),
                        session_id: "1".into(),
                        execution_turn_id: first_turn.into(),
                        kind: ConfirmationRequestKind::Confirm,
                        submitted: b"Different term".to_vec(),
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert_eq!(first.confirmation, ConfirmationDisposition::BlockedTermMismatch);
    let replay_turn = "46464646-4646-4464-8464-464646464646";
    let replay = repository
        .transact_resolution(
            request_command(replay_turn, b"SYNTHETIC_MISMATCH_REPLAY_BYTES_5E5E", 2_001),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "43434343-4343-4434-8434-434343434343".into(),
                        session_id: "1".into(),
                        execution_turn_id: replay_turn.into(),
                        kind: ConfirmationRequestKind::Confirm,
                        submitted: b"SYNTHETIC_MISMATCH_REPLAY_BYTES_5E5E".to_vec(),
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert_eq!(replay.confirmation, ConfirmationDisposition::BlockedAlreadyConsumed);
    assert_eq!(
        terminal_event_sequence(&replay.confirmation),
        vec!["reference_resolution(already_consumed,blocked)", "done"]
    );
    let database = repository.database_for_test().await;
    assert_eq!(database.query_row("SELECT COUNT(*) FROM reference_confirmations", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert!(!database_bytes(&database).windows(b"SYNTHETIC_MISMATCH_REPLAY_BYTES_5E5E".len()).any(|window| window == b"SYNTHETIC_MISMATCH_REPLAY_BYTES_5E5E"));
}

#[tokio::test(flavor = "current_thread")]
async fn exact_confirmation_is_read_only_and_one_use() {
    use crate::reference_resolution::confirmation::{
        ConfirmationDisposition, ConfirmationRequestKind,
    };
    use crate::reference_resolution::repository::{
        ConfirmationRequest, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    seed_pending(
        &repository,
        "47474747-4747-4474-8474-474747474747",
        "48484848-4848-4484-8484-484848484848",
    )
    .await;
    let execution_turn_id = "49494949-4949-4494-8494-494949494949";
    let snapshot = repository
        .transact_resolution(
            request_command(execution_turn_id, b"  synthetic PUBLIC term  ", 2_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "47474747-4747-4474-8474-474747474747".into(),
                        session_id: "1".into(),
                        execution_turn_id: execution_turn_id.into(),
                        kind: ConfirmationRequestKind::Confirm,
                        submitted: b"  synthetic PUBLIC term  ".to_vec(),
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert!(matches!(
        snapshot.confirmation,
        ConfirmationDisposition::Confirmed {
            referent_id,
            mention_id: None,
            provider_scope,
        } if referent_id == "synthetic-referent" && provider_scope == "web_search_fetch"
    ));
    let database = repository.database_for_test().await;
    assert_eq!(database.query_row("SELECT terminal_state FROM reference_confirmation_tombstones", [], |row| row.get::<_, String>(0)).unwrap(), "consumed");
    assert_eq!(database.query_row("SELECT COUNT(*) FROM query_authorizations", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn expired_confirmation_deletes_pending_ciphertext_without_authority() {
    use crate::reference_resolution::confirmation::{
        ConfirmationDisposition, ConfirmationRequestKind,
    };
    use crate::reference_resolution::repository::{
        ConfirmationRequest, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    seed_pending(
        &repository,
        "50505050-5050-4505-8505-505050505050",
        "51515151-5151-4515-8515-515151515151",
    )
    .await;
    let execution_turn_id = "52525252-5252-4525-8525-525252525252";
    let snapshot = repository
        .transact_resolution(
            request_command(execution_turn_id, b"Synthetic Public Term", 301_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "50505050-5050-4505-8505-505050505050".into(),
                        session_id: "1".into(),
                        execution_turn_id: execution_turn_id.into(),
                        kind: ConfirmationRequestKind::Confirm,
                        submitted: b"Synthetic Public Term".to_vec(),
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.confirmation, ConfirmationDisposition::BlockedExpired);
    let database = repository.database_for_test().await;
    assert_eq!(database.query_row("SELECT COUNT(*) FROM reference_confirmations", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert_eq!(database.query_row("SELECT terminal_state FROM reference_confirmation_tombstones", [], |row| row.get::<_, String>(0)).unwrap(), "expired");
    assert_eq!(database.query_row("SELECT COUNT(*) FROM query_authorizations", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn two_exact_confirmation_consumers_have_one_winner() {
    use crate::reference_resolution::confirmation::{
        ConfirmationDisposition, ConfirmationRequestKind,
    };
    use crate::reference_resolution::repository::{
        ConfirmationRequest, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    seed_pending(
        &repository,
        "53535353-5353-4535-8535-535353535353",
        "54545454-5454-4545-8545-545454545454",
    )
    .await;
    let make_request = |turn_id: &'static str| {
        let input = b"Synthetic Public Term".to_vec();
        repository.transact_resolution(
            request_command(turn_id, &input, 2_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "53535353-5353-4535-8535-535353535353".into(),
                        session_id: "1".into(),
                        execution_turn_id: turn_id.into(),
                        kind: ConfirmationRequestKind::Confirm,
                        submitted: input,
                    },
                ))
            }),
        )
    };
    let (left, right) = tokio::join!(
        make_request("55555555-5555-4555-8555-555555555555"),
        make_request("56565656-5656-4565-8565-565656565656"),
    );
    let results = [left, right];
    let winners = results
        .iter()
        .filter(|result| {
            matches!(
                result,
                Ok(snapshot) if matches!(&snapshot.confirmation, ConfirmationDisposition::Confirmed { .. })
            )
        })
        .count();
    assert_eq!(winners, 1);
    let replay_denials = results
        .iter()
        .filter(|result| {
            matches!(
                result,
                Ok(snapshot) if snapshot.confirmation == ConfirmationDisposition::BlockedAlreadyConsumed
            )
        })
        .count();
    assert_eq!(replay_denials, 1);
    let database = repository.database_for_test().await;
    assert_eq!(
        database
            .query_row(
                "SELECT COUNT(*) FROM reference_confirmation_tombstones
                 WHERE terminal_state='consumed'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[tokio::test(flavor = "current_thread")]
async fn pending_confirmation_requires_an_exact_safe_public_projection() {
    use crate::reference_resolution::repository::{
        PendingConfirmationIssue, ReferenceRepository, ResolutionDecision,
    };

    let repository = repository(fresh_connection());
    {
        let database = repository.database_for_test().await;
        insert_session(&database);
    }
    let snapshot = repository
        .transact_resolution(
            request_command(
                "57575757-5757-4575-8575-575757575757",
                b"ambiguous private source",
                1_000,
            ),
            Box::new(|_| {
                Ok(ResolutionDecision::IssuePendingConfirmation(
                    PendingConfirmationIssue {
                        confirmation_id: "58585858-5858-4585-8585-585858585858".into(),
                        mention_id: None,
                        referent_id: "synthetic-referent".into(),
                        provider_scope: "web_search_fetch",
                        sensitivity: "public",
                        proposal: None,
                        normalized: Vec::new(),
                        normalization_version: 1,
                        compatibility_epoch: 1,
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.confirmation, crate::reference_resolution::ConfirmationDisposition::Unchanged);
    let database = repository.database_for_test().await;
    assert_eq!(database.query_row("SELECT COUNT(*) FROM reference_confirmations", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn daemon_reopen_preserves_pending_confirmation_until_one_use() {
    use crate::reference_resolution::confirmation::{
        ConfirmationDisposition, ConfirmationRequestKind,
    };
    use crate::reference_resolution::repository::{
        ConfirmationRequest, ReferenceRepository, ResolutionDecision,
    };

    let file = NamedTempFile::new().unwrap();
    let path = file.path().to_owned();
    let first_repository = repository({
        let mut connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .unwrap();
        crate::embedded::migrations::runner().run(&mut connection).unwrap();
        connection
    });
    seed_pending(
        &first_repository,
        "59595959-5959-4595-8595-595959595959",
        "60606060-6060-4606-8606-606060606060",
    )
    .await;
    drop(first_repository);

    let reopened = repository({
        let mut connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .unwrap();
        crate::embedded::migrations::runner().run(&mut connection).unwrap();
        connection
    });
    let execution_turn_id = "61616161-6161-4616-8616-616161616161";
    let snapshot = reopened
        .transact_resolution(
            request_command(execution_turn_id, b"Synthetic Public Term", 2_000),
            Box::new(move |_| {
                Ok(ResolutionDecision::ConsumeConfirmationRequest(
                    ConfirmationRequest {
                        confirmation_id: "59595959-5959-4595-8595-595959595959".into(),
                        session_id: "1".into(),
                        execution_turn_id: execution_turn_id.into(),
                        kind: ConfirmationRequestKind::Confirm,
                        submitted: b"Synthetic Public Term".to_vec(),
                    },
                ))
            }),
        )
        .await
        .unwrap();
    assert!(matches!(snapshot.confirmation, ConfirmationDisposition::Confirmed { .. }));
}

#[test]
fn confirmation_structural_values_redact_all_user_bytes() {
    use crate::reference_resolution::confirmation::{
        terminal_event_sequence, ConfirmationDisposition,
    };

    let proposal = "SYNTHETIC_PROPOSAL_TEXT_7A7A";
    let replacement = b"SYNTHETIC_REPLACEMENT_TEXT_8B8B".to_vec();
    let mismatch = "SYNTHETIC_MISMATCH_TEXT_9C9C";
    let rendered = format!(
        "{:?} {:?} {:?}",
        ConfirmationDisposition::Pending {
            confirmation_id: "69696969-6969-4696-8696-696969696969".into(),
            proposal: proposal.into(),
            expires_at_ms: 301_000,
        },
        ConfirmationDisposition::EditAccepted { replacement },
        ConfirmationDisposition::BlockedTermMismatch,
    );
    assert!(!rendered.contains(proposal));
    assert!(!rendered.contains(mismatch));
    assert_eq!(
        terminal_event_sequence(&ConfirmationDisposition::BlockedTermMismatch),
        vec!["reference_resolution(term_mismatch,blocked)", "done"]
    );
}
