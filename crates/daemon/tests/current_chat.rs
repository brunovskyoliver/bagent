use bagentd::current_chat::{
    begin_conversation_turn, capture_recovered_approval_presentations, clear_current_chat,
    complete_conversation_turn, complete_conversation_turn_with_artifacts, initialize_schema,
    interrupt_conversation_turn_with_work, open_or_create_current_chat, read_current_chat,
    recover_after_daemon_restart, save_draft, upsert_connector_reference, ClearCurrentChatCommand,
    ConversationTurnState, CurrentChatFailurePoint, SubmittedAttachmentMetadata,
    ValidatedSourceMetadata, MAX_COMPLETED_TURNS, MAX_DRAFT_BYTES, MAX_RETAINED_CONTENT_BYTES,
};
use chrono::Duration;
use chrono::{TimeZone, Utc};
use rusqlite::Connection;

#[test]
fn clear_atomicity() {
    let connection = Connection::open_in_memory().unwrap();
    initialize_schema(&connection).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE memory_items (id TEXT PRIMARY KEY, text TEXT NOT NULL);
             CREATE TABLE automation_sessions (
                automation_session_identity TEXT PRIMARY KEY,
                outcome TEXT NOT NULL
             );
             CREATE TABLE automation_session_attention (
                automation_session_identity TEXT PRIMARY KEY,
                attention_state TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS automation_continuation_provenance (
                identity TEXT PRIMARY KEY,
                source_automation_session_identity TEXT NOT NULL,
                target_current_chat_identity TEXT NOT NULL UNIQUE,
                command_identity TEXT NOT NULL UNIQUE,
                seed TEXT NOT NULL,
                seed_bytes INTEGER NOT NULL,
                source_deleted INTEGER NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE TABLE works (
                identity TEXT PRIMARY KEY,
                origin_kind TEXT NOT NULL,
                origin_primary_identity TEXT NOT NULL,
                state TEXT NOT NULL
             );
             CREATE TABLE work_approvals (
                identity TEXT PRIMARY KEY,
                work_identity TEXT NOT NULL,
                category TEXT NOT NULL,
                state TEXT NOT NULL
             );",
        )
        .unwrap();
    let original_empty = open_or_create_current_chat(&connection).unwrap();
    let edited_at = Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap();
    let drafted = save_draft(
        &connection,
        &original_empty.identity,
        original_empty.revision,
        "draft text",
        &["pending-attachment".to_owned()],
        edited_at,
    )
    .unwrap();
    let begun = begin_conversation_turn(
        &connection,
        &drafted.identity,
        drafted.revision,
        "exact user message",
        &[SubmittedAttachmentMetadata {
            identity: "attachment-a".to_owned(),
            filename: "safe.txt".to_owned(),
            mime: "text/plain".to_owned(),
            size_bytes: 4,
            available: true,
        }],
        edited_at,
    )
    .unwrap();
    connection
        .execute(
            "INSERT INTO works VALUES ('conversation-work', 'conversation', ?1, 'running')",
            [&drafted.identity],
        )
        .unwrap();
    connection.execute(
        "INSERT INTO work_approvals VALUES ('approval-a', 'conversation-work', 'mail_write', 'allowed')",
        [],
    ).unwrap();
    let mut original = complete_conversation_turn_with_artifacts(
        &connection,
        &drafted.identity,
        &begun.identity,
        "exact assistant output",
        edited_at,
        Some("conversation-work"),
        &[ValidatedSourceMetadata {
            identity: "source-a".to_owned(),
            title: "Source".to_owned(),
            domain: "example.com".to_owned(),
        }],
    )
    .unwrap();
    connection
        .execute(
            "UPDATE works SET state='completed' WHERE identity='conversation-work'",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO memory_items (id, text) VALUES ('memory-a', 'retained memory')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO automation_sessions VALUES ('automation-session-a', 'completed')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO automation_session_attention VALUES ('automation-session-a', 'viewed')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO automation_continuation_provenance VALUES
             ('provenance-a', 'automation-session-a', ?1, 'continue-a', 'seed', 4, 0, ?2)",
            rusqlite::params![original.identity, edited_at.to_rfc3339()],
        )
        .unwrap();
    upsert_connector_reference(
        &connection,
        &original.identity,
        "reference-a",
        "mail",
        "{\"rowid\":1}",
        edited_at,
    )
    .unwrap();
    original = read_current_chat(&connection).unwrap();
    assert_eq!(original.validated_sources.len(), 1);
    assert_eq!(original.connector_references.len(), 1);
    assert_eq!(original.completed_approval_presentations.len(), 1);

    connection.execute(
        "INSERT INTO works VALUES ('automation-work', 'automation', 'automation-run', 'waiting_for_approval')",
        [],
    ).unwrap();
    connection.execute(
        "INSERT INTO work_approvals VALUES ('approval-automation', 'automation-work', 'filesystem_write', 'pending')",
        [],
    ).unwrap();
    let active_automation_approval = clear_current_chat(
        &connection,
        ClearCurrentChatCommand {
            current_chat_identity: original.identity.clone(),
            expected_revision: original.revision,
            command_identity: "clear-blocked-by-automation-approval".to_owned(),
            confirmed_non_empty: true,
        },
        None,
    )
    .unwrap_err();
    assert!(active_automation_approval
        .to_string()
        .contains("pending approval"));
    connection
        .execute(
            "DELETE FROM work_approvals WHERE identity='approval-automation'",
            [],
        )
        .unwrap();
    connection
        .execute("DELETE FROM works WHERE identity='automation-work'", [])
        .unwrap();

    let needs_confirmation = clear_current_chat(
        &connection,
        ClearCurrentChatCommand {
            current_chat_identity: original.identity.clone(),
            expected_revision: original.revision,
            command_identity: "clear-needs-confirmation".to_owned(),
            confirmed_non_empty: false,
        },
        None,
    )
    .unwrap_err();
    assert!(needs_confirmation
        .to_string()
        .contains("requires confirmation"));

    let replacement = clear_current_chat(
        &connection,
        ClearCurrentChatCommand {
            current_chat_identity: original.identity.clone(),
            expected_revision: original.revision,
            command_identity: "clear-command-a".to_owned(),
            confirmed_non_empty: true,
        },
        None,
    )
    .unwrap();

    assert_ne!(replacement.identity, original.identity);
    assert_eq!(replacement.revision, 1);
    assert!(replacement.turns.is_empty());
    assert!(replacement.draft.is_none());
    assert_eq!(
        open_or_create_current_chat(&connection).unwrap().identity,
        replacement.identity
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM memory_items", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM automation_sessions", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT attention_state FROM automation_session_attention",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "viewed"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM automation_continuation_provenance",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );

    let retry = clear_current_chat(
        &connection,
        ClearCurrentChatCommand {
            current_chat_identity: original.identity.clone(),
            expected_revision: original.revision,
            command_identity: "clear-command-a".to_owned(),
            confirmed_non_empty: true,
        },
        None,
    )
    .unwrap();
    assert_eq!(retry.identity, replacement.identity);

    let concurrent_loser = clear_current_chat(
        &connection,
        ClearCurrentChatCommand {
            current_chat_identity: original.identity.clone(),
            expected_revision: original.revision,
            command_identity: "concurrent-loser".to_owned(),
            confirmed_non_empty: true,
        },
        None,
    )
    .unwrap_err();
    assert!(concurrent_loser
        .to_string()
        .contains("stale Current Chat identity"));

    for failure_point in [
        CurrentChatFailurePoint::AfterReplacementInsert,
        CurrentChatFailurePoint::AfterScopedContentDelete,
        CurrentChatFailurePoint::AfterAuthoritySwap,
        CurrentChatFailurePoint::BeforeCommit,
    ] {
        let failed_connection = Connection::open_in_memory().unwrap();
        initialize_schema(&failed_connection).unwrap();
        let before = open_or_create_current_chat(&failed_connection).unwrap();
        let error = clear_current_chat(
            &failed_connection,
            ClearCurrentChatCommand {
                current_chat_identity: before.identity.clone(),
                expected_revision: before.revision,
                command_identity: format!("failure-{failure_point:?}"),
                confirmed_non_empty: false,
            },
            Some(failure_point),
        )
        .unwrap_err();
        assert!(error.to_string().contains("injected failure"));
        assert_eq!(
            open_or_create_current_chat(&failed_connection).unwrap(),
            before
        );
        assert_eq!(
            failed_connection
                .query_row(
                    "SELECT COUNT(*) FROM current_chat_clear_commands",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }

    let uppercase_connection = Connection::open_in_memory().unwrap();
    initialize_schema(&uppercase_connection).unwrap();
    let uppercase = open_or_create_current_chat(&uppercase_connection).unwrap();
    let uppercase = save_draft(
        &uppercase_connection,
        &uppercase.identity,
        uppercase.revision,
        "/CLEAR",
        &[],
        edited_at,
    )
    .unwrap();
    clear_current_chat(
        &uppercase_connection,
        ClearCurrentChatCommand {
            current_chat_identity: uppercase.identity,
            expected_revision: uppercase.revision,
            command_identity: "uppercase-clear".to_owned(),
            confirmed_non_empty: false,
        },
        None,
    )
    .unwrap();
}

#[test]
fn restoration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("current-chat.sqlite");
    let edited_at = Utc::now();

    let connection = Connection::open(&path).unwrap();
    initialize_schema(&connection).unwrap();
    connection
        .execute("CREATE TABLE attachments (id TEXT PRIMARY KEY)", [])
        .unwrap();
    connection
        .execute("INSERT INTO attachments VALUES ('restart-attachment')", [])
        .unwrap();
    let initial = open_or_create_current_chat(&connection).unwrap();
    let drafted = save_draft(
        &connection,
        &initial.identity,
        initial.revision,
        "durable exact draft",
        &["pending-a".to_owned()],
        edited_at,
    )
    .unwrap();
    drop(connection);

    let restarted = Connection::open(&path).unwrap();
    initialize_schema(&restarted).unwrap();
    let reopened = open_or_create_current_chat(&restarted).unwrap();
    assert_eq!(reopened.identity, initial.identity);
    assert_eq!(reopened.revision, drafted.revision);
    assert_eq!(reopened.draft.as_ref().unwrap().text, "durable exact draft");
    assert_eq!(
        reopened
            .draft
            .as_ref()
            .unwrap()
            .pending_attachment_references,
        ["pending-a"]
    );

    let active = begin_conversation_turn(
        &restarted,
        &reopened.identity,
        reopened.revision,
        "submitted user message survives",
        &[SubmittedAttachmentMetadata {
            identity: "restart-attachment".to_owned(),
            filename: "restart.txt".to_owned(),
            mime: "text/plain".to_owned(),
            size_bytes: 7,
            available: true,
        }],
        edited_at,
    )
    .unwrap();
    upsert_connector_reference(
        &restarted,
        &reopened.identity,
        "durable-reference",
        "mail",
        "{\"rowid\":42}",
        edited_at,
    )
    .unwrap();
    assert!(read_current_chat(&restarted).unwrap().draft.is_none());
    drop(restarted);

    let recovered = Connection::open(&path).unwrap();
    initialize_schema(&recovered).unwrap();
    recovered
        .execute_batch(
            "CREATE TABLE works (
                identity TEXT PRIMARY KEY,
                origin_kind TEXT NOT NULL,
                origin_primary_identity TEXT NOT NULL,
                state TEXT NOT NULL
             );
             CREATE TABLE work_approvals (
                identity TEXT PRIMARY KEY,
                work_identity TEXT NOT NULL,
                category TEXT NOT NULL,
                state TEXT NOT NULL
             );",
        )
        .unwrap();
    recovered
        .execute(
            "INSERT INTO works VALUES ('restart-work', 'conversation', ?1, 'waiting_for_approval')",
            [&initial.identity],
        )
        .unwrap();
    recovered
        .execute(
            "INSERT INTO work_approvals VALUES
             ('restart-approval', 'restart-work', 'filesystem_write', 'pending')",
            [],
        )
        .unwrap();
    recovered
        .execute("DELETE FROM attachments WHERE id='restart-attachment'", [])
        .unwrap();
    assert_eq!(
        recover_after_daemon_restart(&recovered, edited_at).unwrap(),
        1
    );
    assert_eq!(
        capture_recovered_approval_presentations(&recovered, edited_at).unwrap(),
        0
    );
    recovered
        .execute(
            "UPDATE works SET state='abandoned' WHERE identity='restart-work'",
            [],
        )
        .unwrap();
    recovered
        .execute(
            "UPDATE work_approvals SET state='abandoned' WHERE identity='restart-approval'",
            [],
        )
        .unwrap();
    assert_eq!(
        capture_recovered_approval_presentations(&recovered, edited_at).unwrap(),
        1
    );
    let restored = read_current_chat(&recovered).unwrap();
    assert_eq!(restored.identity, initial.identity);
    assert_eq!(restored.turns.len(), 1);
    assert_eq!(restored.turns[0].identity, active.identity);
    assert_eq!(
        restored.turns[0].user_message,
        "submitted user message survives"
    );
    assert!(restored.turns[0].assistant_output.is_none());
    assert_eq!(
        restored.turns[0].interruption_reason.as_deref(),
        Some("daemon_restart")
    );
    assert_eq!(restored.connector_references.len(), 1);
    assert_eq!(restored.connector_references[0].availability, "unavailable");
    assert_eq!(restored.submitted_attachments.len(), 1);
    assert_eq!(
        restored.submitted_attachments[0].conversation_turn_identity,
        active.identity
    );
    assert!(!restored.submitted_attachments[0].available);
    assert_eq!(restored.completed_approval_presentations.len(), 1);
    assert_eq!(
        restored.completed_approval_presentations[0].outcome,
        "abandoned"
    );

    let max_draft = "d".repeat(MAX_DRAFT_BYTES);
    let max_snapshot = save_draft(
        &recovered,
        &restored.identity,
        restored.revision,
        &max_draft,
        &["pending-max".to_owned()],
        edited_at,
    )
    .unwrap();
    let overflow = save_draft(
        &recovered,
        &restored.identity,
        max_snapshot.revision,
        &(max_draft.clone() + "x"),
        &[],
        edited_at,
    )
    .unwrap_err();
    assert!(overflow.to_string().contains("16 KiB"));
    assert_eq!(
        read_current_chat(&recovered).unwrap().draft.unwrap().text,
        max_draft
    );

    let expired = save_draft(
        &recovered,
        &restored.identity,
        max_snapshot.revision,
        "expired",
        &["expired-pending".to_owned()],
        Utc::now() - Duration::days(8),
    )
    .unwrap();
    assert!(expired.draft.is_some());
    let expired_bytes = expired.content_bytes;
    let expired_revision = expired.revision;
    let after_expiry = read_current_chat(&recovered).unwrap();
    assert!(after_expiry.draft.is_none());
    assert_eq!(after_expiry.revision, expired_revision + 1);
    assert!(after_expiry.content_bytes < expired_bytes);

    let failed_connection = Connection::open_in_memory().unwrap();
    initialize_schema(&failed_connection).unwrap();
    failed_connection
        .execute_batch(
            "CREATE TABLE works (
                identity TEXT PRIMARY KEY,
                origin_kind TEXT NOT NULL,
                origin_primary_identity TEXT NOT NULL,
                state TEXT NOT NULL
             );
             CREATE TABLE work_approvals (
                identity TEXT PRIMARY KEY,
                work_identity TEXT NOT NULL,
                category TEXT NOT NULL,
                state TEXT NOT NULL
             );",
        )
        .unwrap();
    let failed_chat = open_or_create_current_chat(&failed_connection).unwrap();
    let failed_turn = begin_conversation_turn(
        &failed_connection,
        &failed_chat.identity,
        failed_chat.revision,
        "turn that fails after approval",
        &[],
        edited_at,
    )
    .unwrap();
    failed_connection
        .execute(
            "INSERT INTO works VALUES ('failed-work', 'conversation', ?1, 'running')",
            [&failed_chat.identity],
        )
        .unwrap();
    failed_connection
        .execute(
            "INSERT INTO work_approvals VALUES
             ('failed-approval', 'failed-work', 'filesystem_write', 'allowed')",
            [],
        )
        .unwrap();
    let interrupted = interrupt_conversation_turn_with_work(
        &failed_connection,
        &failed_chat.identity,
        &failed_turn.identity,
        edited_at,
        Some("failed-work"),
    )
    .unwrap();
    assert_eq!(interrupted.completed_approval_presentations.len(), 1);
    assert_eq!(
        interrupted.completed_approval_presentations[0].outcome,
        "allowed"
    );

    let count_connection = Connection::open_in_memory().unwrap();
    initialize_schema(&count_connection).unwrap();
    let mut bounded = open_or_create_current_chat(&count_connection).unwrap();
    for index in 0..MAX_COMPLETED_TURNS {
        let begun = begin_conversation_turn(
            &count_connection,
            &bounded.identity,
            bounded.revision,
            &format!("turn-{index}"),
            &[],
            edited_at,
        )
        .unwrap();
        bounded = complete_conversation_turn(
            &count_connection,
            &bounded.identity,
            &begun.identity,
            "answer",
            edited_at,
        )
        .unwrap();
    }
    assert_eq!(bounded.turn_count, MAX_COMPLETED_TURNS);
    let count_error = begin_conversation_turn(
        &count_connection,
        &bounded.identity,
        bounded.revision,
        "one too many",
        &[],
        edited_at,
    )
    .unwrap_err();
    assert!(count_error.to_string().contains("500"));

    let byte_connection = Connection::open_in_memory().unwrap();
    initialize_schema(&byte_connection).unwrap();
    let byte_chat = open_or_create_current_chat(&byte_connection).unwrap();
    let exact_bound = "b".repeat(MAX_RETAINED_CONTENT_BYTES as usize - (64 * 1024) - 512);
    let byte_turn = begin_conversation_turn(
        &byte_connection,
        &byte_chat.identity,
        byte_chat.revision,
        &exact_bound,
        &[],
        edited_at,
    )
    .unwrap();
    let byte_bounded = complete_conversation_turn(
        &byte_connection,
        &byte_chat.identity,
        &byte_turn.identity,
        "",
        edited_at,
    )
    .unwrap();
    assert!(byte_bounded.content_bytes > exact_bound.len() as u64);
    let byte_error = begin_conversation_turn(
        &byte_connection,
        &byte_bounded.identity,
        byte_bounded.revision,
        &"x".repeat(1_024),
        &[],
        edited_at,
    )
    .unwrap_err();
    assert!(byte_error.to_string().contains("16 MiB"));

    let escaped_connection = Connection::open_in_memory().unwrap();
    initialize_schema(&escaped_connection).unwrap();
    let escaped_chat = open_or_create_current_chat(&escaped_connection).unwrap();
    let escape_heavy = "\"".repeat(MAX_RETAINED_CONTENT_BYTES as usize / 2);
    let escaped_error = begin_conversation_turn(
        &escaped_connection,
        &escaped_chat.identity,
        escaped_chat.revision,
        &escape_heavy,
        &[],
        edited_at,
    )
    .unwrap_err();
    assert!(escaped_error.to_string().contains("16 MiB"));
    assert!(read_current_chat(&escaped_connection)
        .unwrap()
        .turns
        .is_empty());
}

