use std::path::PathBuf;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::types::{CodexError, CodexRunResult};

/// Maximum bytes kept from stdout / stderr / result_text before truncation.
const MAX_OUTPUT_BYTES: usize = 64 * 1024; // 64 KiB per stream

// ── Argv builder (pure, no spawn — unit-testable) ─────────────────────────────

/// Build the argv for a non-interactive `codex exec` invocation.
///
/// The binary is invoked with `--sandbox read-only` and `-` so it reads the
/// prompt from stdin. `--dangerously-bypass-*` flags are **never** added here.
pub fn build_exec_argv(binary: &PathBuf) -> Vec<String> {
    vec![
        binary.to_string_lossy().into_owned(),
        "exec".into(),
        "--sandbox".into(),
        "read-only".into(),
        "-".into(), // read prompt from stdin
    ]
}

// ── Subprocess execution ───────────────────────────────────────────────────────

/// Dispatch `codex exec` with the given prompt, enforcing `timeout`.
///
/// Returns a `CodexRunResult` that is always safe to serialise and audit.
/// On timeout the process is killed (SIGKILL via tokio `start_kill`).
///
/// Design note: stdout and stderr handles are taken from `child` *before* the
/// timeout block so that `child` itself remains accessible in the timeout branch
/// for the kill call, without triggering the borrow-after-move that
/// `wait_with_output(self)` would cause.
pub async fn run_codex(
    binary: &std::path::Path,
    prompt: &str,
    timeout: Duration,
) -> Result<CodexRunResult, CodexError> {
    let binary = binary.to_path_buf();
    let argv = build_exec_argv(&binary);
    let (bin, args) = argv.split_first().expect("argv must not be empty");

    // Spawn with piped stdin / stdout / stderr.
    let mut child = Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| CodexError::Spawn(e.to_string()))?;

    // Write prompt to stdin, then close the handle so the child sees EOF.
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .await
            .map_err(|e| CodexError::Io(e.to_string()))?;
        // stdin dropped here → EOF delivered to child
    }

    // Take stdout / stderr pipes out of `child` so we can read them in the
    // timeout future while keeping `child` accessible in the outer scope for
    // the kill path.
    let mut stdout_pipe = child
        .stdout
        .take()
        .unwrap_or_else(|| panic!("stdout was piped but take() returned None"));
    let mut stderr_pipe = child
        .stderr
        .take()
        .unwrap_or_else(|| panic!("stderr was piped but take() returned None"));

    let mut stdout_bytes: Vec<u8> = Vec::new();
    let mut stderr_bytes: Vec<u8> = Vec::new();

    // The async block borrows (by &mut ref) all four things:
    // stdout_pipe, stderr_pipe, stdout_bytes, stderr_bytes, child.
    // Since it's awaited inline (not spawned), Rust's borrow checker can verify
    // the lifetimes without requiring 'static.
    let io_future = async {
        // Read stdout and stderr concurrently to avoid pipe-full deadlocks.
        let (r1, r2) = tokio::join!(
            stdout_pipe.read_to_end(&mut stdout_bytes),
            stderr_pipe.read_to_end(&mut stderr_bytes),
        );
        r1.map_err(|e| CodexError::Io(e.to_string()))?;
        r2.map_err(|e| CodexError::Io(e.to_string()))?;

        // Wait for the child process to exit.
        let status = child
            .wait()
            .await
            .map_err(|e| CodexError::Io(e.to_string()))?;

        Ok::<i32, CodexError>(status.code().unwrap_or(-1))
    };

    match tokio::time::timeout(timeout, io_future).await {
        Ok(Ok(exit_code)) => {
            let stdout_raw = String::from_utf8_lossy(&stdout_bytes).into_owned();
            let stderr_raw = String::from_utf8_lossy(&stderr_bytes).into_owned();
            let stdout = safe_truncate(&stdout_raw, MAX_OUTPUT_BYTES);
            let stderr = safe_truncate(&stderr_raw, MAX_OUTPUT_BYTES);

            let (parsed_output, result_text) = parse_output(&stdout);
            let output_hash = compute_hash(&stdout, &stderr, &result_text);

            Ok(CodexRunResult {
                exit_code: Some(exit_code),
                stdout,
                stderr,
                parsed_output,
                result_text,
                timed_out: false,
                output_hash,
            })
        }

        Ok(Err(e)) => Err(e),

        Err(_elapsed) => {
            // Timeout: io_future was dropped, all borrows released.
            // `child` is accessible again for the kill call.
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;

            let stdout = String::new();
            let stderr = format!(
                "[codex-connector] process terminated after {}s timeout",
                timeout.as_secs()
            );
            let result_text = stderr.clone();
            let output_hash = compute_hash(&stdout, &stderr, &result_text);

            Ok(CodexRunResult {
                exit_code: None,
                stdout,
                stderr,
                parsed_output: None,
                result_text,
                timed_out: true,
                output_hash,
            })
        }
    }
}

