//! Atomic persistence for the product's Automation Session contract.
//!
//! This module deliberately keeps Automation Definition, Automation Run,
//! Automation Session, Task Snapshot, and Completion Attention as separate
//! records. Terminal content is written in one SQLite transaction and has no
//! update path; only Completion Attention can change afterwards.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt};
use uuid::Uuid;

pub const MAX_TASK_TEXT_CHARS: usize = 4_000;
pub const MAX_FINAL_OUTPUT_BYTES: usize = 64 * 1024;
pub const MAX_RESULT_SUMMARY_CHARS: usize = 500;
pub const MAX_ACTIVITY_COUNT: usize = 256;
pub const MAX_ACTIVITY_ENCODED_BYTES: usize = 128 * 1024;
pub const MAX_VALIDATED_SOURCES: usize = 32;
pub const MAX_CONNECTOR_REFERENCES: usize = 32;
pub const MAX_APPROVAL_RECORDS: usize = 64;
pub const MAX_CONTINUATION_SEED_BYTES: usize = 16 * 1024;

const TEXT_OMISSION_MARKER: &str = "\n[… obsah vynechaný …]\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationRunOutcome {
    Completed,
    Partial,
    Failed,
    Skipped,
    Cancelled,
    Abandoned,
}

impl AutomationRunOutcome {
    pub fn all() -> [Self; 6] {
        [
            Self::Completed,
            Self::Partial,
            Self::Failed,
            Self::Skipped,
            Self::Cancelled,
            Self::Abandoned,
        ]
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }

    fn creates_attention(self) -> bool {
        !matches!(self, Self::Skipped)
    }
}

impl std::str::FromStr for AutomationRunOutcome {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "completed" => Ok(Self::Completed),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            "skipped" => Ok(Self::Skipped),
            "cancelled" => Ok(Self::Cancelled),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(format!("unknown automation run outcome: {other}")),
        }
    }
}

/// Captured at claim time or when a scheduler-only skip is recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationTaskSnapshot {
    pub automation_identity: String,
    pub automation_run_identity: String,
    pub automation_session_identity: String,
    pub display_name: String,
    pub task_text: String,
    /// Structured schedule JSON captured verbatim from the definition.
    pub schedule_json: String,
    pub timezone: String,
    pub definition_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeActivity {
    pub category: String,
    pub caption: String,
    pub safety_relevant: bool,
}

/// A source label and stable public identity only. Fetched passages, signed
/// URLs, credentials, and connector-private identities never enter this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValidatedSource {
    pub source_identity: String,
    pub label: String,
}

/// Connector availability without an opaque reference token or connector
/// identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorReference {
    pub connector_kind: String,
    pub availability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalApproval {
    pub category: String,
    pub side_effect_class: String,
    pub occurred_at: String,
    pub resolution: String,
    pub origin: String,
    pub session_scoped_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TruncationDisclosure {
    pub section: String,
    pub original_extent: usize,
    pub retained_extent: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AutomationTerminalization {
    pub snapshot: AutomationTaskSnapshot,
    pub work_identity: String,
    pub outcome: AutomationRunOutcome,
    pub finished_at: String,
    pub result_summary: Option<String>,
    pub final_output: Option<String>,
    pub activity_timeline: Vec<SafeActivity>,
    pub validated_sources: Vec<ValidatedSource>,
    pub connector_references: Vec<ConnectorReference>,
    pub historical_approvals: Vec<HistoricalApproval>,
    pub truncation_disclosures: Vec<TruncationDisclosure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersistedAutomationSession {
    pub task_snapshot: AutomationTaskSnapshot,
    pub outcome: AutomationRunOutcome,
    pub finished_at: String,
    pub result_summary: Option<String>,
    pub final_output: Option<String>,
    pub final_output_available: bool,
    pub activity_timeline: Vec<SafeActivity>,
    pub validated_sources: Vec<ValidatedSource>,
    pub connector_references: Vec<ConnectorReference>,
    pub historical_approvals: Vec<HistoricalApproval>,
    pub truncation_disclosures: Vec<TruncationDisclosure>,
    pub attention: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContinuationProvenance {
    pub identity: String,
    pub source_automation_session_identity: String,
    pub target_current_chat_identity: String,
    pub seed: String,
    pub source_deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionCleanupCounts {
    pub sessions_deleted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutomationSessionError {
    Invalid(String),
    Immutable,
    NotFound,
    ActiveWork(String),
    Storage(String),
}

impl fmt::Display for AutomationSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "invalid automation session: {message}"),
            Self::Immutable => write!(f, "automation session is immutable after terminalization"),
            Self::NotFound => write!(f, "automation session not found"),
            Self::ActiveWork(message) => write!(f, "work is not terminalizable: {message}"),
            Self::Storage(message) => write!(f, "automation session storage error: {message}"),
        }
    }
}

impl Error for AutomationSessionError {}

impl From<rusqlite::Error> for AutomationSessionError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Storage(value.to_string())
    }
}

