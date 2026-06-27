use serde::{Deserialize, Deserializer, Serialize};

// ── Config ────────────────────────────────────────────────────────────────────

/// Runtime configuration pushed by Swift on each launch (never persisted to disk).
#[derive(Clone, Deserialize)]
pub struct OdooConfig {
    /// Base URL of the Odoo instance, e.g. `https://mycompany.odoo.com`
    pub base_url: String,
    /// Database name (shown on the Odoo login screen).
    pub db: String,
    /// Login username (typically an email address).
    pub username: String,
    /// API key generated in Odoo → Settings → Technical → API Keys.
    pub api_key: String,
}

/// Manual Debug impl — redacts `api_key` so it never appears in logs/traces.
impl std::fmt::Debug for OdooConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OdooConfig")
            .field("base_url", &self.base_url)
            .field("db", &self.db)
            .field("username", &self.username)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ── Errors ────────────────────────────────────────────────────────────────────

/// Typed errors from the Odoo connector (not `anyhow` — callers need to branch).
#[derive(Debug)]
pub enum OdooError {
    /// Connector was never configured (no creds pushed yet).
    NotConfigured,
    /// Authentication failed (bad credentials or wrong Odoo URL / DB).
    Auth(String),
    /// HTTP / network error (REST version-check calls only).
    Network(String),
    /// MCP tool returned an error or could not be parsed.
    Rpc(String),
    /// `uvx` binary not found or MCP subprocess failed to start.
    /// Distinct from `Auth` so Settings can show the right hint ("install uv/uvx").
    McpUnavailable(String),
}

impl std::fmt::Display for OdooError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConfigured => write!(
                f,
                "Odoo connector not configured — enter URL, DB, username and API key in Settings"
            ),
            Self::Auth(msg) => write!(f, "Odoo authentication failed: {msg}"),
            Self::Network(msg) => write!(f, "Odoo network error: {msg}"),
            Self::Rpc(msg) => write!(f, "Odoo MCP error: {msg}"),
            Self::McpUnavailable(msg) => write!(
                f,
                "MCP server unavailable — install uv/uvx and ensure it's in PATH: {msg}"
            ),
        }
    }
}

// ── MCP result ────────────────────────────────────────────────────────────────

/// Result from an MCP tool call.
///
/// mcp-server-odoo returns *formatted text* optimised for LLM consumption, not raw JSON.
/// The text is injected directly into the agent context. We also attempt to extract
/// the first record's `id` and `name` for `OdooRecordRef` (the "Open in Safari" button).
#[derive(Debug, Clone)]
pub struct OdooMcpResult {
    /// Formatted text from the MCP server — inject as LLM context.
    pub text: String,
    /// Odoo model, e.g. `"res.partner"`.
    pub model: String,
    /// ID of the first record returned (best-effort; `None` if unparseable).
    pub first_id: Option<i64>,
    /// Display name of the first record (best-effort; `None` if unparseable).
    pub first_name: Option<String>,
}

impl std::error::Error for OdooError {}

// ── Odoo false-field helpers ──────────────────────────────────────────────────
//
// Odoo returns `false` (JSON boolean) for every unset field — `null` is NOT used.
// `Option<String>` + `#[serde(default)]` is NOT enough: `false` is a present bool,
// so serde errors. These helpers map `false | null` → `None`, otherwise unwrap the value.

pub fn false_or_string<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(d)?;
    match val {
        serde_json::Value::Bool(false) | serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => Ok(Some(s)),
        other => Ok(Some(other.to_string())),
    }
}

pub fn false_or_f64<'de, D>(d: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(d)?;
    match val {
        serde_json::Value::Bool(false) | serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(n) => Ok(n.as_f64()),
        _ => Ok(None),
    }
}

// ── Many2one field ────────────────────────────────────────────────────────────
//
// Odoo Many2one fields serialize as `[id, "Display Name"]` when set, or `false` when
// unset. This newtype handles both cases transparently.

