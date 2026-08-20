use crate::agent_exec::{ExecError, ExecOutcome};
use crate::automations_api::{
    finalize_automation_terminal, outcome_to_status, repo_claim_run, repo_create, repo_finish_run,
    repo_finish_run_ambiguous_commit_for_test, repo_prune_runs, repo_recent_runs,
    AutomationTerminalOutcome,
};
use crate::reference_resolution::{ReferenceOutcomeCode, TurnCompletion};
use bagent_automations::{AutomationRun, AutomationRunId, AutomationRunStatus, AutomationSchedule};
use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::json;

fn test_conn() -> Connection {
    let mut conn = Connection::open_in_memory().expect("temporary SQLite database");
    crate::embedded::migrations::runner()
        .run(&mut conn)
        .expect("embedded migrations");
    conn
}

fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 19, 10, 0, 0).unwrap()
}

fn once_at() -> AutomationSchedule {
    AutomationSchedule::Once {
        at: Utc.with_ymd_and_hms(2026, 8, 20, 10, 0, 0).unwrap(),
    }
}

#[test]
fn reference_blocked_completion_maps_to_blocked_with_a_fixed_summary() {
    let completion = TurnCompletion::ReferenceBlocked(ReferenceOutcomeCode::Ambiguous);
    let result = Ok(ExecOutcome {
        final_text: "hostile model output must be ignored".into(),
        tool_calls_used: 0,
        approvals_denied: 0,
        completion: completion,
    });

    let status = outcome_to_status(&result).status;
    assert_eq!(
        status.as_str(),
        "blocked",
        "{completion:?} currently has no typed completion storage"
    );
}

#[test]
fn every_closed_blocked_code_has_the_exact_safe_terminal_tuple() {
    let expected_summary =
        "Blocked: update the automation task with one exact public name, make/model, or URL; unattended runs cannot clarify references.";
    let expected_unavailable_summary =
        "Blocked: reference safety checks were unavailable; no external or model work was attempted.";
    for code in [
        ReferenceOutcomeCode::MissingReferent,
        ReferenceOutcomeCode::Ambiguous,
        ReferenceOutcomeCode::ConfirmationRequired,
        ReferenceOutcomeCode::PrivateSourceDenied,
        ReferenceOutcomeCode::Expired,
        ReferenceOutcomeCode::Unsupported,
        ReferenceOutcomeCode::ResolverUnavailable,
    ] {
        let hostile = "SYNTHETIC_MODEL_TEXT must never be persisted";
        let terminal = outcome_to_status(&Ok(ExecOutcome {
            final_text: hostile.into(),
            tool_calls_used: 999,
            approvals_denied: 999,
            completion: TurnCompletion::ReferenceBlocked(code),
        }));
        assert_eq!(terminal.status, AutomationRunStatus::Blocked);
        assert_eq!(terminal.reference_outcome_code, Some(code));
        assert_eq!(
            terminal.result_summary,
            if code == ReferenceOutcomeCode::ResolverUnavailable {
                expected_unavailable_summary
            } else {
                expected_summary
            }
        );
        assert!(!terminal.result_summary.contains(hostile));
    }
}

#[test]
fn completion_mapping_uses_typed_completion_not_approval_counts_or_text() {
    let completed = outcome_to_status(&Ok(ExecOutcome {
        final_text: "synthetic completed text".into(),
        tool_calls_used: 0,
        approvals_denied: 44,
        completion: TurnCompletion::Completed,
    }));
    assert_eq!(completed.status, AutomationRunStatus::Completed);

    let partial = outcome_to_status(&Ok(ExecOutcome {
        final_text: "synthetic partial text".into(),
        tool_calls_used: 0,
        approvals_denied: 0,
        completion: TurnCompletion::Partial,
    }));
    assert_eq!(partial.status, AutomationRunStatus::Partial);
}

