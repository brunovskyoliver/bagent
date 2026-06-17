use serde::{Deserialize, Serialize};

// ── Connection status ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhatsappConnectionStatus {
    Stopped,
    Starting,
    Qr,
    Authenticated,
    Ready,
    Disconnected,
    Error,
    /// Node runtime not found on this machine.
    MissingNode,
    /// `npm install` has not been run in the bridge directory.
    BridgeNotInstalled,
}

impl std::fmt::Display for WhatsappConnectionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Qr => "qr",
            Self::Authenticated => "authenticated",
            Self::Ready => "ready",
            Self::Disconnected => "disconnected",
            Self::Error => "error",
            Self::MissingNode => "missing_node",
            Self::BridgeNotInstalled => "bridge_not_installed",
        };
        write!(f, "{s}")
    }
}

// ── Status response ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsappStatus {
    pub status: WhatsappConnectionStatus,
    pub me: Option<WhatsappAccount>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsappAccount {
    pub id: String,
    pub name: Option<String>,
    pub push_name: Option<String>,
    pub number: Option<String>,
}

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsappContact {
    pub id: String,
    pub name: Option<String>,
    pub push_name: Option<String>,
    pub phone: Option<String>,
    pub is_business: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsappChat {
    pub id: String,
    pub name: Option<String>,
    pub is_group: bool,
    pub unread_count: u32,
    pub timestamp: Option<i64>,
    pub last_message_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsappMessage {
    pub id: String,
    pub chat_id: String,
    pub from: String,
    pub to: Option<String>,
    pub body: String,
    pub timestamp: i64,
    pub from_me: bool,
    pub has_media: bool,
}

// ── Send target ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum WhatsappSendTarget {
    /// WhatsApp JID (e.g. `15551234567@c.us` or `120363...@g.us` for groups).
    ChatId(String),
    /// Phone number; the bridge normalises it to a JID.
    Phone(String),
}

/// Minimal reference returned after a successful send.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsappMessageRef {
    pub message_id: String,
    pub chat_id: String,
}

// ── Errors ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum WhatsappError {
    /// Node runtime not found (no autodetect, no configured path).
    NodeNotFound,
    /// bridge/node_modules missing — user must run `make whatsapp-bridge-install`.
    BridgeNotInstalled,
    /// Bridge process failed to start.
    Spawn(String),
    /// Bridge is running but not in `ready` state.
    NotReady(WhatsappConnectionStatus),
    /// HTTP / network error talking to the bridge.
    Http(String),
    /// Bridge returned a non-success response.
    BridgeError(String),
    /// JSON parse error on bridge response.
    Parse(String),
    /// I/O error.
    Io(String),
}

impl std::fmt::Display for WhatsappError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NodeNotFound =>
                write!(f, "Node.js not found — install Node ≥18 via Homebrew (`brew install node`) then run `make whatsapp-bridge-install`"),
            Self::BridgeNotInstalled =>
                write!(f, "WhatsApp bridge dependencies missing — run `make whatsapp-bridge-install` in the repo root"),
            Self::Spawn(e) =>
                write!(f, "Failed to start WhatsApp bridge: {e}"),
            Self::NotReady(s) =>
                write!(f, "WhatsApp bridge not ready (status: {s})"),
            Self::Http(e) =>
                write!(f, "WhatsApp bridge HTTP error: {e}"),
            Self::BridgeError(e) =>
                write!(f, "WhatsApp bridge error: {e}"),
            Self::Parse(e) =>
                write!(f, "WhatsApp bridge response parse error: {e}"),
            Self::Io(e) =>
                write!(f, "WhatsApp bridge I/O error: {e}"),
        }
    }
}

impl std::error::Error for WhatsappError {}
