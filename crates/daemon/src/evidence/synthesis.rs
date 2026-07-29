use anyhow::{anyhow, Result};
use async_trait::async_trait;
use basert_connector::{BaseRtClient, Message, ModelInfo, ModelLoadRequest};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, Weak},
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
};

pub(crate) const PREFERRED_SYNTHESIS_MODEL: &str = "basecompute/Qwen3.6-35B-A3B";
pub(crate) const FALLBACK_SYNTHESIS_MODEL: &str = "basecompute/Qwen3-4B-Instruct-2507";
pub(crate) const SYNTHESIS_MODEL_IDLE_TTL: Duration = Duration::from_secs(20 * 60);
pub(crate) const SYNTHESIS_COLD_READY_TIMEOUT: Duration = Duration::from_secs(45);
pub(crate) const SYNTHESIS_WARM_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const SYNTHESIS_FALLBACK_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone)]
pub(crate) struct SynthesisConfig {
    pub preferred_model: String,
    pub preferred_path: PathBuf,
    pub fallback_model: String,
    pub fallback_path: PathBuf,
    pub cold_ready_timeout: Duration,
    pub warm_timeout: Duration,
    pub fallback_timeout: Duration,
    pub idle_ttl: Duration,
    pub maintenance_interval: Duration,
}

impl SynthesisConfig {
    pub(crate) fn from_environment() -> Self {
        let preferred_model = PREFERRED_SYNTHESIS_MODEL.to_string();
        let fallback_model = FALLBACK_SYNTHESIS_MODEL.to_string();
        Self {
            preferred_path: model_path("BAGENT_SYNTHESIS_MODEL_PATH", &preferred_model),
            fallback_path: model_path("BAGENT_SYNTHESIS_FALLBACK_MODEL_PATH", &fallback_model),
            preferred_model,
            fallback_model,
            cold_ready_timeout: SYNTHESIS_COLD_READY_TIMEOUT,
            warm_timeout: SYNTHESIS_WARM_TIMEOUT,
            fallback_timeout: SYNTHESIS_FALLBACK_TIMEOUT,
            idle_ttl: SYNTHESIS_MODEL_IDLE_TTL,
            maintenance_interval: Duration::from_secs(30),
        }
    }
}

fn model_path(variable: &str, model: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(variable) {
        return PathBuf::from(path);
    }
    dirs::home_dir()
        .unwrap_or_default()
        .join("Library/Caches/baseRT/models")
        .join(model)
        .join("default-q4/model.base")
}

#[derive(Debug, Clone)]
pub(crate) struct SynthesisModelRequest {
    pub model_id: String,
    pub messages: Vec<Message>,
    pub temperature: f32,
    pub max_tokens: u32,
}

#[async_trait]
pub(crate) trait SynthesisModelClient: Send + Sync {
    async fn inspect_models(&self) -> Result<Vec<ModelInfo>>;
    async fn load_model(&self, request: &ModelLoadRequest) -> Result<()>;
    async fn unload_model(&self, model: &str) -> Result<()>;
    async fn complete(&self, request: SynthesisModelRequest) -> Result<String>;
}

#[async_trait]
impl SynthesisModelClient for BaseRtClient {
    async fn inspect_models(&self) -> Result<Vec<ModelInfo>> {
        BaseRtClient::inspect_models(self).await
    }

    async fn load_model(&self, request: &ModelLoadRequest) -> Result<()> {
        BaseRtClient::load_model(self, request).await?;
        Ok(())
    }

    async fn unload_model(&self, model: &str) -> Result<()> {
        BaseRtClient::unload_model(self, model).await
    }

    async fn complete(&self, request: SynthesisModelRequest) -> Result<String> {
        BaseRtClient::chat_complete_bounded(
            self,
            &request.model_id,
            request.messages,
            request.temperature,
            request.max_tokens,
        )
        .await
    }
}

#[async_trait]
pub(crate) trait SynthesisClock: Send + Sync {
    fn monotonic(&self) -> Duration;
    async fn sleep(&self, duration: Duration);
}

pub(crate) struct SystemSynthesisClock {
    started: Instant,
}

