use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_EMBED_MODEL: &str = "bge-m3";

/// A single conversation turn for the Ollama chat API.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
    /// Base64-encoded image bytes for vision models (no data: URI prefix).
    /// Serialised only when non-empty so text-only models aren't affected.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub images: Vec<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into(), images: vec![] }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into(), images: vec![] }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into(), images: vec![] }
    }
    /// User message with base64-encoded images attached (for vision models).
    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self { role: "user".into(), content: content.into(), images }
    }
}

#[derive(Clone)]
pub struct OllamaClient {
    base_url: String,
    http: reqwest::Client,
}

impl OllamaClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
        }
    }

    /// Returns true if Ollama is reachable.
    pub async fn is_up(&self) -> bool {
        self.http
            .get(format!("{}/api/tags", self.base_url))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Sorted list of locally available model names.
    pub async fn models(&self) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct Resp { models: Vec<Info> }
        #[derive(Deserialize)]
        struct Info { name: String }

        let resp: Resp = self.http
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .context("GET /api/tags")?
            .json()
            .await
            .context("parse tags response")?;

        let mut names: Vec<String> = resp.models.into_iter().map(|m| m.name).collect();
        names.sort();
        Ok(names)
    }

    /// Streaming chat — yields decoded tokens as they arrive.
    pub fn chat_stream(
        &self,
        model: String,
        messages: Vec<Message>,
    ) -> impl futures_core::Stream<Item = Result<String>> + Send {
        let http = self.http.clone();
        let base_url = self.base_url.clone();

        async_stream::try_stream! {
            let resp = http
                .post(format!("{}/api/chat", base_url))
                .json(&serde_json::json!({
                    "model": model,
                    "messages": messages,
                    "stream": true,
                    "keep_alive": -1
                }))
                .send()
                .await
                .context("POST /api/chat")?;

            let mut stream = resp.bytes_stream();
            let mut buf = String::new();

            while let Some(item) = stream.next().await {
                let chunk = item.context("stream read error")?;
                buf.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim().to_string();
                    buf = buf[pos + 1..].to_string();
                    if line.is_empty() { continue; }

                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
                        if let Some(token) = v["message"]["content"].as_str() {
                            if !token.is_empty() {
                                yield token.to_string();
                            }
                        }
                        if v["done"].as_bool().unwrap_or(false) {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// Generate an embedding vector for `text`.
    pub async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>> {
        let resp: serde_json::Value = self.http
            .post(format!("{}/api/embeddings", self.base_url))
            .json(&serde_json::json!({ "model": model, "prompt": text, "keep_alive": -1 }))
            .send()
            .await
            .context("POST /api/embeddings")?
            .json()
            .await
            .context("parse embeddings response")?;

        let vec = resp["embedding"]
            .as_array()
            .context("no 'embedding' field in response")?
            .iter()
            .filter_map(|v| v.as_f64().map(|f| f as f32))
            .collect();
        Ok(vec)
    }

    /// Single non-streaming generation call — useful for classifiers and extractors.
    pub async fn generate_raw(&self, model: &str, prompt: &str, temperature: f32) -> Result<String> {
        let resp: serde_json::Value = self.http
            .post(format!("{}/api/chat", self.base_url))
            .json(&serde_json::json!({
                "model": model,
                "messages": [{ "role": "user", "content": prompt }],
                "stream": false,
                "keep_alive": -1,
                "options": { "temperature": temperature }
            }))
            .send()
            .await
            .context("generate_raw request")?
            .json()
            .await
            .context("parse generate_raw response")?;

        Ok(resp["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }

    /// Summarise a slice of messages into a few sentences.
    /// Preserves source language (Slovak or English).
    pub async fn summarize(&self, model: &str, messages: &[Message]) -> Result<String> {
        let conversation = messages
            .iter()
            .map(|m| format!("[{}]: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Stručne zhrň nasledujúcu konverzáciu v 2–4 vetách. \
             Zachovaj jazyk, kľúčové fakty, mená a čísla.\n\n{conversation}"
        );

        let resp: serde_json::Value = self.http
            .post(format!("{}/api/chat", self.base_url))
            .json(&serde_json::json!({
                "model": model,
                "messages": [{ "role": "user", "content": prompt }],
                "stream": false,
                "keep_alive": -1
            }))
            .send()
            .await
            .context("summarize request")?
            .json()
            .await
            .context("parse summarize response")?;

        Ok(resp["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string())
    }
}
