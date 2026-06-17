//! Codex external-reasoning connector for bagent.
//!
//! Wraps the OpenAI `codex` CLI as a sandboxed subprocess. Codex receives
//! **only** the daemon-built, user-approved [`CodexContextPacket`] via stdin.
//! It has no direct access to the user's machine, mail, notes, Odoo, WhatsApp,
//! memory database, credentials, or local files.
//!
//! # Safety guarantees
//!
//! - Never invoked through a shell (no interpolation risk).
//! - Always run with `--sandbox read-only`.
//! - `--dangerously-bypass-*` flags are never passed.
//! - Timeout enforced at 120 seconds (configurable).
//! - Proposed actions in the output are **not** executed automatically.

pub mod exec;
pub mod types;

pub use exec::{build_exec_argv, compute_hash, find_codex_in_path, safe_truncate};
pub use types::{
    CodexConfig, CodexContextPacket, CodexError, CodexExpectedOutput, CodexRunResult, CodexTask,
    ContextItem,
};

use std::path::PathBuf;

use tracing::{debug, info, warn};

// ── CodexConnector ────────────────────────────────────────────────────────────

/// The main connector struct. Cloneable; holds only the resolved binary path
/// and runtime configuration.
#[derive(Debug, Clone)]
pub struct CodexConnector {
    binary: PathBuf,
    config: CodexConfig,
}

impl CodexConnector {
    /// Construct a connector, resolving the binary path from config or `$PATH`.
    ///
    /// Returns `Err(CodexError::NotFound)` when the binary cannot be located.
    pub fn new(config: CodexConfig) -> std::result::Result<Self, CodexError> {
        let binary = match &config.binary_path {
            Some(p) => {
                if p.is_file() {
                    p.clone()
                } else {
                    warn!(path = %p.display(), "configured codex path does not exist");
                    return Err(CodexError::NotFound);
                }
            }
            None => find_codex_in_path().ok_or(CodexError::NotFound)?,
        };
        info!(binary = %binary.display(), "CodexConnector initialised");
        Ok(Self { binary, config })
    }

    /// True when the binary exists and is executable.
    pub fn is_accessible(&self) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(&self.binary)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    /// The resolved binary path.
    pub fn resolved_path(&self) -> &PathBuf {
        &self.binary
    }

    /// Attempt to retrieve the `codex` version string (best-effort).
    pub async fn version(&self) -> Option<String> {
        let out = tokio::process::Command::new(&self.binary)
            .arg("--version")
            .output()
            .await
            .ok()?;
        if out.status.success() {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !v.is_empty() {
                return Some(v);
            }
        }
        // Some versions print to stderr
        let v = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        if !v.is_empty() {
            Some(v)
        } else {
            None
        }
    }

    /// Run a `CodexTask` non-interactively via `codex exec`.
    ///
    /// The context packet is serialised to JSON and prepended to a fixed
    /// instruction block, then written to the process stdin.
    ///
    /// Codex is instructed to:
    /// - return structured JSON matching the output contract (Part 7 of the spec)
    /// - not perform side effects
    /// - cite `record_ref` values for all claims
    /// - mark conflicting evidence explicitly
    ///
    /// # Errors
    ///
    /// Returns `Err(CodexError::*)` for spawn / I/O failures. Timeouts and
    /// non-zero exit codes are reported in the `CodexRunResult` rather than
    /// propagated as errors.
    pub async fn run(&self, task: &CodexTask) -> std::result::Result<CodexRunResult, CodexError> {
        debug!(task_id = %task.id, level = %task.task_level, "dispatching codex task");

        let prompt = build_prompt(task);
        exec::run_codex(self.binary.as_path(), &prompt, self.config.timeout).await
    }
}

// ── Prompt construction ───────────────────────────────────────────────────────

