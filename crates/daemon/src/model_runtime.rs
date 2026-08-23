use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

pub const PRELOAD_ON_INPUT_POLICY: bool = false;
pub const SHARED_IDLE_TIMEOUT_SECONDS: u64 = 20 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelRuntimePolicy {
    pub preload_on_input: bool,
    pub shared_idle_timeout_seconds: u64,
}

impl Default for ModelRuntimePolicy {
    fn default() -> Self {
        Self {
            preload_on_input: PRELOAD_ON_INPUT_POLICY,
            shared_idle_timeout_seconds: SHARED_IDLE_TIMEOUT_SECONDS,
        }
    }
}

fn production_policy() -> ModelRuntimePolicy {
    let policy = ModelRuntimePolicy::default();
    #[cfg(feature = "stage8-acceptance")]
    {
        return acceptance_policy_override(
            policy,
            std::env::var("BAGENT_STAGE8_ACCEPTANCE_FIXTURES").as_deref() == Ok("1"),
            std::env::var("BAGENT_STAGE8_IDLE_TIMEOUT_SECONDS")
                .ok()
                .as_deref(),
        );
    }
    #[allow(unreachable_code)]
    policy
}

#[cfg(feature = "stage8-acceptance")]
fn acceptance_policy_override(
    policy: ModelRuntimePolicy,
    fixtures_enabled: bool,
    timeout: Option<&str>,
) -> ModelRuntimePolicy {
    let Some(seconds) = fixtures_enabled
        .then(|| timeout?.parse::<u64>().ok())
        .flatten()
        .filter(|seconds| (1..=60).contains(seconds))
    else {
        return policy;
    };
    ModelRuntimePolicy {
        shared_idle_timeout_seconds: seconds,
        ..policy
    }
}