impl Default for SystemSynthesisClock {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

#[async_trait]
impl SynthesisClock for SystemSynthesisClock {
    fn monotonic(&self) -> Duration {
        self.started.elapsed()
    }

    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[async_trait]
pub(crate) trait MemoryPressureSignal: Send + Sync {
    async fn under_pressure(&self) -> bool;
}

pub(crate) struct SystemMemoryPressureSignal {
    simulation_path: Option<PathBuf>,
}

impl SystemMemoryPressureSignal {
    pub(crate) fn from_environment() -> Self {
        Self {
            simulation_path: std::env::var_os("BAGENT_SYNTHESIS_PRESSURE_FILE").map(PathBuf::from),
        }
    }
}

#[async_trait]
impl MemoryPressureSignal for SystemMemoryPressureSignal {
    async fn under_pressure(&self) -> bool {
        if self
            .simulation_path
            .as_ref()
            .is_some_and(|path| path.is_file())
        {
            return true;
        }
        #[cfg(target_os = "macos")]
        {
            let output = tokio::process::Command::new("/usr/bin/memory_pressure")
                .arg("-Q")
                .output()
                .await;
            let Ok(output) = output else {
                return false;
            };
            let report = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
            if report.contains("critical") || report.contains("warn") {
                return true;
            }
            report
                .split(|character: char| !character.is_ascii_digit())
                .filter_map(|part| part.parse::<u8>().ok())
                .next_back()
                .is_some_and(|free_percent| free_percent <= 15)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SynthesisPhase {
    LoadingModel,
    Synthesizing,
    Repairing,
    FallingBack,
    Validating,
    DeterministicRendering,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SynthesisPhaseEvent {
    pub turn_id: String,
    pub model_id: Option<String>,
    pub phase: SynthesisPhase,
    pub duration_ms: u64,
    pub timed_out: bool,
    pub fallback: bool,
    pub repair: bool,
    pub failure_reason: Option<String>,
}

struct RuntimePhaseEvent {
    model_id: Option<String>,
    phase: SynthesisPhase,
    duration_ms: u64,
    timed_out: bool,
    fallback: bool,
    repair: bool,
    failure_reason: Option<String>,
}

#[async_trait]
pub(crate) trait SynthesisObserver: Send + Sync {
    async fn record(&self, event: SynthesisPhaseEvent);
}

#[async_trait]
trait RuntimeObserver: Send + Sync {
    async fn record(&self, event: RuntimePhaseEvent);
}

pub(crate) struct NoopSynthesisObserver;

#[async_trait]
impl SynthesisObserver for NoopSynthesisObserver {
    async fn record(&self, _event: SynthesisPhaseEvent) {}
}

struct CorrelatedObserver<'a> {
    turn_id: &'a str,
    inner: &'a dyn SynthesisObserver,
}

#[async_trait]
impl RuntimeObserver for CorrelatedObserver<'_> {
    async fn record(&self, event: RuntimePhaseEvent) {
        self.inner
            .record(SynthesisPhaseEvent {
                turn_id: self.turn_id.to_string(),
                model_id: event.model_id,
                phase: event.phase,
                duration_ms: event.duration_ms,
                timed_out: event.timed_out,
                fallback: event.fallback,
                repair: event.repair,
                failure_reason: event.failure_reason,
            })
            .await;
    }
}

pub(crate) trait SynthesisContract: Send + Sync {
    fn turn_id(&self) -> &str;
    fn eligible(&self) -> bool;
    fn initial_request(&self) -> Vec<Message>;
    fn repair_request(&self, validation_errors: &[String]) -> Vec<Message>;
    fn validate(&self, response: &str) -> std::result::Result<(), Vec<String>>;
    fn deterministic_render(&self) -> String;
    fn max_tokens(&self) -> u32;
    fn temperature(&self) -> f32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SynthesisRoute {
    Preferred,
    Repaired,
    Fallback,
    Deterministic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SynthesisOutcome {
    pub text: String,
    pub route: SynthesisRoute,
}

#[derive(Debug, Default)]
struct RuntimeState {
    preferred_last_use: Option<Duration>,
    preferred_active: usize,
    fallback_owned: bool,
}

pub(crate) struct ModelRuntimeManager {
    client: Arc<dyn SynthesisModelClient>,
    clock: Arc<dyn SynthesisClock>,
    pressure: Arc<dyn MemoryPressureSignal>,
    config: SynthesisConfig,
    lifecycle_lock: Mutex<()>,
    state: StdMutex<RuntimeState>,
}

struct PreferredLease {
    runtime: Arc<ModelRuntimeManager>,
}

impl Drop for PreferredLease {
    fn drop(&mut self) {
        let mut state = self.runtime.state.lock().expect("runtime state lock");
        state.preferred_active = state.preferred_active.saturating_sub(1);
        state.preferred_last_use = Some(self.runtime.clock.monotonic());
    }
}

impl ModelRuntimeManager {
    pub(crate) fn new(
        client: Arc<dyn SynthesisModelClient>,
        clock: Arc<dyn SynthesisClock>,
        pressure: Arc<dyn MemoryPressureSignal>,
        config: SynthesisConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            client,
            clock,
            pressure,
            config,
            lifecycle_lock: Mutex::new(()),
            state: StdMutex::new(RuntimeState::default()),
        })
    }

    async fn preferred_lease(
        self: &Arc<Self>,
        observer: &dyn RuntimeObserver,
    ) -> Result<PreferredLease> {
        let started = self.clock.monotonic();
        let _lifecycle = self.lifecycle_lock.lock().await;
        if self.pressure.under_pressure().await {
            self.unload_preferred_if_idle().await;
            observer
                .record(phase_event(
                    Some(&self.config.preferred_model),
                    SynthesisPhase::LoadingModel,
                    elapsed_ms(self.clock.monotonic(), started),
                    false,
                    false,
                    false,
                    Some("memory_pressure"),
                ))
                .await;
            return Err(anyhow!("memory pressure"));
        }

        let loaded = match self.model_loaded(&self.config.preferred_model).await {
            Ok(loaded) => loaded,
            Err(error) => {
                observer
                    .record(phase_event(
                        Some(&self.config.preferred_model),
                        SynthesisPhase::LoadingModel,
                        elapsed_ms(self.clock.monotonic(), started),
                        false,
                        false,
                        false,
                        Some(normalized_failure_reason(&error.to_string())),
                    ))
                    .await;
                return Err(error);
            }
        };
        if !loaded {
            if self.model_loaded(&self.config.fallback_model).await? {
                self.client
                    .unload_model(&self.config.fallback_model)
                    .await?;
                self.state
                    .lock()
                    .expect("runtime state lock")
                    .fallback_owned = false;
            }
            observer
                .record(phase_event(
                    Some(&self.config.preferred_model),
                    SynthesisPhase::LoadingModel,
                    0,
                    false,
                    false,
                    false,
                    None,
                ))
                .await;
            let request = ModelLoadRequest {
                id: self.config.preferred_model.clone(),
                path: self.config.preferred_path.to_string_lossy().into_owned(),
            };
            let readiness = async {
                self.client.load_model(&request).await?;
                self.wait_until_loaded(&self.config.preferred_model).await
            };
            match tokio::time::timeout(self.config.cold_ready_timeout, readiness).await {
                Err(_) => {
                    observer
                        .record(phase_event(
                            Some(&self.config.preferred_model),
                            SynthesisPhase::LoadingModel,
                            elapsed_ms(self.clock.monotonic(), started),
                            true,
                            false,
                            false,
                            Some("timeout"),
                        ))
                        .await;
                    return Err(anyhow!("preferred model readiness timeout"));
                }
                Ok(Err(error)) => {
                    observer
                        .record(phase_event(
                            Some(&self.config.preferred_model),
                            SynthesisPhase::LoadingModel,
                            elapsed_ms(self.clock.monotonic(), started),
                            false,
                            false,
                            false,
                            Some(normalized_failure_reason(&error.to_string())),
                        ))
                        .await;
                    return Err(error);
                }
                Ok(Ok(())) => {}
            }
            if !self.model_loaded(&self.config.preferred_model).await? {
                return Err(anyhow!("preferred model unavailable after load"));
            }
            observer
                .record(phase_event(
                    Some(&self.config.preferred_model),
                    SynthesisPhase::LoadingModel,
                    elapsed_ms(self.clock.monotonic(), started),
                    false,
                    false,
                    false,
                    None,
                ))
                .await;
        }
        {
            let mut state = self.state.lock().expect("runtime state lock");
            state.preferred_active += 1;
            state.preferred_last_use = Some(self.clock.monotonic());
        }
        Ok(PreferredLease {
            runtime: self.clone(),
        })
    }

    async fn ensure_fallback(&self) -> Result<()> {
        let _lifecycle = self.lifecycle_lock.lock().await;
        self.unload_preferred_if_idle().await;
        if self.model_loaded(&self.config.fallback_model).await? {
            return Ok(());
        }
        let request = ModelLoadRequest {
            id: self.config.fallback_model.clone(),
            path: self.config.fallback_path.to_string_lossy().into_owned(),
        };
        self.client.load_model(&request).await?;
        self.wait_until_loaded(&self.config.fallback_model).await?;
        self.state
            .lock()
            .expect("runtime state lock")
            .fallback_owned = true;
        Ok(())
    }

    async fn wait_until_loaded(&self, model: &str) -> Result<()> {
        loop {
            if self.model_loaded(model).await? {
                return Ok(());
            }
            self.clock.sleep(Duration::from_millis(100)).await;
        }
    }

    async fn model_loaded(&self, model: &str) -> Result<bool> {
        Ok(self
            .client
            .inspect_models()
            .await?
            .iter()
            .any(|candidate| candidate.id == model && candidate.loaded))
    }

    async fn unload_preferred_if_idle(&self) {
        let active = self
            .state
            .lock()
            .expect("runtime state lock")
            .preferred_active;
        if active > 0 {
            return;
        }
        if self
            .model_loaded(&self.config.preferred_model)
            .await
            .unwrap_or(false)
        {
            let _ = self.client.unload_model(&self.config.preferred_model).await;
        }
        self.state
            .lock()
            .expect("runtime state lock")
            .preferred_last_use = None;
    }

    pub(crate) async fn maintain(&self) {
        let _lifecycle = self.lifecycle_lock.lock().await;
        let now = self.clock.monotonic();
        let (idle_expired, active) = {
            let state = self.state.lock().expect("runtime state lock");
            (
                state.preferred_active == 0
                    && state
                        .preferred_last_use
                        .is_some_and(|last| now.saturating_sub(last) >= self.config.idle_ttl),
                state.preferred_active,
            )
        };
        let pressure = self.pressure.under_pressure().await;
        if active == 0 && (idle_expired || pressure) {
            self.unload_preferred_if_idle().await;
        }
    }

    pub(crate) async fn shutdown(&self) {
        let _lifecycle = self.lifecycle_lock.lock().await;
        self.unload_preferred_if_idle().await;
    }
}

pub(crate) struct SynthesisService {
    runtime: Arc<ModelRuntimeManager>,
    client: Arc<dyn SynthesisModelClient>,
    config: SynthesisConfig,
    maintenance_stop: Notify,
    maintenance_task: Mutex<Option<JoinHandle<()>>>,
}

impl SynthesisService {
    pub(crate) fn new(
        client: Arc<dyn SynthesisModelClient>,
        clock: Arc<dyn SynthesisClock>,
        pressure: Arc<dyn MemoryPressureSignal>,
        config: SynthesisConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime: ModelRuntimeManager::new(client.clone(), clock, pressure, config.clone()),
            client,
            config,
            maintenance_stop: Notify::new(),
            maintenance_task: Mutex::new(None),
        })
    }

