use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{ser::SerializeStruct, Deserialize, Serialize, Serializer};
use std::{
    collections::BTreeMap,
    fmt,
    path::PathBuf,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8082/v1";
pub const DEFAULT_API_KEY: &str = "basert-local";
pub const DEFAULT_CHAT_MODEL: &str = "basecompute/Qwen3-4B-Instruct-2507";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseRtRuntimeFault {
    MetalOutOfMemory,
    MetalDevice,
    MetalCommandBuffer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseRtCompletionError {
    RuntimeFault(BaseRtRuntimeFault),
    Truncated,
    Empty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaseRtLogCheckpoint {
    length: u64,
    file_id: Option<u64>,
}

impl fmt::Display for BaseRtCompletionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeFault(fault) => {
                write!(formatter, "BaseRT runtime poisoned: {}", fault.category())
            }
            Self::Truncated => formatter.write_str("BaseRT generation truncated at output cap"),
            Self::Empty => formatter.write_str("BaseRT returned an empty completion"),
        }
    }
}

impl std::error::Error for BaseRtCompletionError {}

impl BaseRtRuntimeFault {
    pub fn category(self) -> &'static str {
        match self {
            Self::MetalOutOfMemory => "metal_oom",
            Self::MetalDevice => "metal_device",
            Self::MetalCommandBuffer => "metal_command_buffer",
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::MetalOutOfMemory => 1,
            Self::MetalDevice => 2,
            Self::MetalCommandBuffer => 3,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::MetalOutOfMemory),
            2 => Some(Self::MetalDevice),
            3 => Some(Self::MetalCommandBuffer),
            _ => None,
        }
    }
}