pub fn initialize_schema(connection: &Connection) -> Result<(), AutomationSessionError> {
    crate::current_chat::initialize_schema(connection)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS automation_work_states (
            work_identity TEXT PRIMARY KEY,
            automation_run_identity TEXT NOT NULL UNIQUE,
            state TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_task_snapshots (
            automation_session_identity TEXT PRIMARY KEY,
            automation_run_identity TEXT NOT NULL UNIQUE,
            automation_identity TEXT NOT NULL,
            display_name TEXT NOT NULL,
            task_text TEXT NOT NULL,
            schedule_json TEXT NOT NULL,
            timezone TEXT NOT NULL,
            definition_revision INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_run_outcomes (
            automation_run_identity TEXT PRIMARY KEY,
            automation_session_identity TEXT NOT NULL UNIQUE,
            outcome TEXT NOT NULL,
            finished_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_sessions (
            automation_session_identity TEXT PRIMARY KEY,
            automation_run_identity TEXT NOT NULL UNIQUE,
            outcome TEXT NOT NULL,
            finished_at TEXT NOT NULL,
            result_summary TEXT,
            final_output TEXT,
            final_output_available INTEGER NOT NULL,
            activity_timeline_json TEXT NOT NULL,
            truncation_disclosures_json TEXT NOT NULL,
            validated_sources_json TEXT NOT NULL,
            connector_references_json TEXT NOT NULL,
            historical_approvals_json TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_session_attention (
            automation_session_identity TEXT PRIMARY KEY,
            attention_state TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_session_open_commands (
            command_identity TEXT PRIMARY KEY,
            automation_session_identity TEXT NOT NULL,
            expected_revision INTEGER NOT NULL,
            committed_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_terminal_outbox (
            automation_session_identity TEXT PRIMARY KEY,
            automation_run_identity TEXT NOT NULL UNIQUE,
            outcome TEXT NOT NULL,
            emitted_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_definitions (
            automation_identity TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS automation_continuation_provenance (
            identity TEXT PRIMARY KEY,
            source_automation_session_identity TEXT NOT NULL,
            target_current_chat_identity TEXT NOT NULL UNIQUE,
            command_identity TEXT NOT NULL UNIQUE,
            seed TEXT NOT NULL,
            seed_bytes INTEGER NOT NULL CHECK (seed_bytes <= 16384),
            source_deleted INTEGER NOT NULL DEFAULT 0 CHECK (source_deleted IN (0, 1)),
            created_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_session_pending_approvals (
            automation_session_identity TEXT PRIMARY KEY
        );
        CREATE TABLE IF NOT EXISTS automation_retention_audit (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            deleted_count INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS automation_session_tombstones (
            automation_session_identity TEXT PRIMARY KEY,
            deleted_at TEXT NOT NULL,
            former_outcome TEXT NOT NULL
        );",
    )?;
    ensure_safe_record_columns(connection)?;
    Ok(())
}

fn ensure_safe_record_columns(connection: &Connection) -> Result<(), AutomationSessionError> {
    let mut statement = connection.prepare("PRAGMA table_info(automation_sessions)")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    for column in [
        "validated_sources_json",
        "connector_references_json",
        "historical_approvals_json",
    ] {
        if !columns.iter().any(|existing| existing == column) {
            connection.execute(
                &format!(
                    "ALTER TABLE automation_sessions ADD COLUMN {column} TEXT NOT NULL DEFAULT '[]'"
                ),
                [],
            )?;
        }
    }
    Ok(())
}

pub fn register_definition(
    connection: &Connection,
    automation_identity: &str,
) -> Result<(), AutomationSessionError> {
    connection.execute(
        "INSERT INTO automation_definitions (automation_identity) VALUES (?1)",
        params![automation_identity],
    )?;
    Ok(())
}

pub fn delete_automation_definition(
    connection: &Connection,
    automation_identity: &str,
) -> Result<(), AutomationSessionError> {
    let active: Option<String> = connection
        .query_row(
            "SELECT state FROM automation_work_states w
             JOIN automation_task_snapshots t ON t.automation_run_identity=w.automation_run_identity
             WHERE t.automation_identity=?1 AND w.state NOT LIKE 'terminal:%' LIMIT 1",
            params![automation_identity],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(state) = active {
        return Err(AutomationSessionError::ActiveWork(state));
    }
    let changed = connection.execute(
        "DELETE FROM automation_definitions WHERE automation_identity=?1",
        params![automation_identity],
    )?;
    if changed == 0 {
        return Err(AutomationSessionError::NotFound);
    }
    Ok(())
}

pub fn continue_automation_session_in_new_chat(
    connection: &Connection,
    automation_session_identity: &str,
    seed: &str,
    confirmed_replacement: bool,
    command_identity: &str,
) -> Result<ContinuationProvenance, AutomationSessionError> {
    if seed.len() > MAX_CONTINUATION_SEED_BYTES {
        return Err(AutomationSessionError::Invalid(
            "continuation seed exceeds 16 KiB".to_owned(),
        ));
    }
    let existing: Option<ContinuationProvenance> = connection
        .query_row(
            "SELECT identity, source_automation_session_identity,
                    target_current_chat_identity, seed, source_deleted
             FROM automation_continuation_provenance WHERE command_identity=?1",
            params![command_identity],
            |row| {
                Ok(ContinuationProvenance {
                    identity: row.get(0)?,
                    source_automation_session_identity: row.get(1)?,
                    target_current_chat_identity: row.get(2)?,
                    seed: row.get(3)?,
                    source_deleted: row.get::<_, i64>(4)? != 0,
                })
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        if existing.source_automation_session_identity == automation_session_identity
            && existing.seed == seed
        {
            return Ok(existing);
        }
        return Err(AutomationSessionError::Invalid(
            "continuation command identity was reused with different arguments".to_owned(),
        ));
    }
    let outcome: String = connection
        .query_row(
            "SELECT outcome FROM automation_sessions WHERE automation_session_identity=?1",
            params![automation_session_identity],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(AutomationSessionError::NotFound)?;
    if outcome == AutomationRunOutcome::Skipped.as_str() {
        return Err(AutomationSessionError::Invalid(
            "scheduler-only skipped Automation Sessions cannot continue".to_owned(),
        ));
    }
    let transaction = connection.unchecked_transaction()?;
    let current_chat_identity: String = transaction.query_row(
        "SELECT current_chat_identity FROM current_chat_authority WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    let content_count: i64 = transaction.query_row(
        "SELECT
            (SELECT COUNT(*) FROM current_chat_turns WHERE current_chat_identity=?1)
            + (SELECT COUNT(*) FROM current_chat_drafts WHERE current_chat_identity=?1)
            + (SELECT COUNT(*) FROM current_chat_submitted_attachments WHERE current_chat_identity=?1)
            + (SELECT COUNT(*) FROM current_chat_validated_sources WHERE current_chat_identity=?1)
            + (SELECT COUNT(*) FROM current_chat_connector_references WHERE current_chat_identity=?1)
            + (SELECT COUNT(*) FROM current_chat_approval_presentations WHERE current_chat_identity=?1)
            + (SELECT COUNT(*) FROM automation_continuation_provenance
               WHERE target_current_chat_identity=?1)",
        params![current_chat_identity],
        |row| row.get(0),
    )?;
    if content_count > 0 && !confirmed_replacement {
        return Err(AutomationSessionError::Invalid(
            "Current Chat is not empty; replacement confirmation is required".to_owned(),
        ));
    }

    let active_turn: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM current_chat_turns
         WHERE current_chat_identity=?1 AND state='active'",
        params![current_chat_identity],
        |row| row.get(0),
    )?;
    if active_turn > 0 {
        return Err(AutomationSessionError::ActiveWork(
            "Current Chat has an active Conversation Turn".to_owned(),
        ));
    }

    let identity = format!("continuation:{command_identity}");
    let replacement_identity = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    transaction.execute(
        "INSERT INTO current_chats
         (identity, revision, turn_count, content_bytes, created_at, updated_at)
         VALUES (?1, 1, 0, 0, ?2, ?2)",
        params![replacement_identity, now],
    )?;
    crate::work_coordinator::insert_work_current_chat_if_present(
        &transaction,
        &replacement_identity,
    )?;
    transaction.execute(
        "INSERT INTO automation_continuation_provenance
         (identity, source_automation_session_identity, target_current_chat_identity,
          command_identity, seed, seed_bytes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            identity,
            automation_session_identity,
            replacement_identity,
            command_identity,
            seed,
            seed.len(),
            now,
        ],
    )?;
    transaction.execute(
        "UPDATE automation_session_attention SET attention_state='viewed'
         WHERE automation_session_identity=?1",
        params![automation_session_identity],
    )?;
    crate::work_coordinator::mark_work_automation_session_viewed_if_present(
        &transaction,
        automation_session_identity,
    )?;
    transaction.execute(
        "UPDATE current_chat_authority SET current_chat_identity=?1 WHERE singleton=1",
        params![replacement_identity],
    )?;
    transaction.execute(
        "DELETE FROM current_chats WHERE identity=?1",
        params![current_chat_identity],
    )?;
    crate::current_chat::refresh_retained_content_bytes(&transaction, &replacement_identity)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?;
    transaction.commit()?;
    Ok(ContinuationProvenance {
        identity: format!("continuation:{command_identity}"),
        source_automation_session_identity: automation_session_identity.to_owned(),
        target_current_chat_identity: replacement_identity,
        seed: seed.to_owned(),
        source_deleted: false,
    })
}

pub fn read_continuation_provenance(
    connection: &Connection,
    current_chat_identity: &str,
) -> Result<Option<ContinuationProvenance>, AutomationSessionError> {
    connection
        .query_row(
            "SELECT identity, source_automation_session_identity,
                    target_current_chat_identity, seed, source_deleted
             FROM automation_continuation_provenance
             WHERE target_current_chat_identity=?1",
            params![current_chat_identity],
            |row| {
                Ok(ContinuationProvenance {
                    identity: row.get(0)?,
                    source_automation_session_identity: row.get(1)?,
                    target_current_chat_identity: row.get(2)?,
                    seed: row.get(3)?,
                    source_deleted: row.get::<_, i64>(4)? != 0,
                })
            },
        )
        .optional()
        .map_err(AutomationSessionError::from)
}

pub fn register_work(
    connection: &Connection,
    work_identity: &str,
    automation_run_identity: &str,
) -> Result<(), AutomationSessionError> {
    connection.execute(
        "INSERT INTO automation_work_states (work_identity, automation_run_identity, state)
         VALUES (?1, ?2, 'running')",
        params![work_identity, automation_run_identity],
    )?;
    Ok(())
}

pub fn decode_terminalization(
    bytes: &[u8],
) -> Result<AutomationTerminalization, AutomationSessionError> {
    serde_json::from_slice(bytes).map_err(|error| {
        AutomationSessionError::Invalid(format!("unknown or invalid field: {error}"))
    })
}

pub fn terminalize_automation_session(
    connection: &Connection,
    input: AutomationTerminalization,
) -> Result<(), AutomationSessionError> {
    let transaction = connection.unchecked_transaction()?;
    terminalize_automation_session_in_transaction(&transaction, input)?;
    transaction.commit()?;
    Ok(())
}

/// Commit session data while the caller owns the surrounding SQLite
/// transaction. WorkCoordinator uses this seam so Work terminal state, the
/// session, the legacy run outcome, and both outboxes cannot split apart.
pub(crate) fn terminalize_automation_session_in_transaction(
    transaction: &Transaction<'_>,
    input: AutomationTerminalization,
) -> Result<(), AutomationSessionError> {
    validate_input(&input)?;
    let bounded = bounded_input(input)?;
    ensure_safe_record_columns(transaction)?;

    let existing: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM automation_sessions WHERE automation_session_identity=?1",
            params![bounded.snapshot.automation_session_identity],
            |row| row.get(0),
        )
        .optional()?;
    if existing.is_some() {
        return Err(AutomationSessionError::Immutable);
    }

    let current_state: Option<String> = transaction
        .query_row(
            "SELECT state FROM automation_work_states WHERE work_identity=?1
             AND automation_run_identity=?2",
            params![
                bounded.work_identity,
                bounded.snapshot.automation_run_identity
            ],
            |row| row.get(0),
        )
        .optional()?;
    match current_state.as_deref() {
        Some(state) if !is_terminal_state(state) => {}
        Some(state) => return Err(AutomationSessionError::ActiveWork(state.to_owned())),
        None => return Err(AutomationSessionError::NotFound),
    }

    let snapshot = &bounded.snapshot;
    transaction.execute(
        "INSERT OR IGNORE INTO automation_task_snapshots
         (automation_session_identity, automation_run_identity, automation_identity,
          display_name, task_text, schedule_json, timezone, definition_revision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            snapshot.automation_session_identity,
            snapshot.automation_run_identity,
            snapshot.automation_identity,
            snapshot.display_name,
            snapshot.task_text,
            snapshot.schedule_json,
            snapshot.timezone,
            snapshot.definition_revision,
        ],
    )?;
    let activity_json = serde_json::to_string(&bounded.activity_timeline)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?;
    let disclosures_json = serde_json::to_string(&bounded.truncation_disclosures)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?;
    let sources_json = serde_json::to_string(&bounded.validated_sources)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?;
    let references_json = serde_json::to_string(&bounded.connector_references)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?;
    let approvals_json = serde_json::to_string(&bounded.historical_approvals)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?;
    let outcome = bounded.outcome.as_str();
    transaction.execute(
        "INSERT INTO automation_run_outcomes
         (automation_run_identity, automation_session_identity, outcome, finished_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            snapshot.automation_run_identity,
            snapshot.automation_session_identity,
            outcome,
            bounded.finished_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO automation_sessions
         (automation_session_identity, automation_run_identity, outcome, finished_at,
          result_summary, final_output, final_output_available, activity_timeline_json,
          truncation_disclosures_json, validated_sources_json,
          connector_references_json, historical_approvals_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            snapshot.automation_session_identity,
            snapshot.automation_run_identity,
            outcome,
            bounded.finished_at,
            bounded.result_summary,
            bounded.final_output,
            bounded.final_output.is_some() as i64,
            activity_json,
            disclosures_json,
            sources_json,
            references_json,
            approvals_json,
        ],
    )?;
    transaction.execute(
        "INSERT INTO automation_session_attention
         (automation_session_identity, attention_state) VALUES (?1, ?2)",
        params![
            snapshot.automation_session_identity,
            if bounded.outcome.creates_attention() {
                "unread"
            } else {
                "none"
            },
        ],
    )?;
    let changed = transaction.execute(
        "UPDATE automation_work_states SET state=?1
         WHERE work_identity=?2 AND automation_run_identity=?3 AND state NOT LIKE 'terminal:%'",
        params![
            format!("terminal:{outcome}"),
            bounded.work_identity,
            snapshot.automation_run_identity
        ],
    )?;
    if changed != 1 {
        return Err(AutomationSessionError::ActiveWork(
            "work changed during terminalization".to_owned(),
        ));
    }

    // The legacy run table remains a compatibility record, but its terminal
    // outcome is committed in the same transaction when it exists. Standalone
    // contract tests intentionally omit that table.
    let has_legacy_runs: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master
             WHERE type='table' AND name='automation_runs')",
        [],
        |row| row.get(0),
    )?;
    if has_legacy_runs {
        let current_status: Option<String> = transaction
            .query_row(
                "SELECT status FROM automation_runs WHERE id=?1",
                params![snapshot.automation_run_identity],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(current_status) = current_status {
            if current_status == "running" {
                transaction.execute(
                    "UPDATE automation_runs
                     SET status=?1, finished_at=?2, result_summary=?3
                     WHERE id=?4 AND status='running'",
                    params![
                        outcome,
                        bounded.finished_at,
                        bounded.result_summary,
                        snapshot.automation_run_identity
                    ],
                )?;
            } else if !matches!(bounded.outcome, AutomationRunOutcome::Skipped) {
                return Err(AutomationSessionError::Immutable);
            }
        }
        transaction.execute(
            "UPDATE automations SET last_run_at=?1, last_run_status=?2, last_result_summary=?3
             WHERE id=?4",
            params![
                bounded.finished_at,
                outcome,
                bounded.result_summary,
                snapshot.automation_identity,
            ],
        )?;
    }
    transaction.execute(
        "INSERT INTO automation_terminal_outbox
         (automation_session_identity, automation_run_identity, outcome, emitted_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            snapshot.automation_session_identity,
            snapshot.automation_run_identity,
            outcome,
            bounded.finished_at,
        ],
    )?;
    Ok(())
}

pub fn mark_automation_session_viewed(
    connection: &Connection,
    automation_session_identity: &str,
) -> Result<(), AutomationSessionError> {
    let changed = connection.execute(
        "UPDATE automation_session_attention SET attention_state='viewed'
         WHERE automation_session_identity=?1",
        params![automation_session_identity],
    )?;
    if changed == 0 {
        return Err(AutomationSessionError::NotFound);
    }
    crate::work_coordinator::mark_work_automation_session_viewed_if_present(
        connection,
        automation_session_identity,
    )?;
    Ok(())
}

/// Revisioned, idempotent opening command. Replaying the same command after a
/// lost response returns success without creating another mutation.
pub fn open_automation_session(
    connection: &Connection,
    automation_session_identity: &str,
    command_identity: &str,
    expected_revision: u64,
) -> Result<(), AutomationSessionError> {
    let existing: Option<(String, u64)> = connection
        .query_row(
            "SELECT automation_session_identity, expected_revision
             FROM automation_session_open_commands WHERE command_identity=?1",
            params![command_identity],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((existing_session, existing_revision)) = existing {
        if existing_session == automation_session_identity && existing_revision == expected_revision
        {
            return Ok(());
        }
        return Err(AutomationSessionError::Invalid(
            "opening command identity was reused with different arguments".to_owned(),
        ));
    }
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "UPDATE automation_session_attention SET attention_state='viewed'
         WHERE automation_session_identity=?1",
        params![automation_session_identity],
    )?;
    if changed == 0 {
        return Err(AutomationSessionError::NotFound);
    }
    crate::work_coordinator::mark_work_automation_session_viewed_if_present(
        &transaction,
        automation_session_identity,
    )?;
    transaction.execute(
        "INSERT INTO automation_session_open_commands
         (command_identity, automation_session_identity, expected_revision, committed_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            command_identity,
            automation_session_identity,
            expected_revision,
            Utc::now().to_rfc3339()
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn delete_automation_session(
    connection: &Connection,
    automation_session_identity: &str,
) -> Result<(), AutomationSessionError> {
    let outcome: String = connection
        .query_row(
            "SELECT outcome FROM automation_sessions WHERE automation_session_identity=?1",
            params![automation_session_identity],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(AutomationSessionError::NotFound)?;
    let pending: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM automation_session_pending_approvals
             WHERE automation_session_identity=?1)",
        params![automation_session_identity],
        |row| Ok(row.get::<_, i64>(0)? != 0),
    )?;
    if pending {
        return Err(AutomationSessionError::ActiveWork(
            "pending approval".to_owned(),
        ));
    }
    let run_identity: String = connection.query_row(
        "SELECT automation_run_identity FROM automation_sessions
         WHERE automation_session_identity=?1",
        params![automation_session_identity],
        |row| row.get(0),
    )?;
    let work_identity: Option<String> = connection
        .query_row(
            "SELECT work_identity FROM automation_work_states
             WHERE automation_run_identity=?1",
            params![run_identity],
            |row| row.get(0),
        )
        .optional()?;
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "UPDATE automation_continuation_provenance SET source_deleted=1
         WHERE source_automation_session_identity=?1",
        params![automation_session_identity],
    )?;
    transaction.execute(
        "INSERT INTO automation_session_tombstones
         (automation_session_identity, deleted_at, former_outcome)
         VALUES (?1, ?2, ?3)",
        params![
            automation_session_identity,
            Utc::now().to_rfc3339(),
            outcome
        ],
    )?;
    for table in [
        "automation_session_attention",
        "automation_session_open_commands",
        "automation_terminal_outbox",
        "automation_run_outcomes",
        "automation_sessions",
        "automation_task_snapshots",
    ] {
        transaction.execute(
            &format!("DELETE FROM {table} WHERE automation_session_identity=?1"),
            params![automation_session_identity],
        )?;
    }
    if let Some(work_identity) = work_identity {
        crate::work_coordinator::delete_automation_work_if_present(
            &transaction,
            automation_session_identity,
            &work_identity,
        )?;
    }
    transaction.execute(
        "DELETE FROM automation_work_states
         WHERE automation_run_identity=?1",
        params![run_identity],
    )?;
    transaction.commit()?;
    Ok(())
}

pub fn read_automation_session(
    connection: &Connection,
    automation_session_identity: &str,
) -> Result<Option<PersistedAutomationSession>, AutomationSessionError> {
    connection
        .query_row(
            "SELECT s.outcome, s.finished_at, s.result_summary, s.final_output,
                    s.final_output_available, s.activity_timeline_json,
                    s.truncation_disclosures_json, s.validated_sources_json,
                    s.connector_references_json, s.historical_approvals_json,
                    a.attention_state, t.automation_run_identity,
                    t.automation_identity, t.display_name, t.task_text,
                    t.schedule_json, t.timezone, t.definition_revision
             FROM automation_sessions s
             JOIN automation_session_attention a
               ON a.automation_session_identity=s.automation_session_identity
             JOIN automation_task_snapshots t
               ON t.automation_session_identity=s.automation_session_identity
             WHERE s.automation_session_identity=?1",
            params![automation_session_identity],
            |row| {
                let outcome: String = row.get(0)?;
                let activity_json: String = row.get(5)?;
                let disclosures_json: String = row.get(6)?;
                let sources_json: String = row.get(7)?;
                let references_json: String = row.get(8)?;
                let approvals_json: String = row.get(9)?;
                Ok(PersistedAutomationSession {
                    task_snapshot: AutomationTaskSnapshot {
                        automation_identity: row.get(12)?,
                        automation_run_identity: row.get(11)?,
                        automation_session_identity: automation_session_identity.to_owned(),
                        display_name: row.get(13)?,
                        task_text: row.get(14)?,
                        schedule_json: row.get(15)?,
                        timezone: row.get(16)?,
                        definition_revision: row.get(17)?,
                    },
                    outcome: outcome.parse().map_err(|error: String| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::other(error)),
                        )
                    })?,
                    finished_at: row.get(1)?,
                    result_summary: row.get(2)?,
                    final_output: row.get(3)?,
                    final_output_available: row.get::<_, i64>(4)? != 0,
                    activity_timeline: serde_json::from_str(&activity_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    truncation_disclosures: serde_json::from_str(&disclosures_json).map_err(
                        |error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                6,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        },
                    )?,
                    validated_sources: serde_json::from_str(&sources_json).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    connector_references: serde_json::from_str(&references_json).map_err(
                        |error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                8,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        },
                    )?,
                    historical_approvals: serde_json::from_str(&approvals_json).map_err(
                        |error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                9,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        },
                    )?,
                    attention: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(AutomationSessionError::from)
}

pub fn prune_automation_sessions(
    connection: &Connection,
    automation_identity: &str,
    now: &str,
) -> Result<RetentionCleanupCounts, AutomationSessionError> {
    let now = DateTime::parse_from_rfc3339(now)
        .map_err(|error| {
            AutomationSessionError::Invalid(format!("invalid retention time: {error}"))
        })?
        .with_timezone(&Utc);
    let cutoff = now - Duration::days(90);
    let mut statement = connection.prepare(
        "SELECT s.automation_session_identity, s.automation_run_identity, s.finished_at,
                (SELECT work_identity FROM automation_work_states w
                 WHERE w.automation_run_identity=s.automation_run_identity),
                EXISTS(SELECT 1 FROM automation_session_pending_approvals p
                       WHERE p.automation_session_identity=s.automation_session_identity)
         FROM automation_sessions s
         JOIN automation_task_snapshots t
           ON t.automation_session_identity=s.automation_session_identity
         WHERE t.automation_identity=?1
         ORDER BY s.finished_at DESC, s.automation_run_identity ASC",
    )?;
    let candidates = statement
        .query_map(params![automation_identity], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)? != 0,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);

    let transaction = connection.unchecked_transaction()?;
    let mut deleted = 0usize;
    for (
        index,
        (
            automation_session_identity,
            run_identity,
            finished_at,
            work_identity,
            has_pending_approval,
        ),
    ) in candidates.into_iter().enumerate()
    {
        if has_pending_approval {
            continue;
        }
        let finished_at = DateTime::parse_from_rfc3339(&finished_at)
            .map_err(|error| {
                AutomationSessionError::Invalid(format!("invalid finish time: {error}"))
            })?
            .with_timezone(&Utc);
        if index < 50 && finished_at >= cutoff {
            continue;
        }
        transaction.execute(
            "UPDATE automation_continuation_provenance SET source_deleted=1
             WHERE source_automation_session_identity=?1",
            params![automation_session_identity],
        )?;
        transaction.execute(
            "DELETE FROM automation_session_attention WHERE automation_session_identity=?1",
            params![automation_session_identity],
        )?;
        transaction.execute(
            "DELETE FROM automation_session_open_commands WHERE automation_session_identity=?1",
            params![automation_session_identity],
        )?;
        transaction.execute(
            "DELETE FROM automation_terminal_outbox WHERE automation_session_identity=?1",
            params![automation_session_identity],
        )?;
        transaction.execute(
            "DELETE FROM automation_run_outcomes WHERE automation_session_identity=?1",
            params![automation_session_identity],
        )?;
        transaction.execute(
            "DELETE FROM automation_sessions WHERE automation_session_identity=?1",
            params![automation_session_identity],
        )?;
        transaction.execute(
            "DELETE FROM automation_task_snapshots WHERE automation_session_identity=?1",
            params![automation_session_identity],
        )?;
        if let Some(work_identity) = work_identity {
            crate::work_coordinator::delete_automation_work_if_present(
                &transaction,
                &automation_session_identity,
                &work_identity,
            )?;
        }
        transaction.execute(
            "DELETE FROM automation_work_states
             WHERE automation_run_identity=?1",
            params![run_identity],
        )?;
        deleted += 1;
    }
    if deleted > 0 {
        transaction.execute(
            "INSERT INTO automation_retention_audit (deleted_count) VALUES (?1)",
            params![deleted as i64],
        )?;
    }
    transaction.commit()?;
    Ok(RetentionCleanupCounts {
        sessions_deleted: deleted,
    })
}

/// Bounded maintenance entry point used at startup and by the scheduler.
/// Only counts are retained in the cleanup audit; no session content or
/// identities are copied into audit data.
pub fn prune_all_automation_sessions(
    connection: &Connection,
    now: &str,
) -> Result<RetentionCleanupCounts, AutomationSessionError> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT t.automation_identity
         FROM automation_task_snapshots t ORDER BY t.automation_identity ASC",
    )?;
    let identities = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut total = 0;
    for identity in identities {
        total += prune_automation_sessions(connection, &identity, now)?.sessions_deleted;
    }
    Ok(RetentionCleanupCounts {
        sessions_deleted: total,
    })
}

fn validate_input(input: &AutomationTerminalization) -> Result<(), AutomationSessionError> {
    let snapshot = &input.snapshot;
    if snapshot.automation_run_identity.is_empty()
        || snapshot.automation_session_identity.is_empty()
        || input.work_identity.is_empty()
    {
        return Err(AutomationSessionError::Invalid(
            "identities must not be empty".to_owned(),
        ));
    }
    if let Some(summary) = &input.result_summary {
        if summary.is_empty() {
            return Err(AutomationSessionError::Invalid(
                "result summary must be absent or non-empty".to_owned(),
            ));
        }
    }
    Ok(())
}

fn bounded_input(
    mut input: AutomationTerminalization,
) -> Result<AutomationTerminalization, AutomationSessionError> {
    let (task_text, task_disclosure) =
        bounded_chars(&input.snapshot.task_text, MAX_TASK_TEXT_CHARS, "task_text");
    input.snapshot.task_text = task_text;
    if let Some(disclosure) = task_disclosure {
        input.truncation_disclosures.push(disclosure);
    }
    if let Some(summary) = input.result_summary.take() {
        let (value, disclosure) =
            bounded_chars(&summary, MAX_RESULT_SUMMARY_CHARS, "result_summary");
        input.result_summary = Some(value);
        if let Some(disclosure) = disclosure {
            input.truncation_disclosures.push(disclosure);
        }
    }
    if let Some(output) = input.final_output.take() {
        let (value, disclosure) = bounded_utf8(&output, MAX_FINAL_OUTPUT_BYTES, "final_output");
        input.final_output = Some(value);
        if let Some(disclosure) = disclosure {
            input.truncation_disclosures.push(disclosure);
        }
    }
    if input.validated_sources.len() > MAX_VALIDATED_SOURCES {
        let original = input.validated_sources.len();
        input.validated_sources.truncate(MAX_VALIDATED_SOURCES);
        input.truncation_disclosures.push(TruncationDisclosure {
            section: "validated_sources".to_owned(),
            original_extent: original,
            retained_extent: input.validated_sources.len(),
            reason: "bounded validated source count".to_owned(),
        });
    }
    if input.connector_references.len() > MAX_CONNECTOR_REFERENCES {
        let original = input.connector_references.len();
        input
            .connector_references
            .truncate(MAX_CONNECTOR_REFERENCES);
        input.truncation_disclosures.push(TruncationDisclosure {
            section: "connector_references".to_owned(),
            original_extent: original,
            retained_extent: input.connector_references.len(),
            reason: "bounded connector reference count".to_owned(),
        });
    }
    if input.historical_approvals.len() > MAX_APPROVAL_RECORDS {
        let original = input.historical_approvals.len();
        input.historical_approvals.truncate(MAX_APPROVAL_RECORDS);
        input.truncation_disclosures.push(TruncationDisclosure {
            section: "historical_approvals".to_owned(),
            original_extent: original,
            retained_extent: input.historical_approvals.len(),
            reason: "bounded historical approval count".to_owned(),
        });
    }
    let original_activity_count = input.activity_timeline.len();
    if original_activity_count > MAX_ACTIVITY_COUNT {
        input.activity_timeline =
            retain_activity_entries(input.activity_timeline, MAX_ACTIVITY_COUNT);
        input.truncation_disclosures.push(TruncationDisclosure {
            section: "activity_timeline".to_owned(),
            original_extent: original_activity_count,
            retained_extent: input.activity_timeline.len(),
            reason: "bounded logical activity count".to_owned(),
        });
    }
    while serde_json::to_vec(&input.activity_timeline)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?
        .len()
        > MAX_ACTIVITY_ENCODED_BYTES
        && input.activity_timeline.len() > 1
    {
        let original = input.activity_timeline.len();
        let mut retained = retain_activity_entries(input.activity_timeline, original - 1);
        if retained.len() == original {
            retained.pop();
        }
        input.activity_timeline = retained;
        input.truncation_disclosures.push(TruncationDisclosure {
            section: "activity_timeline".to_owned(),
            original_extent: original,
            retained_extent: input.activity_timeline.len(),
            reason: "bounded encoded activity size".to_owned(),
        });
    }
    if serde_json::to_vec(&input.activity_timeline)
        .map_err(|error| AutomationSessionError::Storage(error.to_string()))?
        .len()
        > MAX_ACTIVITY_ENCODED_BYTES
    {
        if let Some(activity) = input.activity_timeline.first_mut() {
            let (caption, disclosure) = bounded_utf8(
                &activity.caption,
                MAX_ACTIVITY_ENCODED_BYTES.saturating_sub(1024),
                "activity_caption",
            );
            activity.caption = caption;
            if let Some(disclosure) = disclosure {
                input.truncation_disclosures.push(disclosure);
            }
        }
    }
    Ok(input)
}

fn retain_activity_entries(entries: Vec<SafeActivity>, limit: usize) -> Vec<SafeActivity> {
    if entries.len() <= limit {
        return entries;
    }
    let last_index = entries.len() - 1;
    let mut indices = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.safety_relevant)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if !indices.contains(&last_index) {
        indices.push(last_index);
    }
    for index in 0..entries.len() {
        if indices.len() >= limit {
            break;
        }
        if !indices.contains(&index) {
            indices.push(index);
        }
    }
    indices.sort_unstable();
    indices
        .into_iter()
        .take(limit)
        .map(|index| entries[index].clone())
        .collect()
}

fn bounded_chars(
    value: &str,
    limit: usize,
    section: &str,
) -> (String, Option<TruncationDisclosure>) {
    if value.chars().count() <= limit {
        return (value.to_owned(), None);
    }
    let marker_len = TEXT_OMISSION_MARKER.chars().count();
    let available = limit.saturating_sub(marker_len);
    let prefix_len = available / 2;
    let suffix_len = available - prefix_len;
    let chars = value.chars().collect::<Vec<_>>();
    let prefix = chars.iter().take(prefix_len).collect::<String>();
    let suffix = chars.iter().rev().take(suffix_len).collect::<Vec<_>>();
    let suffix = suffix.into_iter().rev().collect::<String>();
    let retained = prefix + TEXT_OMISSION_MARKER + &suffix;
    let disclosure = TruncationDisclosure {
        section: section.to_owned(),
        original_extent: value.chars().count(),
        retained_extent: retained.chars().count(),
        reason: "bounded text size".to_owned(),
    };
    (retained, Some(disclosure))
}

fn bounded_utf8(
    value: &str,
    limit: usize,
    section: &str,
) -> (String, Option<TruncationDisclosure>) {
    if value.len() <= limit {
        return (value.to_owned(), None);
    }
    let marker = TEXT_OMISSION_MARKER;
    let budget = limit.saturating_sub(marker.len());
    let mut prefix = String::new();
    for character in value.chars() {
        if prefix.len() + character.len_utf8() > budget / 2 {
            break;
        }
        prefix.push(character);
    }
    let mut suffix = String::new();
    for character in value.chars().rev() {
        if suffix.len() + character.len_utf8() > budget - prefix.len() {
            break;
        }
        suffix.push(character);
    }
    let suffix = suffix.chars().rev().collect::<String>();
    let retained = prefix + marker + &suffix;
    let disclosure = TruncationDisclosure {
        section: section.to_owned(),
        original_extent: value.len(),
        retained_extent: retained.len(),
        reason: "bounded UTF-8 size".to_owned(),
    };
    (retained, Some(disclosure))
}

fn is_terminal_state(state: &str) -> bool {
    state.starts_with("terminal:")
}