    pub(crate) async fn start_maintenance(self: &Arc<Self>) {
        let mut task = self.maintenance_task.lock().await;
        if task.is_some() {
            return;
        }
        let weak: Weak<Self> = Arc::downgrade(self);
        let interval = self.config.maintenance_interval;
        *task = Some(tokio::spawn(async move {
            loop {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                tokio::select! {
                    _ = service.maintenance_stop.notified() => return,
                    _ = tokio::time::sleep(interval) => service.runtime.maintain().await,
                }
            }
        }));
    }

    pub(crate) async fn shutdown(&self) {
        self.maintenance_stop.notify_one();
        if let Some(task) = self.maintenance_task.lock().await.take() {
            let _ = task.await;
        }
        self.runtime.shutdown().await;
    }

    pub(crate) async fn maintain(&self) {
        self.runtime.maintain().await;
    }

    pub(crate) async fn synthesize(
        self: &Arc<Self>,
        contract: &dyn SynthesisContract,
        observer: &dyn SynthesisObserver,
    ) -> SynthesisOutcome {
        let correlated = CorrelatedObserver {
            turn_id: contract.turn_id(),
            inner: observer,
        };
        let observer = &correlated;
        if !contract.eligible() {
            return self.deterministic(contract, observer, None).await;
        }
        let initial = contract.initial_request();
        if validate_transcript(&initial).is_err() {
            return self
                .deterministic(contract, observer, Some("invalid_transcript"))
                .await;
        }
        let lease = match self.runtime.preferred_lease(observer).await {
            Ok(lease) => lease,
            Err(_) => return self.fallback(contract, observer, initial).await,
        };
        let preferred = self
            .complete_with_phase(
                SynthesisPhase::Synthesizing,
                &self.config.preferred_model,
                initial.clone(),
                contract,
                self.config.warm_timeout,
                observer,
                false,
                false,
            )
            .await;
        match preferred {
            CompletionAttempt::Completed(response) => {
                let validation = self.validate(contract, &response, observer, false).await;
                match validation {
                    Ok(()) => SynthesisOutcome {
                        text: response,
                        route: SynthesisRoute::Preferred,
                    },
                    Err(errors) => {
                        let repair = contract.repair_request(&errors);
                        if validate_transcript(&repair).is_err() {
                            drop(lease);
                            return self
                                .deterministic(
                                    contract,
                                    observer,
                                    Some("invalid_repair_transcript"),
                                )
                                .await;
                        }
                        let repaired = self
                            .complete_with_phase(
                                SynthesisPhase::Repairing,
                                &self.config.preferred_model,
                                repair,
                                contract,
                                self.config.warm_timeout,
                                observer,
                                false,
                                true,
                            )
                            .await;
                        drop(lease);
                        match repaired {
                            CompletionAttempt::Completed(response) => {
                                match self.validate(contract, &response, observer, true).await {
                                    Ok(()) => SynthesisOutcome {
                                        text: response,
                                        route: SynthesisRoute::Repaired,
                                    },
                                    Err(_) => {
                                        self.deterministic(
                                            contract,
                                            observer,
                                            Some("repair_validation_failed"),
                                        )
                                        .await
                                    }
                                }
                            }
                            CompletionAttempt::Failed => {
                                self.deterministic(contract, observer, Some("repair_model_failed"))
                                    .await
                            }
                        }
                    }
                }
            }
            CompletionAttempt::Failed => {
                drop(lease);
                self.fallback(contract, observer, initial).await
            }
        }
    }