#[test]
fn v19_cutover_does_not_resurrect_locally_cleared_continuation() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE automation_current_chats (
            current_chat_identity TEXT PRIMARY KEY,
            content_empty INTEGER NOT NULL
         );
         CREATE TABLE automation_continuation_provenance (
            identity TEXT PRIMARY KEY,
            source_automation_session_identity TEXT NOT NULL,
            target_current_chat_identity TEXT NOT NULL UNIQUE,
            command_identity TEXT NOT NULL UNIQUE,
            seed TEXT NOT NULL,
            seed_bytes INTEGER NOT NULL,
            source_deleted INTEGER NOT NULL,
            created_at TEXT NOT NULL
         );
         CREATE TABLE automation_session_attention (
            automation_session_identity TEXT PRIMARY KEY,
            attention_state TEXT NOT NULL
         );
         INSERT INTO automation_current_chats VALUES ('legacy-chat-old', 0);
         INSERT INTO automation_current_chats VALUES ('legacy-chat', 0);
         INSERT INTO automation_continuation_provenance VALUES
            ('legacy-provenance-old', 'session-old', 'legacy-chat-old', 'command-old',
             'old seed', 8, 0, '2026-07-01T00:00:00Z');
         INSERT INTO automation_continuation_provenance VALUES
            ('legacy-provenance', 'session-a', 'legacy-chat', 'command-a',
             'legacy seed', 11, 0, '2026-08-01T00:00:00Z');
         INSERT INTO automation_session_attention VALUES ('session-a', 'viewed');",
        )
        .unwrap();

    initialize_schema(&connection).unwrap();

    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM automation_continuation_provenance",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let migrated = open_or_create_current_chat(&connection).unwrap();
    assert_ne!(migrated.identity, "legacy-chat");
    assert_ne!(migrated.identity, "legacy-chat-old");
    assert!(migrated.continuation.is_none());
    assert!(migrated.content_bytes > 0);
    assert_eq!(connection.query_row(
        "SELECT attention_state FROM automation_session_attention WHERE automation_session_identity='session-a'",
        [], |row| row.get::<_, String>(0)).unwrap(), "viewed");
}

