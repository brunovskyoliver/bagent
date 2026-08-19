use bagentd::automation_sessions::{
    continue_automation_session_in_new_chat, delete_automation_definition, initialize_schema,
    mark_automation_session_viewed, open_automation_session, prune_automation_sessions,
    read_automation_session, read_continuation_provenance, register_definition, register_work,
    start_current_chat, terminalize_automation_session, AutomationRunOutcome,
    AutomationTaskSnapshot, AutomationTerminalization, SafeActivity,
};
use rusqlite::{params, Connection};

fn connection() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    initialize_schema(&connection).unwrap();
    connection
}

fn snapshot() -> AutomationTaskSnapshot {
    AutomationTaskSnapshot {
        automation_identity: "automation-a".to_owned(),
        automation_run_identity: "run-a".to_owned(),
        automation_session_identity: "automation-session-a".to_owned(),
        display_name: "Ranná pošta".to_owned(),
        task_text: "Skontroluj dnešnú poštu".to_owned(),
        schedule_json: r#"{"kind":"recurring","rule":{"type":"daily","time":"08:00:00"}}"#
            .to_owned(),
        timezone: "Europe/Bratislava".to_owned(),
        definition_revision: 4,
    }
}

fn terminalization() -> AutomationTerminalization {
    AutomationTerminalization {
        snapshot: snapshot(),
        work_identity: "work-a".to_owned(),
        outcome: AutomationRunOutcome::Completed,
        finished_at: "2026-08-19T08:10:00Z".to_owned(),
        result_summary: Some("Dve správy pripravené.".to_owned()),
        final_output: Some("Bezpečný výstup pre používateľa.".to_owned()),
        activity_timeline: vec![SafeActivity {
            category: "mail".to_owned(),
            caption: "Prehľadaná schránka".to_owned(),
            safety_relevant: false,
        }],
        validated_sources: vec![],
        connector_references: vec![],
        historical_approvals: vec![],
        truncation_disclosures: vec![],
    }
}

#[test]
fn terminal_immutability_keeps_content_immutable_but_attention_mutable() {
    let connection = connection();
    register_work(&connection, "work-a", "run-a").unwrap();

    terminalize_automation_session(&connection, terminalization()).unwrap();
    let before = read_automation_session(&connection, "automation-session-a")
        .unwrap()
        .unwrap();
    assert_eq!(before.outcome, AutomationRunOutcome::Completed);
    assert_eq!(before.task_snapshot.task_text, "Skontroluj dnešnú poštu");
    assert_eq!(
        before.final_output.as_deref(),
        Some("Bezpečný výstup pre používateľa.")
    );
    assert_eq!(before.attention, "unread");

    let error = terminalize_automation_session(&connection, terminalization()).unwrap_err();
    assert!(error.to_string().contains("immutable"));
    mark_automation_session_viewed(&connection, "automation-session-a").unwrap();

    let after = read_automation_session(&connection, "automation-session-a")
        .unwrap()
        .unwrap();
    assert_eq!(after.attention, "viewed");
    assert_eq!(after.result_summary, before.result_summary);
    assert_eq!(after.final_output, before.final_output);
    assert_eq!(after.task_snapshot, before.task_snapshot);
}

#[test]
fn opening_is_revisioned_and_idempotent() {
    let connection = connection();
    register_work(&connection, "work-a", "run-a").unwrap();
    terminalize_automation_session(&connection, terminalization()).unwrap();

    open_automation_session(&connection, "automation-session-a", "open-a", 7).unwrap();
    open_automation_session(&connection, "automation-session-a", "open-a", 7).unwrap();
    let error =
        open_automation_session(&connection, "automation-session-a", "open-a", 8).unwrap_err();
    assert!(error.to_string().contains("reused"));
    assert_eq!(
        read_automation_session(&connection, "automation-session-a")
            .unwrap()
            .unwrap()
            .attention,
        "viewed"
    );
}