    async fn validate(
        &self,
        contract: &dyn SynthesisContract,
        response: &str,
        observer: &dyn RuntimeObserver,
        repair: bool,
    ) -> std::result::Result<(), Vec<String>> {
        let started = Instant::now();
        let result = contract.validate(response);
        observer
            .record(phase_event(
                None,
                SynthesisPhase::Validating,
                duration_ms(started.elapsed()),
                false,
                false,
                repair,
                result
                    .as_ref()
                    .err()
                    .and_then(|errors| errors.first().map(String::as_str)),
            ))
            .await;
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn complete_with_phase(
        &self,
        phase: SynthesisPhase,
        model: &str,
        messages: Vec<Message>,
        contract: &dyn SynthesisContract,
        timeout: Duration,
        observer: &dyn RuntimeObserver,
        fallback: bool,
        repair: bool,
    ) -> CompletionAttempt {
        let started = Instant::now();
        observer
            .record(phase_event(
                Some(model),
                phase,
                0,
                false,
                fallback,
                repair,
                None,
            ))
            .await;
        let request = SynthesisModelRequest {
            model_id: model.to_string(),
            messages,
            temperature: contract.temperature(),
            max_tokens: contract.max_tokens(),
        };
        match tokio::time::timeout(timeout, self.client.complete(request)).await {
            Err(_) => {
                observer
                    .record(phase_event(
                        Some(model),
                        phase,
                        duration_ms(started.elapsed()),
                        true,
                        fallback,
                        repair,
                        Some("timeout"),
                    ))
                    .await;
                CompletionAttempt::Failed
            }
            Ok(Err(error)) => {
                observer
                    .record(phase_event(
                        Some(model),
                        phase,
                        duration_ms(started.elapsed()),
                        false,
                        fallback,
                        repair,
                        Some(normalized_failure_reason(&error.to_string())),
                    ))
                    .await;
                CompletionAttempt::Failed
            }
            Ok(Ok(response)) => {
                observer
                    .record(phase_event(
                        Some(model),
                        phase,
                        duration_ms(started.elapsed()),
                        false,
                        fallback,
                        repair,
                        None,
                    ))
                    .await;
                CompletionAttempt::Completed(response)
            }
        }
    }

    async fn fallback(
        &self,
        contract: &dyn SynthesisContract,
        observer: &dyn RuntimeObserver,
        messages: Vec<Message>,
    ) -> SynthesisOutcome {
        let started = Instant::now();
        observer
            .record(phase_event(
                Some(&self.config.fallback_model),
                SynthesisPhase::FallingBack,
                0,
                false,
                true,
                false,
                None,
            ))
            .await;
        let path = async {
            self.runtime.ensure_fallback().await?;
            self.client
                .complete(SynthesisModelRequest {
                    model_id: self.config.fallback_model.clone(),
                    messages,
                    temperature: contract.temperature(),
                    max_tokens: contract.max_tokens(),
                })
                .await
        };
        match tokio::time::timeout(self.config.fallback_timeout, path).await {
            Ok(Ok(response)) => {
                observer
                    .record(phase_event(
                        Some(&self.config.fallback_model),
                        SynthesisPhase::FallingBack,
                        duration_ms(started.elapsed()),
                        false,
                        true,
                        false,
                        None,
                    ))
                    .await;
                match self.validate(contract, &response, observer, false).await {
                    Ok(()) => SynthesisOutcome {
                        text: response,
                        route: SynthesisRoute::Fallback,
                    },
                    Err(_) => {
                        self.deterministic(contract, observer, Some("fallback_validation_failed"))
                            .await
                    }
                }
            }
            Ok(Err(error)) => {
                observer
                    .record(phase_event(
                        Some(&self.config.fallback_model),
                        SynthesisPhase::FallingBack,
                        duration_ms(started.elapsed()),
                        false,
                        true,
                        false,
                        Some(normalized_failure_reason(&error.to_string())),
                    ))
                    .await;
                self.deterministic(contract, observer, Some("model_unavailable"))
                    .await
            }
            Err(_) => {
                observer
                    .record(phase_event(
                        Some(&self.config.fallback_model),
                        SynthesisPhase::FallingBack,
                        duration_ms(started.elapsed()),
                        true,
                        true,
                        false,
                        Some("timeout"),
                    ))
                    .await;
                self.deterministic(contract, observer, Some("model_unavailable"))
                    .await
            }
        }
    }

    async fn deterministic(
        &self,
        contract: &dyn SynthesisContract,
        observer: &dyn RuntimeObserver,
        reason: Option<&str>,
    ) -> SynthesisOutcome {
        observer
            .record(phase_event(
                None,
                SynthesisPhase::DeterministicRendering,
                0,
                false,
                false,
                false,
                reason,
            ))
            .await;
        let mut text = contract.deterministic_render();
        if reason == Some("model_unavailable") {
            text = format!(
                "Model synthesis was unavailable; showing verified evidence directly.\n\n{text}"
            );
        }
        SynthesisOutcome {
            text,
            route: SynthesisRoute::Deterministic,
        }
    }
}

enum CompletionAttempt {
    Completed(String),
    Failed,
}

fn validate_transcript(messages: &[Message]) -> Result<()> {
    if messages.len() != 2
        || messages[0].role != "system"
        || messages[1].role != "user"
        || messages
            .iter()
            .any(|message| !message.tool_calls.is_empty() || message.tool_call_id.is_some())
    {
        return Err(anyhow!(
            "synthesis transcript must contain exactly one system and one user message"
        ));
    }
    Ok(())
}

fn phase_event(
    model_id: Option<&str>,
    phase: SynthesisPhase,
    duration_ms: u64,
    timed_out: bool,
    fallback: bool,
    repair: bool,
    failure_reason: Option<&str>,
) -> RuntimePhaseEvent {
    RuntimePhaseEvent {
        model_id: model_id.map(str::to_string),
        phase,
        duration_ms,
        timed_out,
        fallback,
        repair,
        failure_reason: failure_reason.map(str::to_string),
    }
}

fn elapsed_ms(now: Duration, started: Duration) -> u64 {
    duration_ms(now.saturating_sub(started))
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

pub(crate) fn normalized_failure_reason(error: &str) -> &'static str {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("timeout") || normalized.contains("timed out") {
        "timeout"
    } else if normalized.contains("memory pressure") {
        "memory_pressure"
    } else if normalized.contains("unavailable")
        || normalized.contains("not found")
        || normalized.contains("unknown model")
        || normalized.contains("failed to load model")
        || normalized.contains("failed to open model")
    {
        "model_unavailable"
    } else if normalized.contains("connection")
        || normalized.contains("transport")
        || normalized.contains("sending request")
    {
        "transport"
    } else if normalized.contains("parse")
        || normalized.contains("invalid")
        || normalized.contains("response")
    {
        "invalid_response"
    } else {
        "model_error"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{HashSet, VecDeque},
        sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    };