pub fn classify_basert_runtime_fault(log_delta: &str) -> Option<BaseRtRuntimeFault> {
    let normalized = log_delta.to_ascii_lowercase();
    if normalized.contains("kiogpucommandbuffercallbackerroroutofmemory")
        || (normalized.contains("[basert][metal]")
            && (normalized.contains("insufficient memory") || normalized.contains("out of memory")))
    {
        Some(BaseRtRuntimeFault::MetalOutOfMemory)
    } else if normalized.contains("[basert][metal]")
        && (normalized.contains("device lost")
            || normalized.contains("device was lost")
            || normalized.contains("device removed"))
    {
        Some(BaseRtRuntimeFault::MetalDevice)
    } else if normalized.contains("[basert][metal]") && normalized.contains("command buffer failed")
    {
        Some(BaseRtRuntimeFault::MetalCommandBuffer)
    } else {
        None
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub loaded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelLoadRequest {
    pub id: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelReadiness {
    pub id: String,
    pub known: bool,
    pub loaded: bool,
}

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
    runtime_log_path: Option<PathBuf>,
    runtime_fault: Arc<AtomicU8>,
}

impl BaseRtClient {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let server_root = base_url
            .strip_suffix("/v1")
            .unwrap_or(&base_url)
            .to_string();
        let runtime_log_path = (base_url == DEFAULT_BASE_URL)
            .then(|| std::env::var_os("BAGENT_BASERT_LOG_PATH").map(PathBuf::from))
            .flatten();
        Self {
            base_url,
            server_root,
            api_key: api_key.into(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(310))
                .build()
                .expect("build BaseRT HTTP client"),
            runtime_log_path,
            runtime_fault: Arc::new(AtomicU8::new(0)),
        }
    }

    /// Observe an operational log owned by the configured BaseRT runtime.
    /// Only bytes appended after a completion begins are inspected.
    pub fn with_runtime_log_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.runtime_log_path = Some(path.into());
        self
    }

    pub fn runtime_fault(&self) -> Option<BaseRtRuntimeFault> {
        BaseRtRuntimeFault::from_code(self.runtime_fault.load(Ordering::SeqCst))
    }

    pub async fn runtime_log_checkpoint(&self) -> Option<BaseRtLogCheckpoint> {
        let path = self.runtime_log_path.as_ref()?;
        let metadata = tokio::fs::metadata(path).await.ok();
        #[cfg(unix)]
        let file_id = metadata.as_ref().map(std::os::unix::fs::MetadataExt::ino);
        #[cfg(not(unix))]
        let file_id = None;
        Some(BaseRtLogCheckpoint {
            length: metadata.as_ref().map(|value| value.len()).unwrap_or(0),
            file_id,
        })
    }

    async fn runtime_fault_since(
        &self,
        checkpoint: Option<BaseRtLogCheckpoint>,
    ) -> Option<BaseRtRuntimeFault> {
        let (path, checkpoint) = (self.runtime_log_path.as_ref()?, checkpoint?);
        let mut file = tokio::fs::File::open(path).await.ok()?;
        let metadata = file.metadata().await.ok()?;
        let length = metadata.len();
        #[cfg(unix)]
        let current_file_id = Some(std::os::unix::fs::MetadataExt::ino(&metadata));
        #[cfg(not(unix))]
        let current_file_id = None;
        let same_file = checkpoint.file_id == current_file_id;
        if same_file && length == checkpoint.length {
            return None;
        }
        // Read the newest bounded window. If the file rotated or truncated,
        // start from the new file instead of treating the state as clean.
        const MAX_FAULT_SCAN_BYTES: u64 = 256 * 1024;
        let appended_start = if !same_file || length < checkpoint.length {
            0
        } else {
            checkpoint.length
        };
        let start = appended_start.max(length.saturating_sub(MAX_FAULT_SCAN_BYTES));
        file.seek(std::io::SeekFrom::Start(start)).await.ok()?;
        let mut delta = Vec::with_capacity((length - start) as usize);
        file.take(MAX_FAULT_SCAN_BYTES)
            .read_to_end(&mut delta)
            .await
            .ok()?;
        classify_basert_runtime_fault(&String::from_utf8_lossy(&delta))
    }

    pub async fn detect_runtime_fault_since(
        &self,
        checkpoint: Option<BaseRtLogCheckpoint>,
        wait: Duration,
    ) -> Option<BaseRtRuntimeFault> {
        checkpoint?;
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(fault) = self.runtime_fault_since(checkpoint).await {
                self.runtime_fault.store(fault.code(), Ordering::SeqCst);
                return Some(fault);
            }
            if tokio::time::Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.base_url
    }

    pub fn clear_runtime_fault(&self) {
        self.runtime_fault.store(0, Ordering::SeqCst);
    }

    async fn completion_fault_error_since(
        &self,
        checkpoint: Option<BaseRtLogCheckpoint>,
        wait_for_log_flush: bool,
    ) -> Option<anyhow::Error> {
        let wait = if wait_for_log_flush {
            Duration::from_millis(250)
        } else {
            Duration::ZERO
        };
        self.detect_runtime_fault_since(checkpoint, wait)
            .await
            .map(|fault| anyhow!(BaseRtCompletionError::RuntimeFault(fault)))
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
        Ok(self
            .inspect_models()
            .await?
            .into_iter()
            .map(|model| model.id)
            .collect())
    }

    pub async fn inspect_models(&self) -> Result<Vec<ModelInfo>> {
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
            .filter_map(|item| {
                item["id"].as_str().map(|id| ModelInfo {
                    id: id.to_owned(),
                    loaded: item["loaded"].as_bool().unwrap_or(true),
                })
            })
            .collect::<Vec<_>>();
        models.sort_by(|left, right| left.id.cmp(&right.id));
        models.dedup_by(|left, right| left.id == right.id);
        Ok(models)
    }

    pub async fn model_readiness(&self, model: &str) -> Result<ModelReadiness> {
        let models = self.inspect_models().await?;
        let matching = models.iter().find(|candidate| candidate.id == model);
        Ok(ModelReadiness {
            id: model.to_string(),
            known: matching.is_some(),
            loaded: matching.is_some_and(|candidate| candidate.loaded),
        })
    }

    pub async fn load_model(&self, request: &ModelLoadRequest) -> Result<ModelReadiness> {
        if request.id.trim().is_empty() {
            return Err(anyhow!("model id must not be empty"));
        }
        if request.path.trim().is_empty() {
            return Err(anyhow!("model path must not be empty"));
        }
        let log_cursor = self.runtime_log_checkpoint().await;
        let response = match self
            .post(format!("{}/models/load", self.base_url))
            .json(&serde_json::json!({"path": request.path}))
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                if let Some(fault) = self.completion_fault_error_since(log_cursor, true).await {
                    return Err(fault);
                }
                return Err(error).context("POST /v1/models/load");
            }
        };
        if let Err(error) = response_ok(response, "POST /v1/models/load").await {
            if let Some(fault) = self.completion_fault_error_since(log_cursor, true).await {
                return Err(fault);
            }
            return Err(error);
        }
        if let Some(fault) = self.completion_fault_error_since(log_cursor, true).await {
            return Err(fault);
        }
        self.model_readiness(&request.id).await
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
                    // 0.7 made the 4B chat model narrate tool use ("I will
                    // search ...") instead of emitting the call, sometimes
                    // degenerating into repetition loops. Lower temperature
                    // keeps agentic rounds deterministic enough to act.
                    "temperature": 0.4,
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

            if !done {
                Err(anyhow!("BaseRT stream ended without a completion boundary"))?;
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

    pub async fn chat_complete_json_bounded(
        &self,
        model: &str,
        messages: Vec<Message>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<String> {
        if max_tokens == 0 {
            return Err(anyhow!("max_tokens must be greater than zero"));
        }
        self.chat_complete_request(
            model,
            messages,
            temperature,
            max_tokens,
            Some(serde_json::json!({"type": "json_object"})),
        )
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
        if let Some(fault) = self.runtime_fault() {
            return Err(anyhow!(BaseRtCompletionError::RuntimeFault(fault)));
        }
        let log_cursor = self.runtime_log_checkpoint().await;
        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "temperature": temperature,
            "max_tokens": max_tokens,
            "chat_template_kwargs": {
                "enable_thinking": false
            },
        });
        if let Some(format) = response_format {
            body["response_format"] = format;
        }
        let response = match self
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(error) => {
                if let Some(fault) = self.completion_fault_error_since(log_cursor, true).await {
                    return Err(fault);
                }
                return Err(error).context("POST /v1/chat/completions");
            }
        };
        let value = match response_json(response, "POST /v1/chat/completions").await {
            Ok(value) => value,
            Err(error) => {
                if let Some(fault) = self.completion_fault_error_since(log_cursor, true).await {
                    return Err(fault);
                }
                return Err(error);
            }
        };
        let content = value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if let Some(fault) = self
            .detect_runtime_fault_since(
                log_cursor,
                if content.chars().count() <= 3 {
                    Duration::from_millis(250)
                } else {
                    Duration::ZERO
                },
            )
            .await
        {
            return Err(anyhow!(BaseRtCompletionError::RuntimeFault(fault)));
        }
        if value["choices"][0]["finish_reason"].as_str() == Some("length") {
            return Err(anyhow!(BaseRtCompletionError::Truncated));
        }
        if content.trim().is_empty() {
            return Err(anyhow!(BaseRtCompletionError::Empty));
        }
        Ok(content)
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
