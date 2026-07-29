use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{ser::SerializeStruct, Deserialize, Serialize, Serializer};
use std::{collections::BTreeMap, time::Duration};

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8082/v1";
pub const DEFAULT_API_KEY: &str = "basert-local";
pub const DEFAULT_CHAT_MODEL: &str = "basecompute/Qwen3-4B-Instruct-2507";

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ToolDefFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ToolDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolDefFunction,
}

impl ToolDef {
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

#[derive(Deserialize, Clone, Debug)]
pub struct ToolCallFunction {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub function: ToolCallFunction,
}

impl Serialize for ToolCall {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ToolCall", 3)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("type", "function")?;
        state.serialize_field(
            "function",
            &serde_json::json!({
                "name": self.function.name,
                "arguments": serde_json::to_string(&self.function.arguments)
                    .map_err(serde::ser::Error::custom)?,
            }),
        )?;
        state.end()
    }
}

#[derive(Debug, Clone)]
pub enum ChatStreamEvent {
    Delta(String),
    ToolCalls(Vec<ToolCall>),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
}

impl Message {
    fn plain(role: &str, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::plain("user", content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self::plain("assistant", content)
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self::plain("system", content)
    }

    pub fn assistant_tool_calls(calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: calls,
            tool_call_id: None,
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        _tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: Some(call_id.into()),
        }
    }
}

#[derive(Clone)]
pub struct BaseRtClient {
    base_url: String,
    server_root: String,
    api_key: String,
    http: reqwest::Client,
}

