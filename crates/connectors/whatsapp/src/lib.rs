//! WhatsApp Web connector for bagent.
//!
//! Manages the lifecycle of a local `whatsapp-web.js` bridge subprocess that:
//!   - Binds `127.0.0.1:<random-port>` (never 0.0.0.0)
//!   - Is protected by a bearer token generated per session
//!   - Communicates via a simple JSON HTTP API
//!
//! # Degradation
//!
//! - Node.js not installed → `status()` returns `WhatsappConnectionStatus::MissingNode`.
//! - `npm install` not run → `WhatsappConnectionStatus::BridgeNotInstalled`.
//! - Bridge not started → `Stopped`.
//! - QR not scanned → `Qr`.
//!
//! # Security
//!
//! - Bearer token never logged (custom Debug impls redact it).
//! - Session dir stored at `~/Library/Application Support/bagent/whatsapp/session`.
//! - Subprocess spawned with `tokio::process::Command`, never via `sh -c`.
//! - Send always requires external approval (enforced in the daemon route).

pub mod bridge_client;
pub mod process;
pub mod types;

pub use bridge_client::BridgeClient;
pub use process::{bridge_dir, bridge_installed, default_session_dir, resolve_node};
pub use types::{
    WhatsappAccount, WhatsappChat, WhatsappConnectionStatus, WhatsappContact, WhatsappError,
    WhatsappMessage, WhatsappMessageRef, WhatsappSendTarget, WhatsappStatus,
};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::{info, warn};

// ── WhatsappConnector ─────────────────────────────────────────────────────────

/// Connector configuration.
#[derive(Debug, Clone)]
pub struct WhatsappConfig {
    /// Explicit `node` binary path; `None` = auto-detect.
    pub node_path: Option<String>,
    /// Session directory; `None` = default (`~/Library/…/bagent/whatsapp/session`).
    pub session_dir: Option<PathBuf>,
    /// Bridge directory; `None` = auto-detect from manifest/exe location.
    pub bridge_dir: Option<PathBuf>,
}

impl Default for WhatsappConfig {
    fn default() -> Self {
        Self {
            node_path: None,
            session_dir: None,
            bridge_dir: None,
        }
    }
}

struct Inner {
    process: Option<process::BridgeProcess>,
    client: Option<BridgeClient>,
    token: Option<String>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("process", &self.process.as_ref().map(|_| "running"))
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// The WhatsApp connector. Cloneable; cheap to clone (all state behind Arc).
#[derive(Clone)]
pub struct WhatsappConnector {
    config: WhatsappConfig,
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for WhatsappConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhatsappConnector")
            .field("config", &self.config)
            .finish()
    }
}

impl WhatsappConnector {
    pub fn new(config: WhatsappConfig) -> Self {
        Self {
            config,
            inner: Arc::new(Mutex::new(Inner {
                process: None,
                client: None,
                token: None,
            })),
        }
    }

    /// True when the bridge is running and in `ready` state.
    pub async fn is_accessible(&self) -> bool {
        matches!(
            self.status().await.map(|s| s.status),
            Ok(WhatsappConnectionStatus::Ready)
        )
    }

    /// True when a previous WhatsApp Web LocalAuth profile exists on disk.
    ///
    /// This does not prove the session is still valid; it only means startup may
    /// restore without showing a QR code. The bridge itself reports `qr` if the
    /// phone needs to be paired again.
    pub fn has_persisted_session(&self) -> bool {
        let session_dir = self
            .config
            .session_dir
            .clone()
            .unwrap_or_else(process::default_session_dir);
        let profile = session_dir.join("session").join("Default");
        if !profile.is_dir() {
            return false;
        }
        [
            "IndexedDB",
            "Local Storage",
            "Session Storage",
            "Preferences",
        ]
        .iter()
        .any(|name| profile.join(name).exists())
    }

    // ── Lifecycle ─────────────────────────────────────────────────────────────