// ── Output parsing ────────────────────────────────────────────────────────────

/// Try to parse a structured JSON output from Codex stdout.
///
/// Codex may emit JSONL (one JSON object per line) or a single JSON object.
/// Scans from the last line backwards looking for a valid JSON object.
/// Falls back to plain text when no JSON is found.
fn parse_output(stdout: &str) -> (Option<serde_json::Value>, String) {
    for line in stdout.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                let result_text = v
                    .get("summary")
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_owned())
                    .unwrap_or_else(|| {
                        safe_truncate(&serde_json::to_string_pretty(&v).unwrap_or_default(), 4096)
                    });
                return (Some(v), result_text);
            }
        }
    }

    // No JSON found — return plain text
    let plain = safe_truncate(stdout.trim(), 4096);
    (None, plain.to_owned())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Compute SHA-256 of `stdout + stderr + result_text` and return as lowercase hex.
pub fn compute_hash(stdout: &str, stderr: &str, result_text: &str) -> String {
    let mut h = Sha256::new();
    h.update(stdout.as_bytes());
    h.update(stderr.as_bytes());
    h.update(result_text.as_bytes());
    format!("{:x}", h.finalize())
}

/// Truncate `s` to at most `max_bytes` bytes on a valid UTF-8 char boundary.
/// Appends a clear marker when truncation occurs.
pub fn safe_truncate(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_owned();
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = &s[..end];
    let omitted = s.len() - end;
    format!("{truncated}\n…[truncated {omitted} bytes]")
}

// ── Binary resolution ─────────────────────────────────────────────────────────