impl BaseRtClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let server_root = base_url
            .strip_suffix("/v1")
            .unwrap_or(&base_url)
            .to_string();
        Self {
            base_url,
            server_root,
            api_key: api_key.into(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(310))
                .build()
                .expect("build BaseRT HTTP client"),
        }
    }

    fn get(&self, url: String) -> reqwest::RequestBuilder {
        self.http.get(url).bearer_auth(&self.api_key)
    }

    fn post(&self, url: String) -> reqwest::RequestBuilder {
        self.http.post(url).bearer_auth(&self.api_key)
    }

    pub async fn is_up(&self) -> bool {
        self.get(format!("{}/health", self.server_root))
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    pub async fn models(&self) -> Result<Vec<String>> {
        let response = self
            .get(format!("{}/models", self.base_url))
            .send()
            .await
            .context("GET /v1/models")?;
        let value = response_json(response, "GET /v1/models").await?;
        let mut models = value["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| item["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        models.sort();
        models.dedup();
        Ok(models)
    }

    pub async fn unload_model(&self, model: &str) -> Result<()> {
        let response = self
            .post(format!("{}/models/unload", self.base_url))
            .json(&serde_json::json!({"model": model}))
            .send()
            .await
            .context("POST /v1/models/unload")?;
        response_ok(response, "POST /v1/models/unload").await
    }

    pub fn chat_stream(
        &self,
        model: String,
        messages: Vec<Message>,
    ) -> impl futures_core::Stream<Item = Result<String>> + Send {
        let stream = self.chat_stream_with_tools(model, messages, vec![]);
        async_stream::try_stream! {
            tokio::pin!(stream);
            while let Some(event) = stream.next().await {
                if let ChatStreamEvent::Delta(delta) = event? {
                    yield delta;
                }
            }
        }
    }

    pub fn chat_stream_with_tools(
        &self,
        model: String,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
    ) -> impl futures_core::Stream<Item = Result<ChatStreamEvent>> + Send {
        let client = self.clone();
        async_stream::try_stream! {
            let response = client
                .post(format!("{}/chat/completions", client.base_url))
                .json(&serde_json::json!({
                    "model": model,
                    "messages": messages,
                    "tools": tools,
                    "stream": true,
                    "temperature": 0.7,
                    "max_tokens": 2048,
                }))
                .send()
                .await
                .context("POST /v1/chat/completions")?;

            let response = if response.status().is_success() {
                response
            } else {
                Err(response_error(response, "POST /v1/chat/completions").await)?
            };

            let mut bytes = response.bytes_stream();
            let mut buffer = Vec::<u8>::new();
            let mut calls: BTreeMap<usize, PartialToolCall> = BTreeMap::new();
            let mut done = false;

            while let Some(chunk) = bytes.next().await {
                let chunk = chunk.context("BaseRT stream read")?;
                buffer.extend_from_slice(&chunk);

                while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
                    let line_bytes = buffer[..newline]
                        .strip_suffix(b"\r")
                        .unwrap_or(&buffer[..newline]);
                    let line = std::str::from_utf8(line_bytes)
                        .context("BaseRT SSE line is not valid UTF-8")?
                        .to_string();
                    buffer.drain(..=newline);
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let data = data.trim();
                    if data == "[DONE]" {
                        done = true;
                        break;
                    }
                    if data.is_empty() {
                        continue;
                    }
                    let value: serde_json::Value =
                        serde_json::from_str(data).context("parse BaseRT SSE event")?;
                    if let Some(error) = value.get("error") {
                        Err(anyhow!("BaseRT error: {}", error_message(error)))?;
                    }
                    let Some(delta) = value["choices"]
                        .as_array()
                        .and_then(|choices| choices.first())
                        .and_then(|choice| choice.get("delta"))
                    else {
                        continue;
                    };
                    if let Some(content) = delta["content"].as_str() {
                        if !content.is_empty() {
                            yield ChatStreamEvent::Delta(content.to_string());
                        }
                    }
                    if let Some(parts) = delta["tool_calls"].as_array() {
                        for part in parts {
                            let index = part["index"].as_u64().unwrap_or(0) as usize;
                            let pending = calls.entry(index).or_default();
                            if let Some(id) = part["id"].as_str() {
                                pending.id.push_str(id);
                            }
                            if let Some(name) = part["function"]["name"].as_str() {
                                pending.name.push_str(name);
                            }
                            if let Some(arguments) = part["function"]["arguments"].as_str() {
                                pending.arguments.push_str(arguments);
                            }
                        }
                    }
                }
                if done {
                    break;
                }
            }

            let completed = calls
                .into_values()
                .map(PartialToolCall::finish)
                .collect::<Result<Vec<_>>>()?;
            if !completed.is_empty() {
                yield ChatStreamEvent::ToolCalls(completed);
            }
        }
    }

    /// Execute a non-streamed chat completion with a caller-selected output
    /// bound. This is intended for paths that must validate the complete model
    /// response before making any of it visible.
    pub async fn chat_complete_bounded(
        &self,
        model: &str,
        messages: Vec<Message>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<String> {
        if max_tokens == 0 {
            return Err(anyhow!("max_tokens must be greater than zero"));
        }
        self.chat_complete_request(model, messages, temperature, max_tokens, None)
            .await
    }

    async fn chat_complete_request(
        &self,
        model: &str,
        messages: Vec<Message>,
        temperature: f32,
        max_tokens: u32,
        response_format: Option<serde_json::Value>,
    ) -> Result<String> {
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "temperature": temperature,
            "max_tokens": max_tokens,
        });
        if let Some(format) = response_format {
            body["response_format"] = format;
        }
        let response = self
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body)
            .send()
            .await
            .context("POST /v1/chat/completions")?;
        let value = response_json(response, "POST /v1/chat/completions").await?;
        Ok(value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string())
    }

    pub async fn generate_raw(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
    ) -> Result<String> {
        self.complete(model, prompt, temperature, None).await
    }

    pub async fn generate_json(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
    ) -> Result<String> {
        self.complete(
            model,
            prompt,
            temperature,
            Some(serde_json::json!({"type": "json_object"})),
        )
        .await
    }

    async fn complete(
        &self,
        model: &str,
        prompt: &str,
        temperature: f32,
        response_format: Option<serde_json::Value>,
    ) -> Result<String> {
        self.chat_complete_request(
            model,
            vec![Message::user(prompt)],
            temperature,
            2_048,
            response_format,
        )
        .await
    }

    pub async fn summarize(&self, model: &str, messages: &[Message]) -> Result<String> {
        let conversation = messages
            .iter()
            .map(|message| format!("[{}]: {}", message.role, message.content))
            .collect::<Vec<_>>()
            .join("\n");
        self.generate_raw(
            model,
            &format!(
                "Stručne zhrň nasledujúcu konverzáciu v 2–4 vetách. \
                 Zachovaj jazyk, kľúčové fakty, mená a čísla.\n\n{conversation}"
            ),
            0.0,
        )
        .await
    }
}

#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

impl PartialToolCall {
    fn finish(self) -> Result<ToolCall> {
        if self.id.is_empty() || self.name.is_empty() {
            return Err(anyhow!("BaseRT returned an incomplete tool call"));
        }
        let arguments = if self.arguments.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&self.arguments).with_context(|| {
                format!(
                    "parse arguments for BaseRT tool call {} ({})",
                    self.id, self.name
                )
            })?
        };
        Ok(ToolCall {
            id: self.id,
            function: ToolCallFunction {
                name: self.name,
                arguments,
            },
        })
    }
}

async fn response_ok(response: reqwest::Response, context: &str) -> Result<()> {
    if response.status().is_success() {
        Ok(())
    } else {
        Err(response_error(response, context).await)
    }
}

async fn response_json(response: reqwest::Response, context: &str) -> Result<serde_json::Value> {
    if !response.status().is_success() {
        return Err(response_error(response, context).await);
    }
    response
        .json()
        .await
        .with_context(|| format!("{context}: parse JSON response"))
}

async fn response_error(response: reqwest::Response, context: &str) -> anyhow::Error {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| value.get("error").map(error_message))
        .filter(|message| !message.is_empty())
        .unwrap_or(body);
    anyhow!("{context}: HTTP {status}: {message}")
}

fn error_message(error: &serde_json::Value) -> String {
    error["message"]
        .as_str()
        .or_else(|| error.as_str())
        .unwrap_or_default()
        .to_string()
}
