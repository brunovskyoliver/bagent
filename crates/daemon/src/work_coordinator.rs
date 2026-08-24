//! Daemon-owned authority for admitted Conversation Turns and Automation Runs.

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::VecDeque, fmt, path::Path, sync::Mutex};

use crate::automation_sessions::{
    terminalize_automation_session_in_transaction, AutomationRunOutcome, AutomationTaskSnapshot,
    AutomationTerminalization,
};
use crate::current_chat::{
    begin_conversation_turn_in_transaction, complete_conversation_turn_in_transaction,
    interrupt_conversation_turn_in_transaction, BegunConversationTurn, SubmittedAttachmentMetadata,
    ValidatedSourceMetadata,
};

const SCHEMA: &str = include_str!("../migrations/V19__work_coordinator_foundations.sql");
const CUTOVER_SCHEMA: &str = include_str!("../migrations/V20__unified_work_cutover.sql");
const ACTIVITY_SCHEMA: &str = include_str!("../migrations/V21__work_activity_projection.sql");
const NOTCH_PROJECTION_INDEXES: &str =
    include_str!("../migrations/V22__notch_projection_indexes.sql");
const AUTOMATION_SESSION_SCHEMA: &str =
    include_str!("../migrations/V23__automation_session_contract.sql");
const SCHEMA_VERSION: u32 = 1;

pub(crate) fn insert_work_current_chat_if_present(
    connection: &Connection,
    identity: &str,
) -> rusqlite::Result<()> {
    if canonical_table_exists(connection, "work_current_chats")? {
        connection.execute(
            "INSERT OR IGNORE INTO work_current_chats (identity) VALUES (?1)",
            params![identity],
        )?;
    }
    Ok(())
}

pub(crate) fn mark_work_automation_session_viewed_if_present(
    connection: &Connection,
    automation_session_identity: &str,
) -> rusqlite::Result<()> {
    if canonical_table_exists(connection, "work_automation_sessions")? {
        connection.execute(
            "UPDATE work_automation_sessions SET attention_state='viewed'
             WHERE automation_session_identity=?1",
            params![automation_session_identity],
        )?;
    }
    Ok(())
}

pub(crate) fn delete_automation_work_if_present(
    connection: &Connection,
    automation_session_identity: &str,
    work_identity: &str,
) -> rusqlite::Result<()> {
    if !canonical_table_exists(connection, "works")? {
        return Ok(());
    }
    for table in [
        "work_event_outbox",
        "work_approvals",
        "work_activity_projection",
        "work_projections",
    ] {
        connection.execute(
            &format!("DELETE FROM {table} WHERE work_identity=?1"),
            params![work_identity],
        )?;
    }
    connection.execute(
        "DELETE FROM work_automation_sessions WHERE automation_session_identity=?1",
        params![automation_session_identity],
    )?;
    connection.execute(
        "DELETE FROM work_automation_runs WHERE work_identity=?1",
        params![work_identity],
    )?;
    connection.execute(
        "DELETE FROM works WHERE identity=?1",
        params![work_identity],
    )?;
    Ok(())
}

