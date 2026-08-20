use anyhow::{anyhow, Result};
use async_trait::async_trait;
use basert_connector::{
    BaseRtClient, BaseRtCompletionError, BaseRtLogCheckpoint, BaseRtRuntimeFault, Message,
    ModelInfo, ModelLoadRequest,
};
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

use super::CanonicalGroundedAnswer;

pub(crate) const PREFERRED_SYNTHESIS_MODEL: &str = "basecompute/Qwen3.6-35B-A3B";
pub(crate) const FALLBACK_SYNTHESIS_MODEL: &str = "basecompute/Qwen3-4B-Instruct-2507";
pub(crate) const SYNTHESIS_MODEL_IDLE_TTL: Duration = Duration::from_secs(20 * 60);
pub(crate) const SYNTHESIS_COLD_READY_TIMEOUT: Duration = Duration::from_secs(45);
pub(crate) const SYNTHESIS_WARM_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const SYNTHESIS_FALLBACK_TIMEOUT: Duration = Duration::from_secs(20);
const PREFERRED_MIN_FREE_PERCENT: u64 = 25;
const PREFERRED_MIN_AVAILABLE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

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
    async fn restart_runtime(&self) -> Result<()> {
        Err(anyhow!("BaseRT runtime restart is unsupported"))
    }
    async fn runtime_fault_checkpoint(&self) -> Option<BaseRtLogCheckpoint> {
        None
    }
    async fn detect_runtime_fault_since(
        &self,
        _checkpoint: Option<BaseRtLogCheckpoint>,
        _wait: Duration,
    ) -> Option<BaseRtRuntimeFault> {
        None
    }
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

    async fn restart_runtime(&self) -> Result<()> {
        super::runtime_control::restart_managed_basert(self).await
    }

    async fn runtime_fault_checkpoint(&self) -> Option<BaseRtLogCheckpoint> {
        BaseRtClient::runtime_log_checkpoint(self).await
    }

    async fn detect_runtime_fault_since(
        &self,
        checkpoint: Option<BaseRtLogCheckpoint>,
        wait: Duration,
    ) -> Option<BaseRtRuntimeFault> {
        BaseRtClient::detect_runtime_fault_since(self, checkpoint, wait).await
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
                return true;
            };
            let report = String::from_utf8_lossy(&output.stdout);
            let normalized = report.to_ascii_lowercase();
            if normalized.contains("critical") || normalized.contains("warn") {
                return true;
            }
            !preferred_memory_admitted(&report)
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
}

