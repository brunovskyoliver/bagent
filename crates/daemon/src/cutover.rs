//! Forward-only Stage 4 schema cutover helpers.
//!
//! All functions require explicit paths so production callers cannot silently
//! fall back to the user's live database. Acceptance tests use disposable V14
//! copies and compare hashes around every backup/restore operation.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::Path};
#[cfg(feature = "stage8-acceptance")]
use std::{path::PathBuf, thread, time::Duration};

const WORK_SCHEMA: &str = include_str!("../migrations/V15__work_coordinator_foundations.sql");
const CUTOVER_SCHEMA: &str = include_str!("../migrations/V16__unified_work_cutover.sql");
const STAGE8_SCHEMA: &str = include_str!("../migrations/V23__stage8_canonical_cleanup.sql");
pub const LEGACY_UNAVAILABLE: &str =
    "Legacy result content is unavailable because its privacy provenance cannot be verified.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage8MigrationFailpoint {
    BeforeTransaction,
    DuringCopy,
    AfterCommit,
    BeforeRouteAdmission,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackBoundary {
    PreFirstWork,
    ForwardOnly,
}

/// Pauses an acceptance-only daemon at a named external process-kill seam.
/// Ordinary builds do not compile this hook, so environment variables cannot
/// reactivate it in production.
#[cfg(feature = "stage8-acceptance")]
pub fn stage8_migration_killpoint(name: &str) -> Result<(), String> {
    if std::env::var("BAGENT_STAGE8_MIGRATION_KILLPOINT").as_deref() != Ok(name) {
        return Ok(());
    }
    let marker_dir = std::env::var_os("BAGENT_STAGE8_MIGRATION_KILLPOINT_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| "Stage 8 migration killpoint marker directory is missing".to_owned())?;
    fs::create_dir_all(&marker_dir).map_err(storage)?;
    fs::write(
        marker_dir.join(format!("{name}.ready")),
        std::process::id().to_string(),
    )
    .map_err(storage)?;
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

fn storage(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub fn sha256(path: &Path) -> Result<String, String> {
    fs::read(path)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(storage)
}

pub fn verified_backup(source: &Path, backup: &Path) -> Result<String, String> {
    if source == backup {
        return Err("backup path must differ from source".to_owned());
    }
    fs::copy(source, backup).map_err(storage)?;
    let source_hash = sha256(source)?;
    let backup_hash = sha256(backup)?;
    if source_hash != backup_hash {
        return Err("backup checksum mismatch".to_owned());
    }
    Ok(backup_hash)
}

pub fn prepare_pre_cutover_backup(source: &Path, backup: &Path) -> Result<Option<String>, String> {
    if !source.exists() {
        return Ok(None);
    }
    let connection = Connection::open(source).map_err(storage)?;
    let version = connection
        .query_row(
            "SELECT MAX(version) FROM refinery_schema_history",
            [],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(storage)?
        .flatten()
        .unwrap_or(0);
    if version >= 16 {
        return Ok(None);
    }
    if backup.exists() {
        return sha256(backup).map(Some);
    }
    let target = backup.to_string_lossy().replace('\'', "''");
    connection
        .execute_batch(&format!("VACUUM INTO '{target}'"))
        .map_err(storage)?;
    let backup_hash = sha256(backup)?;
    let check = Connection::open(backup)
        .map_err(storage)?
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map_err(storage)?;
    if check != "ok" {
        return Err("pre-cutover backup failed integrity check".to_owned());
    }
    Ok(Some(backup_hash))
}

pub fn finalize_legacy_boundary(path: &Path) -> Result<(), String> {
    let connection = Connection::open(path).map_err(storage)?;
    let backup_hash = connection
        .query_row(
            "SELECT pre_cutover_backup_sha256 FROM work_cutover WHERE singleton=1",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(storage)?
        .flatten()
        .unwrap_or_else(|| "fresh-install".to_owned());
    drop(connection);
    migrate_v14_copy(path, &backup_hash, true)?;
    finalize_stage8_cleanup(path)
}

pub fn record_pre_cutover_backup(path: &Path, backup_hash: &str) -> Result<(), String> {
    let connection = Connection::open(path).map_err(storage)?;
    connection
        .execute(
            "UPDATE work_cutover SET pre_cutover_backup_sha256=?1 WHERE singleton=1",
            params![backup_hash],
        )
        .map_err(storage)?;
    Ok(())
}

pub fn verified_restore(backup: &Path, destination: &Path, expected: &str) -> Result<(), String> {
    if sha256(backup)? != expected {
        return Err("backup checksum changed".to_owned());
    }
    fs::copy(backup, destination).map_err(storage)?;
    if sha256(destination)? != expected {
        return Err("restore checksum mismatch".to_owned());
    }
    Ok(())
}

fn safe_legacy_summary(summary: Option<String>) -> (String, bool) {
    let Some(summary) = summary else {
        return (LEGACY_UNAVAILABLE.to_owned(), false);
    };
    let normalized = summary.trim();
    let allowed = matches!(
        normalized,
        "Completed successfully."
            | "Completed with partial results."
            | "Execution failed."
            | "Execution was cancelled."
            | "Daemon restarted during execution."
    );
    if allowed {
        (normalized.to_owned(), true)
    } else {
        (LEGACY_UNAVAILABLE.to_owned(), false)
    }
}

pub fn migrate_v14_copy(
    path: &Path,
    backup_sha256: &str,
    safe_changed_pid_boundary: bool,
) -> Result<(), String> {
    let mut connection = Connection::open(path).map_err(storage)?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(storage)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    transaction.execute_batch(WORK_SCHEMA).map_err(storage)?;
    let has_decision_revision = {
        let mut statement = transaction
            .prepare("PRAGMA table_info(work_approvals)")
            .map_err(storage)?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        columns.iter().any(|column| column == "decision_revision")
    };
    if !has_decision_revision {
        transaction.execute_batch(CUTOVER_SCHEMA).map_err(storage)?;
    }
    transaction
        .execute(
            "UPDATE work_cutover SET pre_cutover_backup_sha256=?1 WHERE singleton=1",
            params![backup_sha256],
        )
        .map_err(storage)?;

    let has_legacy_runs = transaction
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='automation_runs'")
        .and_then(|mut statement| statement.exists([]))
        .map_err(storage)?;
    if safe_changed_pid_boundary && has_legacy_runs {
        transaction.execute(
            "UPDATE automation_runs SET status='abandoned', finished_at=COALESCE(finished_at, created_at),
             result_summary='Daemon restarted during execution.' WHERE status='running'",
            [],
        ).map_err(storage)?;
        let has_pending = transaction
            .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name='pending_approvals'")
            .and_then(|mut statement| statement.exists([]))
            .map_err(storage)?;
        if has_pending {
            transaction.execute(
                "UPDATE pending_approvals SET decision='abandoned', decided_at=COALESCE(decided_at, created_at)
                 WHERE decision IS NULL", [],
            ).map_err(storage)?;
        }
    }

    let rows = if has_legacy_runs {
        let mut statement = transaction
            .prepare(
                "SELECT id, automation_id, status, result_summary, created_at, finished_at
             FROM automation_runs
             WHERE status IN ('completed','partial','failed','cancelled','abandoned')
             ORDER BY automation_id ASC, created_at DESC, id ASC",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        rows
    } else {
        Vec::new()
    };
    let mut counts = HashMap::<String, usize>::new();
    for (id, automation, outcome, summary, created, finished) in rows {
        let count = counts.entry(automation.clone()).or_default();
        if *count >= 50 {
            continue;
        }
        *count += 1;
        let (summary, available) = safe_legacy_summary(summary);
        transaction
            .execute(
                "INSERT OR IGNORE INTO legacy_run_records
             (legacy_run_identity, historical_automation_identity, outcome, summary,
              summary_available, viewed, completion_attention, continuation_available,
              created_at, finished_at)
             VALUES (?1,?2,?3,?4,?5,1,0,0,?6,?7)",
                params![
                    id,
                    automation,
                    outcome,
                    summary,
                    available as i64,
                    created,
                    finished
                ],
            )
            .map_err(storage)?;
    }
    transaction
        .execute(
            "DELETE FROM legacy_run_records WHERE legacy_run_identity IN (
           SELECT legacy_run_identity FROM (
             SELECT legacy_run_identity,
                    ROW_NUMBER() OVER (
                      PARTITION BY historical_automation_identity
                      ORDER BY created_at DESC, legacy_run_identity ASC
                    ) AS retained_position
             FROM legacy_run_records
           ) WHERE retained_position > 50
         )",
            [],
        )
        .map_err(storage)?;
    transaction.commit().map_err(storage)
}

fn safe_approval_description(tool_name: &str, _description: &str) -> String {
    // Approval presentations are status data, not a second raw-argument
    // channel. The detailed proposal stays in the short-lived tool call.
    format!("Approval required for {}", tool_name.trim())
}

fn safe_approval_origin(raw: Option<&str>) -> Option<String> {
    let value = raw.and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())?;
    let kind = value.get("kind").and_then(serde_json::Value::as_str)?;
    if kind != "automation" {
        return None;
    }
    // The approval projection needs provenance kind only. Automation names
    // are user-authored identities and must not cross the migration boundary.
    Some(serde_json::json!({"kind": kind}).to_string())
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(storage)
}

fn table_exists_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    table: &str,
) -> Result<bool, String> {
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [table],
            |row| row.get(0),
        )
        .map_err(storage)
}

pub fn finalize_stage8_cleanup(path: &Path) -> Result<(), String> {
    finalize_stage8_cleanup_with_optional_failpoint(path, None)
}

pub fn finalize_stage8_cleanup_with_failpoint(
    path: &Path,
    failpoint: Stage8MigrationFailpoint,
) -> Result<(), String> {
    finalize_stage8_cleanup_with_optional_failpoint(path, Some(failpoint))
}

fn finalize_stage8_cleanup_with_optional_failpoint(
    path: &Path,
    failpoint: Option<Stage8MigrationFailpoint>,
) -> Result<(), String> {
    if failpoint == Some(Stage8MigrationFailpoint::BeforeTransaction) {
        return Err("Stage 8 failpoint before transaction".to_owned());
    }
    let mut connection = Connection::open(path).map_err(storage)?;
    connection
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(storage)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage)?;
    transaction.execute_batch(STAGE8_SCHEMA).map_err(storage)?;

    let has_legacy_runs = table_exists_in_transaction(&transaction, "automation_runs")?;
    if has_legacy_runs {
        let mut statement = transaction
            .prepare(
                "SELECT id, automation_id, scheduled_for, started_at, finished_at,
                        status, result_summary, is_catch_up, is_manual, created_at
                 FROM automation_runs ORDER BY created_at ASC, id ASC",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                ))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        drop(statement);
        for (
            id,
            automation_id,
            scheduled_for,
            started_at,
            finished_at,
            status,
            summary,
            is_catch_up,
            is_manual,
            created_at,
        ) in rows
        {
            let (safe_summary, available) = safe_legacy_summary(summary);
            transaction
                .execute(
                    "INSERT OR IGNORE INTO automation_run_records
                     (id, automation_id, scheduled_for, started_at, finished_at, status,
                      result_summary, is_catch_up, is_manual, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        id,
                        automation_id,
                        scheduled_for,
                        started_at,
                        finished_at,
                        status,
                        safe_summary,
                        is_catch_up,
                        is_manual,
                        created_at,
                    ],
                )
                .map_err(storage)?;
            let _ = available;
        }
    }

    #[cfg(feature = "stage8-acceptance")]
    stage8_migration_killpoint("during-copy")?;

    if failpoint == Some(Stage8MigrationFailpoint::DuringCopy) {
        return Err("Stage 8 failpoint during copy".to_owned());
    }

    if table_exists_in_transaction(&transaction, "pending_approvals")? {
        let mut statement = transaction
            .prepare(
                "SELECT p.id, p.tool_name, p.description, p.expires_at, p.created_at,
                        p.origin_json, a.work_identity
                 FROM pending_approvals p
                 LEFT JOIN work_approvals a ON a.identity=p.id",
            )
            .map_err(storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        drop(statement);
        for (id, tool, description, expires, created, origin, work) in rows {
            let safe_description = safe_approval_description(&tool, &description);
            let safe_origin = safe_approval_origin(origin.as_deref());
            transaction
                .execute(
                    "INSERT OR IGNORE INTO work_approval_requests
                     (identity, work_identity, tool_name, description, expires_at,
                      created_at, origin_json, decision, decided_at)
                     SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, p.decision, p.decided_at
                     FROM pending_approvals p WHERE p.id=?1",
                    params![
                        id,
                        work,
                        tool,
                        safe_description,
                        expires,
                        created,
                        safe_origin
                    ],
                )
                .map_err(storage)?;
        }
    }

    // Drop FTS triggers before their external-content table, then remove the
    // old lifecycle tables. `legacy_run_records` is intentionally retained.
    for trigger in ["chat_turns_ai", "chat_turns_ad", "chat_turns_au"] {
        transaction
            .execute_batch(&format!("DROP TRIGGER IF EXISTS {trigger};"))
            .map_err(storage)?;
    }
    for table in [
        "chat_turns_fts",
        "chat_turns",
        "sessions",
        "pending_approvals",
        "automation_session_pending_approvals",
        "automation_runs",
    ] {
        transaction
            .execute_batch(&format!("DROP TABLE IF EXISTS {table};"))
            .map_err(storage)?;
    }
    transaction
        .execute(
            "UPDATE stage8_cleanup_state SET committed_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')
             WHERE singleton=1",
            [],
        )
        .map_err(storage)?;
    transaction.commit().map_err(storage)?;

    #[cfg(feature = "stage8-acceptance")]
    stage8_migration_killpoint("after-commit")?;

    if failpoint == Some(Stage8MigrationFailpoint::AfterCommit) {
        return Err("Stage 8 failpoint after commit".to_owned());
    }
    if failpoint == Some(Stage8MigrationFailpoint::BeforeRouteAdmission) {
        return Err("Stage 8 failpoint before route admission".to_owned());
    }
    if !table_exists(&connection, "automation_run_records")? {
        return Err("canonical automation run records are missing".to_owned());
    }
    Ok(())
}

pub fn mark_first_post_cutover_work(path: &Path, committed_at: &str) -> Result<(), String> {
    let connection = Connection::open(path).map_err(storage)?;
    connection.execute(
        "UPDATE work_cutover SET first_post_cutover_work_at=COALESCE(first_post_cutover_work_at, ?1)
         WHERE singleton=1", params![committed_at],
    ).map_err(storage)?;
    Ok(())
}

pub fn rollback_boundary(path: &Path) -> Result<RollbackBoundary, String> {
    let connection = Connection::open(path).map_err(storage)?;
    let marker = connection
        .query_row(
            "SELECT first_post_cutover_work_at FROM work_cutover WHERE singleton=1",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(storage)?
        .flatten();
    Ok(if marker.is_some() {
        RollbackBoundary::ForwardOnly
    } else {
        RollbackBoundary::PreFirstWork
    })
}
