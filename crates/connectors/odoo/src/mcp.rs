//! MCP subprocess lifecycle for the Odoo connector.
//!
//! Spawns `uvx mcp-server-odoo` as a child process and maintains a
//! `rmcp` stdio client over its stdin/stdout. The Odoo API key flows
//! exclusively via the child's env — it is **never** written to disk.

use std::path::PathBuf;

use rmcp::{
    RoleClient, ServiceExt,
    model::{Content, RawContent},
    service::RunningService,
    transport::TokioChildProcess,
};
use tokio::process::Command;

use crate::types::{OdooConfig, OdooError};

// ── Type alias ────────────────────────────────────────────────────────────────

/// A running MCP client connected to `uvx mcp-server-odoo`.
/// Implements `Deref<Target = Peer<RoleClient>>`, so `call_tool` / `list_all_tools`
/// can be called directly on this value.
pub type McpClient = RunningService<RoleClient, ()>;

// ── Binary resolution ─────────────────────────────────────────────────────────

/// Locate the `uvx` binary.
///
/// GUI-launched macOS apps have a minimal `$PATH` that omits Homebrew, pip, etc.
/// We check:
/// 1. `override_path` — user-supplied path from Settings.
/// 2. Entries in `$PATH` (works in Terminal and some app launchers).
/// 3. Common install locations (`~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`).
pub fn find_uvx(override_path: Option<&str>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let is_executable = |p: &PathBuf| -> bool {
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };

    // 1. User override
    if let Some(p) = override_path {
        let path = PathBuf::from(p);
        if is_executable(&path) {
            return Some(path);
        }
    }

    // 2. $PATH
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("uvx");
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }

    // 3. Common macOS locations (GUI apps miss these)
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();

    let common: &[&str] = &[
        // uv/uvx installed by `curl -LsSf https://astral.sh/uv/install.sh | sh`
        ".local/bin/uvx",
        ".cargo/bin/uvx", // sometimes ends up here
    ];
    for rel in common {
        let candidate = home.join(rel);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }

    let abs: &[&str] = &[
        "/opt/homebrew/bin/uvx",
        "/usr/local/bin/uvx",
        "/opt/local/bin/uvx",
    ];
    for p in abs {
        let candidate = PathBuf::from(p);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }

    None
}

// ── Subprocess spawn ──────────────────────────────────────────────────────────

/// Spawn `uvx mcp-server-odoo` with credentials in the environment,
/// perform the MCP `initialize` handshake, and return the live client.
///
/// The first-ever invocation downloads the package from PyPI; use a
/// generous timeout at the call site (≥ 60 s).
pub async fn spawn_mcp(cfg: &OdooConfig, uvx_path: &PathBuf) -> Result<McpClient, OdooError> {
    let mut cmd = Command::new(uvx_path);
    // Pass credentials exclusively via environment — never on the command line or to disk.
    cmd.arg("mcp-server-odoo")
        .env("ODOO_URL", cfg.base_url.trim_end_matches('/'))
        .env("ODOO_API_KEY", &cfg.api_key)
        .env("ODOO_DB", &cfg.db);

    let transport = TokioChildProcess::new(cmd)
        .map_err(|e| OdooError::McpUnavailable(format!("failed to spawn uvx: {e}")))?;

    ().serve(transport)
        .await
        .map_err(|e| OdooError::McpUnavailable(format!("MCP initialize handshake failed: {e}")))
}

// ── Content helpers ───────────────────────────────────────────────────────────

/// Join all `Text` blocks in a `CallToolResult.content` into a single string.
pub fn extract_text(content: &[Content]) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for c in content {
        if let RawContent::Text(t) = &c.raw {
            parts.push(t.text.as_str());
        }
    }
    parts.join("\n")
}

/// Best-effort extraction of the first numeric `id` value from MCP text output.
///
/// mcp-server-odoo returns formatted text such as:
/// ```text
/// Found 1 records of res.partner:
///
/// 1. **Tenenet s.r.o.** (ID: 5)
///    Email: info@tenenet.sk
/// ```
///
/// We try several patterns in priority order:
/// 1. `"id": NUMBER` — if the server embeds inline JSON snippets
/// 2. `(ID: NUMBER)` or `ID: NUMBER` — the formatted text style
/// 3. `id: NUMBER` (case-insensitive)
pub fn extract_first_id(text: &str) -> Option<i64> {
    // Pattern 1: JSON-style `"id": N`
    if let Some(n) = find_after(text, "\"id\":", true) {
        return Some(n);
    }
    // Pattern 2: `(ID: N)` or `ID: N`
    if let Some(n) = find_after(text, "ID:", false) {
        return Some(n);
    }
    // Pattern 3: `id: N` (case-insensitive, not inside quotes)
    let lower = text.to_ascii_lowercase();
    if let Some(pos) = lower.find("id:") {
        if let Some(n) = parse_number_at(&text[pos + 3..]) {
            return Some(n);
        }
    }
    None
}

/// Best-effort extraction of the first record name from MCP text.
///
/// Looks for bold markdown like `**Name Here**` which mcp-server-odoo uses.
pub fn extract_first_name(text: &str) -> Option<String> {
    // Bold name pattern: **text**
    if let Some(start) = text.find("**") {
        let rest = &text[start + 2..];
        if let Some(end) = rest.find("**") {
            let name = rest[..end].trim();
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Find `needle` in `text`, then parse the first integer after it.
/// `skip_whitespace_colon`: strip leading whitespace + optional colon before digits.
fn find_after(text: &str, needle: &str, skip_whitespace_colon: bool) -> Option<i64> {
    let pos = text.find(needle)?;
    let rest = text[pos + needle.len()..].trim_start();
    let rest = if skip_whitespace_colon {
        rest.trim_start_matches(':').trim_start()
    } else {
        rest
    };
    parse_number_at(rest)
}

fn parse_number_at(s: &str) -> Option<i64> {
    let s = s.trim_start();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_id_from_json_style() {
        let text = r#"Found 1 records: {"id": 42, "name": "Test"}"#;
        assert_eq!(extract_first_id(text), Some(42));
    }

    #[test]
    fn extract_id_from_formatted_text() {
        let text = "Found 1 records of res.partner:\n\n1. **Tenenet s.r.o.** (ID: 5)\n   Email: info@tenenet.sk";
        assert_eq!(extract_first_id(text), Some(5));
    }

    #[test]
    fn extract_id_lowercase() {
        let text = "Record id: 99\nName: Test Corp";
        assert_eq!(extract_first_id(text), Some(99));
    }

    #[test]
    fn extract_id_none_when_missing() {
        assert_eq!(extract_first_id("No records found."), None);
    }

    #[test]
    fn extract_name_bold_markdown() {
        let text = "1. **Tenenet s.r.o.** (ID: 5)\n   Email: x";
        assert_eq!(
            extract_first_name(text).as_deref(),
            Some("Tenenet s.r.o.")
        );
    }

    #[test]
    fn extract_name_none_when_missing() {
        assert_eq!(extract_first_name("No records found."), None);
    }
}