    #[derive(Clone)]
    enum Behavior {
        Response(&'static str),
        Error(&'static str),
        Pending,
    }

    #[derive(Default)]
    struct FakeModelClient {
        loaded: Mutex<HashSet<String>>,
        behaviors: Mutex<VecDeque<Behavior>>,
        requests: Mutex<Vec<SynthesisModelRequest>>,
        loads: AtomicUsize,
        unloads: Mutex<Vec<String>>,
        load_behavior: Mutex<Option<Behavior>>,
        load_delay: Mutex<Option<Duration>>,
        readiness_polls: AtomicUsize,
        pending_load: Mutex<Option<String>>,
    }

    impl FakeModelClient {
        fn with_behaviors(behaviors: impl IntoIterator<Item = Behavior>) -> Arc<Self> {
            Arc::new(Self {
                behaviors: Mutex::new(behaviors.into_iter().collect()),
                ..Self::default()
            })
        }

        async fn set_loaded(&self, model: &str) {
            self.loaded.lock().await.insert(model.to_string());
        }
    }

    #[async_trait]
    impl SynthesisModelClient for FakeModelClient {
        async fn inspect_models(&self) -> Result<Vec<ModelInfo>> {
            let ready_model = {
                let mut pending = self.pending_load.lock().await;
                if pending.is_some() && self.readiness_polls.fetch_sub(1, Ordering::SeqCst) == 1 {
                    pending.take()
                } else {
                    None
                }
            };
            if let Some(model) = ready_model {
                self.loaded.lock().await.insert(model);
            }
            Ok(self
                .loaded
                .lock()
                .await
                .iter()
                .map(|id| ModelInfo {
                    id: id.clone(),
                    loaded: true,
                })
                .collect())
        }

        async fn load_model(&self, request: &ModelLoadRequest) -> Result<()> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            if let Some(delay) = *self.load_delay.lock().await {
                tokio::time::sleep(delay).await;
            }
            match self.load_behavior.lock().await.take() {
                Some(Behavior::Error(error)) => return Err(anyhow!(error)),
                Some(Behavior::Pending) => std::future::pending::<()>().await,
                _ => {}
            }
            if self.readiness_polls.load(Ordering::SeqCst) > 0 {
                *self.pending_load.lock().await = Some(request.id.clone());
            } else {
                self.loaded.lock().await.insert(request.id.clone());
            }
            Ok(())
        }

        async fn unload_model(&self, model: &str) -> Result<()> {
            self.loaded.lock().await.remove(model);
            self.unloads.lock().await.push(model.to_string());
            Ok(())
        }

        async fn complete(&self, request: SynthesisModelRequest) -> Result<String> {
            self.requests.lock().await.push(request);
            match self.behaviors.lock().await.pop_front() {
                Some(Behavior::Response(response)) => Ok(response.to_string()),
                Some(Behavior::Error(error)) => Err(anyhow!(error)),
                Some(Behavior::Pending) => std::future::pending::<Result<String>>().await,
                None => Err(anyhow!("unexpected model request")),
            }
        }
    }