#[test]
fn terminalization_atomicity_rolls_back_session_run_and_work_together() {
    let connection = connection();
    register_work(&connection, "work-a", "run-a").unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_outbox BEFORE INSERT ON automation_terminal_outbox
             BEGIN SELECT RAISE(ABORT, 'injected terminalization failure'); END;",
        )
        .unwrap();

    assert!(terminalize_automation_session(&connection, terminalization()).is_err());
    assert!(read_automation_session(&connection, "automation-session-a")
        .unwrap()
        .is_none());
    assert_eq!(
        connection
            .query_row(
                "SELECT state FROM automation_work_states WHERE work_identity=?1",
                params!["work-a"],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "running"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM automation_run_outcomes WHERE automation_run_identity=?1",
                params!["run-a"],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn bounded_content_never_truncates_silently() {
    let connection = connection();
    register_work(&connection, "work-a", "run-a").unwrap();
    let mut input = terminalization();
    input.result_summary = Some("x".repeat(501));
    input.activity_timeline = (0..300)
        .map(|index| SafeActivity {
            category: "tool".to_owned(),
            caption: format!("Aktivita {index}"),
            safety_relevant: index == 299,
        })
        .collect();

    terminalize_automation_session(&connection, input).unwrap();
    let stored = read_automation_session(&connection, "automation-session-a")
        .unwrap()
        .unwrap();
    assert!(stored.result_summary.unwrap().chars().count() <= 500);
    assert!(!stored.truncation_disclosures.is_empty());
    assert!(stored
        .activity_timeline
        .iter()
        .any(|activity| activity.caption == "Aktivita 299"));
}

#[test]
fn retention() {
    let connection = connection();
    for index in 0..55 {
        let mut input = terminalization();
        input.work_identity = format!("work-{index}");
        input.snapshot.automation_run_identity = format!("run-{index}");
        input.snapshot.automation_session_identity = format!("automation-session-{index}");
        input.finished_at = format!("2026-08-{:02}T08:00:00Z", (index % 28) + 1);
        register_work(
            &connection,
            &input.work_identity,
            &input.snapshot.automation_run_identity,
        )
        .unwrap();
        terminalize_automation_session(&connection, input).unwrap();
    }

    let deleted =
        prune_automation_sessions(&connection, "automation-a", "2026-08-30T00:00:00Z").unwrap();
    assert_eq!(deleted.sessions_deleted, 5);
    let remaining: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM automation_sessions s
             JOIN automation_task_snapshots t
               ON t.automation_session_identity=s.automation_session_identity
             WHERE t.automation_identity='automation-a'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 50);
}

#[test]
fn continue_in_new_chat() {
    let connection = connection();
    register_work(&connection, "work-a", "run-a").unwrap();
    terminalize_automation_session(&connection, terminalization()).unwrap();
    start_current_chat(&connection, "chat-a", false).unwrap();

    assert!(continue_automation_session_in_new_chat(
        &connection,
        "automation-session-a",
        "chat-a",
        "continuation seed",
        false,
        "continue-command-a",
    )
    .is_err());
    let provenance = continue_automation_session_in_new_chat(
        &connection,
        "automation-session-a",
        "chat-a",
        "continuation seed",
        true,
        "continue-command-a",
    )
    .unwrap();
    assert_eq!(
        provenance.source_automation_session_identity,
        "automation-session-a"
    );
    assert_eq!(provenance.seed, "continuation seed");
    assert_eq!(
        read_automation_session(&connection, "automation-session-a")
            .unwrap()
            .unwrap()
            .attention,
        "viewed"
    );
    assert!(continue_automation_session_in_new_chat(
        &connection,
        "automation-session-a",
        "chat-a",
        "continuation seed",
        true,
        "continue-command-a",
    )
    .is_ok());
    assert_eq!(
        read_continuation_provenance(&connection, "chat-a").unwrap(),
        Some(provenance)
    );
}

#[test]
fn definition_delete() {
    let connection = connection();
    register_definition(&connection, "automation-a").unwrap();
    register_work(&connection, "work-a", "run-a").unwrap();
    terminalize_automation_session(&connection, terminalization()).unwrap();

    delete_automation_definition(&connection, "automation-a").unwrap();
    assert!(read_automation_session(&connection, "automation-session-a")
        .unwrap()
        .is_some());
}

#[test]
fn completion_attention() {
    for outcome in AutomationRunOutcome::all() {
        let connection = connection();
        let mut input = terminalization();
        input.outcome = outcome;
        register_work(&connection, "work-a", "run-a").unwrap();
        terminalize_automation_session(&connection, input).unwrap();
        let attention = read_automation_session(&connection, "automation-session-a")
            .unwrap()
            .unwrap()
            .attention;
        assert_eq!(
            attention,
            if outcome == AutomationRunOutcome::Skipped {
                "none"
            } else {
                "unread"
            }
        );
    }
}