#[test]
fn interruption_storage_failure_keeps_turn_active() {
    let connection = Connection::open_in_memory().unwrap();
    initialize_schema(&connection).unwrap();
    let chat = open_or_create_current_chat(&connection).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 8, 20, 10, 0, 0).unwrap();
    let turn = begin_conversation_turn(
        &connection,
        &chat.identity,
        chat.revision,
        "prompt",
        &[],
        now,
    )
    .unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER inject_interruption_failure
         BEFORE UPDATE ON current_chat_turns
         BEGIN SELECT RAISE(ABORT, 'injected interruption failure'); END;",
        )
        .unwrap();

    assert!(interrupt_conversation_turn_with_work(
        &connection,
        &chat.identity,
        &turn.identity,
        now,
        None
    )
    .is_err());
    connection
        .execute_batch("DROP TRIGGER inject_interruption_failure;")
        .unwrap();
    let unchanged = read_current_chat(&connection).unwrap();
    assert_eq!(unchanged.turns[0].state, ConversationTurnState::Active);
    assert_eq!(unchanged.revision, turn.current_chat_revision);
}

#[test]
fn approval_capture_failure_rolls_back_interruption() {
    let connection = Connection::open_in_memory().unwrap();
    initialize_schema(&connection).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE works (
            identity TEXT PRIMARY KEY, origin_kind TEXT NOT NULL,
            origin_primary_identity TEXT NOT NULL, state TEXT NOT NULL
         );
         CREATE TABLE work_approvals (
            identity TEXT PRIMARY KEY, work_identity TEXT NOT NULL,
            category TEXT NOT NULL, state TEXT NOT NULL
         );",
        )
        .unwrap();
    let chat = open_or_create_current_chat(&connection).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 8, 20, 10, 0, 0).unwrap();
    let turn = begin_conversation_turn(
        &connection,
        &chat.identity,
        chat.revision,
        "prompt",
        &[],
        now,
    )
    .unwrap();
    connection.execute(
        "INSERT INTO work_approvals VALUES ('approval-a', 'work-a', 'filesystem_write', 'allowed')",
        []).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER inject_approval_capture_failure
         BEFORE INSERT ON current_chat_approval_presentations
         BEGIN SELECT RAISE(ABORT, 'injected approval capture failure'); END;",
        )
        .unwrap();

    assert!(interrupt_conversation_turn_with_work(
        &connection,
        &chat.identity,
        &turn.identity,
        now,
        Some("work-a")
    )
    .is_err());
    connection
        .execute_batch("DROP TRIGGER inject_approval_capture_failure;")
        .unwrap();
    let unchanged = read_current_chat(&connection).unwrap();
    assert_eq!(unchanged.turns[0].state, ConversationTurnState::Active);
    assert!(unchanged.completed_approval_presentations.is_empty());
    assert_eq!(unchanged.revision, turn.current_chat_revision);
}