fn preferred_memory_admitted(report: &str) -> bool {
    let physical_bytes = report
        .lines()
        .find_map(|line| line.trim().strip_prefix("The system has "))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse::<u64>().ok());
    let free_percent = report
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("System-wide memory free percentage:")
        })
        .and_then(|rest| rest.trim().trim_end_matches('%').parse::<u64>().ok());
    let (Some(physical_bytes), Some(free_percent)) = (physical_bytes, free_percent) else {
        return false;
    };
    let available_bytes = physical_bytes.saturating_mul(free_percent) / 100;
    free_percent >= PREFERRED_MIN_FREE_PERCENT && available_bytes >= PREFERRED_MIN_AVAILABLE_BYTES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SynthesisPhase {
    LoadingSynthesisModel,
    PreparingAnswer,
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
    fn validate_polish(
        &self,
        response: &str,
        _canonical: &CanonicalGroundedAnswer,
    ) -> std::result::Result<(), Vec<String>> {
        self.validate(response)
    }
    fn render_validated(&self, response: &str) -> std::result::Result<String, Vec<String>> {
        self.validate(response)?;
        Ok(response.to_string())
    }
    fn canonical_answer(&self) -> CanonicalGroundedAnswer;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PolishStatus {
    Skipped,
    Accepted,
    Rejected,
    TimedOut,
    Unavailable,
    MemoryIneligible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SynthesisOutcome {
    pub text: String,
    pub route: SynthesisRoute,
    pub polish_status: PolishStatus,
}

#[derive(Debug, Default)]
struct RuntimeState {
    preferred_last_use: Option<Duration>,
    preferred_active: usize,
    fallback_owned: bool,
    poisoned: bool,
}

pub(crate) struct ModelRuntimeManager {
    client: Arc<dyn SynthesisModelClient>,
    clock: Arc<dyn SynthesisClock>,
    pressure: Arc<dyn MemoryPressureSignal>,
    config: SynthesisConfig,
    lifecycle_lock: Mutex<()>,
    preferred_idle: Notify,
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
        let became_idle = state.preferred_active == 0;
        drop(state);
        if became_idle {
            self.runtime.preferred_idle.notify_waiters();
        }
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
            preferred_idle: Notify::new(),
            state: StdMutex::new(RuntimeState::default()),
        })
    }

    async fn preferred_lease(
        self: &Arc<Self>,
        observer: &dyn RuntimeObserver,
    ) -> Result<PreferredLease> {
        let started = self.clock.monotonic();
        let _lifecycle = self.lifecycle_lock.lock().await;
        if self.state.lock().expect("runtime state lock").poisoned {
            return Err(anyhow!("BaseRT runtime poisoned"));
        }
        let loaded = match self.model_loaded(&self.config.preferred_model).await {
            Ok(loaded) => loaded,
            Err(error) => {
                observer
                    .record(phase_event(
                        Some(&self.config.preferred_model),
                        SynthesisPhase::LoadingSynthesisModel,
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
            if self.pressure.under_pressure().await {
                observer
                    .record(phase_event(
                        Some(&self.config.preferred_model),
                        SynthesisPhase::LoadingSynthesisModel,
                        elapsed_ms(self.clock.monotonic(), started),
                        false,
                        false,
                        false,
                        Some("memory_pressure"),
                    ))
                    .await;
                return Err(anyhow!("memory pressure"));
            }
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
                    SynthesisPhase::LoadingSynthesisModel,
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
            let fault_checkpoint = self.client.runtime_fault_checkpoint().await;
            let readiness = async {
                self.client.load_model(&request).await?;
                self.wait_until_loaded(&self.config.preferred_model).await
            };
            match tokio::time::timeout(self.config.cold_ready_timeout, readiness).await {
                Err(_) => {
                    if let Some(fault) = self
                        .client
                        .detect_runtime_fault_since(fault_checkpoint, Duration::from_millis(250))
                        .await
                    {
                        self.mark_poisoned();
                        observer
                            .record(phase_event(
                                Some(&self.config.preferred_model),
                                SynthesisPhase::LoadingSynthesisModel,
                                elapsed_ms(self.clock.monotonic(), started),
                                false,
                                false,
                                false,
                                Some(fault.category()),
                            ))
                            .await;
                        return Err(anyhow!(BaseRtCompletionError::RuntimeFault(fault)));
                    }
                    // The server-side load may outlive cancellation of the
                    // HTTP future. Treat an unproven timeout as an unhealthy
                    // process boundary so fallback cannot overlap it.
                    self.mark_poisoned();
                    observer
                        .record(phase_event(
                            Some(&self.config.preferred_model),
                            SynthesisPhase::LoadingSynthesisModel,
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
                    let failure = ModelFailure::from_error(&error);
                    if failure.is_poisoning() {
                        self.mark_poisoned();
                    }
                    observer
                        .record(phase_event(
                            Some(&self.config.preferred_model),
                            SynthesisPhase::LoadingSynthesisModel,
                            elapsed_ms(self.clock.monotonic(), started),
                            false,
                            false,
                            false,
                            Some(failure.category()),
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
                    SynthesisPhase::LoadingSynthesisModel,
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
        loop {
            let idle = self.preferred_idle.notified();
            if self
                .state
                .lock()
                .expect("runtime state lock")
                .preferred_active
                == 0
            {
                break;
            }
            idle.await;
        }
        let poisoned = self.state.lock().expect("runtime state lock").poisoned;
        if poisoned {
            self.client.restart_runtime().await?;
            let mut state = self.state.lock().expect("runtime state lock");
            state.preferred_last_use = None;
            state.fallback_owned = false;
            state.poisoned = false;
        } else {
            self.retire_preferred_if_idle().await?;
        }
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

    fn mark_poisoned(&self) {
        self.state.lock().expect("runtime state lock").poisoned = true;
    }

    async fn recover_poisoned(&self) {
        if !self.state.lock().expect("runtime state lock").poisoned {
            return;
        }
        let _lifecycle = self.lifecycle_lock.lock().await;
        if self.client.restart_runtime().await.is_ok() {
            let mut state = self.state.lock().expect("runtime state lock");
            state.preferred_last_use = None;
            state.fallback_owned = false;
            state.poisoned = false;
        }
    }

    async fn retire_preferred_if_idle(&self) -> Result<()> {
        let active = self
            .state
            .lock()
            .expect("runtime state lock")
            .preferred_active;
        if active > 0 {
            return Ok(());
        }
        if self
            .model_loaded(&self.config.preferred_model)
            .await
            .unwrap_or(false)
        {
            self.client
                .unload_model(&self.config.preferred_model)
                .await?;
            // BaseRT can retain Metal/wired allocations after API unload while
            // reporting zero loaded weights and very low RSS. A process restart
            // is the only observed clean boundary before another residency.
            self.client.restart_runtime().await?;
        }
        self.state
            .lock()
            .expect("runtime state lock")
            .preferred_last_use = None;
        Ok(())
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
        if active == 0 && idle_expired {
            let _ = self.retire_preferred_if_idle().await;
        }
    }

    pub(crate) async fn shutdown(&self) {
        let _lifecycle = self.lifecycle_lock.lock().await;
        let _ = self.retire_preferred_if_idle().await;
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
        // The canonical answer is complete before model admission. It remains
        // the byte-for-byte terminal answer unless optional polish is accepted.
        let canonical = contract.canonical_answer();
        let canonical_text = canonical.text.clone();
        if !contract.eligible() {
            return self
                .canonical(canonical_text, observer, PolishStatus::Skipped, None)
                .await;
        }
        let initial = contract.initial_request();
        if validate_transcript(&initial).is_err() {
            return self
                .canonical(
                    canonical_text,
                    observer,
                    PolishStatus::Rejected,
                    Some("invalid_transcript"),
                )
                .await;
        }
        let lease = match self.runtime.preferred_lease(observer).await {
            Ok(lease) => lease,
            Err(error) => {
                let status = if normalized_failure_reason(&error.to_string()) == "memory_pressure" {
                    PolishStatus::MemoryIneligible
                } else {
                    PolishStatus::Unavailable
                };
                self.runtime.recover_poisoned().await;
                return self
                    .canonical(
                        canonical_text,
                        observer,
                        status,
                        Some(normalized_failure_reason(&error.to_string())),
                    )
                    .await;
            }
        };
        let preferred = self
            .complete_with_phase(
                SynthesisPhase::PreparingAnswer,
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
                let validation = self
                    .validate(contract, &response, &canonical, observer, false)
                    .await;
                match validation {
                    Ok(()) => match contract.render_validated(&response) {
                        Ok(text) => SynthesisOutcome {
                            text,
                            route: SynthesisRoute::Preferred,
                            polish_status: PolishStatus::Accepted,
                        },
                        Err(_) => {
                            self.canonical(
                                canonical_text.clone(),
                                observer,
                                PolishStatus::Rejected,
                                Some("validated_render_failed"),
                            )
                            .await
                        }
                    },
                    Err(errors) => {
                        let repair = contract.repair_request(&errors);
                        if validate_transcript(&repair).is_err() {
                            drop(lease);
                            return self
                                .canonical(
                                    canonical_text,
                                    observer,
                                    PolishStatus::Rejected,
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
                                match self
                                    .validate(contract, &response, &canonical, observer, true)
                                    .await
                                {
                                    Ok(()) => match contract.render_validated(&response) {
                                        Ok(text) => SynthesisOutcome {
                                            text,
                                            route: SynthesisRoute::Repaired,
                                            polish_status: PolishStatus::Accepted,
                                        },
                                        Err(_) => {
                                            self.canonical(
                                                canonical_text.clone(),
                                                observer,
                                                PolishStatus::Rejected,
                                                Some("validated_render_failed"),
                                            )
                                            .await
                                        }
                                    },
                                    Err(_) => {
                                        self.canonical(
                                            canonical_text.clone(),
                                            observer,
                                            PolishStatus::Rejected,
                                            Some("repair_validation_failed"),
                                        )
                                        .await
                                    }
                                }
                            }
                            CompletionAttempt::Failed(failure) => {
                                if failure.is_poisoning() {
                                    self.runtime.mark_poisoned();
                                    self.runtime.recover_poisoned().await;
                                }
                                let status =
                                    if matches!(failure, ModelFailure::IndeterminateTimeout) {
                                        PolishStatus::TimedOut
                                    } else {
                                        PolishStatus::Unavailable
                                    };
                                self.canonical(
                                    canonical_text,
                                    observer,
                                    status,
                                    Some(failure.category()),
                                )
                                .await
                            }
                        }
                    }
                }
            }
            CompletionAttempt::Failed(failure) => {
                drop(lease);
                if failure.is_poisoning() {
                    self.runtime.mark_poisoned();
                    self.runtime.recover_poisoned().await;
                }
                let status = if matches!(failure, ModelFailure::IndeterminateTimeout) {
                    PolishStatus::TimedOut
                } else {
                    PolishStatus::Unavailable
                };
                self.canonical(canonical_text, observer, status, Some(failure.category()))
                    .await
            }
        }
    }

    async fn validate(
        &self,
        contract: &dyn SynthesisContract,
        response: &str,
        canonical: &CanonicalGroundedAnswer,
        observer: &dyn RuntimeObserver,
        repair: bool,
    ) -> std::result::Result<(), Vec<String>> {
        let started = Instant::now();
        let result = contract.validate_polish(response, canonical);
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
        let fault_checkpoint = self.client.runtime_fault_checkpoint().await;
        match tokio::time::timeout(timeout, self.client.complete(request)).await {
            Err(_) => {
                if let Some(fault) = self
                    .client
                    .detect_runtime_fault_since(fault_checkpoint, Duration::from_millis(250))
                    .await
                {
                    observer
                        .record(phase_event(
                            Some(model),
                            phase,
                            duration_ms(started.elapsed()),
                            false,
                            fallback,
                            repair,
                            Some(fault.category()),
                        ))
                        .await;
                    return CompletionAttempt::Failed(ModelFailure::Poisoned(fault));
                }
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
                CompletionAttempt::Failed(ModelFailure::IndeterminateTimeout)
            }
            Ok(Err(error)) => {
                let failure = ModelFailure::from_error(&error);
                observer
                    .record(phase_event(
                        Some(model),
                        phase,
                        duration_ms(started.elapsed()),
                        false,
                        fallback,
                        repair,
                        Some(failure.category()),
                    ))
                    .await;
                CompletionAttempt::Failed(failure)
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

    async fn canonical(
        &self,
        text: String,
        observer: &dyn RuntimeObserver,
        polish_status: PolishStatus,
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
        SynthesisOutcome {
            text,
            route: SynthesisRoute::Deterministic,
            polish_status,
        }
    }
}

enum CompletionAttempt {
    Completed(String),
    Failed(ModelFailure),
}

enum ModelFailure {
    Poisoned(BaseRtRuntimeFault),
    IndeterminateTimeout,
    Reason(&'static str),
}

impl ModelFailure {
    fn from_error(error: &anyhow::Error) -> Self {
        match error.downcast_ref::<BaseRtCompletionError>() {
            Some(BaseRtCompletionError::RuntimeFault(fault)) => Self::Poisoned(*fault),
            Some(BaseRtCompletionError::Truncated) => Self::Reason("truncated"),
            Some(BaseRtCompletionError::Empty) => Self::Reason("empty"),
            None => Self::Reason(normalized_failure_reason(&error.to_string())),
        }
    }

    fn category(&self) -> &'static str {
        match self {
            Self::Poisoned(fault) => fault.category(),
            Self::IndeterminateTimeout => "timeout",
            Self::Reason(reason) => reason,
        }
    }

    fn is_poisoning(&self) -> bool {
        matches!(self, Self::Poisoned(_) | Self::IndeterminateTimeout)
    }
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
    if normalized.contains("truncated") {
        "truncated"
    } else if normalized.contains("empty completion") {
        "empty"
    } else if normalized.contains("timeout") || normalized.contains("timed out") {
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
        RuntimeFault(BaseRtRuntimeFault),
        Pending,
    }

    #[derive(Default)]
    struct FakeModelClient {
        loaded: Mutex<HashSet<String>>,
        behaviors: Mutex<VecDeque<Behavior>>,
        requests: Mutex<Vec<SynthesisModelRequest>>,
        loads: AtomicUsize,
        restarts: AtomicUsize,
        restart_behavior: Mutex<Option<Behavior>>,
        timeout_fault: Mutex<Option<BaseRtRuntimeFault>>,
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
                Some(Behavior::RuntimeFault(fault)) => {
                    return Err(anyhow!(BaseRtCompletionError::RuntimeFault(fault)));
                }
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

        async fn restart_runtime(&self) -> Result<()> {
            self.restarts.fetch_add(1, Ordering::SeqCst);
            if let Some(Behavior::Error(error)) = self.restart_behavior.lock().await.take() {
                return Err(anyhow!(error));
            }
            self.loaded.lock().await.clear();
            Ok(())
        }

        async fn runtime_fault_checkpoint(&self) -> Option<BaseRtLogCheckpoint> {
            None
        }

        async fn detect_runtime_fault_since(
            &self,
            _checkpoint: Option<BaseRtLogCheckpoint>,
            _wait: Duration,
        ) -> Option<BaseRtRuntimeFault> {
            self.timeout_fault.lock().await.take()
        }

        async fn complete(&self, request: SynthesisModelRequest) -> Result<String> {
            self.requests.lock().await.push(request);
            match self.behaviors.lock().await.pop_front() {
                Some(Behavior::Response(response)) => Ok(response.to_string()),
                Some(Behavior::Error(error)) => Err(anyhow!(error)),
                Some(Behavior::RuntimeFault(fault)) => {
                    Err(anyhow!(BaseRtCompletionError::RuntimeFault(fault)))
                }
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

        fn canonical_answer(&self) -> CanonicalGroundedAnswer {
            CanonicalGroundedAnswer {
                text: "deterministic".into(),
                completeness: super::super::Completeness::Complete,
                outcome_status: super::super::CanonicalOutcomeStatus::Verified,
                covered_evidence_ids: Vec::new(),
                citation_targets: Vec::new(),
                conflicts: Vec::new(),
                shortfalls: Vec::new(),
                source_identities: Vec::new(),
            }
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

    #[test]
    fn typed_completion_failures_have_specific_reasons() {
        assert_eq!(
            ModelFailure::from_error(&anyhow!(BaseRtCompletionError::RuntimeFault(
                BaseRtRuntimeFault::MetalOutOfMemory
            )))
            .category(),
            "metal_oom"
        );
        assert_eq!(
            ModelFailure::from_error(&anyhow!(BaseRtCompletionError::RuntimeFault(
                BaseRtRuntimeFault::MetalCommandBuffer
            )))
            .category(),
            "metal_command_buffer"
        );
        assert_eq!(
            ModelFailure::from_error(&anyhow!(BaseRtCompletionError::RuntimeFault(
                BaseRtRuntimeFault::MetalDevice
            )))
            .category(),
            "metal_device"
        );
        assert_eq!(
            ModelFailure::from_error(&anyhow!(BaseRtCompletionError::Truncated)).category(),
            "truncated"
        );
        assert_eq!(
            ModelFailure::from_error(&anyhow!(BaseRtCompletionError::Empty)).category(),
            "empty"
        );
    }

    #[test]
    fn preferred_memory_admission_requires_percentage_and_absolute_headroom() {
        let safe = "The system has 34359738368 (2097152 pages with a page size of 16384).\n\
                    System-wide memory free percentage: 30%";
        let low_percentage =
            "The system has 34359738368 (2097152 pages with a page size of 16384).\n\
                              System-wide memory free percentage: 24%";
        let low_absolute =
            "The system has 17179869184 (1048576 pages with a page size of 16384).\n\
                            System-wide memory free percentage: 30%";
        assert!(preferred_memory_admitted(safe));
        assert!(!preferred_memory_admitted(low_percentage));
        assert!(!preferred_memory_admitted(low_absolute));
        assert!(!preferred_memory_admitted("unparseable"));
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
        assert_eq!(result.text, "valid");
        assert_eq!(result.polish_status, PolishStatus::Accepted);
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
    async fn fallback_waits_for_every_concurrent_preferred_lease_before_switching_models() {
        let client = FakeModelClient::with_behaviors([]);
        client.set_loaded(PREFERRED_SYNTHESIS_MODEL).await;
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        let observer = CorrelatedObserver {
            turn_id: "concurrent-fallback",
            inner: &NoopSynthesisObserver,
        };
        let first = service.runtime.preferred_lease(&observer).await.unwrap();
        let second = service.runtime.preferred_lease(&observer).await.unwrap();
        drop(first);

        let runtime = service.runtime.clone();
        let switching = tokio::spawn(async move { runtime.ensure_fallback().await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(
            client.unloads.lock().await.is_empty(),
            "preferred model unloaded while a concurrent request was active"
        );
        assert!(!client
            .loaded
            .lock()
            .await
            .contains(FALLBACK_SYNTHESIS_MODEL));

        drop(second);
        switching.await.unwrap().unwrap();
        assert_eq!(
            client.unloads.lock().await.as_slice(),
            [PREFERRED_SYNTHESIS_MODEL]
        );
        assert_eq!(client.restarts.load(Ordering::SeqCst), 1);
        let loaded = client.loaded.lock().await;
        assert!(!loaded.contains(PREFERRED_SYNTHESIS_MODEL));
        assert!(loaded.contains(FALLBACK_SYNTHESIS_MODEL));
    }

    #[tokio::test]
    async fn unavailable_polish_preserves_canonical_without_loading_fallback() {
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

        assert_eq!(fallback.route, SynthesisRoute::Deterministic);
        assert_eq!(fallback.polish_status, PolishStatus::Unavailable);
        assert_eq!(preferred.route, SynthesisRoute::Preferred);
        assert!(client.unloads.lock().await.is_empty());
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
        assert_eq!(client.restarts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn load_admission_pressure_does_not_evict_an_existing_preferred_residency() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("valid"),
            Behavior::Response("valid"),
        ]);
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
        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert_eq!(result.route, SynthesisRoute::Preferred);
        assert!(client.unloads.lock().await.is_empty());
        assert_eq!(client.restarts.load(Ordering::SeqCst), 0);
        assert_eq!(client.loads.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn memory_pressure_preserves_canonical_without_model_request() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("fallback")]);
        let pressure = Arc::new(FakePressure(AtomicBool::new(true)));
        let service = service(client.clone(), Arc::new(FakeClock::default()), pressure);
        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;
        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(result.text, "deterministic");
        assert_eq!(result.polish_status, PolishStatus::MemoryIneligible);
        assert_eq!(client.loads.load(Ordering::SeqCst), 0);
        let requests = client.requests.lock().await;
        assert!(requests.is_empty());
    }

    #[tokio::test]
    async fn cold_load_timeout_preserves_canonical() {
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
        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(result.text, "deterministic");
        assert_eq!(client.requests.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn preferred_synthesis_timeout_preserves_canonical() {
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
        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(result.polish_status, PolishStatus::TimedOut);
        assert_eq!(client.requests.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn preferred_transport_or_unavailability_preserves_canonical() {
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
            assert_eq!(result.route, SynthesisRoute::Deterministic);
            assert_eq!(result.text, "deterministic");
            assert_eq!(client.requests.lock().await.len(), 1);
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
                polish_status: PolishStatus::Rejected,
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
    async fn failed_polish_preserves_canonical_without_invoking_fallback() {
        let client =
            FakeModelClient::with_behaviors([Behavior::Error("transport"), Behavior::Pending]);
        *client.timeout_fault.lock().await = Some(BaseRtRuntimeFault::MetalDevice);
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
        assert_eq!(client.requests.lock().await.len(), 1);
    }

    #[tokio::test]
    async fn metal_failure_restarts_runtime_before_single_bounded_fallback() {
        let client = FakeModelClient::with_behaviors([
            Behavior::RuntimeFault(BaseRtRuntimeFault::MetalOutOfMemory),
            Behavior::Response("fallback"),
        ]);
        client.set_loaded(PREFERRED_SYNTHESIS_MODEL).await;
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );

        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(client.restarts.load(Ordering::SeqCst), 1);
        let requests = client.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model_id, PREFERRED_SYNTHESIS_MODEL);
    }

    #[tokio::test]
    async fn metal_failure_during_preferred_load_restarts_before_fallback() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("fallback")]);
        *client.load_behavior.lock().await =
            Some(Behavior::RuntimeFault(BaseRtRuntimeFault::MetalOutOfMemory));
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );

        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(client.restarts.load(Ordering::SeqCst), 1);
        let requests = client.requests.lock().await;
        assert!(requests.is_empty());
    }

    #[tokio::test]
    async fn delayed_metal_fault_after_completion_timeout_restarts_before_fallback() {
        let client =
            FakeModelClient::with_behaviors([Behavior::Pending, Behavior::Response("fallback")]);
        *client.timeout_fault.lock().await = Some(BaseRtRuntimeFault::MetalCommandBuffer);
        client.set_loaded(PREFERRED_SYNTHESIS_MODEL).await;
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );

        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(client.restarts.load(Ordering::SeqCst), 1);
        let requests = client.requests.lock().await;
        assert_eq!(requests.len(), 1);
    }

    #[tokio::test]
    async fn metal_failure_during_repair_restarts_before_fallback() {
        let client = FakeModelClient::with_behaviors([
            Behavior::Response("invalid"),
            Behavior::RuntimeFault(BaseRtRuntimeFault::MetalCommandBuffer),
            Behavior::Response("fallback"),
        ]);
        client.set_loaded(PREFERRED_SYNTHESIS_MODEL).await;
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );

        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(client.restarts.load(Ordering::SeqCst), 1);
        let requests = client.requests.lock().await;
        assert_eq!(requests.len(), 2);
    }

    #[tokio::test]
    async fn poisoned_state_suppresses_preferred_request_until_restart() {
        let client = FakeModelClient::with_behaviors([Behavior::Response("fallback")]);
        client.set_loaded(PREFERRED_SYNTHESIS_MODEL).await;
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );
        service.runtime.mark_poisoned();

        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(client.restarts.load(Ordering::SeqCst), 1);
        let requests = client.requests.lock().await;
        assert!(requests.is_empty());
    }

    #[tokio::test]
    async fn failed_poisoned_restart_suppresses_fallback_request() {
        let client = FakeModelClient::with_behaviors([Behavior::RuntimeFault(
            BaseRtRuntimeFault::MetalOutOfMemory,
        )]);
        client.set_loaded(PREFERRED_SYNTHESIS_MODEL).await;
        *client.restart_behavior.lock().await =
            Some(Behavior::Error("runtime restart unavailable"));
        let service = service(
            client.clone(),
            Arc::new(FakeClock::default()),
            Arc::new(FakePressure::default()),
        );

        let result = service
            .synthesize(&TestContract::default(), &NoopSynthesisObserver)
            .await;

        assert_eq!(result.route, SynthesisRoute::Deterministic);
        assert_eq!(client.restarts.load(Ordering::SeqCst), 1);
        let requests = client.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model_id, PREFERRED_SYNTHESIS_MODEL);
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
        assert!(serialized.contains("loading_synthesis_model"));
        assert!(serialized.contains("preparing_answer"));
        assert!(serialized.contains("validating"));
        assert!(serialized.contains("\"turn_id\":\"test-turn\""));
    }
}