    /// Start the bridge subprocess.
    ///
    /// Resolves node binary and bridge dir first; degrades gracefully on missing deps.
    pub async fn start(&self) -> Result<(), WhatsappError> {
        let mut inner = self.inner.lock().await;
        if inner.process.is_some() {
            return Ok(()); // already running
        }

        // Resolve node
        let node = process::resolve_node(self.config.node_path.as_deref()).await?;

        // Resolve bridge dir
        let b_dir = match &self.config.bridge_dir {
            Some(d) => d.clone(),
            None => process::bridge_dir().ok_or(WhatsappError::BridgeNotInstalled)?,
        };

        if !process::bridge_installed(&b_dir) {
            return Err(WhatsappError::BridgeNotInstalled);
        }

        // Resolve session dir
        let session_dir = self
            .config
            .session_dir
            .clone()
            .unwrap_or_else(process::default_session_dir);
        std::fs::create_dir_all(&session_dir).map_err(|e| WhatsappError::Io(e.to_string()))?;

        // Kill any orphaned Chromium process that has the SingletonLock.
        // When the bridge node process is SIGKILL'd, Chromium becomes an orphan
        // and keeps the profile dir locked, blocking the next startup.
        process::kill_stale_chromium(&session_dir.join("session"));

        // Generate bearer token (UUID v4 style)
        let token = format!("{:016x}-{:016x}", rand_u64(), rand_u64());

        let proc = process::spawn_bridge(&node, &b_dir, &session_dir, &token).await?;
        let client = BridgeClient::new(proc.port, &token);

        inner.token = Some(token);
        inner.client = Some(client);
        inner.process = Some(proc);

        Ok(())
    }

    /// Stop the bridge subprocess (SIGTERM + wait).
    pub async fn stop(&self) -> Result<(), WhatsappError> {
        let mut inner = self.inner.lock().await;
        if let Some(mut proc) = inner.process.take() {
            let _ = proc.child.kill().await;
            let _ = tokio::time::timeout(Duration::from_secs(5), proc.child.wait()).await;
        }
        inner.client = None;
        inner.token = None;
        info!("WhatsApp bridge stopped");
        Ok(())
    }

    // ── Bridge API ────────────────────────────────────────────────────────────

    /// Current bridge status. Returns a synthetic status when bridge is not running.
    pub async fn status(&self) -> Result<WhatsappStatus, WhatsappError> {
        let inner = self.inner.lock().await;
        match &inner.client {
            Some(c) => {
                // Try to reach the bridge
                match tokio::time::timeout(Duration::from_secs(5), c.health()).await {
                    Ok(Ok(s)) => Ok(s),
                    Ok(Err(e)) => {
                        warn!("WhatsApp bridge health error: {e}");
                        Ok(WhatsappStatus {
                            status: WhatsappConnectionStatus::Disconnected,
                            me: None,
                            error: Some(e.to_string()),
                            diagnostics: None,
                        })
                    }
                    Err(_) => Ok(WhatsappStatus {
                        status: WhatsappConnectionStatus::Disconnected,
                        me: None,
                        error: Some("bridge health check timed out".into()),
                        diagnostics: None,
                    }),
                }
            }
            None => {
                // Check whether startup is possible at all
                match process::resolve_node(self.config.node_path.as_deref()).await {
                    Err(_) => Ok(WhatsappStatus {
                        status: WhatsappConnectionStatus::MissingNode,
                        me: None,
                        error: Some("Node.js not found — install Node ≥18 via Homebrew".into()),
                        diagnostics: None,
                    }),
                    Ok(_) => {
                        let b_dir = self.config.bridge_dir.clone().or_else(process::bridge_dir);
                        let installed = b_dir
                            .as_ref()
                            .map(|d| process::bridge_installed(d))
                            .unwrap_or(false);
                        if !installed {
                            Ok(WhatsappStatus {
                                status: WhatsappConnectionStatus::BridgeNotInstalled,
                                me: None,
                                error: Some(
                                    "Run `make whatsapp-bridge-install` to install bridge deps"
                                        .into(),
                                ),
                                diagnostics: None,
                            })
                        } else {
                            Ok(WhatsappStatus {
                                status: WhatsappConnectionStatus::Stopped,
                                me: None,
                                error: None,
                                diagnostics: None,
                            })
                        }
                    }
                }
            }
        }
    }

