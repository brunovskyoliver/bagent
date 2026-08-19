//! Durable daemon authority for the single user-controlled Current Chat.

use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, error::Error, fmt};
use uuid::Uuid;

pub const MAX_COMPLETED_TURNS: u64 = 500;
pub const MAX_RETAINED_CONTENT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_DRAFT_BYTES: usize = 16 * 1024;
pub const DRAFT_RETENTION_DAYS: i64 = 7;
const TERMINAL_METADATA_RESERVE_BYTES: u64 = 64 * 1024;

const SCHEMA: &str = include_str!("../migrations/V22__durable_current_chat.sql");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationTurnState {
    Active,
    Completed,
    Interrupted,
}

impl ConversationTurnState {
    fn parse(value: &str) -> Result<Self, CurrentChatError> {
        match value {
            "active" => Ok(Self::Active),
            "completed" => Ok(Self::Completed),
            "interrupted" => Ok(Self::Interrupted),
            other => Err(CurrentChatError::Corrupt(format!(
                "unknown Conversation Turn state: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentChatTurn {
    pub identity: String,
    pub user_message: String,
    pub assistant_output: Option<String>,
    pub state: ConversationTurnState,
    pub interruption_reason: Option<String>,
    pub submitted_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentChatDraft {
    pub text: String,
    pub edited_at: String,
    pub pending_attachment_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmittedAttachmentMetadata {
    pub identity: String,
    pub filename: String,
    pub mime: String,
    pub size_bytes: u64,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentChatSubmittedAttachment {
    pub conversation_turn_identity: String,
    pub identity: String,
    pub filename: String,
    pub mime: String,
    pub size_bytes: u64,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSourceMetadata {
    pub identity: String,
    pub title: String,
    pub domain: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentChatContinuation {
    pub identity: String,
    pub source_automation_session_identity: String,
    pub seed: String,
    pub source_deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentChatSourceAvailability {
    pub identity: String,
    pub label: String,
    pub availability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedApprovalPresentation {
    pub identity: String,
    pub category: String,
    pub outcome: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BegunConversationTurn {
    pub identity: String,
    pub current_chat_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentChatSnapshot {
    pub identity: String,
    pub revision: u64,
    pub turn_count: u64,
    pub content_bytes: u64,
    pub turns: Vec<CurrentChatTurn>,
    pub draft: Option<CurrentChatDraft>,
    pub continuation: Option<CurrentChatContinuation>,
    pub submitted_attachments: Vec<CurrentChatSubmittedAttachment>,
    pub validated_sources: Vec<CurrentChatSourceAvailability>,
    pub connector_references: Vec<CurrentChatSourceAvailability>,
    pub completed_approval_presentations: Vec<CompletedApprovalPresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearCurrentChatCommand {
    pub current_chat_identity: String,
    pub expected_revision: u64,
    pub command_identity: String,
    pub confirmed_non_empty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentChatFailurePoint {
    AfterReplacementInsert,
    AfterScopedContentDelete,
    AfterAuthoritySwap,
    BeforeCommit,
}

#[derive(Debug)]
pub enum CurrentChatError {
    Sql(rusqlite::Error),
    Invalid(String),
    Conflict(String),
    Bound(String),
    Corrupt(String),
    Injected(CurrentChatFailurePoint),
}

impl fmt::Display for CurrentChatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sql(error) => write!(formatter, "{error}"),
            Self::Invalid(message)
            | Self::Conflict(message)
            | Self::Bound(message)
            | Self::Corrupt(message) => formatter.write_str(message),
            Self::Injected(point) => write!(formatter, "injected failure at {point:?}"),
        }
    }
}

impl Error for CurrentChatError {}

impl From<rusqlite::Error> for CurrentChatError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error)
    }
}

pub fn initialize_schema(connection: &Connection) -> Result<(), CurrentChatError> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    connection.execute_batch(SCHEMA)?;
    Ok(())
}

pub fn open_or_create_current_chat(
    connection: &Connection,
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    initialize_schema(connection)?;
    if let Some(identity) = connection
        .query_row(
            "SELECT current_chat_identity FROM current_chat_authority WHERE singleton=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        expire_draft(connection, &identity, Utc::now())?;
        let transaction = connection.unchecked_transaction()?;
        refresh_retained_content_bytes(&transaction, &identity)?;
        transaction.commit()?;
        return read_snapshot(connection, &identity);
    }

    let transaction = connection.unchecked_transaction()?;
    let identity = new_current_chat(&transaction, Utc::now())?;
    transaction.execute(
        "INSERT INTO current_chat_authority (singleton, current_chat_identity) VALUES (1, ?1)",
        params![identity],
    )?;
    insert_work_current_chat_if_present(&transaction, &identity)?;
    transaction.commit()?;
    read_snapshot(connection, &identity)
}

pub fn read_current_chat(connection: &Connection) -> Result<CurrentChatSnapshot, CurrentChatError> {
    let identity = connection
        .query_row(
            "SELECT current_chat_identity FROM current_chat_authority WHERE singleton=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| CurrentChatError::Corrupt("Current Chat authority is missing".to_owned()))?;
    expire_draft(connection, &identity, Utc::now())?;
    read_snapshot(connection, &identity)
}

pub fn save_draft(
    connection: &Connection,
    current_chat_identity: &str,
    expected_revision: u64,
    text: &str,
    pending_attachment_references: &[String],
    edited_at: DateTime<Utc>,
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    if text.len() > MAX_DRAFT_BYTES {
        return Err(CurrentChatError::Bound(
            "Current Chat Draft exceeds 16 KiB UTF-8".to_owned(),
        ));
    }
    let references = serde_json::to_string(pending_attachment_references)
        .map_err(|error| CurrentChatError::Invalid(error.to_string()))?;
    let transaction = connection.unchecked_transaction()?;
    compare_revision(&transaction, current_chat_identity, expected_revision)?;
    if text.is_empty() && pending_attachment_references.is_empty() {
        transaction.execute(
            "DELETE FROM current_chat_drafts WHERE current_chat_identity=?1",
            params![current_chat_identity],
        )?;
    } else {
        transaction.execute(
            "INSERT INTO current_chat_drafts
             (current_chat_identity, text, edited_at, pending_attachment_references)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(current_chat_identity) DO UPDATE SET
                text=excluded.text,
                edited_at=excluded.edited_at,
                pending_attachment_references=excluded.pending_attachment_references",
            params![
                current_chat_identity,
                text,
                edited_at.to_rfc3339(),
                references,
            ],
        )?;
    }
    bump_revision(&transaction, current_chat_identity, edited_at)?;
    let retained_bytes = refresh_retained_content_bytes(&transaction, current_chat_identity)?;
    if retained_bytes > MAX_RETAINED_CONTENT_BYTES {
        return Err(CurrentChatError::Bound(
            "Current Chat Draft would exceed the 16 MiB Current Chat bound".to_owned(),
        ));
    }
    transaction.commit()?;
    read_snapshot(connection, current_chat_identity)
}

pub fn begin_conversation_turn(
    connection: &Connection,
    current_chat_identity: &str,
    expected_revision: u64,
    user_message: &str,
    attachments: &[SubmittedAttachmentMetadata],
    submitted_at: DateTime<Utc>,
) -> Result<BegunConversationTurn, CurrentChatError> {
    let transaction = connection.unchecked_transaction()?;
    let begun = begin_conversation_turn_in_transaction(
        &transaction,
        current_chat_identity,
        expected_revision,
        &Uuid::new_v4().to_string(),
        user_message,
        attachments,
        submitted_at,
    )?;
    transaction.commit()?;
    Ok(begun)
}

pub(crate) fn begin_conversation_turn_in_transaction(
    transaction: &Transaction<'_>,
    current_chat_identity: &str,
    expected_revision: u64,
    turn_identity: &str,
    user_message: &str,
    attachments: &[SubmittedAttachmentMetadata],
    submitted_at: DateTime<Utc>,
) -> Result<BegunConversationTurn, CurrentChatError> {
    let attachment_bytes = serde_json::to_vec(attachments)
        .map_err(|error| CurrentChatError::Invalid(error.to_string()))?
        .len() as u64;
    let new_bytes = user_message.len() as u64 + attachment_bytes;
    let (revision, turn_count): (u64, u64) = transaction.query_row(
        "SELECT revision, turn_count FROM current_chats WHERE identity=?1",
        params![current_chat_identity],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if revision != expected_revision {
        return Err(CurrentChatError::Conflict(format!(
            "stale Current Chat revision; current revision is {revision}"
        )));
    }
    if turn_count >= MAX_COMPLETED_TURNS {
        return Err(CurrentChatError::Bound(
            "Current Chat has reached 500 Conversation Turns; export or clear it".to_owned(),
        ));
    }
    let active: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM current_chat_turns
         WHERE current_chat_identity=?1 AND state='active'",
        params![current_chat_identity],
        |row| row.get(0),
    )?;
    if active > 0 {
        return Err(CurrentChatError::Conflict(
            "Current Chat already has an active Conversation Turn".to_owned(),
        ));
    }
    let ordinal: u64 = transaction.query_row(
        "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM current_chat_turns
         WHERE current_chat_identity=?1",
        params![current_chat_identity],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO current_chat_turns
         (identity, current_chat_identity, ordinal, user_message, assistant_output,
          state, interruption_reason, submitted_at, completed_at, encoded_bytes)
         VALUES (?1, ?2, ?3, ?4, NULL, 'active', NULL, ?5, NULL, ?6)",
        params![
            turn_identity,
            current_chat_identity,
            ordinal,
            user_message,
            submitted_at.to_rfc3339(),
            new_bytes,
        ],
    )?;
    for attachment in attachments {
        transaction.execute(
            "INSERT INTO current_chat_submitted_attachments
             (current_chat_identity, conversation_turn_identity, attachment_identity,
              filename, mime, size_bytes, available)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
            params![
                current_chat_identity,
                turn_identity,
                attachment.identity,
                attachment.filename,
                attachment.mime,
                attachment.size_bytes,
            ],
        )?;
    }
    transaction.execute(
        "DELETE FROM current_chat_drafts WHERE current_chat_identity=?1",
        params![current_chat_identity],
    )?;
    transaction.execute(
        "UPDATE current_chats
         SET revision=revision+1, updated_at=?2
         WHERE identity=?1",
        params![current_chat_identity, submitted_at.to_rfc3339()],
    )?;
    insert_work_current_chat_if_present(transaction, current_chat_identity)?;
    let retained_bytes = refresh_retained_content_bytes(transaction, current_chat_identity)?;
    if retained_bytes > MAX_RETAINED_CONTENT_BYTES - TERMINAL_METADATA_RESERVE_BYTES {
        return Err(CurrentChatError::Bound(
            "Current Chat has reached 16 MiB; export or clear it".to_owned(),
        ));
    }
    Ok(BegunConversationTurn {
        identity: turn_identity.to_owned(),
        current_chat_revision: revision + 1,
    })
}

pub fn complete_conversation_turn(
    connection: &Connection,
    current_chat_identity: &str,
    conversation_turn_identity: &str,
    assistant_output: &str,
    completed_at: DateTime<Utc>,
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    complete_conversation_turn_with_artifacts(
        connection,
        current_chat_identity,
        conversation_turn_identity,
        assistant_output,
        completed_at,
        None,
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
pub fn complete_conversation_turn_with_artifacts(
    connection: &Connection,
    current_chat_identity: &str,
    conversation_turn_identity: &str,
    assistant_output: &str,
    completed_at: DateTime<Utc>,
    work_identity: Option<&str>,
    validated_sources: &[ValidatedSourceMetadata],
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    let transaction = connection.unchecked_transaction()?;
    let content_bound = complete_conversation_turn_in_transaction(
        &transaction,
        current_chat_identity,
        conversation_turn_identity,
        assistant_output,
        completed_at,
        work_identity,
        validated_sources,
    )?;
    transaction.commit()?;
    if content_bound {
        return Err(CurrentChatError::Bound(
            "assistant output would exceed the 16 MiB Current Chat bound".to_owned(),
        ));
    }
    read_snapshot(connection, current_chat_identity)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn complete_conversation_turn_in_transaction(
    transaction: &Transaction<'_>,
    current_chat_identity: &str,
    conversation_turn_identity: &str,
    assistant_output: &str,
    completed_at: DateTime<Utc>,
    work_identity: Option<&str>,
    validated_sources: &[ValidatedSourceMetadata],
) -> Result<bool, CurrentChatError> {
    let changed = transaction.execute(
        "UPDATE current_chat_turns
         SET assistant_output=?3, state='completed', interruption_reason=NULL,
             completed_at=?4, encoded_bytes=encoded_bytes+?5
         WHERE identity=?1 AND current_chat_identity=?2 AND state='active'",
        params![
            conversation_turn_identity,
            current_chat_identity,
            assistant_output,
            completed_at.to_rfc3339(),
            assistant_output.len() as u64,
        ],
    )?;
    if changed != 1 {
        return Err(CurrentChatError::Conflict(
            "Conversation Turn is not active".to_owned(),
        ));
    }
    let mut inserted_source_identities = Vec::new();
    for source in validated_sources {
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO current_chat_validated_sources
             (current_chat_identity, source_identity, title, domain, available)
             VALUES (?1, ?2, ?3, ?4, 1)",
            params![
                current_chat_identity,
                source.identity,
                source.title,
                source.domain,
            ],
        )?;
        if inserted == 1 {
            inserted_source_identities.push(source.identity.as_str());
        }
    }
    if let Some(work_identity) = work_identity {
        capture_work_approval_presentations(transaction, current_chat_identity, work_identity)?;
    }
    transaction.execute(
        "UPDATE current_chats
         SET revision=revision+1, turn_count=turn_count+1, updated_at=?2
         WHERE identity=?1",
        params![current_chat_identity, completed_at.to_rfc3339()],
    )?;
    let retained_bytes = refresh_retained_content_bytes(transaction, current_chat_identity)?;
    if retained_bytes > MAX_RETAINED_CONTENT_BYTES {
        for source_identity in inserted_source_identities {
            transaction.execute(
                "DELETE FROM current_chat_validated_sources
                 WHERE current_chat_identity=?1 AND source_identity=?2",
                params![current_chat_identity, source_identity],
            )?;
        }
        transaction.execute(
            "UPDATE current_chat_turns
             SET assistant_output=NULL, state='interrupted',
                 interruption_reason='content_bound', completed_at=?3,
                 encoded_bytes=length(CAST(user_message AS BLOB))
             WHERE identity=?1 AND current_chat_identity=?2",
            params![
                conversation_turn_identity,
                current_chat_identity,
                completed_at.to_rfc3339(),
            ],
        )?;
        transaction.execute(
            "UPDATE current_chats SET turn_count=turn_count-1 WHERE identity=?1",
            params![current_chat_identity],
        )?;
        refresh_retained_content_bytes(transaction, current_chat_identity)?;
        return Ok(true);
    }
    Ok(false)
}

pub fn interrupt_conversation_turn(
    connection: &Connection,
    current_chat_identity: &str,
    conversation_turn_identity: &str,
    interrupted_at: DateTime<Utc>,
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    interrupt_conversation_turn_with_work(
        connection,
        current_chat_identity,
        conversation_turn_identity,
        interrupted_at,
        None,
    )
}

pub fn interrupt_conversation_turn_with_work(
    connection: &Connection,
    current_chat_identity: &str,
    conversation_turn_identity: &str,
    interrupted_at: DateTime<Utc>,
    work_identity: Option<&str>,
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    let transaction = connection.unchecked_transaction()?;
    interrupt_conversation_turn_in_transaction(
        &transaction,
        current_chat_identity,
        conversation_turn_identity,
        interrupted_at,
        work_identity,
    )?;
    transaction.commit()?;
    read_snapshot(connection, current_chat_identity)
}

pub(crate) fn interrupt_conversation_turn_in_transaction(
    transaction: &Transaction<'_>,
    current_chat_identity: &str,
    conversation_turn_identity: &str,
    interrupted_at: DateTime<Utc>,
    work_identity: Option<&str>,
) -> Result<(), CurrentChatError> {
    let changed = transaction.execute(
        "UPDATE current_chat_turns
         SET assistant_output=NULL, state='interrupted',
             interruption_reason='execution_failed', completed_at=?3
         WHERE identity=?1 AND current_chat_identity=?2 AND state='active'",
        params![
            conversation_turn_identity,
            current_chat_identity,
            interrupted_at.to_rfc3339(),
        ],
    )?;
    if changed != 1 {
        return Err(CurrentChatError::Conflict(
            "Conversation Turn is not active".to_owned(),
        ));
    }
    if let Some(work_identity) = work_identity {
        capture_work_approval_presentations(transaction, current_chat_identity, work_identity)?;
    }
    bump_revision(transaction, current_chat_identity, interrupted_at)?;
    let retained_bytes = refresh_retained_content_bytes(transaction, current_chat_identity)?;
    if retained_bytes > MAX_RETAINED_CONTENT_BYTES {
        return Err(CurrentChatError::Bound(
            "Interrupted turn metadata exceeds the 16 MiB Current Chat bound".to_owned(),
        ));
    }
    Ok(())
}

fn capture_work_approval_presentations(
    transaction: &Transaction<'_>,
    current_chat_identity: &str,
    work_identity: &str,
) -> Result<usize, CurrentChatError> {
    if !table_exists_transaction(transaction, "work_approvals")? {
        return Ok(0);
    }
    Ok(transaction.execute(
        "INSERT OR REPLACE INTO current_chat_approval_presentations
         (current_chat_identity, approval_identity, category, outcome)
         SELECT ?1, identity, category, state FROM work_approvals
         WHERE work_identity=?2 AND state != 'pending'",
        params![current_chat_identity, work_identity],
    )?)
}

pub fn recover_after_daemon_restart(
    connection: &Connection,
    recovered_at: DateTime<Utc>,
) -> Result<usize, CurrentChatError> {
    initialize_schema(connection)?;
    let transaction = connection.unchecked_transaction()?;
    let affected: Vec<String> = {
        let mut statement = transaction.prepare(
            "SELECT DISTINCT current_chat_identity FROM current_chat_turns WHERE state='active'",
        )?;
        let identities = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        identities
    };
    let changed = transaction.execute(
        "UPDATE current_chat_turns
         SET assistant_output=NULL, state='interrupted', interruption_reason='daemon_restart',
             completed_at=?1
         WHERE state='active'",
        params![recovered_at.to_rfc3339()],
    )?;
    let connector_chats = {
        let mut statement = transaction.prepare(
            "SELECT DISTINCT current_chat_identity FROM current_chat_connector_references
             WHERE availability != 'unavailable'",
        )?;
        let identities = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        identities
    };
    transaction.execute(
        "UPDATE current_chat_connector_references SET availability='unavailable'
         WHERE availability != 'unavailable'",
        [],
    )?;
    let changed_chats = affected
        .into_iter()
        .chain(connector_chats)
        .collect::<BTreeSet<_>>();
    for identity in changed_chats {
        bump_revision(&transaction, &identity, recovered_at)?;
        refresh_retained_content_bytes(&transaction, &identity)?;
    }
    transaction.commit()?;
    Ok(changed)
}

/// Copy terminal approval outcomes produced by Work restart recovery into the
/// bounded, privacy-safe Current Chat presentation. Work recovery runs on its
/// own connection, so this is deliberately a second atomic transaction after
/// the coordinator has committed its new daemon generation.
pub fn capture_recovered_approval_presentations(
    connection: &Connection,
    recovered_at: DateTime<Utc>,
) -> Result<usize, CurrentChatError> {
    initialize_schema(connection)?;
    if !table_exists(connection, "works")? || !table_exists(connection, "work_approvals")? {
        return Ok(0);
    }

    let transaction = connection.unchecked_transaction()?;
    let current_chat_identity: String = transaction.query_row(
        "SELECT current_chat_identity FROM current_chat_authority WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    let changed = transaction.execute(
        "INSERT OR IGNORE INTO current_chat_approval_presentations
         (current_chat_identity, approval_identity, category, outcome)
         SELECT ?1, a.identity, a.category, a.state
         FROM work_approvals a
         JOIN works w ON w.identity=a.work_identity
         WHERE w.origin_kind='conversation' AND w.origin_primary_identity=?1
           AND a.state != 'pending'",
        params![current_chat_identity],
    )?;
    if changed > 0 {
        bump_revision(&transaction, &current_chat_identity, recovered_at)?;
        let retained_bytes = refresh_retained_content_bytes(&transaction, &current_chat_identity)?;
        if retained_bytes > MAX_RETAINED_CONTENT_BYTES {
            return Err(CurrentChatError::Bound(
                "Recovered approval presentation exceeds the 16 MiB Current Chat bound".to_owned(),
            ));
        }
    }
    transaction.commit()?;
    Ok(changed)
}

pub fn upsert_connector_reference(
    connection: &Connection,
    current_chat_identity: &str,
    reference_identity: &str,
    connector_kind: &str,
    payload_json: &str,
    recorded_at: DateTime<Utc>,
) -> Result<(), CurrentChatError> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO current_chat_connector_references
         (current_chat_identity, reference_identity, connector_kind, availability, payload_json)
         VALUES (?1, ?2, ?3, 'available', ?4)
         ON CONFLICT(current_chat_identity, reference_identity) DO UPDATE SET
            connector_kind=excluded.connector_kind,
            availability='available', payload_json=excluded.payload_json",
        params![
            current_chat_identity,
            reference_identity,
            connector_kind,
            payload_json,
        ],
    )?;
    bump_revision(&transaction, current_chat_identity, recorded_at)?;
    let bytes = refresh_retained_content_bytes(&transaction, current_chat_identity)?;
    let has_active_turn: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM current_chat_turns
         WHERE current_chat_identity=?1 AND state='active')",
        params![current_chat_identity],
        |row| row.get(0),
    )?;
    let limit = if has_active_turn {
        MAX_RETAINED_CONTENT_BYTES - TERMINAL_METADATA_RESERVE_BYTES
    } else {
        MAX_RETAINED_CONTENT_BYTES
    };
    if bytes > limit {
        return Err(CurrentChatError::Bound(
            "Connector Reference would exceed the 16 MiB Current Chat bound".to_owned(),
        ));
    }
    transaction.commit()?;
    Ok(())
}

fn read_snapshot(
    connection: &Connection,
    identity: &str,
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    refresh_attachment_availability(connection, identity)?;
    let (revision, turn_count, content_bytes): (u64, u64, u64) = connection
        .query_row(
            "SELECT revision, turn_count, content_bytes FROM current_chats WHERE identity=?1",
            params![identity],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| CurrentChatError::Corrupt("Current Chat record is missing".to_owned()))?;
    let mut statement = connection.prepare(
        "SELECT identity, user_message, assistant_output, state, interruption_reason,
                submitted_at, completed_at
         FROM current_chat_turns WHERE current_chat_identity=?1 ORDER BY ordinal",
    )?;
    let turns = statement
        .query_map(params![identity], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?
        .map(|row| {
            let (
                identity,
                user_message,
                assistant_output,
                state,
                interruption_reason,
                submitted_at,
                completed_at,
            ) = row?;
            Ok(CurrentChatTurn {
                identity,
                user_message,
                assistant_output,
                state: ConversationTurnState::parse(&state)?,
                interruption_reason,
                submitted_at,
                completed_at,
            })
        })
        .collect::<Result<Vec<_>, CurrentChatError>>()?;
    let draft = connection
        .query_row(
            "SELECT text, edited_at, pending_attachment_references
             FROM current_chat_drafts WHERE current_chat_identity=?1",
            params![identity],
            |row| {
                let references = row.get::<_, String>(2)?;
                Ok(CurrentChatDraft {
                    text: row.get(0)?,
                    edited_at: row.get(1)?,
                    pending_attachment_references: serde_json::from_str(&references)
                        .unwrap_or_default(),
                })
            },
        )
        .optional()?;
    let continuation = if table_exists(connection, "automation_continuation_provenance")? {
        connection
            .query_row(
                "SELECT identity, source_automation_session_identity, seed, source_deleted
                 FROM automation_continuation_provenance
                 WHERE target_current_chat_identity=?1",
                params![identity],
                |row| {
                    Ok(CurrentChatContinuation {
                        identity: row.get(0)?,
                        source_automation_session_identity: row.get(1)?,
                        seed: row.get(2)?,
                        source_deleted: row.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()?
    } else {
        None
    };
    let submitted_attachments = read_submitted_attachments(connection, identity)?;
    let validated_sources = read_source_availability(
        connection,
        identity,
        "current_chat_validated_sources",
        "source_identity",
        "title",
        "CASE available WHEN 1 THEN 'available' ELSE 'unavailable' END",
    )?;
    let connector_references = read_source_availability(
        connection,
        identity,
        "current_chat_connector_references",
        "reference_identity",
        "connector_kind",
        "availability",
    )?;
    let completed_approval_presentations = {
        let mut statement = connection.prepare(
            "SELECT approval_identity, category, outcome
             FROM current_chat_approval_presentations
             WHERE current_chat_identity=?1 ORDER BY approval_identity",
        )?;
        let presentations = statement
            .query_map(params![identity], |row| {
                Ok(CompletedApprovalPresentation {
                    identity: row.get(0)?,
                    category: row.get(1)?,
                    outcome: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        presentations
    };
    Ok(CurrentChatSnapshot {
        identity: identity.to_owned(),
        revision,
        turn_count,
        content_bytes,
        turns,
        draft,
        continuation,
        submitted_attachments,
        validated_sources,
        connector_references,
        completed_approval_presentations,
    })
}

fn read_submitted_attachments(
    connection: &Connection,
    identity: &str,
) -> Result<Vec<CurrentChatSubmittedAttachment>, CurrentChatError> {
    let mut statement = connection.prepare(
        "SELECT conversation_turn_identity, attachment_identity, filename, mime, size_bytes, available
         FROM current_chat_submitted_attachments
         WHERE current_chat_identity=?1 ORDER BY conversation_turn_identity, attachment_identity",
    )?;
    let attachments = statement
        .query_map(params![identity], |row| {
            Ok(CurrentChatSubmittedAttachment {
                conversation_turn_identity: row.get(0)?,
                identity: row.get(1)?,
                filename: row.get(2)?,
                mime: row.get(3)?,
                size_bytes: row.get(4)?,
                available: row.get::<_, i64>(5)? != 0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CurrentChatError::from)?;
    Ok(attachments)
}

fn refresh_attachment_availability(
    connection: &Connection,
    identity: &str,
) -> Result<(), CurrentChatError> {
    if table_exists(connection, "attachments")? {
        let transaction = connection.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE current_chat_submitted_attachments
             SET available=EXISTS(
                 SELECT 1 FROM attachments a
                 WHERE a.id=current_chat_submitted_attachments.attachment_identity
             )
             WHERE current_chat_identity=?1
               AND available != EXISTS(
                   SELECT 1 FROM attachments a
                   WHERE a.id=current_chat_submitted_attachments.attachment_identity
               )",
            params![identity],
        )?;
        if changed > 0 {
            bump_revision(&transaction, identity, Utc::now())?;
            refresh_retained_content_bytes(&transaction, identity)?;
        }
        transaction.commit()?;
    }
    Ok(())
}

fn read_source_availability(
    connection: &Connection,
    identity: &str,
    table: &str,
    identity_column: &str,
    label_column: &str,
    availability_expression: &str,
) -> Result<Vec<CurrentChatSourceAvailability>, CurrentChatError> {
    let sql = format!(
        "SELECT {identity_column}, {label_column}, {availability_expression}
         FROM {table} WHERE current_chat_identity=?1 ORDER BY {identity_column}"
    );
    let mut statement = connection.prepare(&sql)?;
    let sources = statement
        .query_map(params![identity], |row| {
            Ok(CurrentChatSourceAvailability {
                identity: row.get(0)?,
                label: row.get(1)?,
                availability: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(CurrentChatError::from)?;
    Ok(sources)
}

fn new_current_chat(
    transaction: &Transaction<'_>,
    now: DateTime<Utc>,
) -> Result<String, CurrentChatError> {
    let identity = Uuid::new_v4().to_string();
    let timestamp = now.to_rfc3339();
    transaction.execute(
        "INSERT INTO current_chats
         (identity, revision, turn_count, content_bytes, created_at, updated_at)
         VALUES (?1, 1, 0, 0, ?2, ?2)",
        params![identity, timestamp],
    )?;
    refresh_retained_content_bytes(transaction, &identity)?;
    Ok(identity)
}

pub fn clear_current_chat(
    connection: &Connection,
    command: ClearCurrentChatCommand,
    failure_point: Option<CurrentChatFailurePoint>,
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    initialize_schema(connection)?;
    if let Some(existing) = existing_clear_result(connection, &command)? {
        return read_snapshot(connection, &existing);
    }

    let transaction = connection.unchecked_transaction()?;
    let authoritative_identity: String = transaction.query_row(
        "SELECT current_chat_identity FROM current_chat_authority WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    if authoritative_identity != command.current_chat_identity {
        return Err(CurrentChatError::Conflict(
            "stale Current Chat identity".to_owned(),
        ));
    }
    let current_revision: u64 = transaction.query_row(
        "SELECT revision FROM current_chats WHERE identity=?1",
        params![authoritative_identity],
        |row| row.get(0),
    )?;
    if current_revision != command.expected_revision {
        return Err(CurrentChatError::Conflict(format!(
            "stale Current Chat revision; current revision is {current_revision}"
        )));
    }
    reject_clear_with_active_work(&transaction, &authoritative_identity)?;
    if current_chat_requires_confirmation(&transaction, &authoritative_identity)?
        && !command.confirmed_non_empty
    {
        return Err(CurrentChatError::Conflict(
            "non-empty Current Chat requires confirmation".to_owned(),
        ));
    }

    let now = Utc::now();
    let replacement_identity = new_current_chat(&transaction, now)?;
    insert_work_current_chat_if_present(&transaction, &replacement_identity)?;
    inject(
        failure_point,
        CurrentChatFailurePoint::AfterReplacementInsert,
    )?;

    if table_exists_transaction(&transaction, "automation_continuation_provenance")? {
        transaction.execute(
            "DELETE FROM automation_continuation_provenance
             WHERE target_current_chat_identity=?1",
            params![authoritative_identity],
        )?;
    }
    inject(
        failure_point,
        CurrentChatFailurePoint::AfterScopedContentDelete,
    )?;

    transaction.execute(
        "UPDATE current_chat_authority SET current_chat_identity=?1 WHERE singleton=1",
        params![replacement_identity],
    )?;
    transaction.execute(
        "DELETE FROM current_chats WHERE identity=?1",
        params![authoritative_identity],
    )?;
    inject(failure_point, CurrentChatFailurePoint::AfterAuthoritySwap)?;

    let committed_at = now.to_rfc3339();
    transaction.execute(
        "INSERT INTO current_chat_clear_commands
         (command_identity, old_current_chat_identity, old_revision,
          confirmed_non_empty, new_current_chat_identity, committed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            command.command_identity,
            authoritative_identity,
            current_revision,
            command.confirmed_non_empty as i64,
            replacement_identity,
            committed_at,
        ],
    )?;
    transaction.execute(
        "INSERT INTO current_chat_lifecycle_audit
         (command_identity, canonical_action, old_current_chat_identity,
          new_current_chat_identity, normalized_outcome, committed_at)
         VALUES (?1, 'clear_current_chat', ?2, ?3, 'committed', ?4)",
        params![
            command.command_identity,
            authoritative_identity,
            replacement_identity,
            committed_at,
        ],
    )?;
    inject(failure_point, CurrentChatFailurePoint::BeforeCommit)?;
    transaction.commit()?;
    read_snapshot(connection, &replacement_identity)
}

fn existing_clear_result(
    connection: &Connection,
    command: &ClearCurrentChatCommand,
) -> Result<Option<String>, CurrentChatError> {
    let existing = connection
        .query_row(
            "SELECT old_current_chat_identity, old_revision, confirmed_non_empty,
                    new_current_chat_identity
             FROM current_chat_clear_commands WHERE command_identity=?1",
            params![command.command_identity],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, i64>(2)? != 0,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    match existing {
        Some((old_identity, old_revision, confirmed, new_identity))
            if old_identity == command.current_chat_identity
                && old_revision == command.expected_revision
                && confirmed == command.confirmed_non_empty =>
        {
            Ok(Some(new_identity))
        }
        Some(_) => Err(CurrentChatError::Conflict(
            "clear command identity was reused with different arguments".to_owned(),
        )),
        None => Ok(None),
    }
}

fn current_chat_requires_confirmation(
    transaction: &Transaction<'_>,
    identity: &str,
) -> Result<bool, CurrentChatError> {
    let turns: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM current_chat_turns WHERE current_chat_identity=?1",
        params![identity],
        |row| row.get(0),
    )?;
    if turns > 0 {
        return Ok(true);
    }
    let draft: Option<(String, String)> = transaction
        .query_row(
            "SELECT text, pending_attachment_references
             FROM current_chat_drafts WHERE current_chat_identity=?1",
            params![identity],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if draft.is_some_and(|(text, references)| {
        !text.eq_ignore_ascii_case("/clear") || references != "[]"
    }) {
        return Ok(true);
    }
    for table in [
        "current_chat_submitted_attachments",
        "current_chat_validated_sources",
        "current_chat_connector_references",
        "current_chat_approval_presentations",
    ] {
        let count: i64 = transaction.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE current_chat_identity=?1"),
            params![identity],
            |row| row.get(0),
        )?;
        if count > 0 {
            return Ok(true);
        }
    }
    if table_exists_transaction(transaction, "automation_continuation_provenance")? {
        let count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM automation_continuation_provenance
             WHERE target_current_chat_identity=?1",
            params![identity],
            |row| row.get(0),
        )?;
        if count > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

fn compare_revision(
    transaction: &Transaction<'_>,
    identity: &str,
    expected_revision: u64,
) -> Result<(), CurrentChatError> {
    let current_revision: u64 = transaction
        .query_row(
            "SELECT revision FROM current_chats WHERE identity=?1",
            params![identity],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| CurrentChatError::Conflict("stale Current Chat identity".to_owned()))?;
    if current_revision != expected_revision {
        return Err(CurrentChatError::Conflict(format!(
            "stale Current Chat revision; current revision is {current_revision}"
        )));
    }
    Ok(())
}

fn bump_revision(
    transaction: &Transaction<'_>,
    identity: &str,
    now: DateTime<Utc>,
) -> Result<(), CurrentChatError> {
    transaction.execute(
        "UPDATE current_chats SET revision=revision+1, updated_at=?2 WHERE identity=?1",
        params![identity, now.to_rfc3339()],
    )?;
    Ok(())
}

pub(crate) fn refresh_retained_content_bytes(
    transaction: &Transaction<'_>,
    identity: &str,
) -> Result<u64, CurrentChatError> {
    let bytes = encoded_retained_content_bytes(transaction, identity)?;
    if bytes <= MAX_RETAINED_CONTENT_BYTES {
        transaction.execute(
            "UPDATE current_chats SET content_bytes=?2 WHERE identity=?1",
            params![identity, bytes],
        )?;
    }
    Ok(bytes)
}

#[derive(Serialize)]
struct EncodedRetainedCurrentChat {
    identity: String,
    revision: u64,
    turn_count: u64,
    turns: Vec<CurrentChatTurn>,
    draft: Option<CurrentChatDraft>,
    continuation: Option<CurrentChatContinuation>,
    submitted_attachments: Vec<CurrentChatSubmittedAttachment>,
    validated_sources: Vec<CurrentChatSourceAvailability>,
    connector_references: Vec<EncodedConnectorReference>,
    completed_approval_presentations: Vec<CompletedApprovalPresentation>,
}

#[derive(Serialize)]
struct EncodedConnectorReference {
    identity: String,
    connector_kind: String,
    availability: String,
    payload_json: String,
}

fn encoded_retained_content_bytes(
    connection: &Connection,
    identity: &str,
) -> Result<u64, CurrentChatError> {
    let (revision, turn_count): (u64, u64) = connection.query_row(
        "SELECT revision, turn_count FROM current_chats WHERE identity=?1",
        params![identity],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let turns = {
        let mut statement = connection.prepare(
            "SELECT identity, user_message, assistant_output, state, interruption_reason,
                    submitted_at, completed_at
             FROM current_chat_turns WHERE current_chat_identity=?1 ORDER BY ordinal",
        )?;
        let rows = statement
            .query_map(params![identity], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })?
            .map(|row| {
                let (
                    turn_identity,
                    user_message,
                    assistant_output,
                    state,
                    reason,
                    submitted,
                    completed,
                ) = row?;
                Ok(CurrentChatTurn {
                    identity: turn_identity,
                    user_message,
                    assistant_output,
                    state: ConversationTurnState::parse(&state)?,
                    interruption_reason: reason,
                    submitted_at: submitted,
                    completed_at: completed,
                })
            })
            .collect::<Result<Vec<_>, CurrentChatError>>()?;
        rows
    };
    let draft = connection
        .query_row(
            "SELECT text, edited_at, pending_attachment_references
             FROM current_chat_drafts WHERE current_chat_identity=?1",
            params![identity],
            |row| {
                let references = row.get::<_, String>(2)?;
                Ok(CurrentChatDraft {
                    text: row.get(0)?,
                    edited_at: row.get(1)?,
                    pending_attachment_references: serde_json::from_str(&references)
                        .unwrap_or_default(),
                })
            },
        )
        .optional()?;
    let continuation = if table_exists(connection, "automation_continuation_provenance")? {
        connection
            .query_row(
                "SELECT identity, source_automation_session_identity, seed, source_deleted
                 FROM automation_continuation_provenance
                 WHERE target_current_chat_identity=?1",
                params![identity],
                |row| {
                    Ok(CurrentChatContinuation {
                        identity: row.get(0)?,
                        source_automation_session_identity: row.get(1)?,
                        seed: row.get(2)?,
                        source_deleted: row.get::<_, i64>(3)? != 0,
                    })
                },
            )
            .optional()?
    } else {
        None
    };
    let submitted_attachments = read_submitted_attachments(connection, identity)?;
    let validated_sources = read_source_availability(
        connection,
        identity,
        "current_chat_validated_sources",
        "source_identity",
        "title",
        "CASE available WHEN 1 THEN 'available' ELSE 'unavailable' END",
    )?;
    let connector_references = {
        let mut statement = connection.prepare(
            "SELECT reference_identity, connector_kind, availability, payload_json
             FROM current_chat_connector_references
             WHERE current_chat_identity=?1 ORDER BY reference_identity",
        )?;
        let references = statement
            .query_map(params![identity], |row| {
                Ok(EncodedConnectorReference {
                    identity: row.get(0)?,
                    connector_kind: row.get(1)?,
                    availability: row.get(2)?,
                    payload_json: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        references
    };
    let completed_approval_presentations = {
        let mut statement = connection.prepare(
            "SELECT approval_identity, category, outcome
             FROM current_chat_approval_presentations
             WHERE current_chat_identity=?1 ORDER BY approval_identity",
        )?;
        let presentations = statement
            .query_map(params![identity], |row| {
                Ok(CompletedApprovalPresentation {
                    identity: row.get(0)?,
                    category: row.get(1)?,
                    outcome: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        presentations
    };
    let content = EncodedRetainedCurrentChat {
        identity: identity.to_owned(),
        revision,
        turn_count,
        turns,
        draft,
        continuation,
        submitted_attachments,
        validated_sources,
        connector_references,
        completed_approval_presentations,
    };
    serde_json::to_vec(&content)
        .map(|encoded| encoded.len() as u64)
        .map_err(|error| {
            CurrentChatError::Corrupt(format!("encode retained Current Chat: {error}"))
        })
}

fn reject_clear_with_active_work(
    transaction: &Transaction<'_>,
    identity: &str,
) -> Result<(), CurrentChatError> {
    if table_exists_transaction(transaction, "works")? {
        let active: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM works
             WHERE origin_kind='conversation' AND origin_primary_identity=?1
               AND state NOT IN ('completed','partial','failed','cancelled','abandoned')",
            params![identity],
            |row| row.get(0),
        )?;
        if active > 0 {
            return Err(CurrentChatError::Conflict(
                "Current Chat has an active Conversation Turn".to_owned(),
            ));
        }
        if table_exists_transaction(transaction, "work_approvals")? {
            let approvals: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM work_approvals WHERE state='pending'",
                [],
                |row| row.get(0),
            )?;
            if approvals > 0 {
                return Err(CurrentChatError::Conflict(
                    "Current Chat has a pending approval".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn insert_work_current_chat_if_present(
    transaction: &Transaction<'_>,
    identity: &str,
) -> Result<(), CurrentChatError> {
    crate::work_coordinator::insert_work_current_chat_if_present(transaction, identity)
        .map_err(CurrentChatError::from)
}

fn table_exists_transaction(
    transaction: &Transaction<'_>,
    table: &str,
) -> Result<bool, CurrentChatError> {
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            params![table],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(CurrentChatError::from)
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, CurrentChatError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            params![table],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(CurrentChatError::from)
}

fn inject(
    selected: Option<CurrentChatFailurePoint>,
    current: CurrentChatFailurePoint,
) -> Result<(), CurrentChatError> {
    if selected == Some(current) {
        Err(CurrentChatError::Injected(current))
    } else {
        Ok(())
    }
}

/// Seeds deterministic retained records only inside the disposable, signed
/// Stage 7A acceptance daemon. The records still travel through the production
/// transaction, encoded-size ledger, snapshot, and Swift restoration paths.
#[cfg(feature = "stage7a-acceptance")]
pub fn seed_stage7a_acceptance_records(
    connection: &Connection,
    now: DateTime<Utc>,
) -> Result<CurrentChatSnapshot, CurrentChatError> {
    let transaction = connection.unchecked_transaction()?;
    let chat_identity: String = transaction.query_row(
        "SELECT current_chat_identity FROM current_chat_authority WHERE singleton=1",
        [],
        |row| row.get(0),
    )?;
    let turn_identity: String = transaction.query_row(
        "SELECT identity FROM current_chat_turns
         WHERE current_chat_identity=?1 ORDER BY ordinal DESC LIMIT 1",
        params![chat_identity],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT OR REPLACE INTO current_chat_submitted_attachments
         (current_chat_identity, conversation_turn_identity, attachment_identity,
          filename, mime, size_bytes, available)
         VALUES (?1, ?2, 'missing-attachment', 'missing.txt', 'text/plain', 7, 0)",
        params![chat_identity, turn_identity],
    )?;
    transaction.execute(
        "INSERT OR REPLACE INTO current_chat_validated_sources
         VALUES (?1, 'source-fixture', 'Fixture Source', 'example.test', 1)",
        params![chat_identity],
    )?;
    transaction.execute(
        "INSERT OR REPLACE INTO current_chat_connector_references
         VALUES (?1, 'connector-fixture', 'mail', 'available', '{\"message\":\"fixture\"}')",
        params![chat_identity],
    )?;
    transaction.execute(
        "INSERT OR REPLACE INTO current_chat_approval_presentations
         VALUES (?1, 'approval-fixture', 'filesystem_write', 'allowed')",
        params![chat_identity],
    )?;
    bump_revision(&transaction, &chat_identity, now)?;
    let bytes = refresh_retained_content_bytes(&transaction, &chat_identity)?;
    if bytes > MAX_RETAINED_CONTENT_BYTES {
        return Err(CurrentChatError::Bound(
            "Stage 7A fixture records exceed Current Chat bound".to_owned(),
        ));
    }
    transaction.commit()?;
    read_snapshot(connection, &chat_identity)
}

fn expire_draft(
    connection: &Connection,
    identity: &str,
    now: DateTime<Utc>,
) -> Result<(), CurrentChatError> {
    let cutoff = (now - Duration::days(DRAFT_RETENTION_DAYS)).to_rfc3339();
    let transaction = connection.unchecked_transaction()?;
    let changed = transaction.execute(
        "DELETE FROM current_chat_drafts
         WHERE current_chat_identity=?1 AND edited_at <= ?2",
        params![identity, cutoff],
    )?;
    if changed > 0 {
        bump_revision(&transaction, identity, now)?;
        refresh_retained_content_bytes(&transaction, identity)?;
    }
    transaction.commit()?;
    Ok(())
}