/// Search `$PATH` for an executable named `codex`.
/// Returns `None` when not found or not executable.
pub fn find_codex_in_path() -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("codex");
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                return Some(candidate);
            }
        }
    }
    None
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── Pure unit tests ───────────────────────────────────────────────────────

    #[test]
    fn build_exec_argv_shape() {
        let bin = PathBuf::from("/usr/local/bin/codex");
        let argv = build_exec_argv(&bin);
        assert_eq!(argv[0], "/usr/local/bin/codex");
        assert_eq!(argv[1], "exec");
        assert!(argv.contains(&"--sandbox".to_string()));
        assert!(argv.contains(&"read-only".to_string()));
        assert!(argv.contains(&"-".to_string()));
        // Must never contain dangerous bypass flags
        for arg in &argv {
            assert!(
                !arg.contains("dangerously-bypass"),
                "argv must not contain bypass flags: {argv:?}"
            );
        }
    }

    #[test]
    fn safe_truncate_no_op_for_short_string() {
        assert_eq!(safe_truncate("hello", 100), "hello");
    }

    #[test]
    fn safe_truncate_adds_marker() {
        let s = "a".repeat(200);
        let t = safe_truncate(&s, 100);
        assert!(t.contains("[truncated"), "truncation marker missing: {t}");
    }

    #[test]
    fn safe_truncate_on_char_boundary() {
        // 'ě' is 2 bytes each; truncating mid-char must produce valid UTF-8
        let s = "ě".repeat(50);
        let t = safe_truncate(&s, 45);
        assert!(std::str::from_utf8(t.as_bytes()).is_ok());
    }

    #[test]
    fn compute_hash_is_stable() {
        let h1 = compute_hash("out", "err", "text");
        let h2 = compute_hash("out", "err", "text");
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_hash_differs_on_change() {
        let h1 = compute_hash("out", "err", "text");
        let h2 = compute_hash("out", "err", "different");
        assert_ne!(h1, h2);
    }

    #[test]
    fn parse_output_finds_json() {
        let stdout = "some preamble\n{\"summary\":\"All good\",\"findings\":[]}\n";
        let (parsed, text) = parse_output(stdout);
        assert!(parsed.is_some(), "expected JSON to parse");
        assert_eq!(text, "All good");
    }

    #[test]
    fn parse_output_falls_back_to_plain_text() {
        let stdout = "This is plain text output with no JSON.";
        let (parsed, text) = parse_output(stdout);
        assert!(parsed.is_none());
        assert!(text.contains("plain text"));
    }

    #[test]
    fn parse_output_handles_jsonl_last_line() {
        let stdout = "{\"event\":\"progress\"}\n{\"summary\":\"Done\",\"proposed_actions\":[]}\n";
        let (parsed, text) = parse_output(stdout);
        assert!(parsed.is_some());
        assert_eq!(text, "Done");
    }

    // ── Async fake-binary tests ───────────────────────────────────────────────

    async fn write_fake_binary(script: &str) -> tempfile::NamedTempFile {
        use std::os::unix::fs::PermissionsExt;
        let f = tempfile::NamedTempFile::new().unwrap();
        let path = f.path().to_owned();
        tokio::fs::write(&path, script).await.unwrap();
        let mut perms = tokio::fs::metadata(&path).await.unwrap().permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&path, perms).await.unwrap();
        f
    }

    #[tokio::test]
    async fn fake_binary_success_captures_stdout_stderr() {
        let fake =
            write_fake_binary("#!/bin/sh\necho 'hello stdout'\necho 'hello stderr' >&2\n").await;
        let result = run_codex(fake.path(), "test prompt", Duration::from_secs(10))
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
        let fake = write_fake_binary("#!/bin/sh\nsleep 30\n").await;
        let result = run_codex(
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
        let result = run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        assert!(result.parsed_output.is_some());
        assert_eq!(result.result_text, "Invoice paid");
    }

    #[tokio::test]
    async fn fake_binary_plain_text_fallback() {
        let fake = write_fake_binary("#!/bin/sh\necho 'This is plain text output'\n").await;
        let result = run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        assert!(result.parsed_output.is_none());
        assert!(result.result_text.contains("plain text"));
    }

    #[tokio::test]
    async fn fake_binary_large_output_truncates() {
        // ~70 KiB output, over MAX_OUTPUT_BYTES (64 KiB)
        let line = "x".repeat(100);
        // Use a temp file for the large payload to avoid arg-length limits
        let payload = (0..800)
            .map(|_| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        // Write payload to a temp file and cat it
        use tempfile::NamedTempFile;
        let pf = NamedTempFile::new().unwrap();
        tokio::fs::write(pf.path(), &payload).await.unwrap();
        let script = format!("#!/bin/sh\ncat '{}'\n", pf.path().display());
        let fake = write_fake_binary(&script).await;
        let result = run_codex(fake.path(), "test", Duration::from_secs(10))
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
        let r1 = run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        let r2 = run_codex(fake.path(), "test", Duration::from_secs(10))
            .await
            .unwrap();
        assert_eq!(r1.output_hash, r2.output_hash);
    }

    /// Integration test — run with `cargo test -p codex-connector -- --ignored`
    /// when `codex` is installed and authenticated.
    #[tokio::test]
    #[ignore = "requires codex CLI installed and authenticated"]
    async fn real_codex_exec_returns_output() {
        let binary = find_codex_in_path().expect("codex binary must be in PATH for this test");
        let result = run_codex(
            &binary,
            "{\"user_request\":\"Say hello in one word.\",\"allowed_context\":[],\
             \"constraints\":[\"Return JSON with a summary field only.\"],\
             \"expected_output\":\"analysis\"}",
            Duration::from_secs(60),
        )
        .await
        .expect("codex run should succeed");
        assert!(!result.timed_out);
        assert!(!result.output_hash.is_empty());
    }
}
