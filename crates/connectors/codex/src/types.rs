use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

// ── Config ────────────────────────────────────────────────────────────────────

/// Runtime configuration for the Codex connector.
#[derive(Debug, Clone)]
pub struct CodexConfig {
    /// Explicit path to the `codex` binary. When `None` the connector searches
    /// `$PATH` at construction time.
    pub binary_path: Option<PathBuf>,
    /// Maximum time to wait for `codex exec` to complete.
    /// Defaults to 120 seconds.
    pub timeout: Duration,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            binary_path: None,
            timeout: Duration::from_secs(120),
        }
    }
}

// ── Context packet ────────────────────────────────────────────────────────────

/// The type of output the caller expects Codex to produce.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CodexExpectedOutput {
    #[default]
    Analysis,
    ActionPlan,
    Drafts,
    Comparison,
    Timeline,
    StructuredJson,
}

/// A single piece of approved context passed to Codex.
///
/// Raw bodies are never included unless the caller explicitly sets them.
/// Only summaries, extracted fields, and record references are sent by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextItem {
    /// Source system the item comes from (e.g. "mail", "odoo", "notes").
    pub source: String,
    /// Optional human-readable title of the record.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Summary or extracted fields (never the raw body unless explicitly approved).
    pub summary: String,
    /// Stable reference to the original record (e.g. "mail:rowid:123").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_ref: Option<String>,
    /// Whether this item contains personally identifiable information.
    #[serde(default)]
    pub pii: bool,
}

/// The full context packet sent to Codex via stdin.
///
/// This is the **only** data Codex receives. It is daemon-built and
/// user-approved before dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexContextPacket {
    /// The original user request, verbatim.
    pub user_request: String,
    /// Approved context items (summaries and record refs only by default).
    pub allowed_context: Vec<ContextItem>,
    /// Context types that are explicitly excluded from this packet.
    #[serde(default)]
    pub forbidden_context_types: Vec<String>,
    /// Hard constraints included in the prompt to Codex.
    #[serde(default)]
    pub constraints: Vec<String>,
    /// The output format the caller expects.
    pub expected_output: CodexExpectedOutput,
}

impl Default for CodexContextPacket {
    fn default() -> Self {
        Self {
            user_request: String::new(),
            allowed_context: vec![],
            forbidden_context_types: vec![
                "daemon_token".into(),
                "bearer_token".into(),
                "keychain".into(),
                "credentials".into(),
                "api_keys".into(),
                "raw_mail_database".into(),
                "raw_notes_database".into(),
                "memory_database".into(),
                "screenshots".into(),
                "browser_credential_stores".into(),
                "ssh_keys".into(),
                "gnupg".into(),
                "password_managers".into(),
                "system_files".into(),
                "bagent_app_support".into(),
            ],
            constraints: vec![
                "Do not invent facts.".into(),
                "Do not perform side effects.".into(),
                "Return proposed actions only — do not execute them.".into(),
                "Reference record_ref values when citing source material.".into(),
                "If evidence conflicts, mark it as conflict.".into(),
                "Do not request or use credentials, tokens, or secrets.".into(),
            ],
            expected_output: CodexExpectedOutput::Analysis,
        }
    }
}

// ── Task ──────────────────────────────────────────────────────────────────────

/// A task dispatched to the Codex connector.
///
/// `task_level` and `privacy_risk` are passed as strings to avoid a dep cycle
/// between `codex-connector` and `bagent-agent` (which owns the enums).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTask {
    /// Unique ID for this task run (UUID).
    pub id: String,
    /// Human-readable description of the task (the original user request).
    pub description: String,
    /// Daemon-built, user-approved context packet.
    pub context_packet: CodexContextPacket,
    /// Rating level string (e.g. "CodexRecommended").
    pub task_level: String,
    /// Privacy risk string (e.g. "High").
    pub privacy_risk: String,
}

// ── Result ────────────────────────────────────────────────────────────────────

/// The structured output from a Codex run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexRunResult {
    /// Process exit code. `None` on timeout/kill.
    pub exit_code: Option<i32>,
    /// Raw stdout (may be truncated; see `result_text` for the processed form).
    pub stdout: String,
    /// Raw stderr (may be truncated).
    pub stderr: String,
    /// Parsed JSON output when Codex returned valid JSON. `None` otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed_output: Option<serde_json::Value>,
    /// Human-readable result text. Either extracted from `parsed_output.summary`
    /// or the raw stdout when JSON is unavailable.
    pub result_text: String,
    /// True when the run exceeded the configured timeout.
    pub timed_out: bool,
    /// SHA-256 hex of `stdout + stderr + result_text`. Stable, auditable.
    pub output_hash: String,
}

// ── Errors ────────────────────────────────────────────────────────────────────

/// Typed errors from the Codex connector (not `anyhow` — callers need to branch).
#[derive(Debug)]
pub enum CodexError {
    /// `codex` binary not found on the configured path or in `$PATH`.
    NotFound,
    /// Process could not be spawned (OS-level error).
    Spawn(String),
    /// I/O error reading stdout/stderr.
    Io(String),
}

impl std::fmt::Display for CodexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(
                f,
                "codex binary not found — install Codex CLI or configure the path in Settings"
            ),
            Self::Spawn(msg) => write!(f, "failed to spawn codex: {msg}"),
            Self::Io(msg) => write!(f, "I/O error reading codex output: {msg}"),
        }
    }
}

impl std::error::Error for CodexError {}