pub use crate::work_coordinator::WorkIdentity;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bagent_agent::{AgentInference, InferenceFuture};
use basert_connector::{
    BaseRtClient, BaseRtCompletionError, BaseRtRuntimeFault, ChatStreamEvent, Message, ModelInfo,
    ModelLoadRequest, ToolDef, DEFAULT_BASE_URL,
};
use futures_util::{stream::BoxStream, Stream, StreamExt};
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelClass {
    Chat4B,
    Synthesis35B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePhase {
    Unavailable,
    Unloaded,
    Loading(ModelClass),
    LoadedNotReady(ModelClass),
    Ready(ModelClass),
    Retiring(ModelClass),
    Poisoned(ModelClass),
    Restarting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFault {
    Metal,
    Device,
    CommandBuffer,
    IndeterminateTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DemandPriority {
    Speculative,
    Automation,
    Foreground,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDemand {
    work: Option<WorkIdentity>,
    model: ModelClass,
    priority: DemandPriority,
}

impl ModelDemand {
    pub fn speculative(model: ModelClass) -> Self {
        Self {
            work: None,
            model,
            priority: DemandPriority::Speculative,
        }
    }

    pub fn automation(work: WorkIdentity, model: ModelClass) -> Self {
        Self {
            work: Some(work),
            model,
            priority: DemandPriority::Automation,
        }
    }

    pub fn foreground(work: WorkIdentity, model: ModelClass) -> Self {
        Self {
            work: Some(work),
            model,
            priority: DemandPriority::Foreground,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkDemandOrigin {
    Foreground,
    Automation,
}

#[derive(Clone)]
pub struct CoordinatedModelRuntime {
    runtime: Arc<ModelRuntime>,
    origin: WorkDemandOrigin,
    work: WorkIdentity,
}

impl CoordinatedModelRuntime {
    pub fn new(runtime: Arc<ModelRuntime>, origin: WorkDemandOrigin, work: WorkIdentity) -> Self {
        Self {
            runtime,
            origin,
            work,
        }
    }

    pub fn demand(&self, model: ModelClass) -> ModelDemand {
        match self.origin {
            WorkDemandOrigin::Foreground => ModelDemand::foreground(self.work.clone(), model),
            WorkDemandOrigin::Automation => ModelDemand::automation(self.work.clone(), model),
        }
    }

    pub async fn stream(
        &self,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
    ) -> Result<RuntimeCompletionStream> {
        self.runtime
            .stream_completion(self.demand(ModelClass::Chat4B), messages, tools)
            .await
    }

    pub async fn complete(
        &self,
        model: ModelClass,
        messages: Vec<Message>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<String> {
        self.runtime
            .complete_bounded(self.demand(model), messages, temperature, max_tokens)
            .await
    }
}

impl AgentInference for CoordinatedModelRuntime {
    fn infer_raw<'a>(
        &'a self,
        _model: &'a str,
        prompt: &'a str,
        temperature: f32,
    ) -> InferenceFuture<'a> {
        Box::pin(async move {
            self.complete(
                ModelClass::Chat4B,
                vec![Message::user(prompt)],
                temperature,
                2_048,
            )
            .await
        })
    }

    fn infer_json<'a>(
        &'a self,
        model: &'a str,
        prompt: &'a str,
        temperature: f32,
    ) -> InferenceFuture<'a> {
        let _ = model;
        Box::pin(async move {
            self.runtime
                .complete_json(
                    self.demand(ModelClass::Chat4B),
                    vec![Message::user(prompt)],
                    temperature,
                    2_048,
                )
                .await
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeAction {
    EnsureService,
    Load(ModelClass),
    VerifyReady(ModelClass),
    Unload(ModelClass),
    Restart,
    VerifyHealthyChangedPid,
    VerifyZeroLoadedWeights,
}

#[async_trait]
pub trait ModelRuntimeAdapter: Send + Sync {
    fn recorded_actions(&self) -> Vec<RuntimeAction> {
        Vec::new()
    }

    async fn perform(&self, _action: RuntimeAction) -> Result<()> {
        Ok(())
    }

    async fn health(&self) -> bool {
        false
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    async fn memory_headroom(&self) -> Result<(u64, u64)> {
        Ok((100, u64::MAX))
    }

    fn stream_completion(
        &self,
        _model: ModelClass,
        _messages: Vec<Message>,
        _tools: Vec<ToolDef>,
    ) -> BoxStream<'static, Result<ChatStreamEvent>> {
        futures_util::stream::once(async { Err(anyhow!("completion unsupported")) }).boxed()
    }

    async fn complete_bounded(
        &self,
        _model: ModelClass,
        _messages: Vec<Message>,
        _temperature: f32,
        _max_tokens: u32,
        _format: CompletionFormat,
    ) -> Result<String> {
        Err(anyhow!("completion unsupported"))
    }
}

#[derive(Debug, Clone)]
pub struct ProductionModelConfig {
    pub chat_model: String,
    pub chat_path: std::path::PathBuf,
    pub synthesis_model: String,
    pub synthesis_path: std::path::PathBuf,
    service: ManagedServiceConfig,
}

#[derive(Debug, Clone)]
struct ManagedServiceConfig {
    label: String,
    plist_path: PathBuf,
    log_path: PathBuf,
    model_registry: PathBuf,
    binary_candidates: Vec<PathBuf>,
    api_key: String,
}

impl ProductionModelConfig {
    pub fn from_environment(
        chat_model: impl Into<String>,
        synthesis_model: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        let chat_model = chat_model.into();
        let synthesis_model = synthesis_model.into();
        let home = dirs::home_dir().unwrap_or_default();
        Self {
            chat_path: configured_model_path("BAGENT_CHAT_MODEL_PATH", &chat_model),
            synthesis_path: configured_model_path("BAGENT_SYNTHESIS_MODEL_PATH", &synthesis_model),
            chat_model,
            synthesis_model,
            service: ManagedServiceConfig {
                label: "com.bagent.basert".into(),
                plist_path: home.join("Library/LaunchAgents/com.bagent.basert.plist"),
                log_path: home.join("Library/Logs/bagent/basert.log"),
                model_registry: home.join("Library/Application Support/bagent/basert-models"),
                binary_candidates: vec![
                    home.join(".basert/basert"),
                    home.join(".local/bin/basert"),
                    PathBuf::from("/opt/homebrew/bin/basert"),
                    PathBuf::from("/usr/local/bin/basert"),
                ],
                api_key: api_key.into(),
            },
        }
    }

    fn model(&self, class: ModelClass) -> (&str, &std::path::Path) {
        match class {
            ModelClass::Chat4B => (&self.chat_model, &self.chat_path),
            ModelClass::Synthesis35B => (&self.synthesis_model, &self.synthesis_path),
        }
    }
}

fn configured_model_path(variable: &str, model: &str) -> std::path::PathBuf {
    std::env::var_os(variable)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_default()
                .join("Library/Caches/baseRT/models")
                .join(model)
                .join("default-q4/model.base")
        })
}

fn registered_model_directory(
    registry: &Path,
    model_id: &str,
    package_root: &Path,
) -> Result<PathBuf> {
    let mut destination = registry.to_path_buf();
    for component in Path::new(model_id).components() {
        match component {
            std::path::Component::Normal(component) => destination.push(component),
            _ => return Err(anyhow!("model id is not a safe relative registry path")),
        }
    }
    let variant = package_root
        .file_name()
        .ok_or_else(|| anyhow!("model package has no variant directory"))?;
    destination.push(variant);
    Ok(destination)
}

pub struct ProductionBaseRtAdapter {
    client: BaseRtClient,
    config: ProductionModelConfig,
    restart_pids: AsyncMutex<Option<(u32, u32)>>,
    chat_preload_available: AsyncMutex<Option<u64>>,
    externally_managed: bool,
}

impl ProductionBaseRtAdapter {
    pub fn new(client: BaseRtClient, config: ProductionModelConfig) -> Arc<Self> {
        Arc::new(Self {
            client,
            config,
            restart_pids: AsyncMutex::new(None),
            chat_preload_available: AsyncMutex::new(None),
            externally_managed: false,
        })
    }

    #[cfg(feature = "stage7a-acceptance")]
    pub fn new_external_fixture(client: BaseRtClient, config: ProductionModelConfig) -> Arc<Self> {
        Arc::new(Self {
            client,
            config,
            restart_pids: AsyncMutex::new(None),
            chat_preload_available: AsyncMutex::new(None),
            externally_managed: true,
        })
    }

    async fn managed_pid() -> Option<u32> {
        let uid = tokio::process::Command::new("/usr/bin/id")
            .arg("-u")
            .output()
            .await
            .ok()?;
        let uid = String::from_utf8_lossy(&uid.stdout).trim().to_string();
        let target = format!("gui/{uid}/com.bagent.basert");
        let output = tokio::process::Command::new("/bin/launchctl")
            .args(["print", &target])
            .output()
            .await
            .ok()?;
        output.status.success().then_some(())?;
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .find_map(|line| line.trim().strip_prefix("pid = "))
            .and_then(|value| value.parse().ok())
    }

    async fn pid_owns_listener(pid: u32, port: u16) -> Result<bool> {
        let output = tokio::process::Command::new("/usr/sbin/lsof")
            .args([
                "-nP".to_string(),
                "-a".to_string(),
                "-p".to_string(),
                pid.to_string(),
                format!("-iTCP@127.0.0.1:{port}"),
                "-sTCP:LISTEN".to_string(),
                "-t".to_string(),
            ])
            .output()
            .await
            .context("inspect BaseRT listener ownership")?;
        if !output.status.success() {
            return Ok(false);
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim() == pid.to_string()))
    }

    async fn wait_for_owned_listener(pid: u32, port: u16, timeout: Duration) -> Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if Self::pid_owns_listener(pid, port).await? {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("managed BaseRT PID did not acquire its listener"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn ensure_managed_service(&self) -> Result<()> {
        if self.client.endpoint() != DEFAULT_BASE_URL {
            return Err(anyhow!("managed BaseRT service is restricted to port 8082"));
        }
        self.prepare_model_registry()?;
        let binary = self
            .config
            .service
            .binary_candidates
            .iter()
            .find(|path| path.is_file())
            .ok_or_else(|| anyhow!("BaseRT binary not found"))?;
        if let Some(parent) = self.config.service.plist_path.parent() {
            fs::create_dir_all(parent).context("create BaseRT LaunchAgent directory")?;
        }
        if let Some(parent) = self.config.service.log_path.parent() {
            fs::create_dir_all(parent).context("create BaseRT log directory")?;
        }
        let desired_plist = self.service_plist(binary);
        let configuration_changed = fs::read_to_string(&self.config.service.plist_path)
            .map_or(true, |current| current != desired_plist);
        let staged_plist = self
            .config
            .service
            .plist_path
            .with_extension("plist.stage3.tmp");
        fs::write(&staged_plist, &desired_plist).context("stage BaseRT LaunchAgent")?;
        fs::rename(&staged_plist, &self.config.service.plist_path)
            .context("atomically install BaseRT LaunchAgent")?;
        let uid = Self::user_id().await?;
        if !configuration_changed && self.client.is_up().await {
            if let Some(pid) = Self::managed_pid().await {
                if Self::pid_owns_listener(pid, 8082).await? {
                    return Ok(());
                }
            }
        }
        let target = format!("gui/{uid}/{}", self.config.service.label);
        let previous = Self::managed_pid().await;
        let _ = tokio::process::Command::new("/bin/launchctl")
            .args(["bootout", &target])
            .output()
            .await;
        if let Some(previous) = previous {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            while Self::managed_pid().await == Some(previous) {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow!("legacy BaseRT process did not exit during cutover"));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        let output = tokio::process::Command::new("/bin/launchctl")
            .args([
                "bootstrap",
                &format!("gui/{uid}"),
                self.config.service.plist_path.to_string_lossy().as_ref(),
            ])
            .output()
            .await
            .context("bootstrap managed BaseRT")?;
        if !output.status.success() {
            let kickstart = tokio::process::Command::new("/bin/launchctl")
                .args(["kickstart", "-k", &target])
                .output()
                .await
                .context("kickstart registered BaseRT")?;
            if !kickstart.status.success() {
                return Err(anyhow!("bootstrap or kickstart managed BaseRT failed"));
            }
        }
        self.wait_for_health().await?;
        let managed_pid = Self::managed_pid()
            .await
            .ok_or_else(|| anyhow!("managed BaseRT is healthy without an observable PID"))?;
        Self::wait_for_owned_listener(managed_pid, 8082, Duration::from_secs(10)).await?;
        if !self.client.is_up().await {
            return Err(anyhow!("managed BaseRT listener is not healthy"));
        }
        Ok(())
    }

    async fn user_id() -> Result<String> {
        let output = tokio::process::Command::new("/usr/bin/id")
            .arg("-u")
            .output()
            .await
            .context("resolve user id for BaseRT")?;
        if !output.status.success() {
            return Err(anyhow!("resolve user id for BaseRT failed"));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn prepare_model_registry(&self) -> Result<()> {
        fs::create_dir_all(&self.config.service.model_registry)
            .context("create BaseRT model registry")?;
        for (model_id, source) in [
            self.config.model(ModelClass::Chat4B),
            self.config.model(ModelClass::Synthesis35B),
        ] {
            let Some(package_root) = source.parent() else {
                continue;
            };
            let destination = registered_model_directory(
                &self.config.service.model_registry,
                model_id,
                package_root,
            )?;
            if package_root.starts_with(&self.config.service.model_registry) {
                return Err(anyhow!(
                    "BaseRT model source must be outside the managed registry"
                ));
            }
            fs::create_dir_all(&destination).context("create registered model directory")?;
            for name in ["model.base", "hub.json"] {
                let source = package_root.join(name);
                if !source.exists() {
                    continue;
                }
                let link = destination.join(name);
                if fs::read_link(&link).ok().as_deref() == Some(source.as_path()) {
                    continue;
                }
                if link.symlink_metadata().is_ok() {
                    fs::remove_file(&link).context("replace registered model link")?;
                }
                std::os::unix::fs::symlink(&source, &link)
                    .context("link registered BaseRT model")?;
            }
        }
        Ok(())
    }

    fn service_plist(&self, binary: &Path) -> String {
        let escape = |value: &str| {
            value
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;")
        };
        let args = [
            binary.to_string_lossy().into_owned(),
            "serve".into(),
            "--model-dir".into(),
            self.config
                .service
                .model_registry
                .to_string_lossy()
                .into_owned(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            "8082".into(),
            "--api-key".into(),
            self.config.service.api_key.clone(),
            "--idle-timeout".into(),
            "0".into(),
            "--max-context".into(),
            "4096".into(),
            "--kv-bits".into(),
            "4".into(),
            "--max-tokens".into(),
            "2048".into(),
            "--max-batch-size".into(),
            "1".into(),
            "--request-timeout".into(),
            "300000".into(),
            "--verbose".into(),
        ];
        let arguments = args
            .iter()
            .map(|value| format!("            <string>{}</string>", escape(value)))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n    <key>Label</key><string>{}</string>\n    <key>ProgramArguments</key><array>\n{}\n    </array>\n    <key>RunAtLoad</key><true/>\n    <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n    <key>StandardOutPath</key><string>{}</string>\n    <key>StandardErrorPath</key><string>{}</string>\n</dict>\n</plist>\n",
            escape(&self.config.service.label),
            arguments,
            escape(&self.config.service.log_path.to_string_lossy()),
            escape(&self.config.service.log_path.to_string_lossy()),
        )
    }

    async fn wait_for_health(&self) -> Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if self.client.is_up().await {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("managed BaseRT did not become healthy"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn restart_managed(&self) -> Result<()> {
        if self.client.endpoint() != DEFAULT_BASE_URL {
            return Err(anyhow!(
                "BaseRT restart is restricted to the managed port-8082 endpoint"
            ));
        }
        let previous = Self::managed_pid()
            .await
            .ok_or_else(|| anyhow!("managed BaseRT PID is not observable before restart"))?;
        if !Self::pid_owns_listener(previous, 8082).await? {
            return Err(anyhow!(
                "managed BaseRT PID does not own port 8082 before restart"
            ));
        }
        let uid = Self::user_id().await?;
        let target = format!("gui/{uid}/com.bagent.basert");
        let output = tokio::process::Command::new("/bin/launchctl")
            .args(["kickstart", "-k", &target])
            .output()
            .await
            .context("restart managed BaseRT")?;
        if !output.status.success() {
            return Err(anyhow!("restart managed BaseRT failed"));
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(current) = Self::managed_pid().await {
                if current != previous {
                    *self.restart_pids.lock().await = Some((previous, current));
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(anyhow!("managed BaseRT PID did not change"));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[async_trait]
impl ModelRuntimeAdapter for ProductionBaseRtAdapter {
    async fn perform(&self, action: RuntimeAction) -> Result<()> {
        match action {
            RuntimeAction::Load(class) => {
                let (id, path) = self.config.model(class);
                if class == ModelClass::Chat4B {
                    *self.chat_preload_available.lock().await = Some(measured_memory_headroom()?.1);
                }
                self.client
                    .load_model(&ModelLoadRequest {
                        id: id.to_string(),
                        path: path.to_string_lossy().into_owned(),
                    })
                    .await?;
            }
            RuntimeAction::EnsureService if self.externally_managed => {
                if !self.client.is_up().await {
                    return Err(anyhow!("external BaseRT fixture is unavailable"));
                }
            }
            RuntimeAction::EnsureService => self.ensure_managed_service().await?,
            RuntimeAction::VerifyReady(class) => {
                let (id, _) = self.config.model(class);
                let readiness = self.client.model_readiness(id).await?;
                if !readiness.known || !readiness.loaded {
                    return Err(anyhow!("BaseRT model is not ready after load"));
                }
            }
            RuntimeAction::Unload(class) => {
                let (id, _) = self.config.model(class);
                self.client.unload_model(id).await?;
                if self.client.model_readiness(id).await?.loaded {
                    return Err(anyhow!("BaseRT model remained loaded after unload"));
                }
                if class == ModelClass::Chat4B {
                    let baseline = self.chat_preload_available.lock().await.take();
                    if let Some(baseline) = baseline {
                        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                        loop {
                            let available = measured_memory_headroom()?.1;
                            if available >= baseline.saturating_mul(95) / 100 {
                                break;
                            }
                            if tokio::time::Instant::now() >= deadline {
                                return Err(anyhow!(
                                    "memory headroom did not return after 4B unload"
                                ));
                            }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
            RuntimeAction::Restart => self.restart_managed().await?,
            RuntimeAction::VerifyHealthyChangedPid => {
                let proof = *self.restart_pids.lock().await;
                let (previous, current) =
                    proof.ok_or_else(|| anyhow!("missing restart PID proof"))?;
                if current == previous {
                    return Err(anyhow!("replacement BaseRT is not a healthy changed PID"));
                }
                let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
                loop {
                    let same_managed_pid = Self::managed_pid().await == Some(current);
                    let owns_listener = Self::pid_owns_listener(current, 8082).await?;
                    if same_managed_pid && owns_listener && self.client.is_up().await {
                        break;
                    }
                    if tokio::time::Instant::now() >= deadline {
                        return Err(anyhow!(
                            "replacement BaseRT never became the healthy port-8082 owner"
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
            RuntimeAction::VerifyZeroLoadedWeights => {
                if self
                    .client
                    .inspect_models()
                    .await?
                    .iter()
                    .any(|model| model.loaded)
                {
                    return Err(anyhow!("replacement BaseRT retained loaded weights"));
                }
                *self.chat_preload_available.lock().await = None;
                self.client.clear_runtime_fault();
            }
        }
        Ok(())
    }

    async fn health(&self) -> bool {
        self.client.is_up().await
    }

    async fn models(&self) -> Result<Vec<ModelInfo>> {
        self.client.inspect_models().await
    }

    async fn memory_headroom(&self) -> Result<(u64, u64)> {
        measured_memory_headroom()
    }

    fn stream_completion(
        &self,
        model: ModelClass,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
    ) -> BoxStream<'static, Result<ChatStreamEvent>> {
        let (id, _) = self.config.model(model);
        self.client
            .chat_stream_with_tools(id.to_string(), messages, tools)
            .boxed()
    }

    async fn complete_bounded(
        &self,
        model: ModelClass,
        messages: Vec<Message>,
        temperature: f32,
        max_tokens: u32,
        format: CompletionFormat,
    ) -> Result<String> {
        let (id, _) = self.config.model(model);
        match format {
            CompletionFormat::Text => {
                self.client
                    .chat_complete_bounded(id, messages, temperature, max_tokens)
                    .await
            }
            CompletionFormat::Json => {
                self.client
                    .chat_complete_json_bounded(id, messages, temperature, max_tokens)
                    .await
            }
        }
    }
}

fn measured_memory_headroom() -> Result<(u64, u64)> {
    let total = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .context("measure total memory")?;
    let total = String::from_utf8_lossy(&total.stdout)
        .trim()
        .parse::<u64>()?;
    let vm = std::process::Command::new("/usr/bin/vm_stat")
        .output()
        .context("measure available memory")?;
    let text = String::from_utf8_lossy(&vm.stdout);
    let page_size = text
        .lines()
        .next()
        .and_then(|line| line.split("page size of ").nth(1))
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(16_384);
    let pages = text
        .lines()
        .filter(|line| {
            line.starts_with("Pages free:")
                || line.starts_with("Pages inactive:")
                || line.starts_with("Pages speculative:")
        })
        .filter_map(|line| line.split(':').nth(1))
        .filter_map(|value| value.trim().trim_end_matches('.').parse::<u64>().ok())
        .sum::<u64>();
    let available = pages.saturating_mul(page_size);
    Ok((available.saturating_mul(100) / total.max(1), available))
}

pub trait RuntimeClock: Send + Sync {
    fn now(&self) -> Duration;
}

struct SystemClock(std::time::Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(std::time::Instant::now())
    }
}

impl RuntimeClock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRuntimeSnapshot {
    pub accepting_demand: bool,
    pub queued_demand_count: usize,
    pub lease_count: usize,
    pub residency_pinned: bool,
    pub retirement_timer_starts: usize,
    pub retirement_started_at: Option<Duration>,
    pub generation: u64,
    pub clean_changed_pid_boundary: bool,
    pub phase: RuntimePhase,
}

struct RuntimeState {
    queue: VecDeque<QueuedDemand>,
    next_demand: u64,
    lease_count: usize,
    retirement_requested: Option<ModelClass>,
    retirement_started_at: Option<Duration>,
    retirement_timer_starts: usize,
    resident: Option<ModelClass>,
    generation: u64,
    clean_changed_pid_boundary: bool,
    phase: RuntimePhase,
    shutting_down: bool,
}

#[derive(Clone)]
struct QueuedDemand {
    id: u64,
    demand: ModelDemand,
}

struct QueuedDemandGuard {
    runtime: Arc<ModelRuntime>,
    demand_id: u64,
    transferred: bool,
}

struct LifecycleTransitionGuard<'a> {
    state: &'a Mutex<RuntimeState>,
    changed: &'a tokio::sync::Notify,
    poison_model: Option<ModelClass>,
    armed: bool,
}

impl LifecycleTransitionGuard<'_> {
    fn commit(&mut self) {
        self.armed = false;
    }
}

impl Drop for LifecycleTransitionGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = self.state.lock().expect("model runtime state");
        state.phase = self
            .poison_model
            .map(RuntimePhase::Poisoned)
            .unwrap_or(RuntimePhase::Unavailable);
        state.clean_changed_pid_boundary = false;
        drop(state);
        self.changed.notify_waiters();
    }
}

impl QueuedDemandGuard {
    fn new(runtime: Arc<ModelRuntime>, demand_id: u64) -> Self {
        Self {
            runtime,
            demand_id,
            transferred: false,
        }
    }

    fn transfer(&mut self) {
        self.transferred = true;
    }
}

impl Drop for QueuedDemandGuard {
    fn drop(&mut self) {
        if self.transferred {
            return;
        }
        let mut state = self.runtime.state.lock().expect("model runtime state");
        if let Some(index) = state
            .queue
            .iter()
            .position(|queued| queued.id == self.demand_id)
        {
            state.queue.remove(index);
        }
        drop(state);
        self.runtime.changed.notify_waiters();
    }
}

pub struct ModelRuntime {
    #[allow(dead_code)]
    adapter: Arc<dyn ModelRuntimeAdapter>,
    clock: Arc<dyn RuntimeClock>,
    state: Arc<Mutex<RuntimeState>>,
    transition: AsyncMutex<()>,
    changed: tokio::sync::Notify,
    simulated: bool,
    policy: ModelRuntimePolicy,
}

impl ModelRuntime {
    fn transition_guard(&self, poison_model: Option<ModelClass>) -> LifecycleTransitionGuard<'_> {
        LifecycleTransitionGuard {
            state: &self.state,
            changed: &self.changed,
            poison_model,
            armed: true,
        }
    }

    pub fn for_test(adapter: Arc<dyn ModelRuntimeAdapter>) -> Arc<Self> {
        Self::for_test_with_clock(adapter, Arc::new(SystemClock::default()))
    }

    pub fn for_test_with_clock(
        adapter: Arc<dyn ModelRuntimeAdapter>,
        clock: Arc<dyn RuntimeClock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            adapter,
            clock,
            state: Arc::new(Mutex::new(RuntimeState {
                queue: VecDeque::new(),
                next_demand: 1,
                lease_count: 0,
                retirement_requested: None,
                retirement_started_at: None,
                retirement_timer_starts: 0,
                resident: None,
                generation: 1,
                clean_changed_pid_boundary: false,
                phase: RuntimePhase::Unloaded,
                shutting_down: false,
            })),
            transition: AsyncMutex::new(()),
            changed: tokio::sync::Notify::new(),
            simulated: true,
            policy: ModelRuntimePolicy::default(),
        })
    }

    pub fn production(adapter: Arc<dyn ModelRuntimeAdapter>) -> Arc<Self> {
        Arc::new(Self {
            adapter,
            clock: Arc::new(SystemClock::default()),
            state: Arc::new(Mutex::new(RuntimeState {
                queue: VecDeque::new(),
                next_demand: 1,
                lease_count: 0,
                retirement_requested: None,
                retirement_started_at: None,
                retirement_timer_starts: 0,
                resident: None,
                generation: 1,
                clean_changed_pid_boundary: false,
                phase: RuntimePhase::Unavailable,
                shutting_down: false,
            })),
            transition: AsyncMutex::new(()),
            changed: tokio::sync::Notify::new(),
            simulated: false,
            policy: production_policy(),
        })
    }

    pub fn production_from_endpoint(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        config: ProductionModelConfig,
    ) -> Arc<Self> {
        Self::production(ProductionBaseRtAdapter::new(
            BaseRtClient::new(base_url, api_key),
            config,
        ))
    }

    #[cfg(feature = "stage7a-acceptance")]
    pub fn external_fixture_from_endpoint(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        config: ProductionModelConfig,
    ) -> Arc<Self> {
        Self::production(ProductionBaseRtAdapter::new_external_fixture(
            BaseRtClient::new(base_url, api_key),
            config,
        ))
    }

    #[cfg(feature = "stage7a-acceptance")]
    pub async fn initialize_external_fixture(&self, allow_retained_chat_model: bool) -> Result<()> {
        let _transition = self.transition.lock().await;
        self.adapter.perform(RuntimeAction::EnsureService).await?;
        if allow_retained_chat_model {
            self.state.lock().expect("model runtime state").phase =
                RuntimePhase::Ready(ModelClass::Chat4B);
            return Ok(());
        }
        self.adapter
            .perform(RuntimeAction::VerifyZeroLoadedWeights)
            .await?;
        let mut state = self.state.lock().expect("model runtime state");
        state.phase = RuntimePhase::Unloaded;
        // A newly started disposable BaseRT process has already crossed the
        // changed-PID/zero-weights boundary.  Requiring another restart would
        // incorrectly route an external fixture through the managed 8082
        // lifecycle path.  Retained Chat4B fixtures leave this false because
        // switching away from a resident model still requires a real restart.
        state.clean_changed_pid_boundary = true;
        Ok(())
    }

    pub async fn initialize(&self) -> Result<()> {
        let _transition = self.transition.lock().await;
        if let Err(error) = self.adapter.perform(RuntimeAction::EnsureService).await {
            self.state.lock().expect("model runtime state").phase = RuntimePhase::Unavailable;
            return Err(error);
        }
        self.state.lock().expect("model runtime state").phase = RuntimePhase::Restarting;
        self.restart_clean_locked(None).await
    }

    pub async fn enqueue(&self, demand: ModelDemand) {
        self.enqueue_internal(demand);
    }

    fn enqueue_internal(&self, demand: ModelDemand) -> u64 {
        let mut state = self.state.lock().expect("model runtime state");
        if state.shutting_down {
            return 0;
        }
        let id = state.next_demand;
        state.next_demand += 1;
        state.queue.push_back(QueuedDemand { id, demand });
        drop(state);
        self.changed.notify_waiters();
        id
    }

    pub async fn speculative_preload(&self, model: ModelClass) -> Result<bool> {
        let _transition = self.transition.lock().await;
        {
            let state = self.state.lock().expect("model runtime state");
            if state.shutting_down
                || state.lease_count > 0
                || state
                    .queue
                    .iter()
                    .any(|queued| queued.demand.work.is_some())
                || matches!(
                    state.phase,
                    RuntimePhase::Unavailable
                        | RuntimePhase::Poisoned(_)
                        | RuntimePhase::Restarting
                )
            {
                return Ok(false);
            }
            if state.phase == RuntimePhase::Ready(model) {
                return Ok(true);
            }
        }
        self.ensure_ready_locked(model).await?;
        let mut state = self.state.lock().expect("model runtime state");
        state.retirement_started_at = Some(self.clock.now());
        state.retirement_timer_starts += 1;
        Ok(true)
    }

    pub async fn dispatch_next(self: &Arc<Self>) -> Option<DispatchedDemand> {
        let mut state = self.state.lock().expect("model runtime state");
        if state.shutting_down
            || matches!(
                state.phase,
                RuntimePhase::Unavailable | RuntimePhase::Poisoned(_) | RuntimePhase::Restarting
            )
        {
            return None;
        }
        let selected = state
            .queue
            .iter()
            .enumerate()
            .filter(|(_, queued)| {
                state.lease_count == 0 || state.resident == Some(queued.demand.model)
            })
            .max_by_key(|(index, queued)| (queued.demand.priority, std::cmp::Reverse(*index)))
            .map(|(index, _)| index)?;
        let demand = state.queue.remove(selected)?.demand;
        let owns_lease = demand.work.is_some();
        if self.simulated {
            state.resident = Some(demand.model);
            state.phase = RuntimePhase::Ready(demand.model);
        } else if state.phase != RuntimePhase::Ready(demand.model) {
            let id = state.next_demand;
            state.next_demand += 1;
            state.queue.push_front(QueuedDemand { id, demand });
            return None;
        }
        if owns_lease {
            state.lease_count += 1;
            state.retirement_started_at = None;
        }
        Some(DispatchedDemand {
            runtime: self.clone(),
            demand,
            owns_lease,
            generation: state.generation,
            settled: false,
        })
    }

    async fn acquire(self: &Arc<Self>, demand: ModelDemand) -> Result<DispatchedDemand> {
        let demand_id = self.enqueue_internal(demand.clone());
        if demand_id == 0 {
            return Err(anyhow!("Model Runtime is shutting down"));
        }
        let mut queued_guard = QueuedDemandGuard::new(self.clone(), demand_id);
        loop {
            if self
                .state
                .lock()
                .expect("model runtime state")
                .shutting_down
            {
                return Err(anyhow!("Model Runtime is shutting down"));
            }
            if matches!(self.snapshot().phase, RuntimePhase::Poisoned(_)) {
                self.recover().await?;
            }
            let notified = self.changed.notified();
            let selected = {
                let state = self.state.lock().expect("model runtime state");
                state
                    .queue
                    .iter()
                    .enumerate()
                    .filter(|(_, queued)| {
                        state.lease_count == 0 || state.resident == Some(queued.demand.model)
                    })
                    .max_by_key(|(index, queued)| {
                        (queued.demand.priority, std::cmp::Reverse(*index))
                    })
                    .map(|(_, queued)| queued.id)
            };
            if selected != Some(demand_id) {
                notified.await;
                continue;
            }
            let _transition = self.transition.lock().await;
            if self
                .state
                .lock()
                .expect("model runtime state")
                .shutting_down
            {
                return Err(anyhow!("Model Runtime is shutting down"));
            }
            let selected_after_lock = {
                let state = self.state.lock().expect("model runtime state");
                state
                    .queue
                    .iter()
                    .enumerate()
                    .filter(|(_, queued)| {
                        state.lease_count == 0 || state.resident == Some(queued.demand.model)
                    })
                    .max_by_key(|(index, queued)| {
                        (queued.demand.priority, std::cmp::Reverse(*index))
                    })
                    .map(|(_, queued)| queued.id)
            };
            if selected_after_lock != Some(demand_id) {
                continue;
            }
            self.ensure_ready_locked(demand.model).await?;
            let mut state = self.state.lock().expect("model runtime state");
            if state.shutting_down {
                return Err(anyhow!("Model Runtime is shutting down"));
            }
            let still_selected = state
                .queue
                .iter()
                .enumerate()
                .filter(|(_, queued)| {
                    state.lease_count == 0 || state.resident == Some(queued.demand.model)
                })
                .max_by_key(|(index, queued)| (queued.demand.priority, std::cmp::Reverse(*index)))
                .map(|(_, queued)| queued.id);
            if still_selected != Some(demand_id) {
                drop(state);
                self.changed.notify_waiters();
                continue;
            }
            let index = state
                .queue
                .iter()
                .position(|queued| queued.id == demand_id)
                .expect("selected demand remains queued");
            let demand = state.queue.remove(index).expect("selected demand").demand;
            state.lease_count += 1;
            state.retirement_started_at = None;
            let generation = state.generation;
            drop(state);
            queued_guard.transfer();
            self.changed.notify_waiters();
            return Ok(DispatchedDemand {
                runtime: self.clone(),
                demand,
                owns_lease: true,
                generation,
                settled: false,
            });
        }
    }

    async fn ensure_ready_locked(&self, model: ModelClass) -> Result<()> {
        let (phase, resident, leases) = {
            let state = self.state.lock().expect("model runtime state");
            (state.phase, state.resident, state.lease_count)
        };
        if phase == RuntimePhase::Ready(model) {
            return Ok(());
        }
        if leases > 0 {
            return Err(anyhow!(
                "different-model demand must wait for leases to drain"
            ));
        }
        if resident.is_some() {
            self.perform_retirement(RuntimeAction::Unload(resident.expect("resident")))
                .await?;
        }
        let clean_boundary = self
            .state
            .lock()
            .expect("model runtime state")
            .clean_changed_pid_boundary;
        if model == ModelClass::Synthesis35B && !clean_boundary {
            self.restart_clean_locked(Some(model)).await?;
        }
        if model == ModelClass::Synthesis35B {
            let (free_percent, available) = self.adapter.memory_headroom().await?;
            if free_percent < 25 || available < 8 * 1024 * 1024 * 1024 {
                return Err(anyhow!("35B cold admission rejected by memory headroom"));
            }
        }
        self.state.lock().expect("model runtime state").phase = RuntimePhase::Loading(model);
        let mut transition_guard = self.transition_guard(Some(model));
        self.adapter.perform(RuntimeAction::Load(model)).await?;
        self.state.lock().expect("model runtime state").phase = RuntimePhase::LoadedNotReady(model);
        self.adapter
            .perform(RuntimeAction::VerifyReady(model))
            .await?;
        let mut state = self.state.lock().expect("model runtime state");
        state.resident = Some(model);
        state.phase = RuntimePhase::Ready(model);
        state.clean_changed_pid_boundary = false;
        drop(state);
        transition_guard.commit();
        Ok(())
    }

    async fn restart_clean_locked(&self, poison_model: Option<ModelClass>) -> Result<()> {
        self.state.lock().expect("model runtime state").phase = RuntimePhase::Restarting;
        let mut transition_guard = self.transition_guard(poison_model);
        self.adapter.perform(RuntimeAction::Restart).await?;
        self.adapter
            .perform(RuntimeAction::VerifyHealthyChangedPid)
            .await?;
        self.adapter
            .perform(RuntimeAction::VerifyZeroLoadedWeights)
            .await?;
        let mut state = self.state.lock().expect("model runtime state");
        state.generation += 1;
        state.lease_count = 0;
        state.resident = None;
        state.retirement_requested = None;
        state.retirement_started_at = None;
        state.clean_changed_pid_boundary = true;
        state.phase = RuntimePhase::Unloaded;
        drop(state);
        transition_guard.commit();
        self.changed.notify_waiters();
        Ok(())
    }

    pub async fn health(&self) -> bool {
        self.adapter.health().await
    }

    pub async fn models(&self) -> Result<Vec<ModelInfo>> {
        self.adapter.models().await
    }

    pub async fn complete_bounded(
        self: &Arc<Self>,
        demand: ModelDemand,
        messages: Vec<Message>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<String> {
        self.complete_with_format(
            demand,
            messages,
            temperature,
            max_tokens,
            CompletionFormat::Text,
        )
        .await
    }

    pub async fn complete_json(
        self: &Arc<Self>,
        demand: ModelDemand,
        messages: Vec<Message>,
        temperature: f32,
        max_tokens: u32,
    ) -> Result<String> {
        self.complete_with_format(
            demand,
            messages,
            temperature,
            max_tokens,
            CompletionFormat::Json,
        )
        .await
    }

    async fn complete_with_format(
        self: &Arc<Self>,
        demand: ModelDemand,
        messages: Vec<Message>,
        temperature: f32,
        max_tokens: u32,
        format: CompletionFormat,
    ) -> Result<String> {
        let lease = self.acquire(demand.clone()).await?;
        let result = self
            .adapter
            .complete_bounded(demand.model, messages, temperature, max_tokens, format)
            .await;
        match result {
            Ok(output) => {
                lease.complete().await;
                Ok(output)
            }
            Err(error) if completion_error_poisoning(&error) => {
                lease.indeterminate(RuntimeFault::IndeterminateTimeout);
                Err(error)
            }
            Err(error) => {
                lease.complete().await;
                Err(error)
            }
        }
    }

    pub async fn stream_completion(
        self: &Arc<Self>,
        demand: ModelDemand,
        messages: Vec<Message>,
        tools: Vec<ToolDef>,
    ) -> Result<RuntimeCompletionStream> {
        let lease = self.acquire(demand.clone()).await?;
        let inner = self
            .adapter
            .stream_completion(demand.model, messages, tools);
        Ok(RuntimeCompletionStream {
            inner,
            lease: Some(lease),
            terminal: false,
        })
    }

    pub fn snapshot(&self) -> ModelRuntimeSnapshot {
        let state = self.state.lock().expect("model runtime state");
        ModelRuntimeSnapshot {
            accepting_demand: !state.shutting_down,
            queued_demand_count: state.queue.len(),
            lease_count: state.lease_count,
            residency_pinned: state.lease_count > 0,
            retirement_timer_starts: state.retirement_timer_starts,
            retirement_started_at: state.retirement_started_at,
            generation: state.generation,
            clean_changed_pid_boundary: state.clean_changed_pid_boundary,
            phase: state.phase,
        }
    }

    pub fn policy(&self) -> ModelRuntimePolicy {
        self.policy
    }

    pub async fn request_retirement(&self, model: ModelClass) {
        let mut state = self.state.lock().expect("model runtime state");
        if state.resident != Some(model) {
            return;
        }
        state.retirement_requested = Some(model);
        if state.lease_count == 0 && state.retirement_started_at.is_none() {
            state.retirement_started_at = Some(self.clock.now());
            state.retirement_timer_starts += 1;
        }
    }

    pub async fn request_shutdown_retirement(&self) {
        let mut state = self.state.lock().expect("model runtime state");
        let Some(resident) = state.resident else {
            return;
        };
        state.retirement_requested = Some(resident);
        if state.lease_count == 0 && state.retirement_started_at.is_none() {
            state.retirement_started_at = Some(self.clock.now());
            state.retirement_timer_starts += 1;
        }
    }

    pub async fn shutdown(&self) -> Result<()> {
        {
            let mut state = self.state.lock().expect("model runtime state");
            state.shutting_down = true;
            state.queue.clear();
        }
        self.changed.notify_waiters();

        loop {
            let notified = self.changed.notified();
            let transition = self.transition.lock().await;
            let (phase, leases, resident) = {
                let state = self.state.lock().expect("model runtime state");
                (state.phase, state.lease_count, state.resident)
            };
            if matches!(phase, RuntimePhase::Poisoned(_)) {
                drop(transition);
                self.recover().await?;
                continue;
            }
            if leases > 0 {
                drop(transition);
                notified.await;
                continue;
            }
            let Some(_) = resident else {
                return Ok(());
            };
            {
                let mut state = self.state.lock().expect("model runtime state");
                state.retirement_requested = resident;
            }
            self.perform_retirement(RuntimeAction::Unload(resident.expect("resident")))
                .await?;
            return Ok(());
        }
    }

    pub async fn maintain(&self) -> Result<()> {
        let idle_retirement = Duration::from_secs(self.policy.shared_idle_timeout_seconds);
        let _transition = self.transition.lock().await;
        let action = {
            let state = self.state.lock().expect("model runtime state");
            match (
                state.lease_count,
                state.resident,
                state.retirement_started_at,
            ) {
                (0, Some(model), Some(started))
                    if self.clock.now().saturating_sub(started) >= idle_retirement =>
                {
                    Some(RuntimeAction::Unload(model))
                }
                _ => None,
            }
        };
        let Some(action) = action else {
            return Ok(());
        };
        self.perform_retirement(action).await?;
        Ok(())
    }

    pub async fn retire_now(&self) -> Result<bool> {
        let _transition = self.transition.lock().await;
        let action = {
            let state = self.state.lock().expect("model runtime state");
            if state.lease_count > 0 {
                return Ok(false);
            }
            match (state.resident, state.retirement_requested) {
                (Some(resident), Some(requested)) if resident == requested => {
                    Some(RuntimeAction::Unload(resident))
                }
                _ => None,
            }
        };
        let Some(action) = action else {
            return Ok(false);
        };
        self.perform_retirement(action).await?;
        Ok(true)
    }

    async fn perform_retirement(&self, unload: RuntimeAction) -> Result<()> {
        let retiring_35b = unload == RuntimeAction::Unload(ModelClass::Synthesis35B);
        let RuntimeAction::Unload(model) = unload else {
            return Err(anyhow!("retirement requires an unload action"));
        };
        self.state.lock().expect("model runtime state").phase = RuntimePhase::Retiring(model);
        let mut transition_guard = self.transition_guard(Some(model));
        self.adapter.perform(unload).await?;
        if retiring_35b {
            self.adapter.perform(RuntimeAction::Restart).await?;
            self.adapter
                .perform(RuntimeAction::VerifyHealthyChangedPid)
                .await?;
            self.adapter
                .perform(RuntimeAction::VerifyZeroLoadedWeights)
                .await?;
        }
        let mut state = self.state.lock().expect("model runtime state");
        state.resident = None;
        state.retirement_requested = None;
        state.retirement_started_at = None;
        state.phase = RuntimePhase::Unloaded;
        if retiring_35b {
            state.generation += 1;
            state.clean_changed_pid_boundary = true;
        }
        drop(state);
        transition_guard.commit();
        Ok(())
    }

    pub async fn poison(&self, _fault: RuntimeFault) {
        let mut state = self.state.lock().expect("model runtime state");
        if let Some(model) = state.resident {
            state.phase = RuntimePhase::Poisoned(model);
            state.clean_changed_pid_boundary = false;
        }
    }

    pub async fn recover(&self) -> Result<()> {
        let _transition = self.transition.lock().await;
        let poisoned_model = {
            let mut state = self.state.lock().expect("model runtime state");
            let RuntimePhase::Poisoned(model) = state.phase else {
                return Ok(());
            };
            state.phase = RuntimePhase::Restarting;
            model
        };
        self.restart_clean_locked(Some(poisoned_model)).await
    }

    fn poison_sync(&self, model: ModelClass) {
        let mut state = self.state.lock().expect("model runtime state");
        state.phase = RuntimePhase::Poisoned(model);
        state.clean_changed_pid_boundary = false;
        self.changed.notify_waiters();
    }
}

fn completion_error_poisoning(error: &anyhow::Error) -> bool {
    if let Some(BaseRtCompletionError::RuntimeFault(fault)) = error.downcast_ref() {
        return matches!(
            fault,
            BaseRtRuntimeFault::MetalOutOfMemory
                | BaseRtRuntimeFault::MetalDevice
                | BaseRtRuntimeFault::MetalCommandBuffer
        );
    }
    let normalized = error.to_string().to_ascii_lowercase();
    normalized.contains("timeout")
        || normalized.contains("timed out")
        || normalized.contains("metal")
        || normalized.contains("device lost")
        || normalized.contains("command buffer")
}

pub struct RuntimeCompletionStream {
    inner: BoxStream<'static, Result<ChatStreamEvent>>,
    lease: Option<DispatchedDemand>,
    terminal: bool,
}

impl Stream for RuntimeCompletionStream {
    type Item = Result<ChatStreamEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            Poll::Ready(None) => {
                self.terminal = true;
                if let Some(mut lease) = self.lease.take() {
                    lease.settle();
                }
                Poll::Ready(None)
            }
            Poll::Ready(Some(Err(error))) => {
                if let Some(mut lease) = self.lease.take() {
                    lease.mark_indeterminate(RuntimeFault::IndeterminateTimeout);
                }
                Poll::Ready(Some(Err(error)))
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{registered_model_directory, ProductionBaseRtAdapter};
    use std::path::{Path, PathBuf};
    use std::{sync::Arc, time::Duration};

    #[cfg(feature = "stage8-acceptance")]
    #[test]
    fn stage8_idle_timeout_override_is_bounded_and_fixture_gated() {
        let default = super::ModelRuntimePolicy::default();
        assert_eq!(
            super::acceptance_policy_override(default, true, Some("1")).shared_idle_timeout_seconds,
            1
        );
        assert_eq!(
            super::acceptance_policy_override(default, false, Some("1")),
            default
        );
        assert_eq!(
            super::acceptance_policy_override(default, true, Some("0")),
            default
        );
        assert_eq!(
            super::acceptance_policy_override(default, true, Some("61")),
            default
        );
    }

    #[test]
    fn custom_model_sources_map_beneath_the_managed_registry() {
        let destination = registered_model_directory(
            Path::new("/managed/registry"),
            "basecompute/Qwen3.6-35B-A3B",
            Path::new("/custom/models/q4-variant"),
        )
        .expect("safe destination");
        assert_eq!(
            destination,
            PathBuf::from("/managed/registry/basecompute/Qwen3.6-35B-A3B/q4-variant")
        );
        assert!(registered_model_directory(
            Path::new("/managed/registry"),
            "../../outside",
            Path::new("/custom/models/q4-variant")
        )
        .is_err());
    }

    #[tokio::test]
    async fn listener_proof_rejects_a_competing_process() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("disposable listener");
        let port = listener.local_addr().expect("listener address").port();
        let owner = std::process::id();
        assert!(ProductionBaseRtAdapter::pid_owns_listener(owner, port)
            .await
            .expect("owner proof"));
        assert!(
            !ProductionBaseRtAdapter::pid_owns_listener(owner.saturating_add(1), port)
                .await
                .expect("competing owner rejection")
        );
    }

    #[tokio::test]
    async fn listener_proof_waits_for_a_delayed_bind() {
        let reservation =
            std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve disposable port");
        let port = reservation.local_addr().expect("reserved address").port();
        drop(reservation);
        let release = Arc::new(tokio::sync::Notify::new());
        let child_release = release.clone();
        let delayed_listener = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .expect("delayed listener");
            child_release.notified().await;
        });

        ProductionBaseRtAdapter::wait_for_owned_listener(
            std::process::id(),
            port,
            Duration::from_secs(2),
        )
        .await
        .expect("delayed listener proof");
        release.notify_one();
        delayed_listener.await.expect("delayed listener task");
    }

    #[cfg(feature = "stage7a-acceptance")]
    #[tokio::test]
    async fn fresh_external_fixture_is_a_clean_changed_pid_boundary() {
        struct ExternalFixtureAdapter;

        #[async_trait::async_trait]
        impl super::ModelRuntimeAdapter for ExternalFixtureAdapter {
            async fn perform(&self, _action: super::RuntimeAction) -> anyhow::Result<()> {
                Ok(())
            }
        }

        let runtime = super::ModelRuntime::production(Arc::new(ExternalFixtureAdapter));
        runtime
            .initialize_external_fixture(false)
            .await
            .expect("fresh external fixture initialization");

        let snapshot = runtime.snapshot();
        assert_eq!(snapshot.phase, super::RuntimePhase::Unloaded);
        assert!(snapshot.clean_changed_pid_boundary);
    }
}

impl Drop for RuntimeCompletionStream {
    fn drop(&mut self) {
        if !self.terminal {
            if let Some(mut lease) = self.lease.take() {
                lease.mark_indeterminate(RuntimeFault::IndeterminateTimeout);
            }
        }
    }
}

pub struct DispatchedDemand {
    runtime: Arc<ModelRuntime>,
    demand: ModelDemand,
    owns_lease: bool,
    generation: u64,
    settled: bool,
}

impl DispatchedDemand {
    pub fn priority(&self) -> DemandPriority {
        self.demand.priority
    }

    pub fn lease(&self) -> Option<&WorkIdentity> {
        self.demand.work.as_ref()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub async fn complete(mut self) {
        self.settle();
    }

    pub async fn discard(mut self) {
        self.settle();
    }

    pub fn indeterminate(mut self, fault: RuntimeFault) {
        self.mark_indeterminate(fault);
    }

    fn mark_indeterminate(&mut self, _fault: RuntimeFault) {
        if self.settled {
            return;
        }
        self.runtime.poison_sync(self.demand.model);
        self.settled = true;
    }

    fn settle(&mut self) {
        if self.settled {
            return;
        }
        {
            let mut state = self.runtime.state.lock().expect("model runtime state");
            if self.owns_lease && state.generation == self.generation {
                state.lease_count = state.lease_count.saturating_sub(1);
            }
            if state.lease_count == 0
                && state.resident.is_some()
                && state.retirement_started_at.is_none()
            {
                state.retirement_started_at = Some(self.runtime.clock.now());
                state.retirement_timer_starts += 1;
            }
        }
        self.settled = true;
        self.runtime.changed.notify_waiters();
    }
}

impl Drop for DispatchedDemand {
    fn drop(&mut self) {
        if !self.settled {
            self.mark_indeterminate(RuntimeFault::IndeterminateTimeout);
        }
    }
}
