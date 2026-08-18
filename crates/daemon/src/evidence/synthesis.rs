use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bagentd::model_runtime::{ModelDemand, ModelRuntime};
use basert_connector::{BaseRtCompletionError, BaseRtRuntimeFault, Message};
use serde::Serialize;
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, Notify},
    task::JoinHandle,
};

use super::CanonicalGroundedAnswer;

pub(crate) const PREFERRED_SYNTHESIS_MODEL: &str = "basecompute/Qwen3.6-35B-A3B";
pub(crate) const SYNTHESIS_WARM_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const SYNTHESIS_MODEL_IDLE_TTL: Duration = Duration::from_secs(20 * 60);

#[derive(Debug, Clone)]
pub(crate) struct SynthesisConfig {
    pub preferred_model: String,
    pub warm_timeout: Duration,
    pub maintenance_interval: Duration,
}

impl SynthesisConfig {
    pub(crate) fn from_environment() -> Self {
        Self {
            preferred_model: PREFERRED_SYNTHESIS_MODEL.to_string(),
            warm_timeout: SYNTHESIS_WARM_TIMEOUT,
            maintenance_interval: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SynthesisPhase {
    LoadingSynthesisModel,
    PreparingAnswer,
    Repairing,
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

pub(crate) struct SynthesisService {
    runtime: Arc<ModelRuntime>,
    config: SynthesisConfig,
    maintenance_stop: Notify,
    maintenance_task: Mutex<Option<JoinHandle<()>>>,
}

impl SynthesisService {
    pub(crate) fn new(runtime: Arc<ModelRuntime>, config: SynthesisConfig) -> Arc<Self> {
        Arc::new(Self {
            runtime,
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
                    _ = tokio::time::sleep(interval) => { let _ = service.runtime.maintain().await; },
                }
            }
        }));
    }

    pub(crate) async fn shutdown(&self) -> Result<()> {
        self.maintenance_stop.notify_one();
        if let Some(task) = self.maintenance_task.lock().await.take() {
            let _ = task.await;
        }
        self.runtime.shutdown().await
    }

    pub(crate) async fn maintain(&self) {
        let _ = self.runtime.maintain().await;
    }

    pub(crate) async fn synthesize(
        self: &Arc<Self>,
        demand: ModelDemand,
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
        let preferred = self
            .complete_with_phase(
                demand.clone(),
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
                                demand.clone(),
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
        demand: ModelDemand,
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
        match tokio::time::timeout(
            timeout,
            self.runtime.complete_bounded(
                demand,
                messages,
                contract.temperature(),
                contract.max_tokens(),
            ),
        )
        .await
        {
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
    use bagentd::model_runtime::{ModelClass, ModelRuntimeAdapter, RuntimeAction, WorkIdentity};

    struct LowMemoryAdapter;

    #[async_trait]
    impl ModelRuntimeAdapter for LowMemoryAdapter {
        async fn perform(&self, _action: RuntimeAction) -> Result<()> {
            Ok(())
        }

        async fn memory_headroom(&self) -> Result<(u64, u64)> {
            Ok((24, 16 * 1024 * 1024 * 1024))
        }
    }

    struct Contract;

    impl SynthesisContract for Contract {
        fn turn_id(&self) -> &str {
            "memory-ineligible"
        }

        fn eligible(&self) -> bool {
            true
        }

        fn initial_request(&self) -> Vec<Message> {
            vec![Message::system("polish"), Message::user("answer")]
        }

        fn repair_request(&self, _validation_errors: &[String]) -> Vec<Message> {
            self.initial_request()
        }

        fn validate(&self, _response: &str) -> std::result::Result<(), Vec<String>> {
            Ok(())
        }

        fn canonical_answer(&self) -> CanonicalGroundedAnswer {
            CanonicalGroundedAnswer {
                text: "deterministic answer".into(),
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
            32
        }

        fn temperature(&self) -> f32 {
            0.0
        }
    }

    #[tokio::test]
    async fn failed_35b_admission_renders_the_canonical_answer_directly() {
        let runtime = ModelRuntime::production(Arc::new(LowMemoryAdapter));
        runtime.initialize().await.expect("clean runtime boundary");
        let service = SynthesisService::new(
            runtime,
            SynthesisConfig {
                preferred_model: PREFERRED_SYNTHESIS_MODEL.into(),
                warm_timeout: Duration::from_secs(1),
                maintenance_interval: Duration::from_secs(60),
            },
        );

        let outcome = service
            .synthesize(
                ModelDemand::automation(
                    WorkIdentity::new("memory-ineligible"),
                    ModelClass::Synthesis35B,
                ),
                &Contract,
                &NoopSynthesisObserver,
            )
            .await;

        assert_eq!(outcome.route, SynthesisRoute::Deterministic);
        assert_eq!(outcome.text, "deterministic answer");
        assert_eq!(outcome.polish_status, PolishStatus::Unavailable);
    }
}
