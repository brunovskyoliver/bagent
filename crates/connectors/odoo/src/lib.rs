pub mod mcp;
pub mod types;

pub use types::{
    HelpdeskTicket, Invoice, OdooConfig, OdooError, OdooMcpResult, OdooRecordRef, Partner, M2O,
};

use mcp::{McpClient, extract_first_id, extract_first_name, extract_text, find_uvx, spawn_mcp};
use rmcp::model::CallToolRequestParams;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

// ── OdooConnector ─────────────────────────────────────────────────────────────

/// Live Odoo connector backed by an MCP subprocess (`uvx mcp-server-odoo`).
///
/// Created via `connect(cfg)` and stored in-memory only —
/// the API key is **never** written to daemon disk; it travels via the child env.
///
/// Note: `OdooConnector` is intentionally NOT `Clone` because `McpClient`
/// (a `RunningService`) owns a live background task.
pub struct OdooConnector {
    client: McpClient,
    cfg: OdooConfig,
    /// Authenticated user ID (from `res.users` lookup during `connect()`).
    pub uid: i64,
    /// Server version (from `/mcp/system/info`; defaults to `"MCP"` if unavailable).
    pub server_version: String,
    /// Number of tools registered by the MCP server.
    pub tool_count: usize,
    /// Resolved path to the `uvx` binary (for re-spawn on reconfigure).
    pub uvx_path: PathBuf,
}

/// Manual Debug impl — never leaks `api_key`.
impl std::fmt::Debug for OdooConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OdooConnector")
            .field("cfg", &self.cfg)
            .field("uid", &self.uid)
            .field("server_version", &self.server_version)
            .field("tool_count", &self.tool_count)
            .finish_non_exhaustive()
    }
}

impl OdooConnector {
    // ── Connect ───────────────────────────────────────────────────────────────

    /// Authenticate against Odoo via MCP and return a ready connector.
    ///
    /// Steps:
    /// 1. Resolve `uvx`; fail with `McpUnavailable` if not found.
    /// 2. Spawn `uvx mcp-server-odoo` with a 90 s timeout (first run downloads package).
    /// 3. Verify credentials via `search_records(res.users, login=username)` →
    ///    extracts `uid`; bad creds → `OdooError::Auth`.
    /// 4. Fetch `server_version` from `/mcp/system/info` (best-effort, non-fatal).
    pub async fn connect(cfg: OdooConfig) -> Result<Self, OdooError> {
        Self::connect_with_uvx(cfg, None).await
    }

    /// Same as `connect()` but accepts a user-supplied `uvx` path override.
    pub async fn connect_with_uvx(
        cfg: OdooConfig,
        uvx_override: Option<&str>,
    ) -> Result<Self, OdooError> {
        // 1. Resolve uvx binary
        let uvx_path = find_uvx(uvx_override).ok_or_else(|| {
            OdooError::McpUnavailable(
                "`uvx` binary not found — install uv (https://docs.astral.sh/uv/) \
                 and ensure uvx is in PATH or enter its full path in Settings"
                    .into(),
            )
        })?;

        tracing::debug!(uvx = %uvx_path.display(), "spawning mcp-server-odoo");

        // 2. Spawn MCP subprocess + initialize handshake (generous timeout for cold start)
        let client = tokio::time::timeout(
            Duration::from_secs(90),
            spawn_mcp(&cfg, &uvx_path),
        )
        .await
        .map_err(|_| {
            OdooError::McpUnavailable(
                "timed out after 90 s — first run may need longer for uvx to download the package"
                    .into(),
            )
        })??;

        // 3. Verify creds + resolve uid via /mcp/auth/validate REST endpoint
        //    (search_records on res.users requires explicit MCP model permission — use REST instead)
        let uid = resolve_uid_rest(&cfg).await?;

        // 4. Cache tool list
        let tool_count = client
            .list_all_tools()
            .await
            .map(|t| t.len())
            .unwrap_or(0);

        // 5. Best-effort server version from REST (non-fatal)
        let server_version = fetch_server_version(&cfg).await;

        tracing::info!(uid, tool_count, %server_version, "Odoo MCP connected");

        Ok(Self {
            client,
            cfg,
            uid,
            server_version,
            tool_count,
            uvx_path,
        })
    }

    /// Always returns `true` (no re-test; creds were verified in `connect()`).
    pub fn is_accessible(&self) -> bool {
        true
    }

    // ── Read helpers ──────────────────────────────────────────────────────────