    #[derive(Default)]
    struct FakeClock(AtomicU64);

    impl FakeClock {
        fn advance(&self, duration: Duration) {
            self.0.fetch_add(
                duration.as_millis().try_into().unwrap_or(u64::MAX),
                Ordering::SeqCst,
            );
        }
    }

    #[async_trait]
    impl SynthesisClock for FakeClock {
        fn monotonic(&self) -> Duration {
            Duration::from_millis(self.0.load(Ordering::SeqCst))
        }

        async fn sleep(&self, duration: Duration) {
            self.advance(duration);
            tokio::task::yield_now().await;
        }
    }

    #[derive(Default)]
    struct FakePressure(AtomicBool);

    #[async_trait]
    impl MemoryPressureSignal for FakePressure {
        async fn under_pressure(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[derive(Default)]
    struct RecordingObserver(Mutex<Vec<SynthesisPhaseEvent>>);

    #[async_trait]
    impl SynthesisObserver for RecordingObserver {
        async fn record(&self, event: SynthesisPhaseEvent) {
            self.0.lock().await.push(event);
        }
    }

    struct TestContract {
        eligible: bool,
        initial_calls: AtomicUsize,
        repair_calls: AtomicUsize,
    }

    impl Default for TestContract {
        fn default() -> Self {
            Self {
                eligible: true,
                initial_calls: AtomicUsize::new(0),
                repair_calls: AtomicUsize::new(0),
            }
        }
    }

    impl SynthesisContract for TestContract {
        fn turn_id(&self) -> &str {
            "test-turn"
        }

        fn eligible(&self) -> bool {
            self.eligible
        }

        fn initial_request(&self) -> Vec<Message> {
            self.initial_calls.fetch_add(1, Ordering::SeqCst);
            vec![
                Message::system("system"),
                Message::user("EVIDENCE_BUNDLE={\"value\":\"bounded\"}"),
            ]
        }

        fn repair_request(&self, validation_errors: &[String]) -> Vec<Message> {
            self.repair_calls.fetch_add(1, Ordering::SeqCst);
            vec![
                Message::system("system repair"),
                Message::user(format!(
                    "EVIDENCE_BUNDLE={{\"value\":\"bounded\"}}\nVALIDATION_ERRORS={}",
                    serde_json::to_string(validation_errors).unwrap()
                )),
            ]
        }

        fn validate(&self, response: &str) -> std::result::Result<(), Vec<String>> {
            if matches!(response, "valid" | "repaired" | "fallback") {
                Ok(())
            } else {
                Err(vec!["unsupported_claim".into()])
            }
        }

        fn deterministic_render(&self) -> String {
            "deterministic".into()
        }

        fn max_tokens(&self) -> u32 {
            512
        }

        fn temperature(&self) -> f32 {
            0.1
        }
    }

    fn test_config() -> SynthesisConfig {
        SynthesisConfig {
            preferred_model: PREFERRED_SYNTHESIS_MODEL.into(),
            preferred_path: "/models/35b.base".into(),
            fallback_model: FALLBACK_SYNTHESIS_MODEL.into(),
            fallback_path: "/models/4b.base".into(),
            cold_ready_timeout: Duration::from_millis(30),
            warm_timeout: Duration::from_millis(20),
            fallback_timeout: Duration::from_millis(30),
            idle_ttl: SYNTHESIS_MODEL_IDLE_TTL,
            maintenance_interval: Duration::from_secs(60),
        }
    }

    fn service(
        client: Arc<FakeModelClient>,
        clock: Arc<FakeClock>,
        pressure: Arc<FakePressure>,
    ) -> Arc<SynthesisService> {
        SynthesisService::new(client, clock, pressure, test_config())
    }

    #[test]
    fn production_policy_admits_only_preferred_35b_and_fallback_4b() {
        let config = SynthesisConfig::from_environment();
        assert_eq!(config.preferred_model, PREFERRED_SYNTHESIS_MODEL);
        assert_eq!(config.fallback_model, FALLBACK_SYNTHESIS_MODEL);
        assert!(!config.preferred_model.contains("8B"));
        assert!(!config.fallback_model.contains("8B"));
    }

    #[test]
    fn model_load_failures_have_a_normalized_unavailability_reason() {
        assert_eq!(
            normalized_failure_reason(
                "Failed to load model: Failed to open model: /private/path/model.base"
            ),
            "model_unavailable"
        );
    }

    #[tokio::test]
    async fn first_eligible_request_cold_loads_preferred_once() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("valid")]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert_eq!(result.route, SynthesisRoute::Preferred);
        assert_eq!(client.loads.load(Ordering::SeqCst), 1);
        assert_eq!(
            client.requests.lock().await[0].model_id,
            PREFERRED_SYNTHESIS_MODEL
        );
    }