#[derive(Debug, Clone, Serialize, Default)]
pub struct M2O {
    pub id: Option<i64>,
    pub name: Option<String>,
}

impl M2O {
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub fn display(&self) -> &str {
        self.name.as_deref().unwrap_or("—")
    }
}

impl<'de> Deserialize<'de> for M2O {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let val = serde_json::Value::deserialize(d)?;
        match val {
            serde_json::Value::Bool(false) | serde_json::Value::Null => Ok(M2O {
                id: None,
                name: None,
            }),
            serde_json::Value::Array(arr) if arr.len() == 2 => Ok(M2O {
                id: arr[0].as_i64(),
                name: arr[1].as_str().map(|s| s.to_string()),
            }),
            _ => Ok(M2O {
                id: None,
                name: None,
            }),
        }
    }
}

// ── Record structs ────────────────────────────────────────────────────────────

/// `res.partner` — contacts and companies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partner {
    pub id: i64,
    #[serde(deserialize_with = "false_or_string", default)]
    pub name: Option<String>,
    #[serde(deserialize_with = "false_or_string", default)]
    pub email: Option<String>,
    #[serde(deserialize_with = "false_or_string", default)]
    pub phone: Option<String>,
    /// IČO / DIČ / VAT — preserved verbatim.
    #[serde(deserialize_with = "false_or_string", default)]
    pub vat: Option<String>,
    #[serde(deserialize_with = "false_or_string", default)]
    pub city: Option<String>,
}

/// `account.move` — customer and vendor invoices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: i64,
    /// Invoice number (e.g. "BILL/2025/00042").
    #[serde(deserialize_with = "false_or_string", default)]
    pub name: Option<String>,
    /// Many2one → partner name.
    #[serde(default)]
    pub partner_id: M2O,
    /// Total amount with tax.
    #[serde(deserialize_with = "false_or_f64", default)]
    pub amount_total: Option<f64>,
    /// Many2one → currency symbol/name.
    #[serde(default)]
    pub currency_id: M2O,
    /// `draft` | `posted` | `cancel`.
    #[serde(deserialize_with = "false_or_string", default)]
    pub state: Option<String>,
    /// `not_paid` | `in_payment` | `paid` | `partial`.
    #[serde(deserialize_with = "false_or_string", default)]
    pub payment_state: Option<String>,
    #[serde(deserialize_with = "false_or_string", default)]
    pub invoice_date: Option<String>,
    #[serde(deserialize_with = "false_or_string", default)]
    pub invoice_date_due: Option<String>,
}

/// `helpdesk.ticket` — support tickets (Odoo Enterprise).
/// `user_id` is the ticket assignee (confirm on your instance — standard for Helpdesk).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelpdeskTicket {
    pub id: i64,
    #[serde(deserialize_with = "false_or_string", default)]
    pub name: Option<String>,
    /// Stage Many2one.
    #[serde(default)]
    pub stage_id: M2O,
    /// Partner / customer Many2one.
    #[serde(default)]
    pub partner_id: M2O,
    /// Assignee Many2one (`user_id`).
    #[serde(default)]
    pub user_id: M2O,
    /// `0` | `1` | `2` | `3` (low/normal/high/urgent).
    #[serde(deserialize_with = "false_or_string", default)]
    pub priority: Option<String>,
    #[serde(deserialize_with = "false_or_string", default)]
    pub create_date: Option<String>,
}

// ── OdooRecordRef ─────────────────────────────────────────────────────────────

/// Stable reference to a live Odoo record — analogue of `MailRef` / `FileRef`.
/// Emitted via SSE `odoo_found`; persisted to session metadata for cross-turn coreference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OdooRecordRef {
    /// Odoo model name, e.g. `res.partner`.
    pub model: String,
    /// Numeric record ID.
    pub id: i64,
    /// Human-readable display name of the record.
    pub name: String,
    /// Deep-link URL to open the record in the browser.
    pub url: String,
}
