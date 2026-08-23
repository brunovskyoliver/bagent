use bagentd::automation_sessions::{
    AutomationRunOutcome, AutomationTaskSnapshot, AutomationTerminalization,
};
use bagentd::cutover::{
    finalize_stage8_cleanup, mark_first_post_cutover_work, migrate_v14_copy, rollback_boundary,
    sha256, verified_backup, verified_restore, RollbackBoundary, LEGACY_UNAVAILABLE,
};
use bagentd::unified_work::UnifiedWorkAuthority;
use bagentd::work_coordinator::{
    AutomationDefinitionIdentity, AutomationDefinitionRevision, AutomationRunIdentity,
    AutomationSessionIdentity, CoordinatorConfig, DaemonGeneration, WorkCoordinator, WorkState,
};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const CANONICAL_TABLES: &[&str] = &[
    "works",
    "work_command_results",
    "work_event_outbox",
    "work_coordinator_metadata",
    "work_current_chats",
    "work_conversation_turns",
    "work_automation_runs",
    "work_automation_sessions",
    "work_approvals",
    "work_projections",
    "work_continuations",
    "work_interruption_markers",
    "work_model_runtime_recovery",
    "work_cutover",
    "legacy_run_records",
    "automation_work_states",
    "automation_task_snapshots",
    "automation_run_outcomes",
    "automation_sessions",
    "automation_session_attention",
    "automation_session_open_commands",
    "automation_terminal_outbox",
    "automation_session_tombstones",
    "automation_continuation_provenance",
    "automation_definitions",
    "automation_retention_audit",
    "automations",
    "current_chats",
    "current_chat_authority",
    "current_chat_turns",
    "current_chat_drafts",
    "current_chat_submitted_attachments",
    "current_chat_validated_sources",
    "current_chat_connector_references",
    "current_chat_approval_presentations",
    "current_chat_clear_commands",
    "current_chat_lifecycle_audit",
    "automation_run_records",
    "work_approval_requests",
    "stage8_cleanup_state",
];