    #[tokio::test]
    async fn cold_readiness_polling_uses_the_injected_clock() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("valid")]);
        client.readiness_polls.store(3, Ordering::SeqCst);
        let clock = Arc::new(FakeClock::default());
        let service = service(
            client.clone(),
            clock.clone(),
            Arc::new(FakePressure::default()),
        );

        let outcome = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(outcome.route, SynthesisRoute::Preferred);
        assert_eq!(client.loads.load(Ordering::SeqCst), 1);
        assert_eq!(clock.monotonic(), Duration::from_millis(200));
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_preferred_load() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("valid"),
            Behavior::Response("valid"),
        ]);
        *client.load_delay.lock().await = Some(Duration::from_millis(10));
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let contract = TestContract::default();
        let (left, right) = tokio::join!(
            service.synthesize(&contract, &NoopSynthesisObserver),
            service.synthesize(&contract, &NoopSynthesisObserver)
        );
        assert_eq!(left.route, SynthesisRoute::Preferred);
        assert_eq!(right.route, SynthesisRoute::Preferred);
        assert_eq!(client.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn warm_requests_reuse_preferred_model() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("valid"),
            Behavior::Response("valid"),
        ]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert_eq!(client.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn runtime_owned_fallback_is_unloaded_before_preferred_is_reloaded() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("fallback"),
            Behavior::Response("valid"),
        ]);
        *client.load_behavior.lock().await = Some(Behavior::Error("preferred unavailable"));
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );

        let fallback = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        let preferred = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(fallback.route, SynthesisRoute::Fallback);
        assert_eq!(preferred.route, SynthesisRoute::Preferred);
        assert!(client
            .unloads
            .lock()
            .await
            .iter()
            .any(|model| model == FALLBACK_SYNTHESIS_MODEL));
        let loaded = client.loaded.lock().await;
        assert!(loaded.contains(PREFERRED_SYNTHESIS_MODEL));
        assert!(!loaded.contains(FALLBACK_SYNTHESIS_MODEL));
    }

    #[tokio::test]
    async fn externally_loaded_fallback_is_unloaded_before_preferred_load() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("valid")]);
        client.set_loaded(FALLBACK_SYNTHESIS_MODEL).await;
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );

        let outcome = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(outcome.route, SynthesisRoute::Preferred);
        assert_eq!(
            client.unloads.lock().await.as_slice(),
            [FALLBACK_SYNTHESIS_MODEL]
        );
        let loaded = client.loaded.lock().await;
        assert!(loaded.contains(PREFERRED_SYNTHESIS_MODEL));
        assert!(!loaded.contains(FALLBACK_SYNTHESIS_MODEL));
    }

    #[tokio::test]
    async fn mail_and_web_contracts_use_the_same_synthesis_service() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("valid"),
            Behavior::Response("valid"),
        ]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let mail_contract = TestContract::default();
        let web_contract = TestContract::default();

        let mail = service
            .synthesize(&mail_contract, &NoopSynthesisObserver)
            .await;
        let web = service
            .synthesize(&web_contract, &NoopSynthesisObserver)
            .await;

        assert_eq!(mail.route, SynthesisRoute::Preferred);
        assert_eq!(web.route, SynthesisRoute::Preferred);
        assert_eq!(client.loads.load(Ordering::SeqCst), 1);
        assert_eq!(client.requests.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn idle_expiry_unloads_preferred_after_twenty_minutes() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("valid")]);
        let clock = Arc::new(FakeClock::default());
        let service = service(
            client.clone(),
            clock.clone(),
            Arc::new(FakePressure::default()),
        );
        service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        clock.advance(SYNTHESIS_MODEL_IDLE_TTL);
        service.maintain().await;
        assert_eq!(
            client.unloads.lock().await.as_slice(),
            [PREFERRED_SYNTHESIS_MODEL]
        );
    }

    #[tokio::test]
    async fn memory_pressure_unloads_idle_preferred() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("valid")]);
        let pressure = Arc::new(FakePressure::default());
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            pressure.clone(),
        );
        service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        pressure.0.store(true, Ordering::SeqCst);
        service.maintain().await;
        assert_eq!(
            client.unloads.lock().await.as_slice(),
            [PREFERRED_SYNTHESIS_MODEL]
        );
    }

    #[tokio::test]
    async fn cold_load_timeout_falls_back_once() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("fallback")]);
        *client.load_behavior.lock().await = Some(Behavior::Pending);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert_eq!(result.route, SynthesisRoute::Fallback);
        assert_eq!(client.requests.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn preferred_synthesis_timeout_falls_back_once() {
        let client =
            FakeModelClient::with_behaviors([Behavior::Pending, Behavior::Response("fallback")]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert_eq!(result.route, SynthesisRoute::Fallback);
        assert_eq!(client.requests.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn preferred_transport_or_unavailability_falls_back_once() {
        for error in ["transport failure", "model unavailable"] {
            let client = FakeModelClient::with_behaviors([
                Behavior::Error(error),
                Behavior::Response("fallback"),
            ]);
            let service = service(
                client.clone(),
                Arc::new(FakeClock::default()),
                Arc::new(FakePressure::default()),
            );
            let result = service
                .synthesize(&TestContract::default(), &NoopSynthesisObserver)
                .await;
            assert_eq!(result.route, SynthesisRoute::Fallback);
            assert_eq!(client.requests.lock().await.len(), 2);
        }
    }

    #[tokio::test]
    async fn correctable_invalid_output_performs_one_fresh_preferred_repair() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("invalid"),
            Behavior::Response("repaired"),
        ]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let contract = TestContract::default();
        let result = service.synthesize(&contract, &NoopSynthesisObserver).await;
        assert_eq!(result.route, SynthesisRoute::Repaired);
        assert_eq!(contract.repair_calls.load(Ordering::SeqCst), 1);
        let requests = client.requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert!(requests[1].messages[1]
            .content
            .contains("VALIDATION_ERRORS"));
    }

    #[tokio::test]
    async fn successful_repair_never_invokes_fallback() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("invalid"),
            Behavior::Response("repaired"),
        ]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert!(client
            .requests
            .lock()
            .await
            .iter()
            .all(|request| request.model_id == PREFERRED_SYNTHESIS_MODEL));
    }

    #[tokio::test]
    async fn failed_repair_uses_deterministic_rendering() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("invalid"),
            Behavior::Response("still invalid"),
        ]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert_eq!(
            result,
            SynthesisOutcome {
                text: "deterministic".into(),
                route: SynthesisRoute::Deterministic,
            }
        );
        assert_eq!(client.requests.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn invalid_fallback_uses_deterministic_rendering() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Error("transport"),
            Behavior::Response("invalid"),
        ]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(result.text, "deterministic");
    }

    #[tokio::test]
    async fn zero_usable_evidence_invokes_no_model() {
        let client = FakeModelClient::with_behaviors([]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let contract = TestContract {
            eligible: false,
            ..TestContract::default()
        };
        let result = service.synthesize(&contract, &NoopSynthesisObserver).await;
        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(client.loads.load(Ordering::SeqCst), 0);
        assert!(client.requests.lock().await.is_empty());
    }

    #[tokio::test]
    async fn every_request_is_fresh_system_user_and_tool_free_and_never_uses_8b() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("invalid"),
            Behavior::Error("transport"),
            Behavior::Response("fallback"),
        ]);
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        for request in client.requests.lock().await.iter() {
            validate_transcript(&request.messages).unwrap();
            assert!(!request.model_id.contains("8B"));
        }
    }

    #[tokio::test]
    async fn repair_and_fallback_reuse_bundle_without_evidence_reacquisition() {
        let repair_client = FakeModelClient::with_behaviors([
            Behavior::Response("invalid"),
            Behavior::Error("transport"),
        ]);
        let repair_service = service(
            repair_client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let repair_contract = TestContract::default();
        repair_service
            .synthesize(&repair_contract, &NoopSynthesisObserver)
            .await;
        assert_eq!(repair_contract.initial_calls.load(Ordering::SeqCst), 1);
        assert_eq!(repair_contract.repair_calls.load(Ordering::SeqCst), 1);
        let repair_requests = repair_client.requests.lock().await;
        assert_eq!(repair_requests.len(), 2);
        assert!(repair_requests.iter().all(|request| request.messages[1]
            .content
            .contains("EVIDENCE_BUNDLE={\"value\":\"bounded\"}")));
        drop(repair_requests);

        let fallback_client = FakeModelClient::with_behaviors([
            Behavior::Error("transport"),
            Behavior::Response("fallback"),
        ]);
        let fallback_service = service(
            fallback_client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let fallback_contract = TestContract::default();
        fallback_service
            .synthesize(&fallback_contract, &NoopSynthesisObserver)
            .await;
        assert_eq!(fallback_contract.initial_calls.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_contract.repair_calls.load(Ordering::SeqCst), 0);
        assert!(fallback_client
            .requests
            .lock()
            .await
            .iter()
            .all(|request| request.messages[1]
                .content
                .contains("EVIDENCE_BUNDLE={\"value\":\"bounded\"}")));
    }

    #[tokio::test]
    async fn phase_events_contain_only_sanitized_runtime_metadata() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("valid")]);
        let service = service(
            client,
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let observer = RecordingObserver::default();
        service
            .synthesize(&TestContract::default(), &observer)
            .await;
        let serialized = serde_json::to_string(&*observer.0.lock().await).unwrap();
        assert!(!serialized.contains("EVIDENCE_BUNDLE"));
        assert!(!serialized.contains("bounded"));
        assert!(serialized.contains("loading_model"));
        assert!(serialized.contains("synthesizing"));
        assert!(serialized.contains("validating"));
        assert!(serialized.contains("\"turn_id\":\"test-turn\""));
    }
}
