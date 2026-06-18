use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";
pub const DEFAULT_EMBED_MODEL: &str = "bge-m3";

// ── Tool-calling types ────────────────────────────────────────────────────────

/// A function definition inside a tool.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ToolDefFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A tool definition passed in the `tools` array of a chat request.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolDefFunction,
}

impl ToolDef {
    /// Build a `function` type tool definition.
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            kind: "function".into(),
            function: ToolDefFunction {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// A single function call emitted by the model.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ToolCallFunction {
    pub name: String,
    /// Arguments as a JSON value (may be an object or a pre-serialised string).
    pub arguments: serde_json::Value,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ToolCall {
    pub function: ToolCallFunction,
}

/// The result of a single non-streaming `chat_once_with_tools` call.
#[derive(Debug)]
pub enum ChatTurn {
    /// The model issued one or more tool calls — execute them and continue the loop.
    ToolCalls(Vec<ToolCall>),
    /// The model produced a text answer — the agentic loop is done.
    Content(String),
}

// ── Message ───────────────────────────────────────────────────────────────────

/// A single conversation turn for the Ollama chat API.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
    /// Base64-encoded image bytes for vision models (no data: URI prefix).
    /// Serialised only when non-empty so text-only models aren't affected.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub images: Vec<String>,
    /// Tool calls emitted by an assistant turn (skip when empty).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
    /// Tool name for `role: "tool"` result messages (skip when absent).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            images: vec![],
            tool_calls: vec![],
            name: None,
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            images: vec![],
            tool_calls: vec![],
            name: None,
        }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            images: vec![],
            tool_calls: vec![],
            name: None,
        }
    }
    /// User message with base64-encoded images attached (for vision models).
    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            images,
            tool_calls: vec![],
            name: None,
        }
    }
    /// Assistant message carrying tool calls (no text content).
    pub fn assistant_tool_calls(calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: String::new(),
            images: vec![],
            tool_calls: calls,
            name: None,
        }
    }
    /// Tool result message fed back after executing a tool call.
    pub fn tool_result(tool_name: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            images: vec![],
            tool_calls: vec![],
            name: Some(tool_name.into()),
        }
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
        struct Resp {
            models: Vec<Info>,
        }
        #[derive(Deserialize)]
        struct Info {
            name: String,
        }

        let resp: Resp = self
            .http
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
        let resp: serde_json::Value = self
            .http
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
    pub async fn generate_raw(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
    ) -> Result<String> {
        let resp: serde_json::Value = self
            .http
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

    /// Like `generate_raw` but requests guaranteed JSON output via `"format": "json"`.
    /// Use this for all classifier / extractor calls where the response must be parsed.
    pub async fn generate_json(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
    ) -> Result<String> {
        let resp: serde_json::Value = self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .json(&serde_json::json!({
                "model": model,
                "messages": [{ "role": "user", "content": prompt }],
                "stream": false,
                "keep_alive": -1,
                "format": "json",
                "options": { "temperature": temperature }
            }))
            .send()
            .await
            .context("generate_json request")?
            .json()
            .await
            .context("parse generate_json response")?;

        Ok(resp["message"]["content"]
            .as_str()
            .unwrap_or("{}")
            .to_string())
    }

    /// Like `generate_json`, but passes a concrete JSON schema as Ollama's
    /// `format` parameter so classifier responses are constrained at decode time.
    pub async fn generate_json_schema(
        &self,
        model: &str,
        prompt: &str,
        schema: serde_json::Value,
        temperature: f32,
    ) -> Result<String> {
        let resp: serde_json::Value = self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .json(&serde_json::json!({
                "model": model,
                "messages": [{ "role": "user", "content": prompt }],
                "stream": false,
                "keep_alive": -1,
                "format": schema,
                "options": { "temperature": temperature }
            }))
            .send()
            .await
            .context("generate_json_schema request")?
            .json()
            .await
            .context("parse generate_json_schema response")?;

        Ok(resp["message"]["content"]
            .as_str()
            .unwrap_or("{}")
            .to_string())
    }

    /// Single non-streaming chat call with tool definitions.
    ///
    /// Returns `ChatTurn::ToolCalls` when the model emits one or more tool calls,
    /// or `ChatTurn::Content` when it produces a plain text answer.
    /// Use this for the tool-calling loop; do a final `chat_stream` for the streamed answer.
    pub async fn chat_once_with_tools(
        &self,
        model: String,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
    ) -> Result<ChatTurn> {
        let resp: serde_json::Value = self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .json(&serde_json::json!({
                "model": model,
                "messages": messages,
                "tools": tools,
                "stream": false,
                "keep_alive": -1
            }))
            .send()
            .await
            .context("chat_once_with_tools request")?
            .json()
            .await
            .context("parse chat_once_with_tools response")?;

        // Check for tool calls first
        if let Some(calls_val) = resp["message"]["tool_calls"].as_array() {
            if !calls_val.is_empty() {
                let calls: Vec<ToolCall> = calls_val
                    .iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect();
                if !calls.is_empty() {
                    return Ok(ChatTurn::ToolCalls(calls));
                }
            }
        }

        // Otherwise treat as content
        let content = resp["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        Ok(ChatTurn::Content(content))
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

        let resp: serde_json::Value = self
            .http
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