#[test]
fn red_v14_schema_has_typed_reference_outcome_storage() {
    let conn = test_conn();
    let run_columns: Vec<String> = conn
        .prepare("PRAGMA table_info(automation_runs)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let automation_columns: Vec<String> = conn
        .prepare("PRAGMA table_info(automations)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    assert!(run_columns.iter().any(|column| column == "reference_outcome_code"));
    assert!(automation_columns
        .iter()
        .any(|column| column == "last_reference_outcome_code"));
}

#[test]
fn never_run_automation_keeps_both_snapshot_fields_null() {
    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic never-run automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();
    assert_eq!(automation.last_run_status, None);
    assert_eq!(automation.last_reference_outcome_code, None);
    let snapshot: (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT last_run_status, last_reference_outcome_code
             FROM automations WHERE id=?1",
            params![automation.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(snapshot, (None, None));
}

#[test]
fn red_finish_failure_cannot_expose_a_partial_terminal_update() {
    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();
    let run = repo_claim_run(
        &conn,
        &automation,
        now(),
        false,
        true,
        now(),
    )
    .unwrap();
    conn.execute(
        "CREATE TRIGGER fail_automation_snapshot BEFORE UPDATE OF last_run_status ON automations
         BEGIN SELECT RAISE(ABORT, 'synthetic snapshot failure'); END",
        [],
    )
    .unwrap();

    let result = repo_finish_run(
        &conn,
        &run.id.to_string(),
        &automation.id.to_string(),
        AutomationRunStatus::Completed,
        Some("Synthetic summary"),
        None,
        now(),
    );
    assert!(result.is_err());

    let stored_status: String = conn
        .query_row(
            "SELECT status FROM automation_runs WHERE id=?1",
            params![run.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let stored_snapshot: Option<String> = conn
        .query_row(
            "SELECT last_run_status FROM automations WHERE id=?1",
            params![automation.id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
        .flatten();
    assert_eq!(stored_status, "running");
    assert_eq!(stored_snapshot, None);
}

#[test]
fn atomic_finish_updates_run_and_automation_snapshot_with_the_same_tuple() {
    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();
    let run = repo_claim_run(&conn, &automation, now(), false, true, now()).unwrap();
    let finished_at = now() + chrono::Duration::minutes(1);
    let terminal = AutomationTerminalOutcome {
        status: AutomationRunStatus::Blocked,
        result_summary: "Blocked: update the automation task with one exact public name, make/model, or URL; unattended runs cannot clarify references.".into(),
        reference_outcome_code: Some(ReferenceOutcomeCode::Ambiguous),
    };

    assert!(repo_finish_run(
        &conn,
        &run.id.to_string(),
        &automation.id.to_string(),
        terminal.status,
        Some(&terminal.result_summary),
        terminal.reference_outcome_code,
        finished_at,
    )
    .is_ok());

    let stored = repo_recent_runs(&conn, &automation.id.to_string(), 1).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].status, AutomationRunStatus::Blocked);
    assert_eq!(stored[0].finished_at, Some(finished_at));
    assert_eq!(stored[0].result_summary, Some(terminal.result_summary.clone()));
    assert_eq!(
        stored[0].reference_outcome_code.as_deref(),
        Some("ambiguous")
    );
    let snapshot: (Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT last_run_at, last_run_status, last_reference_outcome_code
             FROM automations WHERE id=?1",
            params![automation.id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(snapshot.0.as_deref(), Some(finished_at.to_rfc3339().as_str()));
    assert_eq!(snapshot.1.as_deref(), Some("blocked"));
    assert_eq!(snapshot.2.as_deref(), Some("ambiguous"));
}

#[test]
fn identical_retry_is_idempotent_and_conflicting_retry_fails_closed() {
    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();
    let run = repo_claim_run(&conn, &automation, now(), false, true, now()).unwrap();
    let finished_at = now() + chrono::Duration::minutes(1);
    let args = (
        AutomationRunStatus::Blocked,
        Some("safe summary"),
        Some(ReferenceOutcomeCode::Ambiguous),
        finished_at,
    );
    repo_finish_run(
        &conn,
        &run.id.to_string(),
        &automation.id.to_string(),
        args.0,
        args.1,
        args.2,
        args.3,
    )
    .unwrap();
    assert!(repo_finish_run(
        &conn,
        &run.id.to_string(),
        &automation.id.to_string(),
        args.0,
        args.1,
        args.2,
        args.3,
    )
    .is_ok());
    assert!(repo_finish_run(
        &conn,
        &run.id.to_string(),
        &automation.id.to_string(),
        AutomationRunStatus::Completed,
        Some("different"),
        None,
        finished_at,
    )
    .is_err());
    let status: String = conn
        .query_row(
            "SELECT status FROM automation_runs WHERE id=?1",
            params![run.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "blocked");
}

#[test]
fn ambiguous_commit_is_accepted_only_after_exact_terminal_readback() {
    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();
    let run = repo_claim_run(&conn, &automation, now(), false, true, now()).unwrap();
    let finished_at = now() + chrono::Duration::minutes(1);
    assert!(repo_finish_run_ambiguous_commit_for_test(
        &conn,
        &run.id.to_string(),
        &automation.id.to_string(),
        AutomationRunStatus::Blocked,
        Some("hostile ignored"),
        Some(ReferenceOutcomeCode::Ambiguous),
        finished_at,
    )
    .is_ok());
    let stored = repo_recent_runs(&conn, &automation.id.to_string(), 1).unwrap();
    assert_eq!(stored[0].status, AutomationRunStatus::Blocked);
    assert_eq!(stored[0].finished_at, Some(finished_at));
    assert_eq!(stored[0].reference_outcome_code.as_deref(), Some("ambiguous"));
}

#[test]
fn blocked_rows_count_toward_the_newest_fifty_run_cap() {
    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();
    for index in 0..55 {
        let run = AutomationRun {
            id: AutomationRunId::new(),
            automation_id: automation.id,
            scheduled_for: now() + chrono::Duration::seconds(index),
            started_at: Some(now()),
            finished_at: Some(now()),
            status: AutomationRunStatus::Blocked,
            result_summary: Some("safe summary".into()),
            reference_outcome_code: Some("ambiguous".into()),
            is_catch_up: false,
            is_manual: true,
        };
        crate::automations_api::repo_insert_run(&conn, &run).unwrap();
    }
    repo_prune_runs(&conn, &automation.id.to_string()).unwrap();
    assert_eq!(
        repo_recent_runs(&conn, &automation.id.to_string(), 100)
            .unwrap()
            .len(),
        50
    );
}

#[test]
fn scheduled_and_run_now_blocked_runs_satisfy_the_occurrence_without_rescheduling() {
    use crate::scheduler::scheduler_step;
    use bagent_automations::RecurrenceRule;

    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic recurring automation",
        "Synthetic task",
        "Europe/Bratislava",
        &AutomationSchedule::Recurring {
            rule: RecurrenceRule::EveryNHours { hours: 2 },
        },
        true,
        now() - chrono::Duration::hours(3),
    )
    .unwrap();
    let scheduled_next = automation.next_run_at.unwrap();
    let pass = scheduler_step(&conn, scheduled_next);
    assert_eq!(pass.claimed.len(), 1);
    let scheduled_run = &pass.claimed[0].1;
    let advanced_next = crate::automations_api::repo_get(&conn, &automation.id.to_string())
        .unwrap()
        .next_run_at;
    assert_ne!(advanced_next, Some(scheduled_next));
    repo_finish_run(
        &conn,
        &scheduled_run.id.to_string(),
        &automation.id.to_string(),
        AutomationRunStatus::Blocked,
        Some("safe summary"),
        Some(ReferenceOutcomeCode::Ambiguous),
        scheduled_next,
    )
    .unwrap();
    assert_eq!(
        crate::automations_api::repo_get(&conn, &automation.id.to_string())
            .unwrap()
            .next_run_at,
        advanced_next
    );

    let manual = repo_claim_run(
        &conn,
        &automation,
        now() + chrono::Duration::hours(1),
        false,
        true,
        now() + chrono::Duration::hours(1),
    )
    .unwrap();
    let before_manual = crate::automations_api::repo_get(&conn, &automation.id.to_string())
        .unwrap()
        .next_run_at;
    repo_finish_run(
        &conn,
        &manual.id.to_string(),
        &automation.id.to_string(),
        AutomationRunStatus::Blocked,
        Some("safe summary"),
        Some(ReferenceOutcomeCode::Ambiguous),
        now() + chrono::Duration::hours(1),
    )
    .unwrap();
    assert_eq!(
        crate::automations_api::repo_get(&conn, &automation.id.to_string())
            .unwrap()
            .next_run_at,
        before_manual
    );
}

#[test]
fn committed_blocked_rows_survive_reopen_and_restart_recovery() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_owned();
    let (automation_id, run_id) = {
        let mut conn = Connection::open(&path).unwrap();
        crate::embedded::migrations::runner().run(&mut conn).unwrap();
        let automation = repo_create(
            &conn,
            "Synthetic automation",
            "Synthetic task",
            "Europe/Bratislava",
            &once_at(),
            true,
            now(),
        )
        .unwrap();
        let run = repo_claim_run(&conn, &automation, now(), false, true, now()).unwrap();
        repo_finish_run(
            &conn,
            &run.id.to_string(),
            &automation.id.to_string(),
            AutomationRunStatus::Blocked,
            Some("safe summary"),
            Some(ReferenceOutcomeCode::Ambiguous),
            now(),
        )
        .unwrap();
        (automation.id.to_string(), run.id.to_string())
    };
    let mut conn = Connection::open(path).unwrap();
    crate::embedded::migrations::runner().run(&mut conn).unwrap();
    assert_eq!(crate::scheduler::recover_on_startup(&conn, now()), 0);
    let run = repo_recent_runs(&conn, &automation_id, 1).unwrap();
    assert_eq!(run[0].id.to_string(), run_id);
    assert_eq!(run[0].status, AutomationRunStatus::Blocked);
    assert_eq!(run[0].reference_outcome_code.as_deref(), Some("ambiguous"));
}

#[test]
fn terminal_event_order_and_persistence_failure_are_structural() {
    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();
    let run = repo_claim_run(&conn, &automation, now(), false, true, now()).unwrap();
    let terminal = AutomationTerminalOutcome {
        status: AutomationRunStatus::Blocked,
        result_summary: "safe summary".into(),
        reference_outcome_code: Some(ReferenceOutcomeCode::Ambiguous),
    };
    let mut events = Vec::new();
    let mut audits = Vec::new();
    assert!(finalize_automation_terminal(
        &conn,
        &automation.id.to_string(),
        &run.id.to_string(),
        &terminal,
        now(),
        |event| events.push(event),
        |action, payload| audits.push((action.to_string(), payload)),
    ));
    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["reference_resolution", "automation_run_finished"]
    );
    assert_eq!(events[0]["reference_outcome_code"], json!("ambiguous"));
    assert_eq!(events[1]["status"], json!("blocked"));
    assert_eq!(audits.len(), 1);
    let reference_keys: std::collections::BTreeSet<_> = events[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        reference_keys,
        [
            "type",
            "automation_id",
            "version",
            "turn_id",
            "origin",
            "run_id",
            "disposition",
            "reference_outcome_code",
        ]
        .into_iter()
        .collect()
    );

    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();
    let run = repo_claim_run(&conn, &automation, now(), false, true, now()).unwrap();
    conn.execute(
        "CREATE TRIGGER fail_snapshot BEFORE UPDATE OF last_run_status ON automations
         BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END",
        [],
    )
    .unwrap();
    let mut failed_events = Vec::new();
    let mut failed_audits = Vec::new();
    assert!(!finalize_automation_terminal(
        &conn,
        &automation.id.to_string(),
        &run.id.to_string(),
        &terminal,
        now(),
        |event| failed_events.push(event),
        |action, payload| failed_audits.push((action.to_string(), payload)),
    ));
    assert_eq!(failed_events.len(), 1);
    assert_eq!(failed_events[0]["type"], json!("automation_run_persistence_failed"));
    assert!(failed_audits.is_empty());
    assert_eq!(failed_events[0].as_object().unwrap().len(), 3);
    let status: String = conn
        .query_row(
            "SELECT status FROM automation_runs WHERE id=?1",
            params![run.id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "running");
}

#[test]
fn migration_runs_on_a_fresh_temporary_database() {
    let conn = test_conn();
    let version: i64 = conn
        .query_row(
            "SELECT MAX(version) FROM refinery_schema_history",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 18);
}

#[test]
fn migration_runs_as_a_real_v14_upgrade_without_backfill() {
    let mut conn = Connection::open_in_memory().unwrap();
    crate::embedded::migrations::runner()
        .set_target(refinery::Target::Version(14))
        .run(&mut conn)
        .unwrap();

    let automation_id = "synthetic-automation-v14";
    let run_id = "synthetic-run-v14";
    conn.execute(
        "INSERT INTO automations
         (id, name, prompt, enabled, timezone, schedule_json, next_run_at,
          created_at, updated_at, last_run_at, last_run_status, last_result_summary)
         VALUES (?1, 'Synthetic', 'Synthetic task', 1, 'UTC', '{}', NULL,
                 '2026-08-19T10:00:00Z', '2026-08-19T10:00:00Z',
                 '2026-08-19T10:01:00Z', 'completed', 'old summary')",
        params![automation_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO automation_runs
         (id, automation_id, scheduled_for, started_at, finished_at, status,
          result_summary, is_catch_up, is_manual, created_at)
         VALUES (?1, ?2, '2026-08-19T10:00:00Z', '2026-08-19T10:00:00Z',
                 '2026-08-19T10:01:00Z', 'completed', 'old summary', 0, 1,
                 '2026-08-19T10:01:00Z')",
        params![run_id, automation_id],
    )
    .unwrap();

    crate::embedded::migrations::runner()
        .run(&mut conn)
        .unwrap();

    let historical: (String, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT status, reference_outcome_code, result_summary
             FROM automation_runs WHERE id=?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(historical, ("completed".into(), None, Some("old summary".into())));
    let snapshot: (Option<String>, Option<String>, Option<String>) = conn
        .query_row(
            "SELECT last_run_status, last_reference_outcome_code, last_result_summary
             FROM automations WHERE id=?1",
            params![automation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(snapshot, (Some("completed".into()), None, Some("old summary".into())));
}

#[test]
fn sqlite_rejects_invalid_status_and_reference_code_tuples() {
    let conn = test_conn();
    let automation = repo_create(
        &conn,
        "Synthetic automation",
        "Synthetic task",
        "Europe/Bratislava",
        &once_at(),
        true,
        now(),
    )
    .unwrap();

    let invalid_code = conn.execute(
        "INSERT INTO automation_runs
         (id, automation_id, scheduled_for, status, created_at, reference_outcome_code)
         VALUES ('synthetic-invalid-code', ?1, ?2, 'blocked', ?2, 'not_allowed')",
        params![automation.id.to_string(), now().to_rfc3339()],
    );
    assert!(invalid_code.is_err());

    let non_blocked_with_code = conn.execute(
        "INSERT INTO automation_runs
         (id, automation_id, scheduled_for, status, created_at, reference_outcome_code)
         VALUES ('synthetic-invalid-status', ?1, ?2, 'completed', ?2, 'ambiguous')",
        params![automation.id.to_string(), now().to_rfc3339()],
    );
    assert!(non_blocked_with_code.is_err());

    let blocked_without_code = conn.execute(
        "INSERT INTO automation_runs
         (id, automation_id, scheduled_for, status, created_at)
         VALUES ('synthetic-missing-code', ?1, ?2, 'blocked', ?2)",
        params![automation.id.to_string(), now().to_rfc3339()],
    );
    assert!(blocked_without_code.is_err());

    let null_snapshot = conn.execute(
        "UPDATE automations SET last_run_status='blocked', last_reference_outcome_code=NULL
         WHERE id=?1",
        params![automation.id.to_string()],
    );
    assert!(null_snapshot.is_err());
}

#[allow(dead_code)]
fn _red_evidence_keeps_error_type_reachable(_: ExecError) {}
