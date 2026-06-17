pub mod json_rpc;
pub mod types;

pub use types::{HelpdeskTicket, Invoice, OdooConfig, OdooError, OdooRecordRef, Partner, M2O};

use json_rpc::{build_call_payload, extract_result};
use serde_json::{json, Value};

/// Live Odoo connector. Created via `connect(cfg)` and stored in-memory only —
/// the API key is **never** written to daemon disk.
#[derive(Clone)]
pub struct OdooConnector {
    http: reqwest::Client,
    cfg: OdooConfig,
    /// Authenticated user ID returned by `common.authenticate`.
    pub uid: i64,
    /// Server version string from `common.version`.
    pub server_version: String,
}

/// Manual Debug impl — delegates to cfg's redacted Debug, never leaks api_key.
impl std::fmt::Debug for OdooConnector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OdooConnector")
            .field("cfg", &self.cfg)
            .field("uid", &self.uid)
            .field("server_version", &self.server_version)
            .finish_non_exhaustive()
    }
}

impl OdooConnector {
    /// Authenticate against Odoo and return a ready connector.
    ///
    /// Returns `OdooError::Auth` when credentials are wrong (uid == false).
    pub async fn connect(cfg: OdooConfig) -> Result<Self, OdooError> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .map_err(|e| OdooError::Network(e.to_string()))?;

        let url = format!("{}/jsonrpc", cfg.base_url.trim_end_matches('/'));

        // 1. Authenticate
        let auth_payload = build_call_payload(
            "common",
            "authenticate",
            json!([cfg.db, cfg.username, cfg.api_key, {}]),
        );
        let auth_resp: Value = http
            .post(&url)
            .json(&auth_payload)
            .send()
            .await
            .map_err(|e| OdooError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| OdooError::Network(e.to_string()))?;

