//! Forward-only Stage 4 schema cutover helpers.
//!
//! All functions require explicit paths so production callers cannot silently
//! fall back to the user's live database. Acceptance tests use disposable V14
//! copies and compare hashes around every backup/restore operation.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs, path::Path};

const WORK_SCHEMA: &str = include_str!("../migrations/V15__work_coordinator_foundations.sql");
const CUTOVER_SCHEMA: &str = include_str!("../migrations/V16__unified_work_cutover.sql");
pub const LEGACY_UNAVAILABLE: &str =
    "Legacy result content is unavailable because its privacy provenance cannot be verified.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackBoundary {
    PreFirstWork,
    ForwardOnly,
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
    migrate_v14_copy(path, &backup_hash, true)
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

    if safe_changed_pid_boundary {
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

    let rows = {
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