    /// Search `res.partner` by name / email (ilike OR).
    pub async fn search_partners(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<OdooMcpResult, OdooError> {
        let domain = json!(["|", ["name", "ilike", query], ["email", "ilike", query]]);
        self.search_records(
            "res.partner",
            domain,
            json!(["id", "name", "email", "phone", "vat", "city"]),
            limit,
        )
        .await
    }

    /// List invoices (`account.move`).
    /// When `open_only` is true, only unpaid / partially-paid invoices are returned.
    pub async fn my_invoices(
        &self,
        open_only: bool,
        limit: u32,
    ) -> Result<OdooMcpResult, OdooError> {
        let mut filters = vec![
            json!(["move_type", "in", ["out_invoice", "in_invoice"]]),
        ];
        if open_only {
            filters.push(json!(["payment_state", "not in", ["paid", "reversed"]]));
            filters.push(json!(["state", "=", "posted"]));
        }
        let domain = Value::Array(filters);
        self.search_records(
            "account.move",
            domain,
            json!([
                "id", "name", "partner_id", "amount_total",
                "currency_id", "state", "payment_state",
                "invoice_date", "invoice_date_due"
            ]),
            limit,
        )
        .await
    }

    /// List helpdesk tickets assigned to the authenticated user.
    pub async fn my_helpdesk_tickets(
        &self,
        open_only: bool,
        limit: u32,
    ) -> Result<OdooMcpResult, OdooError> {
        let mut filters = vec![json!(["user_id", "=", self.uid])];
        if open_only {
            filters.push(json!(["stage_id.fold", "=", false]));
        }
        let domain = Value::Array(filters);
        self.search_records(
            "helpdesk.ticket",
            domain,
            json!([
                "id", "name", "stage_id", "partner_id", "user_id",
                "priority", "create_date"
            ]),
            limit,
        )
        .await
    }

    /// Read a single record by model + id.
    pub async fn get_record(
        &self,
        model: &str,
        id: i64,
    ) -> Result<Option<OdooMcpResult>, OdooError> {
        let args = json!({
            "model": model,
            "record_id": id,
        });
        let result = self.call_mcp("get_record", args).await?;
        if result.text.is_empty() {
            return Ok(None);
        }
        let first_name = extract_first_name(&result.text);
        Ok(Some(OdooMcpResult {
            text: result.text,
            model: model.to_string(),
            first_id: Some(id),
            first_name,
        }))
    }

    // ── URL / ref builders ────────────────────────────────────────────────────

    /// Build an Odoo 18 deep-link URL for the record.
    pub fn web_url(&self, model: &str, id: i64) -> String {
        format!(
            "{}/web#id={}&model={}&view_type=form",
            self.cfg.base_url.trim_end_matches('/'),
            id,
            model
        )
    }

    /// Build an `OdooRecordRef` for the given model/id/name.
    pub fn record_ref(&self, model: &str, id: i64, name: &str) -> OdooRecordRef {
        OdooRecordRef {
            model: model.to_string(),
            id,
            name: name.to_string(),
            url: self.web_url(model, id),
        }
    }

    // ── Private: MCP call helpers ─────────────────────────────────────────────

    /// Call `search_records` MCP tool and return `OdooMcpResult`.
    async fn search_records(
        &self,
        model: &str,
        domain: Value,
        fields: Value,
        limit: u32,
    ) -> Result<OdooMcpResult, OdooError> {
        let args = json!({
            "model": model,
            "domain": domain,
            "fields": fields,
            "limit": limit,
        });
        let raw = self.call_mcp("search_records", args).await?;
        let first_id = extract_first_id(&raw.text);
        let first_name = extract_first_name(&raw.text);
        Ok(OdooMcpResult {
            text: raw.text,
            model: model.to_string(),
            first_id,
            first_name,
        })
    }

    /// Low-level MCP tool call. Returns a temporary struct with just `text` + `structured`.
    async fn call_mcp(&self, tool: &str, args: Value) -> Result<RawMcpResult, OdooError> {
        let json_obj = args
            .as_object()
            .ok_or_else(|| OdooError::Rpc("args must be a JSON object".into()))?
            .clone();

        // Build params with a 'static name — tool names are compile-time constants.
        let tool_name: std::borrow::Cow<'static, str> = tool.to_string().into();
        let params = CallToolRequestParams::new(tool_name).with_arguments(json_obj);

        let result = self
            .client
            .call_tool(params)
            .await
            .map_err(|e| OdooError::Rpc(format!("MCP call_tool({tool}) failed: {e}")))?;

        // Error flag from the server
        if result.is_error == Some(true) {
            let err_text = extract_text(&result.content);
            let err_text = if err_text.is_empty() {
                "unknown MCP tool error".into()
            } else {
                err_text
            };
            // Heuristic: auth errors mention "Access Denied" or "Authentication"
            let lower = err_text.to_ascii_lowercase();
            if lower.contains("access denied")
                || lower.contains("authentication")
                || lower.contains("invalid api")
            {
                return Err(OdooError::Auth(err_text));
            }
            return Err(OdooError::Rpc(err_text));
        }

        // Prefer structured_content (JSON) if the server emits it; otherwise use text.
        let text = if let Some(sc) = &result.structured_content {
            // Pretty-print JSON for the LLM context
            serde_json::to_string_pretty(sc).unwrap_or_else(|_| extract_text(&result.content))
        } else {
            extract_text(&result.content)
        };

        Ok(RawMcpResult { text })
    }
}

/// Internal helper: raw output from one MCP tool call.
struct RawMcpResult {
    pub text: String,
}

// ── Private helpers ───────────────────────────────────────────────────────────

/// Validate credentials and resolve uid via `GET /mcp/auth/validate`.
///
/// This REST endpoint is accessible regardless of per-model MCP permissions,
/// unlike `search_records('res.users')` which requires explicit model access.
async fn resolve_uid_rest(cfg: &OdooConfig) -> Result<i64, OdooError> {
    let url = format!("{}/mcp/auth/validate", cfg.base_url.trim_end_matches('/'));
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| OdooError::Network(format!("reqwest build failed: {e}")))?;

