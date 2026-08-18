use bagentd::cutover::{
    mark_first_post_cutover_work, migrate_v14_copy, rollback_boundary, sha256, verified_backup,
    verified_restore, RollbackBoundary, LEGACY_UNAVAILABLE,
};
use rusqlite::{params, Connection};
use tempfile::TempDir;

fn v14_fixture() -> (TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v14.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "PRAGMA journal_mode=DELETE;
         CREATE TABLE automations (id TEXT PRIMARY KEY, name TEXT NOT NULL);
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
        )
        .unwrap();
    connection
        .execute("INSERT INTO automations VALUES ('a','A')", [])
        .unwrap();
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
                        "Completed successfully."
                    } else {
                        "CANARY raw evidence token"
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
    connection.execute(
        "INSERT INTO pending_approvals
         (id,tool_name,description,expires_at,created_at,origin_json)
         VALUES ('approval','write','CANARY raw args','2026-08-19T00:00:00Z','2026-08-18T19:00:00Z','{}')", [],
    ).unwrap();
    drop(connection);
    (dir, path)
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
    assert!(!format!("{connection:?}").contains("CANARY"));
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
    assert_eq!(
        rollback_boundary(&path).unwrap(),
        RollbackBoundary::PreFirstWork
    );
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
    verified_restore(&backup, &restored, &backup_hash).unwrap();
    assert!(archive.exists());
    assert!(old_binary_can_open(&restored));
    assert_ne!(
        sha256(&archive).unwrap(),
        backup_hash,
        "forward records remain archived and disclosed"
    );
}