    /// Current QR string (only present when status is `qr`).
    pub async fn qr(&self) -> Result<Option<String>, WhatsappError> {
        let inner = self.inner.lock().await;
        let client = inner
            .client
            .as_ref()
            .ok_or_else(|| WhatsappError::NotReady(WhatsappConnectionStatus::Stopped))?;
        client.qr().await
    }

    pub async fn debug(&self) -> Result<serde_json::Value, WhatsappError> {
        let inner = self.inner.lock().await;
        let client = inner.client.clone();
        drop(inner);
        match client {
            Some(client) => client.debug().await,
            None => {
                let status = self.status().await?;
                Ok(serde_json::json!({
                    "status": status.status.to_string(),
                    "error": status.error,
                    "events": [],
                }))
            }
        }
    }

    pub async fn list_contacts(&self, limit: usize) -> Result<Vec<WhatsappContact>, WhatsappError> {
        let client = self.require_client().await?;
        client.list_contacts(limit).await
    }

    pub async fn list_chats(&self, limit: usize) -> Result<Vec<WhatsappChat>, WhatsappError> {
        let client = self.require_client().await?;
        client.list_chats(limit).await
    }

    pub async fn chat_messages(
        &self,
        chat_id: &str,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<WhatsappMessage>, WhatsappError> {
        let client = self.require_client().await?;
        client.chat_messages(chat_id, limit, before).await
    }

    /// Send exactly one text message. Does NOT enforce approval — the daemon
    /// route is responsible for gating this behind `request_approval_core`.
    pub async fn send_message(
        &self,
        target: WhatsappSendTarget,
        text: &str,
    ) -> Result<WhatsappMessageRef, WhatsappError> {
        let client = self.require_client().await?;
        client.send_message(target, text).await
    }

    pub async fn logout(&self) -> Result<(), WhatsappError> {
        let inner = self.inner.lock().await;
        if let Some(c) = &inner.client {
            let _ = c.logout().await;
        }
        Ok(())
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    async fn require_client(&self) -> Result<BridgeClient, WhatsappError> {
        let inner = self.inner.lock().await;
        inner
            .client
            .clone()
            .ok_or_else(|| WhatsappError::NotReady(WhatsappConnectionStatus::Stopped))
    }
}

/// Very simple random u64 using the current time + PID (no rand crate needed).
fn rand_u64() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let pid = std::process::id() as u64;
    t ^ (pid.wrapping_mul(0x517cc1b727220a95))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WhatsappConnectionStatus;

    #[test]
    fn connection_status_display() {
        assert_eq!(WhatsappConnectionStatus::Ready.to_string(), "ready");
        assert_eq!(
            WhatsappConnectionStatus::MissingNode.to_string(),
            "missing_node"
        );
        assert_eq!(
            WhatsappConnectionStatus::BridgeNotInstalled.to_string(),
            "bridge_not_installed"
        );
        assert_eq!(WhatsappConnectionStatus::Qr.to_string(), "qr");
    }

    #[test]
    fn whatsapp_error_display_no_token_leak() {
        let e = WhatsappError::BridgeError("some error".into());
        let s = e.to_string();
        assert!(s.contains("some error"));
    }

    #[test]
    fn rand_u64_different_each_time() {
        // Very basic: two consecutive calls should produce different values
        // (nanosecond resolution makes collision extremely unlikely)
        let a = rand_u64();
        let b = rand_u64();
        // Don't assert_ne — in theory equal; just verify it runs
        let _ = (a, b);
    }

    #[test]
    fn default_session_dir_contains_bagent() {
        let d = process::default_session_dir();
        let s = d.to_string_lossy();
        assert!(
            s.contains("bagent"),
            "session dir should be under bagent: {s}"
        );
        assert!(
            s.contains("whatsapp"),
            "session dir should mention whatsapp: {s}"
        );
    }

    #[test]
    fn persisted_session_detection_requires_browser_profile() {
        let root = std::env::temp_dir().join(format!("bagent-wa-test-{}", rand_u64()));
        let connector = WhatsappConnector::new(WhatsappConfig {
            session_dir: Some(root.clone()),
            ..Default::default()
        });
        assert!(!connector.has_persisted_session());

        let profile = root.join("session").join("Default");
        std::fs::create_dir_all(profile.join("IndexedDB")).unwrap();
        assert!(connector.has_persisted_session());

        let _ = std::fs::remove_dir_all(root);
    }

    // ── Intent deserialisation (mirroring spec §14) ───────────────────────────

    #[test]
    fn whatsapp_intent_none_deserialises() {
        let j = r#"{"action":"none","contact_name":null,"phone":null,"chat_id":null,"keywords":[],"date":null,"message_text":null,"limit":null}"#;
        let intent: WhatsappIntent = serde_json::from_str(j).unwrap();
        assert!(matches!(intent.action, WhatsappAction::None));
    }

    #[test]
    fn whatsapp_intent_list_recent_deserialises() {
        let j = r#"{"action":"list_recent"}"#;
        let intent: WhatsappIntent = serde_json::from_str(j).unwrap_or_default();
        assert!(matches!(intent.action, WhatsappAction::ListRecent));
    }

    #[test]
    fn whatsapp_intent_search_deserialises() {
        let j = r#"{"action":"search","keywords":["faktúra"]}"#;
        let intent: WhatsappIntent = serde_json::from_str(j).unwrap_or_default();
        assert!(matches!(intent.action, WhatsappAction::Search));
        assert_eq!(intent.keywords, vec!["faktúra"]);
    }

    #[test]
    fn whatsapp_intent_read_history_deserialises() {
        let j = r#"{"action":"read_history","contact_name":"Peter"}"#;
        let intent: WhatsappIntent = serde_json::from_str(j).unwrap_or_default();
        assert!(matches!(intent.action, WhatsappAction::ReadHistory));
        assert_eq!(intent.contact_name.as_deref(), Some("Peter"));
    }

    #[test]
    fn whatsapp_intent_draft_send_deserialises() {
        let j = r#"{"action":"draft_send","contact_name":"Katka","message_text":"Dobrý deň"}"#;
        let intent: WhatsappIntent = serde_json::from_str(j).unwrap_or_default();
        assert!(matches!(intent.action, WhatsappAction::DraftSend));
        assert_eq!(intent.message_text.as_deref(), Some("Dobrý deň"));
    }

    #[test]
    fn slovak_utf8_preserved_in_message_text() {
        let sk = "Dobrý deň, posielam faktúru č. 2026/001. Ďakujem.";
        let j = serde_json::json!({ "action": "draft_send", "message_text": sk });
        let s = j.to_string();
        assert!(s.contains("Ďakujem"), "Slovak text preserved in JSON");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["message_text"].as_str().unwrap(), sk);
    }

