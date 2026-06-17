//! HTTP client for the local WhatsApp bridge loopback API.

use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

use crate::types::{
    WhatsappChat, WhatsappContact, WhatsappError, WhatsappMessage, WhatsappMessageRef,
    WhatsappSendTarget, WhatsappStatus,
};

#[derive(Clone)]
pub struct BridgeClient {
    http: Client,
    base: String,
    token: String,
}

impl std::fmt::Debug for BridgeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeClient")
            .field("base", &self.base)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl BridgeClient {
    pub fn new(port: u16, token: &str) -> Self {
        Self {
            http: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("reqwest client build failed"),
            base: format!("http://127.0.0.1:{port}"),
            token: token.to_string(),
        }
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    async fn get_json(&self, path: &str) -> Result<Value, WhatsappError> {
        self.http
            .get(self.url(path))
            .header("Authorization", self.auth())
            .send()
            .await
            .map_err(|e| WhatsappError::Http(e.to_string()))?
            .json::<Value>()
            .await
            .map_err(|e| WhatsappError::Parse(e.to_string()))
    }

    async fn post_json(&self, path: &str, body: Value) -> Result<Value, WhatsappError> {
        self.http
            .post(self.url(path))
            .header("Authorization", self.auth())
            .json(&body)
            .send()
            .await
            .map_err(|e| WhatsappError::Http(e.to_string()))?
            .json::<Value>()
            .await
            .map_err(|e| WhatsappError::Parse(e.to_string()))
    }

    // ── API calls ─────────────────────────────────────────────────────────────

    pub async fn health(&self) -> Result<WhatsappStatus, WhatsappError> {
        let v = self.get_json("/health").await?;
        serde_json::from_value(v).map_err(|e| WhatsappError::Parse(e.to_string()))
    }

    pub async fn qr(&self) -> Result<Option<String>, WhatsappError> {
        let v = self.get_json("/qr").await?;
        Ok(v.get("qr").and_then(|q| q.as_str()).map(|s| s.to_string()))
    }

    pub async fn list_contacts(&self, limit: usize) -> Result<Vec<WhatsappContact>, WhatsappError> {
        let v = self.get_json(&format!("/contacts?limit={limit}")).await?;
        serde_json::from_value(v).map_err(|e| WhatsappError::Parse(e.to_string()))
    }

    pub async fn list_chats(&self, limit: usize) -> Result<Vec<WhatsappChat>, WhatsappError> {
        let v = self.get_json(&format!("/chats?limit={limit}")).await?;
        serde_json::from_value(v).map_err(|e| WhatsappError::Parse(e.to_string()))
    }

    pub async fn chat_messages(
        &self,
        chat_id: &str,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<WhatsappMessage>, WhatsappError> {
        let mut path = format!("/chats/{}/messages?limit={limit}", urlenc(chat_id));
        if let Some(b) = before {
            path.push_str(&format!("&before={b}"));
        }
        let v = self.get_json(&path).await?;
        serde_json::from_value(v).map_err(|e| WhatsappError::Parse(e.to_string()))
    }

    /// Send exactly one text message (no bulk, no media in v1).
    pub async fn send_message(
        &self,
        target: WhatsappSendTarget,
        text: &str,
    ) -> Result<WhatsappMessageRef, WhatsappError> {
        let (body, chat_id_for_ref) = match &target {
            WhatsappSendTarget::ChatId(id) => (
                serde_json::json!({ "chat_id": id, "text": text }),
                id.clone(),
            ),
            WhatsappSendTarget::Phone(phone) => (
                serde_json::json!({ "phone": phone, "text": text }),
                phone.clone(),
            ),
        };
        let v = self.post_json("/send", body).await?;
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            return Err(WhatsappError::BridgeError(err.to_string()));
        }
        let message_id = v
            .get("message_id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        Ok(WhatsappMessageRef {
            message_id,
            chat_id: chat_id_for_ref,
        })
    }

    pub async fn logout(&self) -> Result<(), WhatsappError> {
        self.post_json("/logout", serde_json::json!({})).await?;
        Ok(())
    }
}

/// URL-encode characters that are significant in URL path segments.
/// Exposed as `urlenc_public` for tests.
pub fn urlenc_public(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '@' => "%40".to_string(),
            '/' => "%2F".to_string(),
            _ => c.to_string(),
        })
        .collect()
}

fn urlenc(s: &str) -> String {
    urlenc_public(s)
}