const AUTOMATION_DEFINITION_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS automations (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    prompt              TEXT NOT NULL,
    enabled             INTEGER NOT NULL DEFAULT 1,
    timezone            TEXT NOT NULL,
    schedule_json       TEXT NOT NULL,
    next_run_at         TEXT,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL,
    last_run_at         TEXT,
    last_run_status     TEXT,
    last_result_summary TEXT,
    definition_revision INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_automations_due ON automations (enabled, next_run_at);
"#;

fn stage8_privacy_canary_bundle() -> String {
    std::env::var("BAGENT_STAGE8_PRIVACY_CANARY_BUNDLE")
        .unwrap_or_else(|_| "CANARY_PRIVATE_IDENTITY:CANARY_RAW_ARGUMENT".to_owned())
}

fn write_stage8_privacy_capture(name: &str, value: serde_json::Value) {
    let Ok(directory) = std::env::var("BAGENT_STAGE8_PRIVACY_CAPTURE_DIR") else {
        return;
    };
    let path = std::path::Path::new(&directory).join(name);
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn canonical_schema_checksum(connection: &Connection) -> String {
    let mut statement = connection
        .prepare(
            "SELECT type, name, COALESCE(tbl_name, ''), COALESCE(sql, '')
             FROM sqlite_master
             WHERE type IN ('table', 'index', 'trigger')
             ORDER BY type, name",
        )
        .unwrap();
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let normalized = rows
        .into_iter()
        .filter(|(_, name, table, _)| {
            CANONICAL_TABLES.contains(&name.as_str()) || CANONICAL_TABLES.contains(&table.as_str())
        })
        .map(|(kind, name, table, sql)| format!("{kind}|{name}|{table}|{sql}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("{:x}", Sha256::digest(normalized.as_bytes()))
}

fn assert_no_private_projection_values(connection: &Connection) {
    let markers = [
        "canary_private_identity",
        "canary_raw_argument",
        "stage8_canary",
        "hidden reasoning",
        "raw arguments",
        "credential",
        "evidence content",
        "private identity",
        "unknown field",
        "handoff data",
        "diagnostic export",
        "failure payload",
    ];
    let projections = [
        (
            "legacy_run_records",
            "SELECT COALESCE(summary, '') FROM legacy_run_records",
        ),
        (
            "automation_run_records",
            "SELECT COALESCE(result_summary, '') FROM automation_run_records",
        ),
        (
            "work_approval_requests",
            "SELECT COALESCE(description, '') || ' ' || COALESCE(origin_json, '')
             FROM work_approval_requests",
        ),
        (
            "automation_sessions",
            "SELECT COALESCE(result_summary, '') || ' ' || COALESCE(final_output, '') || ' '
                    || COALESCE(activity_timeline_json, '') || ' '
                    || COALESCE(validated_sources_json, '') || ' '
                    || COALESCE(connector_references_json, '') || ' '
                    || COALESCE(historical_approvals_json, '')
             FROM automation_sessions",
        ),
        (
            "current_chat_turns",
            "SELECT COALESCE(user_message, '') || ' ' || COALESCE(assistant_output, '')
             FROM current_chat_turns",
        ),
        (
            "current_chat_drafts",
            "SELECT COALESCE(text, '') FROM current_chat_drafts",
        ),
    ];
    for (table, query) in projections {
        let exists: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        if !exists {
            continue;
        }
        let mut statement = connection.prepare(query).unwrap();
        let values = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        for value in values {
            let lower = value.to_ascii_lowercase();
            assert!(
                !markers.iter().any(|marker| lower.contains(marker)),
                "private migration canary leaked into {table}: {value}"
            );
        }
    }
}

fn canonicalize(path: &std::path::Path, backup_hash: &str) -> Connection {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(AUTOMATION_DEFINITION_SCHEMA)
        .unwrap();
    drop(connection);
    migrate_v14_copy(path, backup_hash, true).unwrap();
    finalize_stage8_cleanup(path).unwrap();
    let connection = Connection::open(path).unwrap();
    bagentd::automation_sessions::initialize_schema(&connection).unwrap();
    let snapshot = bagentd::current_chat::open_or_create_current_chat(&connection).unwrap();
    assert!(snapshot.identity.len() > 8);
    connection
}

fn automation_definition_contract(path: &std::path::Path) -> String {
    let connection = Connection::open(path).unwrap();
    let columns = connection
        .prepare("PRAGMA table_info(automations)")
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        columns,
        vec![
            ("id".to_owned(), "TEXT".to_owned(), 0, None),
            ("name".to_owned(), "TEXT".to_owned(), 1, None),
            ("prompt".to_owned(), "TEXT".to_owned(), 1, None),
            (
                "enabled".to_owned(),
                "INTEGER".to_owned(),
                1,
                Some("1".to_owned())
            ),
            ("timezone".to_owned(), "TEXT".to_owned(), 1, None),
            ("schedule_json".to_owned(), "TEXT".to_owned(), 1, None),
            ("next_run_at".to_owned(), "TEXT".to_owned(), 0, None),
            ("created_at".to_owned(), "TEXT".to_owned(), 1, None),
            ("updated_at".to_owned(), "TEXT".to_owned(), 1, None),
            ("last_run_at".to_owned(), "TEXT".to_owned(), 0, None),
            ("last_run_status".to_owned(), "TEXT".to_owned(), 0, None),
            ("last_result_summary".to_owned(), "TEXT".to_owned(), 0, None),
            (
                "definition_revision".to_owned(),
                "INTEGER".to_owned(),
                1,
                Some("1".to_owned())
            ),
        ]
    );
    let due_index: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_automations_due'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(due_index, 1, "scheduler due-time index is canonical");

    let id = "00000000-0000-0000-0000-0000000000a2";
    connection
        .execute(
            "INSERT INTO automations
             (id,name,prompt,enabled,timezone,schedule_json,next_run_at,created_at,updated_at,definition_revision)
             VALUES (?1,'A52 contract','safe prompt',1,'UTC','{\"kind\":\"once\"}',
                     '2099-01-01T00:00:00Z','2026-08-18T20:00:00Z','2026-08-18T20:00:00Z',1)",
            [id],
        )
        .unwrap();
    let enabled_due: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM automations WHERE enabled=1 AND next_run_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        enabled_due >= 1,
        "enabled scheduled definition is discoverable"
    );
    connection
        .execute(
            "UPDATE automations SET name='A52 updated', enabled=0, updated_at='2026-08-18T20:01:00Z' WHERE id=?1",
            [id],
        )
        .unwrap();
    let updated: (String, i64, i64) = connection
        .query_row(
            "SELECT name, enabled, definition_revision FROM automations WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(updated, ("A52 updated".to_owned(), 0, 1));
    let disabled_due: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM automations WHERE enabled=1 AND next_run_at IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(disabled_due, enabled_due - 1);
    connection
        .execute("DELETE FROM automations WHERE id=?1", [id])
        .unwrap();
    let remaining: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM automations WHERE id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
    format!("columns={columns:?};index={due_index};enabled_due={enabled_due};disabled_due={disabled_due};updated={updated:?}")
}

fn exercise_canonical_work_session(path: &std::path::Path) {
    let generation = DaemonGeneration::new("stage8-a52-generation");
    let coordinator =
        WorkCoordinator::open(path, CoordinatorConfig::default(), generation.clone()).unwrap();
    let authority = UnifiedWorkAuthority::new(std::sync::Arc::new(coordinator), generation);
    let work = authority
        .submit_automation(
            "stage8-a52-submit",
            AutomationRunIdentity::new("stage8-a52-run"),
            AutomationSessionIdentity::new("stage8-a52-session"),
            AutomationDefinitionIdentity::new("a"),
            AutomationDefinitionRevision::new(1),
            0,
        )
        .unwrap();
    let registration = Connection::open(path).unwrap();
    bagentd::automation_sessions::register_work(&registration, work.as_str(), "stage8-a52-run")
        .unwrap();
    drop(registration);
    let revision = authority.current(&work).unwrap().unwrap().revision;
    let revision = authority
        .transition(
            "stage8-a52-waiting",
            work.clone(),
            revision,
            WorkState::WaitingForModel,
        )
        .unwrap();
    let revision = authority
        .transition(
            "stage8-a52-running",
            work.clone(),
            revision,
            WorkState::Running,
        )
        .unwrap();
    let input = AutomationTerminalization {
        snapshot: AutomationTaskSnapshot {
            automation_identity: "a".to_owned(),
            automation_run_identity: "stage8-a52-run".to_owned(),
            automation_session_identity: "stage8-a52-session".to_owned(),
            display_name: "A".to_owned(),
            task_text: "safe migration fixture".to_owned(),
            schedule_json: "{}".to_owned(),
            timezone: "UTC".to_owned(),
            definition_revision: 1,
        },
        work_identity: work.as_str().to_owned(),
        outcome: AutomationRunOutcome::Completed,
        finished_at: "2026-08-18T20:00:00Z".to_owned(),
        result_summary: Some("Completed successfully.".to_owned()),
        final_output: Some("safe result".to_owned()),
        activity_timeline: Vec::new(),
        validated_sources: Vec::new(),
        connector_references: Vec::new(),
        historical_approvals: Vec::new(),
        truncation_disclosures: Vec::new(),
    };
    authority
        .terminalize_automation_session(
            "stage8-a52-terminalize",
            work,
            revision,
            WorkState::Completed,
            input,
        )
        .unwrap();
    let connection = Connection::open(path).unwrap();
    let state: String = connection
        .query_row(
            "SELECT state FROM automation_work_states WHERE automation_run_identity='stage8-a52-run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "terminal:completed");
    let session_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM automation_sessions WHERE automation_session_identity='stage8-a52-session'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(session_count, 1);
    assert_no_private_projection_values(&connection);
}

fn v14_fixture() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v14.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("PRAGMA journal_mode=DELETE;")
        .unwrap();
    connection
        .execute_batch(&format!(
            "{AUTOMATION_DEFINITION_SCHEMA}
         CREATE TABLE automation_runs (
           id TEXT PRIMARY KEY, automation_id TEXT NOT NULL, scheduled_for TEXT NOT NULL,
           started_at TEXT, finished_at TEXT, status TEXT NOT NULL, result_summary TEXT,
           is_catch_up INTEGER NOT NULL DEFAULT 0, is_manual INTEGER NOT NULL DEFAULT 0,
           created_at TEXT NOT NULL
         );
         CREATE TABLE pending_approvals (
           id TEXT PRIMARY KEY, tool_name TEXT NOT NULL, description TEXT NOT NULL,
           expires_at TEXT NOT NULL, created_at TEXT NOT NULL, decision TEXT, decided_at TEXT,
           origin_json TEXT
         );",
        ))
        .unwrap();
    connection
        .execute(
            "INSERT INTO automations
             (id,name,prompt,enabled,timezone,schedule_json,created_at,updated_at,definition_revision)
             VALUES ('a','A','safe migration automation',1,'UTC','{\"kind\":\"once\"}',
                     '2026-08-18T18:00:00Z','2026-08-18T18:00:00Z',1)",
            [],
        )
        .unwrap();
    let private_bundle = stage8_privacy_canary_bundle();
    for index in 0..55 {
        connection
            .execute(
                "INSERT INTO automation_runs
             (id,automation_id,scheduled_for,finished_at,status,result_summary,created_at)
             VALUES (?1,'a',?2,?2,'completed',?3,?2)",
                params![
                    format!("terminal-{index:02}"),
                    format!("2026-08-18T18:{index:02}:00Z"),
                    if index == 54 {
                        "Completed successfully.".to_owned()
                    } else {
                        private_bundle.clone()
                    }
                ],
            )
            .unwrap();
    }
    connection.execute(
        "INSERT INTO automation_runs
         (id,automation_id,scheduled_for,status,result_summary,created_at)
         VALUES ('active','a','2026-08-18T19:00:00Z','running','PRIVATE ACTIVE','2026-08-18T19:00:00Z')", [],
    ).unwrap();
    let private_origin = serde_json::json!({
        "kind": "automation",
        "automation_name": private_bundle,
        "run_id": stage8_privacy_canary_bundle(),
    })
    .to_string();
    connection
        .execute(
            "INSERT INTO pending_approvals
         (id,tool_name,description,expires_at,created_at,origin_json)
         VALUES ('approval','write',?1,'2026-08-19T00:00:00Z','2026-08-18T19:00:00Z',?2)",
            params![stage8_privacy_canary_bundle(), private_origin],
        )
        .unwrap();
    drop(connection);
    (dir, path)
}

#[test]
fn clean_and_v14() {
    let clean_dir = tempfile::tempdir().unwrap();
    let clean_path = clean_dir.path().join("clean.sqlite");
    let clean_hash = "clean-install";
    let clean = canonicalize(&clean_path, clean_hash);
    let (_v14_dir, v14_path) = v14_fixture();
    let v14_hash = sha256(&v14_path).unwrap();
    let v14 = canonicalize(&v14_path, &v14_hash);

    assert_eq!(
        canonical_schema_checksum(&clean),
        canonical_schema_checksum(&v14),
        "clean install and V14 upgrade must have one canonical schema checksum"
    );
    assert_eq!(
        automation_definition_contract(&clean_path),
        automation_definition_contract(&v14_path),
        "clean install and V14 upgrade must expose identical automation definition behavior"
    );
    for connection in [&clean, &v14] {
        let integrity: String = connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .unwrap();
        assert_eq!(integrity, "ok");
        let authority_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM current_chat_authority", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(authority_count, 1);
        let turn_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM current_chat_turns", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(turn_count, 0);
        let pending_placeholder: i64 = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='automation_session_pending_approvals')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending_placeholder, 0);
        assert_no_private_projection_values(connection);
    }

    let canonical_count: i64 = v14
        .query_row("SELECT COUNT(*) FROM automation_run_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(canonical_count, 56);
    for obsolete in [
        "automation_runs",
        "pending_approvals",
        "sessions",
        "chat_turns",
        "chat_turns_fts",
    ] {
        let exists: i64 = v14
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1)",
                [obsolete],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "obsolete table remains: {obsolete}");
    }
    let current_chat_count: i64 = v14
        .query_row("SELECT COUNT(*) FROM current_chats", [], |row| row.get(0))
        .unwrap_or(0);
    assert_eq!(
        current_chat_count, 1,
        "Current Chat authority must initialize once"
    );
    assert_eq!(
        v14.query_row("SELECT COUNT(*) FROM current_chats", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let migrated_origin: Option<String> = v14
        .query_row(
            "SELECT origin_json FROM work_approval_requests WHERE identity='approval'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let migrated_approval = migrated_origin.unwrap();
    let migrated_origin_value: serde_json::Value =
        serde_json::from_str(&migrated_approval).unwrap();
    assert_eq!(
        migrated_origin_value,
        serde_json::json!({"kind": "automation"})
    );
    assert!(!migrated_approval.contains("CANARY_PRIVATE_IDENTITY"));
    assert!(!migrated_approval.contains("CANARY_RAW_ARGUMENT"));
    assert!(!migrated_approval.contains("run_id"));
    write_stage8_privacy_capture(
        "migration.json",
        serde_json::json!({
            "surface": "migration",
            "schema_checksum": canonical_schema_checksum(&v14),
            "canonical_record_count": canonical_count,
            "integrity": "ok",
            "private_projection_matches": 0,
        }),
    );
    drop(clean);
    drop(v14);
    exercise_canonical_work_session(&clean_path);
    exercise_canonical_work_session(&v14_path);
}

#[test]
fn interrupted_migration() {
    for failpoint in [
        bagentd::cutover::Stage8MigrationFailpoint::BeforeTransaction,
        bagentd::cutover::Stage8MigrationFailpoint::DuringCopy,
        bagentd::cutover::Stage8MigrationFailpoint::AfterCommit,
        bagentd::cutover::Stage8MigrationFailpoint::BeforeRouteAdmission,
    ] {
        let (_dir, path) = v14_fixture();
        let backup_hash = sha256(&path).unwrap();
        migrate_v14_copy(&path, &backup_hash, true).unwrap();
        let result = bagentd::cutover::finalize_stage8_cleanup_with_failpoint(&path, failpoint);
        assert!(result.is_err(), "failpoint must interrupt cleanup");
        let connection = Connection::open(&path).unwrap();
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .map(|value| assert_eq!(value, "ok"))
            .unwrap();
        let records: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='automation_run_records'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        match failpoint {
            bagentd::cutover::Stage8MigrationFailpoint::BeforeTransaction
            | bagentd::cutover::Stage8MigrationFailpoint::DuringCopy => {
                assert_eq!(
                    records, 0,
                    "pre-commit failure must leave canonical schema absent"
                );
                let old_runs: i64 = connection
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='automation_runs'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(old_runs, 1, "pre-commit retry must retain the source");
            }
            bagentd::cutover::Stage8MigrationFailpoint::AfterCommit
            | bagentd::cutover::Stage8MigrationFailpoint::BeforeRouteAdmission => {
                assert_eq!(
                    records, 1,
                    "post-commit failure must retain canonical schema"
                );
                let old_runs: i64 = connection
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='automation_runs'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(
                    old_runs, 0,
                    "post-commit state must not re-admit old routes"
                );
            }
        }
        drop(connection);
        finalize_stage8_cleanup(&path).unwrap();
        finalize_stage8_cleanup(&path).unwrap();
        let connection = Connection::open(&path).unwrap();
        let converted: i64 = connection
            .query_row("SELECT COUNT(*) FROM automation_run_records", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(converted, 56, "retry must not duplicate conversion");
    }
}

#[test]
fn legacy_run_records() {
    let (_dir, path) = v14_fixture();
    let backup_hash = sha256(&path).unwrap();
    migrate_v14_copy(&path, &backup_hash, false).unwrap();
    let connection = Connection::open(&path).unwrap();
    let count: i64 = connection
        .query_row("SELECT COUNT(*) FROM legacy_run_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 50, "migration is bounded per automation");
    let safe: (String, i64) = connection.query_row(
        "SELECT summary,summary_available FROM legacy_run_records WHERE legacy_run_identity='terminal-54'", [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(safe, ("Completed successfully.".to_owned(), 1));
    let unsafe_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM legacy_run_records WHERE summary=?1 AND summary_available=0
         AND viewed=1 AND completion_attention=0 AND continuation_available=0",
            params![LEGACY_UNAVAILABLE],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unsafe_count, 49);
    let active: String = connection
        .query_row(
            "SELECT status FROM automation_runs WHERE id='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        active, "running",
        "active row waits for safe changed-PID boundary"
    );
    drop(connection);

    migrate_v14_copy(&path, &backup_hash, true).unwrap();
    migrate_v14_copy(&path, &backup_hash, true).unwrap();
    let connection = Connection::open(&path).unwrap();
    let count_after: i64 = connection
        .query_row("SELECT COUNT(*) FROM legacy_run_records", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count_after, 50, "rerun is idempotent and stays bounded");
    let active: String = connection
        .query_row(
            "SELECT status FROM automation_runs WHERE id='active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active, "abandoned");
    let decision: String = connection
        .query_row(
            "SELECT decision FROM pending_approvals WHERE id='approval'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(decision, "abandoned");
    assert_no_private_projection_values(&connection);
}

fn old_binary_can_open(path: &std::path::Path) -> bool {
    Connection::open(path)
        .and_then(|connection| {
            connection.query_row("SELECT COUNT(*) FROM automation_runs", [], |row| {
                row.get::<_, i64>(0)
            })
        })
        .is_ok()
}

fn old_binary_refuses_canonical(path: &std::path::Path) -> bool {
    !old_binary_can_open(path)
}

fn new_binary_can_open(path: &std::path::Path) -> bool {
    Connection::open(path)
        .and_then(|connection| {
            connection.query_row("SELECT COUNT(*) FROM automation_run_records", [], |row| {
                row.get::<_, i64>(0)
            })
        })
        .is_ok()
}

#[test]
fn cutover_boundary() {
    let (dir, path) = v14_fixture();
    let backup = dir.path().join("pre-cutover.sqlite");
    let backup_hash = verified_backup(&path, &backup).unwrap();
    assert!(old_binary_can_open(&backup));

    let dry_run = dir.path().join("dry-run.sqlite");
    verified_restore(&backup, &dry_run, &backup_hash).unwrap();
    migrate_v14_copy(&dry_run, &backup_hash, true).unwrap();
    assert_eq!(
        rollback_boundary(&dry_run).unwrap(),
        RollbackBoundary::PreFirstWork
    );

    let interrupted = dir.path().join("interrupted.sqlite");
    std::fs::write(&interrupted, b"interrupted-before-commit").unwrap();
    verified_restore(&backup, &interrupted, &backup_hash).unwrap();
    assert_eq!(sha256(&interrupted).unwrap(), backup_hash);

    migrate_v14_copy(&path, &backup_hash, true).unwrap();
    finalize_stage8_cleanup(&path).unwrap();
    assert_eq!(
        rollback_boundary(&path).unwrap(),
        RollbackBoundary::PreFirstWork
    );
    assert!(new_binary_can_open(&path));
    assert!(old_binary_refuses_canonical(&path));
    println!("A54 pre-Work backup SHA-256: {backup_hash}");
    let restored = dir.path().join("restored-old.sqlite");
    verified_restore(&backup, &restored, &backup_hash).unwrap();
    assert!(old_binary_can_open(&restored));

    mark_first_post_cutover_work(&path, "2026-08-18T20:00:00Z").unwrap();
    assert_eq!(
        rollback_boundary(&path).unwrap(),
        RollbackBoundary::ForwardOnly
    );
    let archive = dir.path().join("forward-archive.sqlite");
    std::fs::copy(&path, &archive).unwrap();
    let archive_hash = sha256(&archive).unwrap();
    assert!(new_binary_can_open(&archive));
    assert!(old_binary_refuses_canonical(&archive));
    verified_restore(&backup, &restored, &backup_hash).unwrap();
    assert!(archive.exists());
    assert!(
        old_binary_can_open(&restored),
        "downgrade uses the verified old backup"
    );
    assert_eq!(sha256(&archive).unwrap(), archive_hash);
    assert_ne!(
        archive_hash, backup_hash,
        "forward records remain archived and disclosed"
    );
    write_stage8_privacy_capture(
        "rollback.json",
        serde_json::json!({
            "surface": "rollback",
            "pre_work_boundary": "pre_first_work",
            "post_work_boundary": "forward_only",
            "old_backup_restored": true,
            "forward_archive_preserved": true,
            "private_projection_matches": 0,
        }),
    );
    println!("A54 post-Work canonical archive SHA-256: {archive_hash}");
}
