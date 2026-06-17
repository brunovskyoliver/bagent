//! Bridge subprocess lifecycle.
//!
//! Resolves the `node` binary, verifies that `node_modules` is installed,
//! spawns the bridge, reads the `PORT=<n>` first-stdout-line, and health-polls
//! until the bridge leaves the `stopped` status.
//!
//! Uses `tokio::process::Command` — never `sh -c`.

use std::path::PathBuf;
use std::time::Duration;

use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tracing::{info, warn};

#[cfg(unix)]
use libc;

use crate::types::WhatsappError;

// ── Node binary resolution ────────────────────────────────────────────────────

const NODE_CANDIDATES: &[&str] = &["/opt/homebrew/bin/node", "/usr/local/bin/node"];

/// Resolve the `node` binary path.
///
/// Order: configured path → known well-known paths → `which node`.
/// Returns `Err(WhatsappError::NodeNotFound)` when none is found.
pub async fn resolve_node(configured: Option<&str>) -> Result<PathBuf, WhatsappError> {

    if let Some(p) = configured {
        let path = PathBuf::from(p);
        if is_executable(&path) {
            return Ok(path);
        }
        warn!(path = %path.display(), "configured node path not executable");
    }

    for candidate in NODE_CANDIDATES {
        let path = PathBuf::from(candidate);
        if is_executable(&path) {
            return Ok(path);
        }
    }

    // Last resort: `which node`
    if let Ok(out) = Command::new("which").arg("node").output().await {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                let path = PathBuf::from(&s);
                if is_executable(&path) {
                    return Ok(path);
                }
            }
        }
    }

    Err(WhatsappError::NodeNotFound)
}

fn is_executable(path: &PathBuf) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

// ── Bridge directory resolution ───────────────────────────────────────────────

/// Resolve the bridge directory.
///
/// Dev: uses the compile-time `CARGO_MANIFEST_DIR` of this crate (always correct on the
/// build machine).  Production: looks for `whatsapp-bridge/` next to the daemon binary.
pub fn bridge_dir() -> Option<PathBuf> {
    // Compile-time path: <crate-root>/bridge — valid on the build machine.
    let compiled_in = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("bridge");
    if compiled_in.exists() {
        return Some(compiled_in);
    }

    // Production bundle: bridge shipped as `whatsapp-bridge/` beside the exe.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("whatsapp-bridge");
            if p.exists() {
                return Some(p);
            }
        }
    }

    None
}

/// Return true if `node_modules/whatsapp-web.js` exists in the bridge dir.
pub fn bridge_installed(bridge_dir: &PathBuf) -> bool {
    bridge_dir
        .join("node_modules")
        .join("whatsapp-web.js")
        .exists()
}

// ── Stale Chromium cleanup ────────────────────────────────────────────────────

/// Kill any orphaned Chromium that owns `SingletonLock` in `profile_dir`.
///
/// When the bridge node process is SIGKILL'd it leaves Chromium orphaned — it
/// keeps the profile dir locked and blocks the next `initialize()`.
/// `SingletonLock` is a symlink whose target encodes `<hostname>-<pid>`.
#[cfg(unix)]
pub fn kill_stale_chromium(profile_dir: &std::path::Path) {
    let lock = profile_dir.join("SingletonLock");
    let target = match std::fs::read_link(&lock) {
        Ok(t) => t,
        Err(_) => return,
    };
    // Target format: "<hostname>-<pid>"
    let s = target.to_string_lossy();
    if let Some(pid_str) = s.rsplit('-').next() {
        if let Ok(pid) = pid_str.parse::<u32>() {
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) };
            std::thread::sleep(std::time::Duration::from_millis(200));
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            warn!(pid, "killed stale Chromium from previous bridge session");
        }
    }
    let _ = std::fs::remove_file(&lock);
}

#[cfg(not(unix))]
pub fn kill_stale_chromium(_profile_dir: &std::path::Path) {}

// ── Spawned bridge process ─────────────────────────────────────────────────────

/// Live bridge process: child handle + discovered loopback port.
pub struct BridgeProcess {
    pub child: tokio::process::Child,
    pub port: u16,
    pub token: String,
}

impl std::fmt::Debug for BridgeProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeProcess")
            .field("port", &self.port)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// Spawn the bridge subprocess.
///
/// - Generates a random bearer token (UUIDv4).
/// - Passes token + session dir via env (`BAGENT_WA_TOKEN`, `BAGENT_WA_SESSION`).
/// - Reads the first stdout line for `PORT=<n>`.
/// - Returns `BridgeProcess` with child, port, and token.
pub async fn spawn_bridge(
    node: &PathBuf,
    bridge_dir: &PathBuf,
    session_dir: &PathBuf,
    token: &str,
) -> Result<BridgeProcess, WhatsappError> {
    let index_js = bridge_dir.join("index.js");
    if !index_js.exists() {
        return Err(WhatsappError::BridgeNotInstalled);
    }

    let session_str = session_dir.to_string_lossy().into_owned();

    let mut child = Command::new(node)
        .arg(&index_js)
        .env("BAGENT_WA_TOKEN", token)
        .env("BAGENT_WA_SESSION", &session_str)
        .env("NODE_ENV", "production")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit()) // let stderr flow to daemon logs
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| WhatsappError::Spawn(e.to_string()))?;

    // Read the first stdout line (must be "PORT=<n>") with a 30 s timeout.
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| WhatsappError::Spawn("stdout pipe unavailable".into()))?;
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();

    tokio::time::timeout(Duration::from_secs(30), reader.read_line(&mut line))
        .await
        .map_err(|_| WhatsappError::Spawn("timed out waiting for PORT= line".into()))?
        .map_err(|e| WhatsappError::Io(e.to_string()))?;

    let line = line.trim();
    let port: u16 = line
        .strip_prefix("PORT=")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            WhatsappError::Spawn(format!(
                "expected 'PORT=<n>' as first stdout line, got: {line:?}"
            ))
        })?;

    info!(port, "WhatsApp bridge started");

    Ok(BridgeProcess {
        child,
        port,
        token: token.to_string(),
    })
}

// ── Session directory ─────────────────────────────────────────────────────────

/// Default session directory: `~/Library/Application Support/bagent/whatsapp/session`.
pub fn default_session_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bagent")
        .join("whatsapp")
        .join("session")
}