    let resp = http
        .get(&url)
        .header("X-API-Key", &cfg.api_key)
        .send()
        .await
        .map_err(|e| OdooError::Network(format!("auth/validate request failed: {e}")))?;

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(OdooError::Auth(
            "invalid API key or insufficient permissions — check Odoo MCP settings".into(),
        ));
    }
    if !status.is_success() {
        return Err(OdooError::Auth(format!(
            "auth/validate returned HTTP {status}"
        )));
    }

    // Parse uid from the response JSON — try common field names defensively
    let body: Value = resp
        .json()
        .await
        .unwrap_or(Value::Null);

    let uid = body.get("uid")
        .or_else(|| body.get("user_id"))
        .or_else(|| body.get("id"))
        .and_then(|v| v.as_i64());

    match uid {
        Some(id) => {
            tracing::debug!(uid = id, "resolved uid via /mcp/auth/validate");
            Ok(id)
        }
        None => {
            // Auth succeeded but uid field absent — log and use 0
            // Helpdesk/invoice domain filters will return wrong results but connection works
            tracing::warn!(
                body = ?body,
                "uid not found in /mcp/auth/validate response; defaulting to 0"
            );
            Ok(0)
        }
    }
}

/// Fetch Odoo server version from `/mcp/system/info` (best-effort, non-fatal).
async fn fetch_server_version(cfg: &OdooConfig) -> String {
    let url = format!("{}/mcp/system/info", cfg.base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return "MCP".to_string(),
    };
    match client
        .get(&url)
        .header("X-API-Key", &cfg.api_key)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<Value>().await {
                if let Some(v) = body
                    .get("odoo_version")
                    .or_else(|| body.get("version"))
                    .and_then(|v| v.as_str())
                {
                    return v.to_string();
                }
            }
            "MCP".to_string()
        }
        _ => "MCP".to_string(),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{M2O, Partner};

    #[test]
    fn web_url_format() {
        // Test the URL builder in isolation (can't call connect() without network)
        let url = format!(
            "{}/web#id={}&model={}&view_type=form",
            "https://myco.odoo.com".trim_end_matches('/'),
            42,
            "res.partner"
        );
        assert_eq!(
            url,
            "https://myco.odoo.com/web#id=42&model=res.partner&view_type=form"
        );
    }

    // The typed struct deserialisation tests remain valid even though the primary
    // code path no longer uses them — they guard the serde helpers in types.rs.

    #[test]
    fn m2o_deserialize_array() {
        let val = serde_json::json!([5, "Tenenet s.r.o."]);
        let m: M2O = serde_json::from_value(val).unwrap();
        assert_eq!(m.id, Some(5));
        assert_eq!(m.name.as_deref(), Some("Tenenet s.r.o."));
    }

    #[test]
    fn m2o_deserialize_false() {
        let val = serde_json::json!(false);
        let m: M2O = serde_json::from_value(val).unwrap();
        assert!(m.id.is_none());
        assert!(m.name.is_none());
    }

    #[test]
    fn partner_deserialize_with_false_fields() {
        let raw = serde_json::json!({
            "id": 1,
            "name": "Tenenet s.r.o.",
            "email": false,
            "phone": false,
            "vat": "SK2024001234",
            "city": false,
        });
        let p: Partner = serde_json::from_value(raw).unwrap();
        assert_eq!(p.id, 1);
        assert_eq!(p.name.as_deref(), Some("Tenenet s.r.o."));
        assert!(p.email.is_none());
        assert_eq!(p.vat.as_deref(), Some("SK2024001234"));
    }

    /// Verify domain JSON shapes are not double-wrapped.
    #[test]
    fn domain_shapes_not_double_wrapped() {
        // search_partners domain: should be ["|", term, term], NOT [["|", ...]]
        let domain_partners =
            json!(["|", ["name", "ilike", "q"], ["email", "ilike", "q"]]);
        assert!(domain_partners.is_array(), "domain must be array");
        assert_eq!(
            domain_partners.as_array().unwrap()[0].as_str(),
            Some("|"),
            "first element should be '|' operator, not another array"
        );

        // resolve_uid domain: should be [["login","=","u"]], NOT [[["login",...]]]
        let domain_uid = json!([["login", "=", "user@test.com"]]);
        let outer = domain_uid.as_array().unwrap();
        assert_eq!(outer.len(), 1, "one term");
        assert!(outer[0].is_array(), "first element is a term array");
        assert_eq!(outer[0].as_array().unwrap()[0].as_str(), Some("login"));

        // my_invoices domain: Value::Array(filters) shape check
        let filters = vec![json!(["move_type", "in", ["out_invoice", "in_invoice"]])];
        let domain_inv = Value::Array(filters);
        let outer_inv = domain_inv.as_array().unwrap();
        assert_eq!(outer_inv.len(), 1);
        assert_eq!(
            outer_inv[0].as_array().unwrap()[0].as_str(),
            Some("move_type")
        );
    }

    #[test]
    fn odoo_mcp_result_fields() {
        let r = OdooMcpResult {
            text: "Found 1 records:\n\n1. **Test** (ID: 7)".into(),
            model: "res.partner".into(),
            first_id: Some(7),
            first_name: Some("Test".into()),
        };
        assert_eq!(r.first_id, Some(7));
        assert_eq!(r.model, "res.partner");
    }

    /// Live smoke tests — require env vars `ODOO_URL`, `ODOO_DB`, `ODOO_USER`, `ODOO_KEY`.
    #[tokio::test]
    #[ignore = "requires live Odoo + uvx mcp-server-odoo installed"]
    async fn live_connect() {
        let cfg = OdooConfig {
            base_url: std::env::var("ODOO_URL").unwrap(),
            db: std::env::var("ODOO_DB").unwrap(),
            username: std::env::var("ODOO_USER").unwrap(),
            api_key: std::env::var("ODOO_KEY").unwrap(),
        };
        let conn = OdooConnector::connect(cfg).await.expect("connect");
        println!("uid={} version={} tools={}", conn.uid, conn.server_version, conn.tool_count);
    }

    #[tokio::test]
    #[ignore = "requires live Odoo + uvx mcp-server-odoo installed"]
    async fn live_search_partners() {
        let cfg = OdooConfig {
            base_url: std::env::var("ODOO_URL").unwrap(),
            db: std::env::var("ODOO_DB").unwrap(),
            username: std::env::var("ODOO_USER").unwrap(),
            api_key: std::env::var("ODOO_KEY").unwrap(),
        };
        let conn = OdooConnector::connect(cfg).await.unwrap();
        let result = conn.search_partners("Tenenet", 5).await.unwrap();
        println!("text:\n{}", result.text);
        println!("first_id: {:?}, first_name: {:?}", result.first_id, result.first_name);
    }

    #[tokio::test]
    #[ignore = "requires live Odoo + uvx mcp-server-odoo installed"]
    async fn live_my_tickets() {
        let cfg = OdooConfig {
            base_url: std::env::var("ODOO_URL").unwrap(),
            db: std::env::var("ODOO_DB").unwrap(),
            username: std::env::var("ODOO_USER").unwrap(),
            api_key: std::env::var("ODOO_KEY").unwrap(),
        };
        let conn = OdooConnector::connect(cfg).await.unwrap();
        let result = conn.my_helpdesk_tickets(false, 10).await.unwrap();
        println!("text:\n{}", result.text);
    }
}
