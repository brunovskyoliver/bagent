use serde_json::{json, Value};

static CALL_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Build a JSON-RPC 2.0 `call` envelope.
///
/// The Odoo RPC convention:
/// - service `"common"` for `authenticate` / `version`
/// - service `"object"` for `execute_kw`
///
/// All parameters go under `params.args`.
pub fn build_call_payload(service: &str, method: &str, args: Value) -> Value {
    let id = CALL_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    json!({
        "jsonrpc": "2.0",
        "method": "call",
        "id": id,
        "params": {
            "service": service,
            "method": method,
            "args": args,
        }
    })
}

/// Extract `result` from a JSON-RPC response, mapping `error` → `Err`.
pub fn extract_result(resp: Value) -> Result<Value, String> {
    if let Some(err) = resp.get("error") {
        let msg = err
            .get("data")
            .and_then(|d| d.get("message"))
            .and_then(|m| m.as_str())
            .or_else(|| err.get("message").and_then(|m| m.as_str()))
            .unwrap_or("unknown RPC error");
        return Err(msg.to_string());
    }
    resp.get("result")
        .cloned()
        .ok_or_else(|| "JSON-RPC response missing 'result'".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_shape() {
        let p = build_call_payload(
            "common",
            "authenticate",
            json!(["mydb", "admin", "key", {}]),
        );
        assert_eq!(p["jsonrpc"], "2.0");
        assert_eq!(p["method"], "call");
        assert_eq!(p["params"]["service"], "common");
        assert_eq!(p["params"]["method"], "authenticate");
        assert_eq!(p["params"]["args"][0], "mydb");
    }

    #[test]
    fn extract_ok() {
        let resp = json!({ "jsonrpc": "2.0", "id": 1, "result": [1, 2, 3] });
        let r = extract_result(resp).unwrap();
        assert_eq!(r, json!([1, 2, 3]));
    }

    #[test]
    fn extract_err() {
        let resp = json!({
            "jsonrpc": "2.0", "id": 1,
            "error": { "message": "top", "data": { "message": "inner error" } }
        });
        let e = extract_result(resp).unwrap_err();
        assert_eq!(e, "inner error");
    }
}