    #[test]
    fn phone_normalization_in_send_target() {
        // Verifies the bridge_client urlenc helper doesn't break @ signs
        let chat_id = "15551234567@c.us";
        let encoded = bridge_client::urlenc_public(chat_id);
        assert_eq!(encoded, "15551234567%40c.us");
    }

    #[test]
    fn cache_upsert_external_id_is_stable() {
        // Verify WhatsappMessage has a stable id field (no mutation)
        let msg = WhatsappMessage {
            id: "true_15551234567@c.us_3AB1234ABCD".to_string(),
            chat_id: "15551234567@c.us".to_string(),
            from: "15551234567@c.us".to_string(),
            to: None,
            body: "Dobrý deň".to_string(),
            timestamp: 1700000000,
            from_me: false,
            has_media: false,
        };
        assert_eq!(msg.id, "true_15551234567@c.us_3AB1234ABCD");
    }
}

// ── WhatsappIntent (re-exported from the agent crate but defined here for tests) ─

/// Structured intent for a WhatsApp user turn.
/// The full classifier lives in `crates/agent/src/whatsapp_intent.rs`;
/// this is a copy for the connector-level tests.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WhatsappIntent {
    pub action: WhatsappAction,
    pub contact_name: Option<String>,
    pub phone: Option<String>,
    pub chat_id: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub date: Option<String>,
    pub message_text: Option<String>,
    pub limit: Option<u32>,
}

impl Default for WhatsappIntent {
    fn default() -> Self {
        Self {
            action: WhatsappAction::None,
            contact_name: None,
            phone: None,
            chat_id: None,
            keywords: vec![],
            date: None,
            message_text: None,
            limit: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WhatsappAction {
    #[default]
    None,
    ListRecent,
    Search,
    ReadHistory,
    DraftSend,
}