/// Build the full stdin prompt for `codex exec`.
///
/// The prompt includes:
/// 1. A fixed instruction block describing the output contract.
/// 2. The serialised context packet (user request + approved context items).
fn build_prompt(task: &CodexTask) -> String {
    let packet_json =
        serde_json::to_string_pretty(&task.context_packet).unwrap_or_else(|_| "{}".to_string());

    format!(
        r#"You are an advanced external reasoning harness for a business agent application (bagent).
You receive a controlled context packet prepared by the daemon. You have NO direct access to
the user's machine, mail database, notes database, Odoo, WhatsApp, files, credentials, or
memory store. Everything you need is in the context packet below.

TASK ID: {task_id}
TASK LEVEL: {task_level}
PRIVACY RISK: {privacy_risk}

CONSTRAINTS (non-negotiable):
- Do not invent facts or figures.
- Do not perform any side effects. Return proposed actions only.
- Do not request or use credentials, tokens, API keys, or secrets.
- Cite record_ref values when referencing source material.
- If evidence conflicts, explicitly mark it as a conflict.
- Do not access any resource outside this context packet.

EXPECTED OUTPUT:
Return ONLY valid JSON matching this schema. No markdown, no preamble, no explanation outside the JSON:
{{
  "summary": "<concise summary of findings>",
  "findings": [
    {{
      "claim": "<specific finding>",
      "source_refs": ["<record_ref>", ...],
      "confidence": 0.0-1.0
    }}
  ],
  "conflicts": [
    {{
      "description": "<description of the conflict>",
      "source_refs": ["<record_ref1>", "<record_ref2>"]
    }}
  ],
  "proposed_actions": [
    {{
      "kind": "draft_email|update_odoo|create_reminder|other",
      "description": "<what should be done>",
      "requires_user_approval": true,
      "target_ref": "<record_ref or null>"
    }}
  ],
  "drafts": [
    {{
      "channel": "email|whatsapp|notes",
      "language": "sk|en|...",
      "body": "<draft text>"
    }}
  ],
  "questions_for_user": [
    "<question that needs user clarification>"
  ]
}}

All proposed_actions MUST have requires_user_approval: true.
The agent will present them to the user for approval — do NOT suggest they be executed automatically.

CONTEXT PACKET:
{packet_json}
"#,
        task_id = task.id,
        task_level = task.task_level,
        privacy_risk = task.privacy_risk,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::NamedTempFile;

    fn make_task() -> CodexTask {
        CodexTask {
            id: "test-1".into(),
            description: "Test task".into(),
            context_packet: CodexContextPacket {
                user_request: "Summarise the situation.".into(),
                allowed_context: vec![ContextItem {
                    source: "mail".into(),
                    title: Some("Test email".into()),
                    summary: "Client says invoice was paid.".into(),
                    record_ref: Some("mail:rowid:1".into()),
                    pii: false,
                }],
                ..Default::default()
            },
            task_level: "CodexRecommended".into(),
            privacy_risk: "High".into(),
        }
    }

    #[test]
    fn prompt_contains_no_bypass_flags() {
        let task = make_task();
        let prompt = build_prompt(&task);
        assert!(!prompt.contains("dangerously-bypass"));
        assert!(!prompt.contains("danger-full-access"));
    }

    #[test]
    fn prompt_contains_task_id() {
        let task = make_task();
        let prompt = build_prompt(&task);
        assert!(prompt.contains("test-1"));
    }

    #[test]
    fn prompt_contains_context_packet_json() {
        let task = make_task();
        let prompt = build_prompt(&task);
        // The context packet's user_request should appear in the prompt
        assert!(prompt.contains("Summarise the situation"));
    }

    #[test]
    fn prompt_requires_approval_for_proposed_actions() {
        let task = make_task();
        let prompt = build_prompt(&task);
        assert!(prompt.contains("requires_user_approval: true"));
    }

    // ── Fake binary tests ─────────────────────────────────────────────────────

    /// Write a fake `codex` shell script to a temp file, make it executable.
    async fn write_fake_binary(script: &str) -> NamedTempFile {
        use std::os::unix::fs::PermissionsExt;
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_owned();
        tokio::fs::write(&path, script).await.unwrap();
        let mut perms = tokio::fs::metadata(&path).await.unwrap().permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&path, perms).await.unwrap();
        f
    }

    #[tokio::test]
    async fn fake_binary_not_found_returns_typed_error() {
        let config = CodexConfig {
            binary_path: Some(PathBuf::from("/nonexistent/codex")),
            timeout: Duration::from_secs(10),
        };
        let err = CodexConnector::new(config).unwrap_err();
        assert!(matches!(err, CodexError::NotFound));
    }

    #[tokio::test]
    async fn fake_binary_success_captures_stdout_stderr() {
        let fake =
            write_fake_binary("#!/bin/sh\necho 'hello stdout'\necho 'hello stderr' >&2\n").await;
        let config = CodexConfig {
            binary_path: Some(fake.path().to_path_buf()),
            timeout: Duration::from_secs(10),
        };
        // Temporarily override argv by calling exec::run_codex directly
        let result = exec::run_codex(fake.path(), "test prompt", Duration::from_secs(10))
            .await
            .unwrap();
        assert!(!result.timed_out);
        assert!(result.stdout.contains("hello stdout"));
        assert!(result.stderr.contains("hello stderr"));
        assert_eq!(result.exit_code, Some(0));
        assert!(!result.output_hash.is_empty());
    }

    #[tokio::test]
    async fn fake_binary_timeout_sets_timed_out() {
        // Script that sleeps longer than the timeout
        let fake = write_fake_binary("#!/bin/sh\nsleep 30\n").await;
        let result = exec::run_codex(
            fake.path(),
            "test",
            Duration::from_millis(200), // very short timeout
        )
        .await
        .unwrap();
        assert!(result.timed_out, "expected timed_out=true");
        assert!(result.exit_code.is_none());
        assert!(result.stderr.contains("timeout"));
    }

    #[tokio::test]
    async fn fake_binary_json_output_parses() {
        let json = r#"{"summary":"Invoice paid","findings":[],"conflicts":[],"proposed_actions":[],"drafts":[],"questions_for_user":[]}"#;
        let script = format!("#!/bin/sh\necho '{}'\n", json);
        let fake = write_fake_binary(&script).await;
        let result = exec::run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        assert!(result.parsed_output.is_some());
        assert_eq!(result.result_text, "Invoice paid");
    }

    #[tokio::test]
    async fn fake_binary_plain_text_fallback() {
        let fake = write_fake_binary("#!/bin/sh\necho 'This is plain text output'\n").await;
        let result = exec::run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        assert!(result.parsed_output.is_none());
        assert!(result.result_text.contains("plain text"));
    }

    #[tokio::test]
    async fn fake_binary_large_output_truncates() {
        // Generate output larger than MAX_OUTPUT_BYTES (64 KiB)
        let line = "x".repeat(100);
        let lines = (0..700)
            .map(|_| line.clone())
            .collect::<Vec<_>>()
            .join("\n");
        let script = format!("#!/bin/sh\necho '{}'\n", lines);
        let fake = write_fake_binary(&script).await;
        let result = exec::run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        assert!(
            result.stdout.contains("[truncated"),
            "expected truncation marker in stdout"
        );
    }

    #[tokio::test]
    async fn output_hash_is_stable() {
        let fake = write_fake_binary("#!/bin/sh\necho 'stable output'\n").await;
        let r1 = exec::run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        let r2 = exec::run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(r1.output_hash, r2.output_hash);
    }
}