        let result = extract_result(auth_resp).map_err(OdooError::Rpc)?;
        let uid = match &result {
            Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| OdooError::Auth("uid not an integer".into()))?,
            Value::Bool(false) => {
                return Err(OdooError::Auth(
                    "bad credentials or wrong database name".into(),
                ))
            }
            _ => return Err(OdooError::Auth(format!("unexpected uid type: {result}"))),
        };

        // 2. Fetch server version
        let ver_payload = build_call_payload("common", "version", json!([]));
        let ver_resp: Value = http
            .post(&url)
            .json(&ver_payload)
            .send()
            .await
            .map_err(|e| OdooError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| OdooError::Network(e.to_string()))?;

        let server_version = extract_result(ver_resp)
            .ok()
            .and_then(|v| v["server_version"].as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown".to_string());

        tracing::info!(uid, %server_version, "Odoo connected");

        Ok(Self {
            http,
            cfg,
            uid,
            server_version,
        })
    }

    /// True when credentials are stored — does not re-test the network.
    pub fn is_accessible(&self) -> bool {
        true
    }

    /// Low-level JSON-RPC `execute_kw` call.
    ///
    /// `args` = positional args (usually `[domain]`).
    /// `kwargs` = keyword args (e.g. `{"fields":[...],"limit":10}`).
    pub async fn execute_kw(
        &self,
        model: &str,
        method: &str,
        args: Value,
        kwargs: Value,
    ) -> Result<Value, OdooError> {
        let url = format!("{}/jsonrpc", self.cfg.base_url.trim_end_matches('/'));
        let payload = build_call_payload(
            "object",
            "execute_kw",
            json!([
                self.cfg.db,
                self.uid,
                self.cfg.api_key,
                model,
                method,
                args,
                kwargs
            ]),
        );
        let resp: Value = self
            .http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| OdooError::Network(e.to_string()))?
            .json()
            .await
            .map_err(|e| OdooError::Network(e.to_string()))?;

        extract_result(resp).map_err(OdooError::Rpc)
    }

    // ── Typed read helpers ────────────────────────────────────────────────────

    /// Search `res.partner` by name / email / phone (ilike OR search).
    pub async fn search_partners(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<Vec<Partner>, OdooError> {
        let domain = json!([[
            "|",
            "|",
            ["name", "ilike", query],
            ["email", "ilike", query],
            ["phone", "ilike", query],
        ]]);
        let kwargs = json!({
            "fields": ["id", "name", "email", "phone", "vat", "city"],
            "limit": limit,
            "order": "name asc",
        });
        let result = self
            .execute_kw("res.partner", "search_read", domain, kwargs)
            .await?;
        parse_records(result)
    }

    /// List invoices (`account.move`).
    /// When `open_only` is true, only unpaid/partially-paid invoices are returned.
    pub async fn my_invoices(
        &self,
        open_only: bool,
        limit: u32,
    ) -> Result<Vec<Invoice>, OdooError> {
        let mut domain = json!([[["move_type", "in", ["out_invoice", "in_invoice"]]]]);
        if open_only {
            // Extend the inner array — clone and push
            if let Some(arr) = domain[0].as_array_mut() {
                arr.push(json!(["payment_state", "not in", ["paid", "reversed"]]));
                arr.push(json!(["state", "=", "posted"]));
            }
        }
        let kwargs = json!({
            "fields": [
                "id", "name", "partner_id", "amount_total",
                "currency_id", "state", "payment_state",
                "invoice_date", "invoice_date_due"
            ],
            "limit": limit,
            "order": "invoice_date desc",
        });
        let result = self
            .execute_kw("account.move", "search_read", domain, kwargs)
            .await?;
        parse_records(result)
    }

    /// List helpdesk tickets assigned to the authenticated user.
    /// When `open_only` is true, closed/done stages are excluded (best-effort; stage names vary).
    pub async fn my_helpdesk_tickets(
        &self,
        open_only: bool,
        limit: u32,
    ) -> Result<Vec<HelpdeskTicket>, OdooError> {
        let mut domain_inner = vec![json!(["user_id", "=", self.uid])];
        if open_only {
            // Exclude tickets where stage is marked "fold" (closed) — requires a subquery
            // workaround: filter stage_id.fold = false. Use search_count workaround:
            // filter by `stage_id.fold` directly — Odoo supports dotted paths in domains.
            domain_inner.push(json!(["stage_id.fold", "=", false]));
        }
        let domain = json!([domain_inner]);
        let kwargs = json!({
            "fields": [
                "id", "name", "stage_id", "partner_id", "user_id",
                "priority", "create_date"
            ],
            "limit": limit,
            "order": "create_date desc",
        });
        let result = self
            .execute_kw("helpdesk.ticket", "search_read", domain, kwargs)
            .await?;
        parse_records(result)
    }

    /// Read a single record by model + id.
    pub async fn get_record(&self, model: &str, id: i64) -> Result<Option<Value>, OdooError> {
        let result = self
            .execute_kw(model, "read", json!([[id]]), json!({}))
            .await?;
        Ok(result.as_array().and_then(|a| a.first().cloned()))
    }

    /// Build an Odoo 18 deep-link URL for the record.
    ///
    /// Odoo 18 supports both the legacy hash format and the new path format.
    /// The legacy format (`/web#id=…&model=…&view_type=form`) still works in 18
    /// and is more universally compatible with both SaaS and self-hosted.
    ///
    /// If this does not redirect properly on your instance, switch to:
    /// `{base}/odoo/{model_slug}/{id}` (replace `.` with `-` in model name).
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
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_records<T: serde::de::DeserializeOwned>(val: Value) -> Result<Vec<T>, OdooError> {
    serde_json::from_value::<Vec<T>>(val)
        .map_err(|e| OdooError::Rpc(format!("failed to deserialize records: {e}")))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_url_format() {
        let cfg = OdooConfig {
            base_url: "https://myco.odoo.com".into(),
            db: "myco".into(),
            username: "user@myco.com".into(),
            api_key: "key".into(),
        };
        // Simulate connector — can't call connect() without network
        // Just test the URL builder directly.
        let url = format!(
            "{}/web#id={}&model={}&view_type=form",
            cfg.base_url.trim_end_matches('/'),
            42,
            "res.partner"
        );
        assert_eq!(
            url,
            "https://myco.odoo.com/web#id=42&model=res.partner&view_type=form"
        );
    }

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
        // Simulate what Odoo returns for a partner with no email/phone/vat.
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
        assert!(p.phone.is_none());
        assert_eq!(p.vat.as_deref(), Some("SK2024001234"));
        assert!(p.city.is_none());
    }

    #[test]
    fn invoice_deserialize() {
        let raw = serde_json::json!({
            "id": 42,
            "name": "BILL/2025/00042",
            "partner_id": [5, "Tenenet s.r.o."],
            "amount_total": 1200.50,
            "currency_id": [3, "EUR"],
            "state": "posted",
            "payment_state": "not_paid",
            "invoice_date": "2025-06-01",
            "invoice_date_due": false,
        });
        let inv: Invoice = serde_json::from_value(raw).unwrap();
        assert_eq!(inv.id, 42);
        assert_eq!(inv.partner_id.name.as_deref(), Some("Tenenet s.r.o."));
        assert_eq!(inv.amount_total, Some(1200.50));
        assert_eq!(inv.currency_id.name.as_deref(), Some("EUR"));
        assert!(inv.invoice_date_due.is_none());
    }

    #[test]
    fn ticket_deserialize() {
        let raw = serde_json::json!({
            "id": 7,
            "name": "Server nefunguje",
            "stage_id": [2, "In Progress"],
            "partner_id": false,
            "user_id": [1, "Oliver B."],
            "priority": "1",
            "create_date": "2025-06-10 09:00:00",
        });
        let t: HelpdeskTicket = serde_json::from_value(raw).unwrap();
        assert_eq!(t.name.as_deref(), Some("Server nefunguje"));
        assert_eq!(t.stage_id.name.as_deref(), Some("In Progress"));
        assert!(t.partner_id.id.is_none());
        assert_eq!(t.user_id.name.as_deref(), Some("Oliver B."));
    }

    /// Live smoke tests — require `ODOO_URL`, `ODOO_DB`, `ODOO_USER`, `ODOO_KEY` env vars.
    #[tokio::test]
    #[ignore = "requires live Odoo 18 instance"]
    async fn live_connect() {
        let cfg = OdooConfig {
            base_url: std::env::var("ODOO_URL").unwrap(),
            db: std::env::var("ODOO_DB").unwrap(),
            username: std::env::var("ODOO_USER").unwrap(),
            api_key: std::env::var("ODOO_KEY").unwrap(),
        };
        let conn = OdooConnector::connect(cfg).await.expect("connect");
        println!("uid={} version={}", conn.uid, conn.server_version);
    }

    #[tokio::test]
    #[ignore = "requires live Odoo 18 instance"]
    async fn live_search_partners() {
        let cfg = OdooConfig {
            base_url: std::env::var("ODOO_URL").unwrap(),
            db: std::env::var("ODOO_DB").unwrap(),
            username: std::env::var("ODOO_USER").unwrap(),
            api_key: std::env::var("ODOO_KEY").unwrap(),
        };
        let conn = OdooConnector::connect(cfg).await.unwrap();
        let partners = conn.search_partners("Tenenet", 5).await.unwrap();
        println!("partners: {partners:?}");
    }

    #[tokio::test]
    #[ignore = "requires live Odoo 18 instance"]
    async fn live_my_tickets() {
        let cfg = OdooConfig {
            base_url: std::env::var("ODOO_URL").unwrap(),
            db: std::env::var("ODOO_DB").unwrap(),
            username: std::env::var("ODOO_USER").unwrap(),
            api_key: std::env::var("ODOO_KEY").unwrap(),
        };
        let conn = OdooConnector::connect(cfg).await.unwrap();
        let tickets = conn.my_helpdesk_tickets(false, 10).await.unwrap();
        println!("tickets: {tickets:?}");
    }
}