fn canonical_table_exists(connection: &Connection, table: &str) -> rusqlite::Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            params![table],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordinatorConfig {
    pub max_events: usize,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self { max_events: 1_024 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DaemonGeneration(String);

impl DaemonGeneration {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub trait WorkIdentitySource: Send {
    fn next_identity(&mut self) -> Result<WorkIdentity, CommandError>;
}

pub struct RandomWorkIdentitySource;

impl WorkIdentitySource for RandomWorkIdentitySource {
    fn next_identity(&mut self) -> Result<WorkIdentity, CommandError> {
        Ok(WorkIdentity::new(uuid::Uuid::new_v4().to_string()))
    }
}

pub struct DeterministicWorkIdentitySource {
    identities: VecDeque<String>,
}

impl DeterministicWorkIdentitySource {
    pub fn new(identities: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            identities: identities.into_iter().map(Into::into).collect(),
        }
    }
}

impl WorkIdentitySource for DeterministicWorkIdentitySource {
    fn next_identity(&mut self) -> Result<WorkIdentity, CommandError> {
        self.identities
            .pop_front()
            .map(WorkIdentity::new)
            .ok_or(CommandError::IdentitySourceExhausted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkIdentity(String);

impl WorkIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for WorkIdentity {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for WorkIdentity {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for WorkIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommandIdentity(String);

impl CommandIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for CommandIdentity {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for CommandIdentity {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

macro_rules! opaque_identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

opaque_identifier!(CurrentChatIdentity);
opaque_identifier!(ConversationTurnIdentity);
opaque_identifier!(AutomationRunIdentity);
opaque_identifier!(AutomationSessionIdentity);
opaque_identifier!(AutomationDefinitionIdentity);
opaque_identifier!(ApprovalIdentity);
opaque_identifier!(ModelRuntimeGeneration);

macro_rules! monotonic_value {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(u64);

        impl $name {
            pub fn new(value: u64) -> Self {
                Self(value)
            }

            pub fn value(self) -> u64 {
                self.0
            }
        }
    };
}

monotonic_value!(WorkRevision);
monotonic_value!(EventCursor);
monotonic_value!(AutomationDefinitionRevision);

pub trait CoordinatorClock: Send {
    fn now(&mut self) -> String;
}

pub struct SystemCoordinatorClock;

impl CoordinatorClock for SystemCoordinatorClock {
    fn now(&mut self) -> String {
        chrono::Utc::now().to_rfc3339()
    }
}

pub struct FixedCoordinatorClock {
    now: String,
}

impl FixedCoordinatorClock {
    pub fn new(now: impl Into<String>) -> Self {
        Self { now: now.into() }
    }
}

impl CoordinatorClock for FixedCoordinatorClock {
    fn now(&mut self) -> String {
        self.now.clone()
    }
}

pub struct CoordinatorDependencies {
    pub identity_source: Box<dyn WorkIdentitySource>,
    pub clock: Box<dyn CoordinatorClock>,
}

impl CoordinatorDependencies {
    pub fn production() -> Self {
        Self {
            identity_source: Box::new(RandomWorkIdentitySource),
            clock: Box::new(SystemCoordinatorClock),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkState {
    Queued,
    WaitingForModel,
    Running,
    WaitingForApproval,
    Cancelling,
    Completed,
    Partial,
    Failed,
    Cancelled,
    Abandoned,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkActivityCategory {
    Mail,
    Web,
    Filesystem,
    #[serde(rename = "odoo")]
    Odoo,
    Codex,
    Chat,
    Automation,
    GenericTool,
}

impl WorkActivityCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mail => "mail",
            Self::Web => "web",
            Self::Filesystem => "filesystem",
            Self::Odoo => "odoo",
            Self::Codex => "codex",
            Self::Chat => "chat",
            Self::Automation => "automation",
            Self::GenericTool => "generic_tool",
        }
    }

    fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "mail" => Ok(Self::Mail),
            "web" => Ok(Self::Web),
            "filesystem" => Ok(Self::Filesystem),
            "odoo" => Ok(Self::Odoo),
            "codex" => Ok(Self::Codex),
            "chat" => Ok(Self::Chat),
            "automation" => Ok(Self::Automation),
            "generic_tool" => Ok(Self::GenericTool),
            other => Err(CommandError::CorruptState(other.to_owned())),
        }
    }
}

impl WorkState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::WaitingForModel => "waiting_for_model",
            Self::Running => "running",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::Cancelling => "cancelling",
            Self::Completed => "completed",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }

    fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "queued" => Ok(Self::Queued),
            "waiting_for_model" => Ok(Self::WaitingForModel),
            "running" => Ok(Self::Running),
            "waiting_for_approval" => Ok(Self::WaitingForApproval),
            "cancelling" => Ok(Self::Cancelling),
            "completed" => Ok(Self::Completed),
            "partial" => Ok(Self::Partial),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(CommandError::CorruptState(other.to_owned())),
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Partial | Self::Failed | Self::Cancelled | Self::Abandoned
        )
    }

    fn permits(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::Queued,
                Self::WaitingForModel | Self::Cancelling | Self::Failed
            ) | (
                Self::WaitingForModel,
                Self::Running | Self::Cancelling | Self::Failed
            ) | (
                Self::Running,
                Self::WaitingForModel
                    | Self::Cancelling
                    | Self::Completed
                    | Self::Partial
                    | Self::Failed,
            ) | (
                Self::WaitingForApproval,
                Self::Running | Self::WaitingForModel | Self::Cancelling | Self::Failed,
            ) | (Self::Cancelling, Self::Cancelled | Self::Failed)
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin_kind", rename_all = "snake_case")]
pub enum WorkOrigin {
    Conversation {
        current_chat_identity: CurrentChatIdentity,
        conversation_turn_identity: ConversationTurnIdentity,
    },
    Automation {
        automation_run_identity: AutomationRunIdentity,
        automation_session_identity: AutomationSessionIdentity,
        historical_automation_identity: AutomationDefinitionIdentity,
        frozen_definition_revision: AutomationDefinitionRevision,
    },
}

impl WorkOrigin {
    fn database_parts(&self) -> (&'static str, &str, &str, Option<&str>, Option<u64>) {
        match self {
            Self::Conversation {
                current_chat_identity,
                conversation_turn_identity,
            } => (
                "conversation",
                current_chat_identity.as_str(),
                conversation_turn_identity.as_str(),
                None,
                None,
            ),
            Self::Automation {
                automation_run_identity,
                automation_session_identity,
                historical_automation_identity,
                frozen_definition_revision,
            } => (
                "automation",
                automation_run_identity.as_str(),
                automation_session_identity.as_str(),
                Some(historical_automation_identity.as_str()),
                Some(frozen_definition_revision.value()),
            ),
        }
    }

    fn from_database(
        kind: &str,
        primary: String,
        secondary: String,
        historical: Option<String>,
        definition_revision: Option<u64>,
    ) -> Result<Self, CommandError> {
        match kind {
            "conversation" => Ok(Self::Conversation {
                current_chat_identity: CurrentChatIdentity::new(primary),
                conversation_turn_identity: ConversationTurnIdentity::new(secondary),
            }),
            "automation" => Ok(Self::Automation {
                automation_run_identity: AutomationRunIdentity::new(primary),
                automation_session_identity: AutomationSessionIdentity::new(secondary),
                historical_automation_identity: AutomationDefinitionIdentity::new(
                    historical.ok_or_else(|| {
                        CommandError::CorruptState(
                            "missing historical automation identity".to_owned(),
                        )
                    })?,
                ),
                frozen_definition_revision: AutomationDefinitionRevision::new(
                    definition_revision.ok_or_else(|| {
                        CommandError::CorruptState("missing frozen definition revision".to_owned())
                    })?,
                ),
            }),
            other => Err(CommandError::CorruptState(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command_kind", rename_all = "snake_case")]
enum CommandKind {
    Create {
        origin: WorkOrigin,
    },
    Transition {
        work_identity: WorkIdentity,
        expected_revision: WorkRevision,
        next_state: WorkState,
        model_runtime_generation: Option<ModelRuntimeGeneration>,
    },
    SetActivity {
        work_identity: WorkIdentity,
        expected_revision: WorkRevision,
        category: Option<WorkActivityCategory>,
    },
    AcknowledgeAttention {
        work_identity: WorkIdentity,
        expected_revision: WorkRevision,
    },
    RequestApproval {
        work_identity: WorkIdentity,
        expected_revision: WorkRevision,
        approval_identity: ApprovalIdentity,
        category: String,
    },
    ResolveApproval {
        work_identity: WorkIdentity,
        expected_revision: WorkRevision,
        approval_identity: ApprovalIdentity,
        allow: bool,
        expected_decision_revision: u64,
    },
    Cancel {
        work_identity: WorkIdentity,
        expected_revision: WorkRevision,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    command_schema_version: u32,
    command_identity: CommandIdentity,
    expected_daemon_generation: DaemonGeneration,
    #[serde(flatten)]
    kind: CommandKind,
}

impl Command {
    pub fn create_conversation(
        command_identity: impl Into<CommandIdentity>,
        current_chat_identity: CurrentChatIdentity,
        conversation_turn_identity: ConversationTurnIdentity,
        generation: DaemonGeneration,
    ) -> Self {
        Self {
            command_schema_version: SCHEMA_VERSION,
            command_identity: command_identity.into(),
            expected_daemon_generation: generation,
            kind: CommandKind::Create {
                origin: WorkOrigin::Conversation {
                    current_chat_identity,
                    conversation_turn_identity,
                },
            },
        }
    }

    pub fn create_automation(
        command_identity: impl Into<CommandIdentity>,
        automation_run_identity: AutomationRunIdentity,
        automation_session_identity: AutomationSessionIdentity,
        historical_automation_identity: AutomationDefinitionIdentity,
        frozen_definition_revision: AutomationDefinitionRevision,
        generation: DaemonGeneration,
    ) -> Self {
        Self {
            command_schema_version: SCHEMA_VERSION,
            command_identity: command_identity.into(),
            expected_daemon_generation: generation,
            kind: CommandKind::Create {
                origin: WorkOrigin::Automation {
                    automation_run_identity,
                    automation_session_identity,
                    historical_automation_identity,
                    frozen_definition_revision,
                },
            },
        }
    }

    pub fn transition(
        command_identity: impl Into<CommandIdentity>,
        work_identity: impl Into<WorkIdentity>,
        expected_revision: WorkRevision,
        next_state: WorkState,
        generation: DaemonGeneration,
    ) -> Self {
        Self {
            command_schema_version: SCHEMA_VERSION,
            command_identity: command_identity.into(),
            expected_daemon_generation: generation,
            kind: CommandKind::Transition {
                work_identity: work_identity.into(),
                expected_revision,
                next_state,
                model_runtime_generation: None,
            },
        }
    }

    pub fn transition_with_model_runtime(
        command_identity: impl Into<CommandIdentity>,
        work_identity: impl Into<WorkIdentity>,
        expected_revision: WorkRevision,
        next_state: WorkState,
        model_runtime_generation: ModelRuntimeGeneration,
        generation: DaemonGeneration,
    ) -> Self {
        let mut command = Self::transition(
            command_identity,
            work_identity,
            expected_revision,
            next_state,
            generation,
        );
        if let CommandKind::Transition {
            model_runtime_generation: stored,
            ..
        } = &mut command.kind
        {
            *stored = Some(model_runtime_generation);
        }
        command
    }

    pub fn set_activity(
        command_identity: impl Into<CommandIdentity>,
        work_identity: impl Into<WorkIdentity>,
        expected_revision: WorkRevision,
        category: Option<WorkActivityCategory>,
        generation: DaemonGeneration,
    ) -> Self {
        Self {
            command_schema_version: SCHEMA_VERSION,
            command_identity: command_identity.into(),
            expected_daemon_generation: generation,
            kind: CommandKind::SetActivity {
                work_identity: work_identity.into(),
                expected_revision,
                category,
            },
        }
    }

    pub fn acknowledge_attention(
        command_identity: impl Into<CommandIdentity>,
        work_identity: impl Into<WorkIdentity>,
        expected_revision: WorkRevision,
        generation: DaemonGeneration,
    ) -> Self {
        Self {
            command_schema_version: SCHEMA_VERSION,
            command_identity: command_identity.into(),
            expected_daemon_generation: generation,
            kind: CommandKind::AcknowledgeAttention {
                work_identity: work_identity.into(),
                expected_revision,
            },
        }
    }

    pub fn request_approval(
        command_identity: impl Into<CommandIdentity>,
        work_identity: impl Into<WorkIdentity>,
        expected_revision: WorkRevision,
        approval_identity: ApprovalIdentity,
        category: impl Into<String>,
        generation: DaemonGeneration,
    ) -> Self {
        Self {
            command_schema_version: SCHEMA_VERSION,
            command_identity: command_identity.into(),
            expected_daemon_generation: generation,
            kind: CommandKind::RequestApproval {
                work_identity: work_identity.into(),
                expected_revision,
                approval_identity,
                category: category.into(),
            },
        }
    }

    pub fn resolve_approval(
        command_identity: impl Into<CommandIdentity>,
        work_identity: impl Into<WorkIdentity>,
        expected_revision: WorkRevision,
        approval_identity: ApprovalIdentity,
        allow: bool,
        expected_decision_revision: u64,
        generation: DaemonGeneration,
    ) -> Self {
        Self {
            command_schema_version: SCHEMA_VERSION,
            command_identity: command_identity.into(),
            expected_daemon_generation: generation,
            kind: CommandKind::ResolveApproval {
                work_identity: work_identity.into(),
                expected_revision,
                approval_identity,
                allow,
                expected_decision_revision,
            },
        }
    }

    pub fn cancel(
        command_identity: impl Into<CommandIdentity>,
        work_identity: impl Into<WorkIdentity>,
        expected_revision: WorkRevision,
        generation: DaemonGeneration,
    ) -> Self {
        Self {
            command_schema_version: SCHEMA_VERSION,
            command_identity: command_identity.into(),
            expected_daemon_generation: generation,
            kind: CommandKind::Cancel {
                work_identity: work_identity.into(),
                expected_revision,
            },
        }
    }

    fn hash(&self) -> Result<String, CommandError> {
        let bytes = serde_json::to_vec(self).map_err(CommandError::serialization)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitReceipt {
    pub work_identity: WorkIdentity,
    pub work_revision: WorkRevision,
    pub state: WorkState,
    pub event_cursor: EventCursor,
    pub daemon_generation: DaemonGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandAcknowledgement {
    Committed(CommitReceipt),
}

impl CommandAcknowledgement {
    pub fn receipt(&self) -> &CommitReceipt {
        match self {
            Self::Committed(receipt) => receipt,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailurePoint {
    BeforeTransaction,
    AfterStateMutation,
    AfterOutboxInsert,
    AtCommit,
    AfterCommitBeforeResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentChatTerminalOutcome {
    Completed,
    InterruptedContentBound,
    InterruptedExecutionFailed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommandError {
    InjectedFailure(FailurePoint),
    CommandIdentityConflict,
    Conflict {
        current_revision: Option<WorkRevision>,
    },
    IllegalTransition {
        from: WorkState,
        to: WorkState,
    },
    TerminalTarget,
    StaleDaemonGeneration {
        current: DaemonGeneration,
    },
    IdentitySourceExhausted,
    CorruptState(String),
    Storage(String),
}

impl CommandError {
    fn storage(error: impl fmt::Display) -> Self {
        Self::Storage(error.to_string())
    }

    fn serialization(error: impl fmt::Display) -> Self {
        Self::Storage(error.to_string())
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for CommandError {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRecord {
    pub identity: WorkIdentity,
    pub origin: WorkOrigin,
    pub state: WorkState,
    pub revision: WorkRevision,
    pub activity: Option<WorkActivityCategory>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkSnapshot {
    pub schema_version: u32,
    pub cursor: EventCursor,
    pub daemon_generation: DaemonGeneration,
    pub works: Vec<WorkRecord>,
    pub automation_runs: Vec<AutomationRunRecord>,
    pub approvals: Vec<ApprovalRecord>,
    pub interruptions: Vec<InterruptionMarker>,
    pub model_runtime_generation: Option<ModelRuntimeGeneration>,
    pub model_runtime_trusted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationRunRecord {
    pub identity: AutomationRunIdentity,
    pub work_identity: WorkIdentity,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Pending,
    Allowed,
    Denied,
    Expired,
    Withdrawn,
    Abandoned,
}

impl ApprovalState {
    fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "pending" => Ok(Self::Pending),
            "allowed" => Ok(Self::Allowed),
            "denied" => Ok(Self::Denied),
            "expired" => Ok(Self::Expired),
            "withdrawn" => Ok(Self::Withdrawn),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(CommandError::CorruptState(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub identity: ApprovalIdentity,
    pub work_identity: WorkIdentity,
    pub category: String,
    pub state: ApprovalState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterruptionMarker {
    pub conversation_turn_identity: ConversationTurnIdentity,
    pub daemon_generation: DaemonGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkEvent {
    pub schema_version: u32,
    pub event_cursor: EventCursor,
    pub daemon_generation: DaemonGeneration,
    pub committed_at: String,
    pub event_kind: EventKind,
    pub work_identity: WorkIdentity,
    pub work_revision: WorkRevision,
    pub state: WorkState,
    pub activity: Option<WorkActivityCategory>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    WorkCreated,
    WorkStateChanged,
    WorkRecovered,
    WorkActivityChanged,
    CompletionAttentionAcknowledged,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::WorkCreated => "work_created",
            Self::WorkStateChanged => "work_state_changed",
            Self::WorkRecovered => "work_recovered",
            Self::WorkActivityChanged => "work_activity_changed",
            Self::CompletionAttentionAcknowledged => "completion_attention_acknowledged",
        }
    }

    fn parse(value: &str) -> Result<Self, CommandError> {
        match value {
            "work_created" => Ok(Self::WorkCreated),
            "work_state_changed" => Ok(Self::WorkStateChanged),
            "work_recovered" => Ok(Self::WorkRecovered),
            "work_activity_changed" => Ok(Self::WorkActivityChanged),
            "completion_attention_acknowledged" => Ok(Self::CompletionAttentionAcknowledged),
            other => Err(CommandError::CorruptState(other.to_owned())),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventRead {
    Events(Vec<WorkEvent>),
    Gap { snapshot: WorkSnapshot },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactWorkProjection {
    pub schema_version: u32,
    pub event_cursor: EventCursor,
    pub daemon_generation: DaemonGeneration,
    pub work: Vec<CompactWorkItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactWorkItem {
    pub work_identity: WorkIdentity,
    pub revision: WorkRevision,
    pub state: WorkState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkDiagnostic {
    pub schema_version: u32,
    pub event_cursor: EventCursor,
    pub work_count: usize,
    pub pending_approval_count: usize,
}

impl WorkSnapshot {
    pub fn compact_projection(&self) -> CompactWorkProjection {
        CompactWorkProjection {
            schema_version: self.schema_version,
            event_cursor: self.cursor,
            daemon_generation: self.daemon_generation.clone(),
            work: self
                .works
                .iter()
                .map(|work| CompactWorkItem {
                    work_identity: work.identity.clone(),
                    revision: work.revision,
                    state: work.state,
                })
                .collect(),
        }
    }

    pub fn structural_diagnostic(&self) -> WorkDiagnostic {
        WorkDiagnostic {
            schema_version: self.schema_version,
            event_cursor: self.cursor,
            work_count: self.works.len(),
            pending_approval_count: self
                .approvals
                .iter()
                .filter(|approval| approval.state == ApprovalState::Pending)
                .count(),
        }
    }
}

pub struct WorkCoordinator {
    connection: Mutex<Connection>,
    config: CoordinatorConfig,
    dependencies: Mutex<CoordinatorDependencies>,
}

impl WorkCoordinator {
    pub fn open(
        path: impl AsRef<Path>,
        config: CoordinatorConfig,
        generation: DaemonGeneration,
    ) -> Result<Self, CommandError> {
        Self::open_with_dependencies(
            path,
            config,
            generation,
            CoordinatorDependencies::production(),
        )
    }

    pub fn open_with_dependencies(
        path: impl AsRef<Path>,
        config: CoordinatorConfig,
        generation: DaemonGeneration,
        mut dependencies: CoordinatorDependencies,
    ) -> Result<Self, CommandError> {
        if config.max_events == 0 {
            return Err(CommandError::Storage(
                "max_events must be greater than zero".to_owned(),
            ));
        }
        let connection = Connection::open(path).map_err(CommandError::storage)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(CommandError::storage)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
            .map_err(CommandError::storage)?;
        connection
            .execute_batch(SCHEMA)
            .map_err(CommandError::storage)?;
        let has_decision_revision = {
            let mut statement = connection
                .prepare("PRAGMA table_info(work_approvals)")
                .map_err(CommandError::storage)?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(CommandError::storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommandError::storage)?;
            columns.iter().any(|column| column == "decision_revision")
        };
        if !has_decision_revision {
            connection
                .execute_batch(CUTOVER_SCHEMA)
                .map_err(CommandError::storage)?;
        }
        connection
            .execute_batch(ACTIVITY_SCHEMA)
            .map_err(CommandError::storage)?;
        connection
            .execute_batch(NOTCH_PROJECTION_INDEXES)
            .map_err(CommandError::storage)?;
        connection
            .execute_batch(AUTOMATION_SESSION_SCHEMA)
            .map_err(CommandError::storage)?;
        // V19 is also used as an idempotent bootstrap schema. Its temporary
        // chat-identity table must not be resurrected after the V22 cutover.
        connection
            .execute_batch("DROP TABLE IF EXISTS automation_current_chats;")
            .map_err(CommandError::storage)?;
        connection
            .execute_batch("DROP TABLE IF EXISTS automation_session_pending_approvals;")
            .map_err(CommandError::storage)?;
        initialize_generation(
            &connection,
            &generation,
            config.max_events,
            dependencies.clock.as_mut(),
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            config,
            dependencies: Mutex::new(dependencies),
        })
    }

    pub fn submit(&self, command: Command) -> Result<CommandAcknowledgement, CommandError> {
        self.submit_with_optional_failure(command, None)
    }

    /// Admit the durable user turn and its foreground Work under one SQLite
    /// commit. Neither record can become visible without the other.
    #[allow(clippy::too_many_arguments)]
    pub fn submit_current_chat_turn(
        &self,
        command_identity: impl Into<CommandIdentity>,
        current_chat_identity: CurrentChatIdentity,
        expected_revision: u64,
        user_message: &str,
        attachments: &[SubmittedAttachmentMetadata],
        submitted_at: chrono::DateTime<chrono::Utc>,
        generation: DaemonGeneration,
    ) -> Result<(CommandAcknowledgement, BegunConversationTurn), CommandError> {
        let turn_identity = ConversationTurnIdentity::new(uuid::Uuid::new_v4().to_string());
        let command = Command::create_conversation(
            command_identity,
            current_chat_identity.clone(),
            turn_identity.clone(),
            generation,
        );
        let command_hash = command.hash()?;
        let mut connection = self.connection.lock().expect("coordinator mutex poisoned");
        let current_generation = metadata(&connection, "daemon_generation")?
            .ok_or_else(|| CommandError::CorruptState("missing daemon generation".to_owned()))?;
        if command.expected_daemon_generation.as_str() != current_generation {
            return Err(CommandError::StaleDaemonGeneration {
                current: DaemonGeneration::new(current_generation),
            });
        }

        let mut dependencies = self.dependencies.lock().expect("dependency mutex poisoned");
        let generated_identity = dependencies.identity_source.next_identity()?;
        let committed_at = dependencies.clock.now();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(CommandError::storage)?;
        let begun = begin_conversation_turn_in_transaction(
            &transaction,
            current_chat_identity.as_str(),
            expected_revision,
            turn_identity.as_str(),
            user_message,
            attachments,
            submitted_at,
        )
        .map_err(CommandError::storage)?;
        let (work_identity, revision, state, event_kind) = apply_command(
            &transaction,
            &command,
            Some(&generated_identity),
            &committed_at,
        )?;
        let activity = load_work_activity(&transaction, &work_identity)?;
        let payload = serde_json::json!({ "state": state, "activity": activity });
        transaction
            .execute(
                "INSERT INTO work_event_outbox
                 (schema_version, daemon_generation, committed_at, event_kind,
                  work_identity, work_revision, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    SCHEMA_VERSION,
                    current_generation,
                    committed_at,
                    event_kind.as_str(),
                    work_identity.as_str(),
                    revision.value(),
                    payload.to_string(),
                ],
            )
            .map_err(CommandError::storage)?;
        let event_cursor = EventCursor::new(transaction.last_insert_rowid() as u64);
        set_metadata(
            &transaction,
            "event_cursor",
            &event_cursor.value().to_string(),
        )?;
        let receipt = CommitReceipt {
            work_identity,
            work_revision: revision,
            state,
            event_cursor,
            daemon_generation: DaemonGeneration::new(&current_generation),
        };
        transaction
            .execute(
                "INSERT INTO work_command_results
                 (command_identity, command_hash, acknowledgement, committed_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    command.command_identity.as_str(),
                    command_hash,
                    serde_json::to_string(&receipt).map_err(CommandError::serialization)?,
                    committed_at,
                ],
            )
            .map_err(CommandError::storage)?;
        prune_events(&transaction, self.config.max_events)?;
        transaction.commit().map_err(CommandError::storage)?;
        Ok((CommandAcknowledgement::Committed(receipt), begun))
    }

    /// Terminalize an Automation Session and its Work in one SQLite
    /// transaction. The ordinary transition command is intentionally not
    /// used here: session content and the Work terminal mutation have one
    /// rollback boundary.
    pub fn terminalize_automation_session(
        &self,
        command_identity: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        next_state: WorkState,
        input: crate::automation_sessions::AutomationTerminalization,
    ) -> Result<WorkRevision, CommandError> {
        if !next_state.is_terminal() {
            return Err(CommandError::IllegalTransition {
                from: WorkState::Running,
                to: next_state,
            });
        }
        let mut connection = self.connection.lock().expect("coordinator mutex poisoned");
        let generation = metadata(&connection, "daemon_generation")?
            .ok_or_else(|| CommandError::CorruptState("missing daemon generation".to_owned()))?;
        let committed_at = self
            .dependencies
            .lock()
            .expect("dependency mutex poisoned")
            .clock
            .now();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(CommandError::storage)?;
        let command = Command::transition(
            command_identity,
            work.clone(),
            revision,
            next_state,
            DaemonGeneration::new(generation.clone()),
        );
        let (work_identity, next_revision, state, event_kind) =
            apply_command(&transaction, &command, None, &committed_at)?;
        if input.work_identity != work_identity.as_str() {
            return Err(CommandError::Conflict {
                current_revision: Some(next_revision),
            });
        }
        crate::automation_sessions::terminalize_automation_session_in_transaction(
            &transaction,
            input,
        )
        .map_err(|error| CommandError::Storage(error.to_string()))?;
        let activity = load_work_activity(&transaction, &work_identity)?;
        let payload = serde_json::json!({ "state": state, "activity": activity });
        transaction
            .execute(
                "INSERT INTO work_event_outbox
                    (schema_version, daemon_generation, committed_at, event_kind, work_identity, work_revision, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    SCHEMA_VERSION,
                    generation,
                    committed_at,
                    event_kind.as_str(),
                    work_identity.as_str(),
                    next_revision.value(),
                    payload.to_string()
                ],
            )
            .map_err(CommandError::storage)?;
        let cursor = EventCursor::new(transaction.last_insert_rowid() as u64);
        set_metadata(&transaction, "event_cursor", &cursor.value().to_string())?;
        prune_events(&transaction, self.config.max_events)?;
        transaction.commit().map_err(CommandError::storage)?;
        Ok(next_revision)
    }

    /// Terminalize one Conversation Turn and its Work through a single
    /// immediate SQLite transaction. `assistant_output == None` is the
    /// normalized execution-failure path; a completion that exceeds the
    /// retained-content bound atomically interrupts the turn and fails Work.
    #[allow(clippy::too_many_arguments)]
    pub fn terminalize_current_chat_turn(
        &self,
        command_identity: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        current_chat_identity: &str,
        conversation_turn_identity: &str,
        assistant_output: Option<&str>,
        validated_sources: &[ValidatedSourceMetadata],
        terminal_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(WorkRevision, CurrentChatTerminalOutcome), CommandError> {
        self.terminalize_current_chat_turn_with_optional_failure(
            command_identity,
            work,
            revision,
            current_chat_identity,
            conversation_turn_identity,
            assistant_output,
            validated_sources,
            terminal_at,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn terminalize_current_chat_turn_with_failure(
        &self,
        command_identity: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        current_chat_identity: &str,
        conversation_turn_identity: &str,
        assistant_output: Option<&str>,
        validated_sources: &[ValidatedSourceMetadata],
        terminal_at: chrono::DateTime<chrono::Utc>,
        failure: FailurePoint,
    ) -> Result<(WorkRevision, CurrentChatTerminalOutcome), CommandError> {
        self.terminalize_current_chat_turn_with_optional_failure(
            command_identity,
            work,
            revision,
            current_chat_identity,
            conversation_turn_identity,
            assistant_output,
            validated_sources,
            terminal_at,
            Some(failure),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn terminalize_current_chat_turn_with_optional_failure(
        &self,
        command_identity: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        current_chat_identity: &str,
        conversation_turn_identity: &str,
        assistant_output: Option<&str>,
        validated_sources: &[ValidatedSourceMetadata],
        terminal_at: chrono::DateTime<chrono::Utc>,
        failure: Option<FailurePoint>,
    ) -> Result<(WorkRevision, CurrentChatTerminalOutcome), CommandError> {
        let mut connection = self.connection.lock().expect("coordinator mutex poisoned");
        let generation = metadata(&connection, "daemon_generation")?
            .ok_or_else(|| CommandError::CorruptState("missing daemon generation".to_owned()))?;
        let committed_at = terminal_at.to_rfc3339();
        inject(failure, FailurePoint::BeforeTransaction)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(CommandError::storage)?;
        let origin_matches: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM works
                 WHERE identity=?1 AND origin_kind='conversation'
                   AND origin_primary_identity=?2 AND origin_secondary_identity=?3)",
                params![
                    work.as_str(),
                    current_chat_identity,
                    conversation_turn_identity
                ],
                |row| row.get(0),
            )
            .map_err(CommandError::storage)?;
        if !origin_matches {
            return Err(CommandError::CorruptState(
                "Conversation Work origin does not match its Current Chat turn".to_owned(),
            ));
        }

        let (outcome, next_state) = if let Some(output) = assistant_output {
            let content_bound = complete_conversation_turn_in_transaction(
                &transaction,
                current_chat_identity,
                conversation_turn_identity,
                output,
                terminal_at,
                Some(work.as_str()),
                validated_sources,
            )
            .map_err(CommandError::storage)?;
            if content_bound {
                (
                    CurrentChatTerminalOutcome::InterruptedContentBound,
                    WorkState::Failed,
                )
            } else {
                (CurrentChatTerminalOutcome::Completed, WorkState::Completed)
            }
        } else {
            interrupt_conversation_turn_in_transaction(
                &transaction,
                current_chat_identity,
                conversation_turn_identity,
                terminal_at,
                Some(work.as_str()),
            )
            .map_err(CommandError::storage)?;
            (
                CurrentChatTerminalOutcome::InterruptedExecutionFailed,
                WorkState::Failed,
            )
        };
        inject(failure, FailurePoint::AfterStateMutation)?;

        let command = Command::transition(
            command_identity,
            work.clone(),
            revision,
            next_state,
            DaemonGeneration::new(generation.clone()),
        );
        let (work_identity, next_revision, state, event_kind) =
            apply_command(&transaction, &command, None, &committed_at)?;
        let activity = load_work_activity(&transaction, &work_identity)?;
        let payload = serde_json::json!({ "state": state, "activity": activity });
        transaction
            .execute(
                "INSERT INTO work_event_outbox
                 (schema_version, daemon_generation, committed_at, event_kind,
                  work_identity, work_revision, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    SCHEMA_VERSION,
                    generation,
                    committed_at,
                    event_kind.as_str(),
                    work_identity.as_str(),
                    next_revision.value(),
                    payload.to_string(),
                ],
            )
            .map_err(CommandError::storage)?;
        inject(failure, FailurePoint::AfterOutboxInsert)?;
        let cursor = EventCursor::new(transaction.last_insert_rowid() as u64);
        set_metadata(&transaction, "event_cursor", &cursor.value().to_string())?;
        prune_events(&transaction, self.config.max_events)?;
        inject(failure, FailurePoint::AtCommit)?;
        transaction.commit().map_err(CommandError::storage)?;
        Ok((next_revision, outcome))
    }

    pub fn submit_with_failure(
        &self,
        command: Command,
        failure: FailurePoint,
    ) -> Result<CommandAcknowledgement, CommandError> {
        self.submit_with_optional_failure(command, Some(failure))
    }

    fn submit_with_optional_failure(
        &self,
        command: Command,
        failure: Option<FailurePoint>,
    ) -> Result<CommandAcknowledgement, CommandError> {
        let command_hash = command.hash()?;
        let mut connection = self.connection.lock().expect("coordinator mutex poisoned");
        if let Some((stored_hash, acknowledgement)) = connection
            .query_row(
                "SELECT command_hash, acknowledgement FROM work_command_results WHERE command_identity = ?1",
                params![command.command_identity.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(CommandError::storage)?
        {
            if stored_hash != command_hash {
                return Err(CommandError::CommandIdentityConflict);
            }
            let receipt = serde_json::from_str(&acknowledgement)
                .map_err(CommandError::serialization)?;
            return Ok(CommandAcknowledgement::Committed(receipt));
        }

        let current_generation = metadata(&connection, "daemon_generation")?
            .ok_or_else(|| CommandError::CorruptState("missing daemon generation".to_owned()))?;
        if command.expected_daemon_generation.as_str() != current_generation {
            return Err(CommandError::StaleDaemonGeneration {
                current: DaemonGeneration::new(current_generation),
            });
        }

        inject(failure, FailurePoint::BeforeTransaction)?;
        let mut dependencies = self.dependencies.lock().expect("dependency mutex poisoned");
        let generated_identity = if matches!(command.kind, CommandKind::Create { .. }) {
            Some(dependencies.identity_source.next_identity()?)
        } else {
            None
        };
        let committed_at = dependencies.clock.now();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(CommandError::storage)?;
        let (work_identity, revision, state, event_kind) = apply_command(
            &transaction,
            &command,
            generated_identity.as_ref(),
            &committed_at,
        )?;
        inject(failure, FailurePoint::AfterStateMutation)?;

        let activity = load_work_activity(&transaction, &work_identity)?;
        let payload = serde_json::json!({ "state": state, "activity": activity });
        transaction
            .execute(
                "INSERT INTO work_event_outbox
                    (schema_version, daemon_generation, committed_at, event_kind, work_identity, work_revision, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    SCHEMA_VERSION,
                    current_generation,
                    committed_at,
                    event_kind.as_str(),
                    work_identity.as_str(),
                    revision.value(),
                    payload.to_string()
                ],
            )
            .map_err(CommandError::storage)?;
        let event_cursor = EventCursor::new(transaction.last_insert_rowid() as u64);
        set_metadata(
            &transaction,
            "event_cursor",
            &event_cursor.value().to_string(),
        )?;
        inject(failure, FailurePoint::AfterOutboxInsert)?;

        let receipt = CommitReceipt {
            work_identity,
            work_revision: revision,
            state,
            event_cursor,
            daemon_generation: DaemonGeneration::new(&current_generation),
        };
        transaction
            .execute(
                "INSERT INTO work_command_results
                    (command_identity, command_hash, acknowledgement, committed_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    command.command_identity.as_str(),
                    command_hash,
                    serde_json::to_string(&receipt).map_err(CommandError::serialization)?,
                    committed_at
                ],
            )
            .map_err(CommandError::storage)?;
        prune_events(&transaction, self.config.max_events)?;
        inject(failure, FailurePoint::AtCommit)?;
        transaction.commit().map_err(CommandError::storage)?;
        inject(failure, FailurePoint::AfterCommitBeforeResponse)?;
        Ok(CommandAcknowledgement::Committed(receipt))
    }

    pub fn snapshot(&self) -> Result<WorkSnapshot, CommandError> {
        let connection = self.connection.lock().expect("coordinator mutex poisoned");
        let transaction = connection
            .unchecked_transaction()
            .map_err(CommandError::storage)?;
        let snapshot = snapshot_from(&transaction)?;
        transaction.commit().map_err(CommandError::storage)?;
        Ok(snapshot)
    }

    pub fn work(&self, identity: &WorkIdentity) -> Result<Option<WorkRecord>, CommandError> {
        let connection = self.connection.lock().expect("coordinator mutex poisoned");
        connection
            .query_row(
                "SELECT w.origin_kind, w.origin_primary_identity, w.origin_secondary_identity,
                        w.origin_historical_identity, w.origin_definition_revision, w.state,
                        w.revision, p.category
                 FROM works w
                 LEFT JOIN work_activity_projection p ON p.work_identity = w.identity
                 WHERE w.identity = ?1",
                params![identity.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<u64>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, u64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(CommandError::storage)?
            .map(
                |(
                    kind,
                    primary,
                    secondary,
                    historical,
                    definition_revision,
                    state,
                    revision,
                    activity,
                )| {
                    Ok(WorkRecord {
                        identity: identity.clone(),
                        origin: WorkOrigin::from_database(
                            &kind,
                            primary,
                            secondary,
                            historical,
                            definition_revision,
                        )?,
                        state: WorkState::parse(&state)?,
                        revision: WorkRevision::new(revision),
                        activity: activity
                            .as_deref()
                            .map(WorkActivityCategory::parse)
                            .transpose()?,
                    })
                },
            )
            .transpose()
    }

    /// Reads projection metadata from the same SQLite transaction as the Work
    /// snapshot so a UI cannot observe mixed revisions across tables.
    pub fn projected_snapshot<T>(
        &self,
        project: impl FnOnce(&Connection, &WorkSnapshot) -> Result<T, CommandError>,
    ) -> Result<(WorkSnapshot, T), CommandError> {
        let connection = self.connection.lock().expect("coordinator mutex poisoned");
        let transaction = connection
            .unchecked_transaction()
            .map_err(CommandError::storage)?;
        let snapshot = notch_snapshot_from(&transaction)?;
        let projection = project(&transaction, &snapshot)?;
        transaction.commit().map_err(CommandError::storage)?;
        Ok((snapshot, projection))
    }

    pub fn events(
        &self,
        after_cursor: Option<EventCursor>,
        expected_generation: &DaemonGeneration,
    ) -> Result<EventRead, CommandError> {
        let connection = self.connection.lock().expect("coordinator mutex poisoned");
        let transaction = connection
            .unchecked_transaction()
            .map_err(CommandError::storage)?;
        let snapshot = snapshot_from(&transaction)?;
        let Some(after_cursor) = after_cursor else {
            transaction.commit().map_err(CommandError::storage)?;
            return Ok(EventRead::Gap { snapshot });
        };
        let earliest = transaction
            .query_row("SELECT MIN(cursor) FROM work_event_outbox", [], |row| {
                row.get::<_, Option<u64>>(0)
            })
            .map_err(CommandError::storage)?;
        if expected_generation != &snapshot.daemon_generation
            || after_cursor > snapshot.cursor
            || earliest.is_some_and(|cursor| after_cursor.value().saturating_add(1) < cursor)
        {
            transaction.commit().map_err(CommandError::storage)?;
            return Ok(EventRead::Gap { snapshot });
        }

        let events = {
            let mut statement = transaction
                .prepare(
                    "SELECT cursor, schema_version, daemon_generation, committed_at, event_kind,
                        work_identity, work_revision, payload
                 FROM work_event_outbox WHERE cursor > ?1 ORDER BY cursor ASC",
                )
                .map_err(CommandError::storage)?;
            let rows = statement
                .query_map(params![after_cursor.value()], row_to_event)
                .map_err(CommandError::storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommandError::storage)?;
            rows
        };
        transaction.commit().map_err(CommandError::storage)?;
        Ok(EventRead::Events(events))
    }

    /// Event polling seam for the compact notch. Empty reads touch only
    /// metadata and the bounded outbox; gap snapshots use the bounded notch
    /// projection instead of materializing retained history.
    pub fn notch_events(
        &self,
        after_cursor: EventCursor,
        expected_generation: &DaemonGeneration,
    ) -> Result<EventRead, CommandError> {
        let connection = self.connection.lock().expect("coordinator mutex poisoned");
        let transaction = connection
            .unchecked_transaction()
            .map_err(CommandError::storage)?;
        let daemon_generation = metadata(&transaction, "daemon_generation")?
            .ok_or_else(|| CommandError::CorruptState("missing daemon generation".to_owned()))?;
        let cursor = EventCursor::new(
            metadata(&transaction, "event_cursor")?
                .unwrap_or_else(|| "0".to_owned())
                .parse::<u64>()
                .map_err(CommandError::serialization)?,
        );
        let earliest = transaction
            .query_row("SELECT MIN(cursor) FROM work_event_outbox", [], |row| {
                row.get::<_, Option<u64>>(0)
            })
            .map_err(CommandError::storage)?;
        if expected_generation.as_str() != daemon_generation
            || after_cursor > cursor
            || earliest.is_some_and(|value| after_cursor.value().saturating_add(1) < value)
        {
            let snapshot = notch_snapshot_from(&transaction)?;
            transaction.commit().map_err(CommandError::storage)?;
            return Ok(EventRead::Gap { snapshot });
        }
        let events = {
            let mut statement = transaction
                .prepare(
                    "SELECT cursor, schema_version, daemon_generation, committed_at, event_kind,
                            work_identity, work_revision, payload
                     FROM work_event_outbox WHERE cursor > ?1 ORDER BY cursor ASC",
                )
                .map_err(CommandError::storage)?;
            let rows = statement
                .query_map(params![after_cursor.value()], row_to_event)
                .map_err(CommandError::storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommandError::storage)?;
            rows
        };
        transaction.commit().map_err(CommandError::storage)?;
        Ok(EventRead::Events(events))
    }

    pub fn verify_integrity(&self) -> Result<String, CommandError> {
        let connection = self.connection.lock().expect("coordinator mutex poisoned");
        connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(CommandError::storage)
    }
}

fn initialize_generation(
    connection: &Connection,
    generation: &DaemonGeneration,
    max_events: usize,
    clock: &mut dyn CoordinatorClock,
) -> Result<(), CommandError> {
    let previous = metadata(connection, "daemon_generation")?;
    if previous.as_deref() == Some(generation.as_str()) {
        return Ok(());
    }
    let transaction = connection
        .unchecked_transaction()
        .map_err(CommandError::storage)?;
    set_metadata(&transaction, "daemon_generation", generation.as_str())?;
    if metadata(&transaction, "event_cursor")?.is_none() {
        set_metadata(&transaction, "event_cursor", "0")?;
    }
    if previous.is_some() {
        let recoverable = {
            let mut statement = transaction
                .prepare(
                    "SELECT identity, revision, state, origin_kind, origin_secondary_identity
                     FROM works ORDER BY identity ASC",
                )
                .map_err(CommandError::storage)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .map_err(CommandError::storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(CommandError::storage)?;
            rows.into_iter()
                .map(
                    |(identity, revision, state, origin_kind, origin_identity)| {
                        Ok((
                            identity,
                            revision,
                            WorkState::parse(&state)?,
                            origin_kind,
                            origin_identity,
                        ))
                    },
                )
                .collect::<Result<Vec<_>, CommandError>>()?
        };
        for (work_identity, old_revision, _old_state, origin_kind, origin_identity) in recoverable
            .into_iter()
            .filter(|(_, _, state, _, _)| !state.is_terminal())
        {
            let revision = old_revision + 1;
            let committed_at = clock.now();
            transaction
                .execute(
                    "UPDATE works SET state = 'abandoned', revision = ?1, updated_at = ?2
                     WHERE identity = ?3 AND revision = ?4",
                    params![revision, committed_at, work_identity, old_revision],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "UPDATE work_approvals
                     SET state = 'abandoned', resolved_at = ?1
                     WHERE work_identity = ?2 AND state = 'pending'",
                    params![committed_at, work_identity],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "UPDATE work_automation_runs SET active = 0 WHERE work_identity = ?1",
                    params![work_identity],
                )
                .map_err(CommandError::storage)?;
            if origin_kind == "automation" {
                let session = transaction
                    .query_row(
                        "SELECT r.automation_run_identity, r.automation_session_identity,
                                r.historical_automation_identity,
                                t.display_name,
                                t.task_text, t.schedule_json, t.timezone,
                                COALESCE(t.definition_revision, r.frozen_definition_revision)
                         FROM work_automation_runs r
                         LEFT JOIN automation_task_snapshots t
                           ON t.automation_session_identity=r.automation_session_identity
                         WHERE r.work_identity=?1",
                        params![work_identity],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, Option<String>>(3)?,
                                row.get::<_, Option<String>>(4)?,
                                row.get::<_, Option<String>>(5)?,
                                row.get::<_, Option<String>>(6)?,
                                row.get::<_, i64>(7)?,
                            ))
                        },
                    )
                    .optional()
                    .map_err(CommandError::storage)?
                    .ok_or_else(|| {
                        CommandError::CorruptState(
                            "automation Work is missing its Run linkage".to_owned(),
                        )
                    })?;
                let (
                    run_identity,
                    session_identity,
                    definition_identity,
                    display_name,
                    task_text,
                    schedule_json,
                    timezone,
                    definition_revision,
                ) = session;
                if let (Some(display_name), Some(task_text), Some(schedule_json), Some(timezone)) =
                    (display_name, task_text, schedule_json, timezone)
                {
                    transaction
                        .execute(
                            "INSERT OR IGNORE INTO automation_work_states
                            (work_identity, automation_run_identity, state)
                         VALUES (?1, ?2, 'running')",
                            params![work_identity, run_identity],
                        )
                        .map_err(CommandError::storage)?;
                    terminalize_automation_session_in_transaction(
                        &transaction,
                        AutomationTerminalization {
                            snapshot: AutomationTaskSnapshot {
                                automation_identity: definition_identity,
                                automation_run_identity: run_identity,
                                automation_session_identity: session_identity.clone(),
                                display_name,
                                task_text,
                                schedule_json,
                                timezone,
                                definition_revision,
                            },
                            work_identity: work_identity.clone(),
                            outcome: AutomationRunOutcome::Abandoned,
                            finished_at: committed_at.clone(),
                            result_summary: Some(
                                "Daemon sa reštartoval pred dokončením Automation Run.".to_owned(),
                            ),
                            final_output: None,
                            activity_timeline: Vec::new(),
                            validated_sources: Vec::new(),
                            connector_references: Vec::new(),
                            historical_approvals: Vec::new(),
                            truncation_disclosures: Vec::new(),
                        },
                    )
                    .map_err(|error| CommandError::Storage(error.to_string()))?;
                    transaction
                        .execute(
                            "UPDATE work_automation_sessions SET attention_state='unread'
                         WHERE automation_session_identity=?1",
                            params![session_identity],
                        )
                        .map_err(CommandError::storage)?;
                }
            }
            if origin_kind == "conversation" {
                transaction
                    .execute(
                        "INSERT INTO work_interruption_markers
                            (conversation_turn_identity, daemon_generation, reason, created_at)
                         VALUES (?1, ?2, 'daemon_restart', ?3)",
                        params![origin_identity, generation.as_str(), committed_at],
                    )
                    .map_err(CommandError::storage)?;
            }
            transaction
                .execute(
                    "INSERT INTO work_event_outbox
                        (schema_version, daemon_generation, committed_at, event_kind,
                         work_identity, work_revision, payload)
                     VALUES (?1, ?2, ?3, 'work_recovered', ?4, ?5, ?6)",
                    params![
                        SCHEMA_VERSION,
                        generation.as_str(),
                        committed_at,
                        work_identity,
                        revision,
                        serde_json::json!({ "state": WorkState::Abandoned }).to_string()
                    ],
                )
                .map_err(CommandError::storage)?;
            let cursor = transaction.last_insert_rowid() as u64;
            set_metadata(&transaction, "event_cursor", &cursor.to_string())?;
        }
        transaction
            .execute(
                "UPDATE work_model_runtime_recovery
                 SET model_runtime_generation = NULL, trusted = 0 WHERE singleton = 1",
                [],
            )
            .map_err(CommandError::storage)?;
        prune_events(&transaction, max_events)?;
    }
    transaction.commit().map_err(CommandError::storage)
}

fn apply_command(
    transaction: &Transaction<'_>,
    command: &Command,
    generated_identity: Option<&WorkIdentity>,
    committed_at: &str,
) -> Result<(WorkIdentity, WorkRevision, WorkState, EventKind), CommandError> {
    match &command.kind {
        CommandKind::Create { origin } => {
            let work_identity = generated_identity.ok_or(CommandError::IdentitySourceExhausted)?;
            let (origin_kind, primary, secondary, historical, definition_revision) =
                origin.database_parts();
            transaction
                .execute(
                    "INSERT INTO works
                        (identity, origin_kind, origin_primary_identity, origin_secondary_identity,
                         origin_historical_identity, origin_definition_revision,
                         state, revision, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', 1, ?7, ?7)",
                    params![
                        work_identity.as_str(),
                        origin_kind,
                        primary,
                        secondary,
                        historical,
                        definition_revision,
                        committed_at
                    ],
                )
                .map_err(|error| match error {
                    rusqlite::Error::SqliteFailure(inner, _)
                        if inner.code == rusqlite::ErrorCode::ConstraintViolation =>
                    {
                        CommandError::Conflict {
                            current_revision: None,
                        }
                    }
                    other => CommandError::storage(other),
                })?;
            match origin {
                WorkOrigin::Conversation {
                    current_chat_identity,
                    conversation_turn_identity,
                } => {
                    transaction
                        .execute(
                            "INSERT OR IGNORE INTO work_current_chats (identity) VALUES (?1)",
                            params![current_chat_identity.as_str()],
                        )
                        .map_err(CommandError::storage)?;
                    transaction
                        .execute(
                            "INSERT INTO work_conversation_turns
                                (identity, current_chat_identity, work_identity)
                             VALUES (?1, ?2, ?3)",
                            params![
                                conversation_turn_identity.as_str(),
                                current_chat_identity.as_str(),
                                work_identity.as_str()
                            ],
                        )
                        .map_err(CommandError::storage)?;
                }
                WorkOrigin::Automation {
                    automation_run_identity,
                    automation_session_identity,
                    historical_automation_identity,
                    frozen_definition_revision,
                } => {
                    transaction
                        .execute(
                            "INSERT INTO work_automation_runs
                                (automation_run_identity, automation_session_identity,
                                 historical_automation_identity, frozen_definition_revision,
                                 work_identity, active)
                             VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                            params![
                                automation_run_identity.as_str(),
                                automation_session_identity.as_str(),
                                historical_automation_identity.as_str(),
                                frozen_definition_revision.value(),
                                work_identity.as_str()
                            ],
                        )
                        .map_err(CommandError::storage)?;
                    transaction
                        .execute(
                            "INSERT INTO work_automation_sessions
                                (automation_session_identity, automation_run_identity)
                             VALUES (?1, ?2)",
                            params![
                                automation_session_identity.as_str(),
                                automation_run_identity.as_str()
                            ],
                        )
                        .map_err(CommandError::storage)?;
                }
            }
            transaction
                .execute(
                    "INSERT INTO work_projections (work_identity, revision, available)
                     VALUES (?1, 1, 0)",
                    params![work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "UPDATE work_cutover
                     SET first_post_cutover_work_at = COALESCE(first_post_cutover_work_at, ?1)
                     WHERE singleton = 1",
                    params![committed_at],
                )
                .map_err(CommandError::storage)?;
            Ok((
                work_identity.clone(),
                WorkRevision::new(1),
                WorkState::Queued,
                EventKind::WorkCreated,
            ))
        }
        CommandKind::Transition {
            work_identity,
            expected_revision,
            next_state,
            model_runtime_generation,
        } => {
            let (current_state, current_revision) =
                load_work_state_revision(transaction, work_identity)?;
            if current_revision != *expected_revision {
                return Err(CommandError::Conflict {
                    current_revision: Some(current_revision),
                });
            }
            if current_state.is_terminal() {
                return Err(CommandError::TerminalTarget);
            }
            if !current_state.permits(*next_state) {
                return Err(CommandError::IllegalTransition {
                    from: current_state,
                    to: *next_state,
                });
            }
            let revision = WorkRevision::new(current_revision.value() + 1);
            let changed = transaction
                .execute(
                    "UPDATE works SET state = ?1, revision = ?2, updated_at = ?3
                     WHERE identity = ?4 AND revision = ?5",
                    params![
                        next_state.as_str(),
                        revision.value(),
                        committed_at,
                        work_identity.as_str(),
                        current_revision.value()
                    ],
                )
                .map_err(CommandError::storage)?;
            if changed != 1 {
                let actual = transaction
                    .query_row(
                        "SELECT revision FROM works WHERE identity = ?1",
                        params![work_identity.as_str()],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(CommandError::storage)?
                    .map(WorkRevision::new);
                return Err(CommandError::Conflict {
                    current_revision: actual,
                });
            }
            transaction
                .execute(
                    "UPDATE work_projections SET revision = ?1 WHERE work_identity = ?2",
                    params![revision.value(), work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            if next_state.is_terminal() {
                transaction
                    .execute(
                        "UPDATE work_automation_runs SET active = 0 WHERE work_identity = ?1",
                        params![work_identity.as_str()],
                    )
                    .map_err(CommandError::storage)?;
                transaction
                    .execute(
                        "DELETE FROM work_activity_projection WHERE work_identity = ?1",
                        params![work_identity.as_str()],
                    )
                    .map_err(CommandError::storage)?;
                let attention_state = matches!(
                    *next_state,
                    WorkState::Completed
                        | WorkState::Partial
                        | WorkState::Failed
                        | WorkState::Cancelled
                        | WorkState::Abandoned
                )
                .then_some("unread")
                .unwrap_or("none");
                transaction
                    .execute(
                        "UPDATE work_automation_sessions
                         SET attention_state = ?1
                         WHERE automation_run_identity IN (
                           SELECT automation_run_identity FROM work_automation_runs
                           WHERE work_identity = ?2
                         )",
                        params![attention_state, work_identity.as_str()],
                    )
                    .map_err(CommandError::storage)?;
            }
            if let Some(model_runtime_generation) = model_runtime_generation {
                transaction
                    .execute(
                        "UPDATE work_model_runtime_recovery
                         SET model_runtime_generation = ?1, trusted = 1 WHERE singleton = 1",
                        params![model_runtime_generation.as_str()],
                    )
                    .map_err(CommandError::storage)?;
            }
            Ok((
                work_identity.clone(),
                revision,
                *next_state,
                EventKind::WorkStateChanged,
            ))
        }
        CommandKind::AcknowledgeAttention {
            work_identity,
            expected_revision,
        } => {
            let (current_state, current_revision) =
                load_work_state_revision(transaction, work_identity)?;
            if current_revision != *expected_revision {
                return Err(CommandError::Conflict {
                    current_revision: Some(current_revision),
                });
            }
            if !matches!(
                current_state,
                WorkState::Completed
                    | WorkState::Partial
                    | WorkState::Failed
                    | WorkState::Cancelled
                    | WorkState::Abandoned
            ) {
                return Err(CommandError::TerminalTarget);
            }
            let changed = transaction
                .execute(
                    "UPDATE work_automation_sessions
                     SET attention_state = 'viewed'
                     WHERE attention_state = 'unread' AND automation_run_identity IN (
                       SELECT automation_run_identity FROM work_automation_runs
                       WHERE work_identity = ?1
                     )",
                    params![work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            if changed != 1 {
                return Err(CommandError::Conflict {
                    current_revision: Some(current_revision),
                });
            }
            let revision = WorkRevision::new(current_revision.value() + 1);
            transaction
                .execute(
                    "UPDATE works SET revision = ?1, updated_at = ?2
                     WHERE identity = ?3 AND revision = ?4",
                    params![
                        revision.value(),
                        committed_at,
                        work_identity.as_str(),
                        current_revision.value(),
                    ],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "UPDATE work_projections SET revision = ?1 WHERE work_identity = ?2",
                    params![revision.value(), work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            Ok((
                work_identity.clone(),
                revision,
                current_state,
                EventKind::CompletionAttentionAcknowledged,
            ))
        }
        CommandKind::SetActivity {
            work_identity,
            expected_revision,
            category,
        } => {
            let (current_state, current_revision) =
                load_work_state_revision(transaction, work_identity)?;
            if current_revision != *expected_revision {
                return Err(CommandError::Conflict {
                    current_revision: Some(current_revision),
                });
            }
            if current_state.is_terminal() {
                return Err(CommandError::TerminalTarget);
            }
            if current_state != WorkState::Running {
                return Err(CommandError::IllegalTransition {
                    from: current_state,
                    to: WorkState::Running,
                });
            }
            let revision = WorkRevision::new(current_revision.value() + 1);
            transaction
                .execute(
                    "UPDATE works SET revision = ?1, updated_at = ?2
                     WHERE identity = ?3 AND revision = ?4",
                    params![
                        revision.value(),
                        committed_at,
                        work_identity.as_str(),
                        current_revision.value(),
                    ],
                )
                .map_err(CommandError::storage)?;
            if let Some(category) = category {
                transaction
                    .execute(
                        "INSERT INTO work_activity_projection (work_identity, category)
                         VALUES (?1, ?2)
                         ON CONFLICT(work_identity) DO UPDATE SET category = excluded.category",
                        params![work_identity.as_str(), category.as_str()],
                    )
                    .map_err(CommandError::storage)?;
            } else {
                transaction
                    .execute(
                        "DELETE FROM work_activity_projection WHERE work_identity = ?1",
                        params![work_identity.as_str()],
                    )
                    .map_err(CommandError::storage)?;
            }
            transaction
                .execute(
                    "UPDATE work_projections SET revision = ?1 WHERE work_identity = ?2",
                    params![revision.value(), work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            Ok((
                work_identity.clone(),
                revision,
                current_state,
                EventKind::WorkActivityChanged,
            ))
        }
        CommandKind::RequestApproval {
            work_identity,
            expected_revision,
            approval_identity,
            category,
        } => {
            let (current_state, current_revision) =
                load_work_state_revision(transaction, work_identity)?;
            if current_revision != *expected_revision {
                return Err(CommandError::Conflict {
                    current_revision: Some(current_revision),
                });
            }
            if current_state != WorkState::Running {
                return Err(CommandError::IllegalTransition {
                    from: current_state,
                    to: WorkState::WaitingForApproval,
                });
            }
            let revision = WorkRevision::new(current_revision.value() + 1);
            transaction
                .execute(
                    "UPDATE works SET state = 'waiting_for_approval', revision = ?1, updated_at = ?2
                     WHERE identity = ?3 AND revision = ?4",
                    params![
                        revision.value(),
                        committed_at,
                        work_identity.as_str(),
                        current_revision.value()
                    ],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "INSERT INTO work_approvals
                        (identity, work_identity, category, state, created_at)
                     VALUES (?1, ?2, ?3, 'pending', ?4)",
                    params![
                        approval_identity.as_str(),
                        work_identity.as_str(),
                        category,
                        committed_at
                    ],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "UPDATE work_projections SET revision = ?1 WHERE work_identity = ?2",
                    params![revision.value(), work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            Ok((
                work_identity.clone(),
                revision,
                WorkState::WaitingForApproval,
                EventKind::WorkStateChanged,
            ))
        }
        CommandKind::ResolveApproval {
            work_identity,
            expected_revision,
            approval_identity,
            allow,
            expected_decision_revision,
        } => {
            let (current_state, current_revision) =
                load_work_state_revision(transaction, work_identity)?;
            if current_revision != *expected_revision {
                return Err(CommandError::Conflict {
                    current_revision: Some(current_revision),
                });
            }
            if current_state != WorkState::WaitingForApproval {
                return Err(CommandError::IllegalTransition {
                    from: current_state,
                    to: WorkState::Running,
                });
            }
            let approval_changed = transaction
                .execute(
                    "UPDATE work_approvals
                     SET state = ?1, resolved_at = ?2, decision_revision = decision_revision + 1
                     WHERE identity = ?3 AND work_identity = ?4 AND state = 'pending'
                       AND decision_revision = ?5",
                    params![
                        if *allow { "allowed" } else { "denied" },
                        committed_at,
                        approval_identity.as_str(),
                        work_identity.as_str(),
                        expected_decision_revision,
                    ],
                )
                .map_err(CommandError::storage)?;
            if approval_changed != 1 {
                return Err(CommandError::Conflict {
                    current_revision: Some(current_revision),
                });
            }
            let next_state = WorkState::Running;
            let revision = WorkRevision::new(current_revision.value() + 1);
            transaction
                .execute(
                    "UPDATE works SET state = ?1, revision = ?2, updated_at = ?3
                     WHERE identity = ?4 AND revision = ?5",
                    params![
                        next_state.as_str(),
                        revision.value(),
                        committed_at,
                        work_identity.as_str(),
                        current_revision.value(),
                    ],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "UPDATE work_projections SET revision = ?1 WHERE work_identity = ?2",
                    params![revision.value(), work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            Ok((
                work_identity.clone(),
                revision,
                next_state,
                EventKind::WorkStateChanged,
            ))
        }
        CommandKind::Cancel {
            work_identity,
            expected_revision,
        } => {
            let (current_state, current_revision) =
                load_work_state_revision(transaction, work_identity)?;
            if current_revision != *expected_revision {
                return Err(CommandError::Conflict {
                    current_revision: Some(current_revision),
                });
            }
            if current_state.is_terminal() {
                return Err(CommandError::TerminalTarget);
            }
            let revision = WorkRevision::new(current_revision.value() + 1);
            transaction
                .execute(
                    "UPDATE works SET state = 'cancelling', revision = ?1, updated_at = ?2
                     WHERE identity = ?3 AND revision = ?4",
                    params![
                        revision.value(),
                        committed_at,
                        work_identity.as_str(),
                        current_revision.value(),
                    ],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "UPDATE work_approvals SET state = 'withdrawn', resolved_at = ?1,
                         decision_revision = decision_revision + 1
                     WHERE work_identity = ?2 AND state = 'pending'",
                    params![committed_at, work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            transaction
                .execute(
                    "UPDATE work_projections SET revision = ?1 WHERE work_identity = ?2",
                    params![revision.value(), work_identity.as_str()],
                )
                .map_err(CommandError::storage)?;
            Ok((
                work_identity.clone(),
                revision,
                WorkState::Cancelling,
                EventKind::WorkStateChanged,
            ))
        }
    }
}

fn load_work_state_revision(
    transaction: &Transaction<'_>,
    work_identity: &WorkIdentity,
) -> Result<(WorkState, WorkRevision), CommandError> {
    let current = transaction
        .query_row(
            "SELECT state, revision FROM works WHERE identity = ?1",
            params![work_identity.as_str()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
        )
        .optional()
        .map_err(CommandError::storage)?;
    let Some((state, revision)) = current else {
        return Err(CommandError::Conflict {
            current_revision: None,
        });
    };
    Ok((WorkState::parse(&state)?, WorkRevision::new(revision)))
}

fn load_work_activity(
    transaction: &Transaction<'_>,
    work_identity: &WorkIdentity,
) -> Result<Option<WorkActivityCategory>, CommandError> {
    transaction
        .query_row(
            "SELECT category FROM work_activity_projection WHERE work_identity = ?1",
            params![work_identity.as_str()],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(CommandError::storage)?
        .as_deref()
        .map(WorkActivityCategory::parse)
        .transpose()
}

fn metadata(connection: &Connection, key: &str) -> Result<Option<String>, CommandError> {
    connection
        .query_row(
            "SELECT value FROM work_coordinator_metadata WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(CommandError::storage)
}

fn set_metadata(connection: &Connection, key: &str, value: &str) -> Result<(), CommandError> {
    connection
        .execute(
            "INSERT INTO work_coordinator_metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .map(|_| ())
        .map_err(CommandError::storage)
}

fn snapshot_from(connection: &Connection) -> Result<WorkSnapshot, CommandError> {
    let daemon_generation = metadata(connection, "daemon_generation")?
        .ok_or_else(|| CommandError::CorruptState("missing daemon generation".to_owned()))?;
    let cursor = EventCursor::new(
        metadata(connection, "event_cursor")?
            .unwrap_or_else(|| "0".to_owned())
            .parse::<u64>()
            .map_err(CommandError::serialization)?,
    );
    let mut statement = connection
        .prepare(
            "SELECT identity, origin_kind, origin_primary_identity, origin_secondary_identity,
                    origin_historical_identity, origin_definition_revision, state, revision,
                    work_activity_projection.category
             FROM works
             LEFT JOIN work_activity_projection ON work_activity_projection.work_identity = works.identity
             ORDER BY identity ASC",
        )
        .map_err(CommandError::storage)?;
    let works = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<u64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, u64>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(CommandError::storage)?
        .map(|result| {
            let (
                identity,
                kind,
                primary,
                secondary,
                historical,
                definition_revision,
                state,
                revision,
                activity,
            ) = result.map_err(CommandError::storage)?;
            Ok(WorkRecord {
                identity: WorkIdentity::new(identity),
                origin: WorkOrigin::from_database(
                    &kind,
                    primary,
                    secondary,
                    historical,
                    definition_revision,
                )?,
                state: WorkState::parse(&state)?,
                revision: WorkRevision::new(revision),
                activity: activity
                    .as_deref()
                    .map(WorkActivityCategory::parse)
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>, CommandError>>()?;
    let approvals = {
        let mut statement = connection
            .prepare(
                "SELECT identity, work_identity, category, state
                 FROM work_approvals ORDER BY identity ASC",
            )
            .map_err(CommandError::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(CommandError::storage)?
            .map(|row| {
                let (identity, work_identity, category, state) =
                    row.map_err(CommandError::storage)?;
                Ok(ApprovalRecord {
                    identity: ApprovalIdentity::new(identity),
                    work_identity: WorkIdentity::new(work_identity),
                    category,
                    state: ApprovalState::parse(&state)?,
                })
            })
            .collect::<Result<Vec<_>, CommandError>>()?;
        rows
    };
    let automation_runs = {
        let mut statement = connection
            .prepare(
                "SELECT automation_run_identity, work_identity, active
                 FROM work_automation_runs ORDER BY automation_run_identity ASC",
            )
            .map_err(CommandError::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok(AutomationRunRecord {
                    identity: AutomationRunIdentity::new(row.get::<_, String>(0)?),
                    work_identity: WorkIdentity::new(row.get::<_, String>(1)?),
                    active: row.get(2)?,
                })
            })
            .map_err(CommandError::storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::storage)?;
        rows
    };
    let interruptions = {
        let mut statement = connection
            .prepare(
                "SELECT conversation_turn_identity, daemon_generation
                 FROM work_interruption_markers ORDER BY conversation_turn_identity ASC",
            )
            .map_err(CommandError::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok(InterruptionMarker {
                    conversation_turn_identity: ConversationTurnIdentity::new(
                        row.get::<_, String>(0)?,
                    ),
                    daemon_generation: DaemonGeneration::new(row.get::<_, String>(1)?),
                })
            })
            .map_err(CommandError::storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::storage)?;
        rows
    };
    let (model_runtime_generation, model_runtime_trusted) = connection
        .query_row(
            "SELECT model_runtime_generation, trusted
             FROM work_model_runtime_recovery WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, bool>(1)?)),
        )
        .map_err(CommandError::storage)?;
    Ok(WorkSnapshot {
        schema_version: SCHEMA_VERSION,
        cursor,
        daemon_generation: DaemonGeneration::new(daemon_generation),
        works,
        automation_runs,
        approvals,
        interruptions,
        model_runtime_generation: model_runtime_generation.map(ModelRuntimeGeneration::new),
        model_runtime_trusted,
    })
}

fn notch_snapshot_from(connection: &Connection) -> Result<WorkSnapshot, CommandError> {
    let daemon_generation = metadata(connection, "daemon_generation")?
        .ok_or_else(|| CommandError::CorruptState("missing daemon generation".to_owned()))?;
    let cursor = EventCursor::new(
        metadata(connection, "event_cursor")?
            .unwrap_or_else(|| "0".to_owned())
            .parse::<u64>()
            .map_err(CommandError::serialization)?,
    );
    let mut statement = connection
        .prepare(
            "WITH projected(identity) AS (
                 SELECT identity FROM works
                 WHERE state NOT IN ('completed', 'partial', 'failed', 'cancelled', 'abandoned')
                 UNION
                 SELECT work_identity FROM (
                     SELECT r.work_identity,
                            ROW_NUMBER() OVER (
                                PARTITION BY w.state
                                ORDER BY w.updated_at DESC, w.identity ASC
                            ) AS terminal_rank
                     FROM works w
                     JOIN work_automation_runs r ON r.work_identity = w.identity
                     JOIN work_automation_sessions s
                       ON s.automation_session_identity = r.automation_session_identity
                     WHERE w.origin_kind = 'automation'
                       AND w.state IN ('completed', 'partial', 'failed', 'cancelled', 'abandoned')
                       AND s.attention_state = 'unread'
                 )
                 WHERE terminal_rank = 1
                 UNION
                 SELECT identity FROM (
                     SELECT identity FROM works
                     WHERE origin_kind = 'conversation'
                       AND state IN ('completed', 'partial', 'failed')
                     ORDER BY updated_at DESC, identity ASC LIMIT 1
                 )
             )
             SELECT w.identity, w.origin_kind, w.origin_primary_identity,
                    w.origin_secondary_identity, w.origin_historical_identity,
                    w.origin_definition_revision, w.state, w.revision,
                    p.category
             FROM projected
             JOIN works w ON w.identity = projected.identity
             LEFT JOIN work_activity_projection p ON p.work_identity = w.identity
             ORDER BY w.identity ASC",
        )
        .map_err(CommandError::storage)?;
    let works = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<u64>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, u64>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(CommandError::storage)?
        .map(|result| {
            let (
                identity,
                kind,
                primary,
                secondary,
                historical,
                definition_revision,
                state,
                revision,
                activity,
            ) = result.map_err(CommandError::storage)?;
            Ok(WorkRecord {
                identity: WorkIdentity::new(identity),
                origin: WorkOrigin::from_database(
                    &kind,
                    primary,
                    secondary,
                    historical,
                    definition_revision,
                )?,
                state: WorkState::parse(&state)?,
                revision: WorkRevision::new(revision),
                activity: activity
                    .as_deref()
                    .map(WorkActivityCategory::parse)
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>, CommandError>>()?;
    let approvals = {
        let mut statement = connection
            .prepare(
                "SELECT identity, work_identity, category, state
                 FROM work_approvals WHERE state = 'pending' ORDER BY identity ASC",
            )
            .map_err(CommandError::storage)?;
        let rows = statement
            .query_map([], |row| {
                Ok(ApprovalRecord {
                    identity: ApprovalIdentity::new(row.get::<_, String>(0)?),
                    work_identity: WorkIdentity::new(row.get::<_, String>(1)?),
                    category: row.get(2)?,
                    state: ApprovalState::Pending,
                })
            })
            .map_err(CommandError::storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(CommandError::storage)?;
        rows
    };
    let (model_runtime_generation, model_runtime_trusted) = connection
        .query_row(
            "SELECT model_runtime_generation, trusted
             FROM work_model_runtime_recovery WHERE singleton = 1",
            [],
            |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, bool>(1)?)),
        )
        .map_err(CommandError::storage)?;
    Ok(WorkSnapshot {
        schema_version: SCHEMA_VERSION,
        cursor,
        daemon_generation: DaemonGeneration::new(daemon_generation),
        works,
        automation_runs: vec![],
        approvals,
        interruptions: vec![],
        model_runtime_generation: model_runtime_generation.map(ModelRuntimeGeneration::new),
        model_runtime_trusted,
    })
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkEvent> {
    let payload: String = row.get(7)?;
    let payload = serde_json::from_str::<serde_json::Value>(&payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let state = Some(&payload)
        .and_then(|value| value.get("state").cloned())
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid Work event payload",
                )),
            )
        })?;
    let activity = payload
        .get("activity")
        .and_then(serde_json::Value::as_str)
        .map(WorkActivityCategory::parse)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(WorkEvent {
        event_cursor: EventCursor::new(row.get(0)?),
        schema_version: row.get(1)?,
        daemon_generation: DaemonGeneration::new(row.get::<_, String>(2)?),
        committed_at: row.get(3)?,
        event_kind: EventKind::parse(&row.get::<_, String>(4)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        work_identity: WorkIdentity::new(row.get::<_, String>(5)?),
        work_revision: WorkRevision::new(row.get(6)?),
        state,
        activity,
    })
}

fn prune_events(transaction: &Transaction<'_>, max_events: usize) -> Result<(), CommandError> {
    transaction
        .execute(
            "DELETE FROM work_event_outbox
             WHERE cursor NOT IN (
                 SELECT cursor FROM work_event_outbox ORDER BY cursor DESC LIMIT ?1
             )",
            params![max_events as u64],
        )
        .map(|_| ())
        .map_err(CommandError::storage)
}

fn inject(actual: Option<FailurePoint>, expected: FailurePoint) -> Result<(), CommandError> {
    if actual == Some(expected) {
        Err(CommandError::InjectedFailure(expected))
    } else {
        Ok(())
    }
}
