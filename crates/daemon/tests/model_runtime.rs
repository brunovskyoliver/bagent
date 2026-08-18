use anyhow::Result;
use async_trait::async_trait;
use bagentd::model_runtime::{
    CompletionFormat, DemandPriority, ModelClass, ModelDemand, ModelRuntime, ModelRuntimeAdapter,
    RuntimeAction, RuntimeClock, RuntimeFault, RuntimePhase, WorkIdentity,
};
use basert_connector::Message;
use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr, TcpListener as StdTcpListener},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Barrier, Notify};

static EXACT_PORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn reserve_isolated_port(address: SocketAddr) -> StdTcpListener {
    let mut last_error = None;
    for _ in 0..200 {
        match StdTcpListener::bind(address) {
            Ok(listener) => return listener,
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "isolated {address} must become free: {}",
        last_error.expect("bind attempted")
    );
}

struct RecordingAdapter {
    actions: Mutex<Vec<RuntimeAction>>,
    headroom: Mutex<(u64, u64)>,
    memory_checks: Mutex<usize>,
}

impl Default for RecordingAdapter {
    fn default() -> Self {
        Self {
            actions: Mutex::new(Vec::new()),
            headroom: Mutex::new((100, u64::MAX)),
            memory_checks: Mutex::new(0),
        }
    }
}

#[async_trait]
impl ModelRuntimeAdapter for RecordingAdapter {
    fn recorded_actions(&self) -> Vec<RuntimeAction> {
        self.actions.lock().expect("actions lock").clone()
    }

    async fn perform(&self, action: RuntimeAction) -> Result<()> {
        self.actions.lock().expect("actions lock").push(action);
        Ok(())
    }

    async fn memory_headroom(&self) -> Result<(u64, u64)> {
        *self.memory_checks.lock().expect("memory checks") += 1;
        Ok(*self.headroom.lock().expect("headroom"))
    }
}

impl RecordingAdapter {
    fn clear(&self) {
        self.actions.lock().expect("actions lock").clear();
    }

    fn set_headroom(&self, free_percent: u64, available: u64) {
        *self.headroom.lock().expect("headroom") = (free_percent, available);
    }

    fn memory_checks(&self) -> usize {
        *self.memory_checks.lock().expect("memory checks")
    }
}

struct BlockingAdapter {
    blocked_action: RuntimeAction,
    entered: Barrier,
    release: Notify,
    actions: Mutex<Vec<RuntimeAction>>,
    completions: Mutex<Vec<ModelClass>>,
}

impl BlockingAdapter {
    fn new(blocked_action: RuntimeAction) -> Arc<Self> {
        Arc::new(Self {
            blocked_action,
            entered: Barrier::new(2),
            release: Notify::new(),
            actions: Mutex::new(Vec::new()),
            completions: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl ModelRuntimeAdapter for BlockingAdapter {
    fn recorded_actions(&self) -> Vec<RuntimeAction> {
        self.actions.lock().expect("actions lock").clone()
    }

    async fn perform(&self, action: RuntimeAction) -> Result<()> {
        self.actions.lock().expect("actions lock").push(action);
        if action == self.blocked_action {
            self.entered.wait().await;
            self.release.notified().await;
        }
        Ok(())
    }

    async fn complete_bounded(
        &self,
        model: ModelClass,
        _messages: Vec<Message>,
        _temperature: f32,
        _max_tokens: u32,
        _format: CompletionFormat,
    ) -> Result<String> {
        self.completions
            .lock()
            .expect("completions lock")
            .push(model);
        anyhow::bail!("controlled completion stop")
    }
}

#[derive(Default)]
struct FakeClock {
    now: Mutex<Duration>,
}

impl RuntimeClock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().expect("clock lock")
    }
}

impl FakeClock {
    fn set(&self, now: Duration) {
        *self.now.lock().expect("clock lock") = now;
    }
}

struct SubprocessAdapter {
    address: SocketAddr,
    child: Mutex<Option<Child>>,
    previous_pid: Mutex<Option<u32>>,
    actions: Mutex<Vec<RuntimeAction>>,
}

struct SentinelProcess {
    address: SocketAddr,
    child: Child,
}

impl SentinelProcess {
    async fn start(address: SocketAddr) -> Self {
        let child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--ignored",
                "--exact",
                "port_sentinel_process",
                "--nocapture",
            ])
            .env("BAGENT_MODEL_RUNTIME_SENTINEL", address.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn port sentinel");
        let sentinel = Self { address, child };
        for _ in 0..100 {
            if sentinel.state().await.is_ok() {
                return sentinel;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("port sentinel did not become healthy");
    }

    async fn state(&self) -> Result<serde_json::Value> {
        Ok(reqwest::get(format!("http://{}/state", self.address))
            .await?
            .json()
            .await?)
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for SentinelProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn protected_listener_pids(port: u16) -> String {
    let output = Command::new("lsof")
        .args([
            "-nP",
            &format!("-iTCP@127.0.0.1:{port}"),
            "-sTCP:LISTEN",
            "-t",
        ])
        .output()
        .expect("inspect protected listener");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

impl SubprocessAdapter {
    async fn start(address: SocketAddr) -> Arc<Self> {
        let adapter = Arc::new(Self {
            address,
            child: Mutex::new(None),
            previous_pid: Mutex::new(None),
            actions: Mutex::new(Vec::new()),
        });
        adapter.spawn_fixture().await;
        adapter
    }

    async fn spawn_fixture(&self) {
        let child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--ignored",
                "--exact",
                "basert_fixture_process",
                "--nocapture",
            ])
            .env("BAGENT_MODEL_RUNTIME_FIXTURE", self.address.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn BaseRT fixture");
        *self.child.lock().expect("fixture child lock") = Some(child);
        for _ in 0..100 {
            if reqwest::get(format!("http://{}/health", self.address))
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("BaseRT fixture did not become healthy");
    }

    fn pid(&self) -> u32 {
        self.child
            .lock()
            .expect("fixture child lock")
            .as_ref()
            .expect("fixture child")
            .id()
    }
}

impl Drop for SubprocessAdapter {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.lock().expect("fixture child lock").take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[async_trait]
impl ModelRuntimeAdapter for SubprocessAdapter {
    fn recorded_actions(&self) -> Vec<RuntimeAction> {
        self.actions.lock().expect("actions lock").clone()
    }

    async fn perform(&self, action: RuntimeAction) -> Result<()> {
        self.actions.lock().expect("actions lock").push(action);
        match action {
            RuntimeAction::EnsureService => {}
            RuntimeAction::Restart => {
                let old_pid = {
                    let mut child = self.child.lock().expect("fixture child lock");
                    let mut child = child.take().expect("old fixture child");
                    let old_pid = child.id();
                    let kill_result = child.kill();
                    child.wait()?;
                    kill_result?;
                    old_pid
                };
                *self.previous_pid.lock().expect("previous pid lock") = Some(old_pid);
                self.spawn_fixture().await;
            }
            RuntimeAction::VerifyHealthyChangedPid => {
                let old_pid = self
                    .previous_pid
                    .lock()
                    .expect("previous pid lock")
                    .expect("old pid");
                anyhow::ensure!(self.pid() != old_pid, "replacement PID did not change");
                let response = reqwest::get(format!("http://{}/health", self.address)).await?;
                anyhow::ensure!(response.status().is_success(), "replacement is unhealthy");
            }
            RuntimeAction::VerifyZeroLoadedWeights => {
                let value: serde_json::Value =
                    reqwest::get(format!("http://{}/v1/models", self.address))
                        .await?
                        .json()
                        .await?;
                anyhow::ensure!(
                    value["data"].as_array().is_some_and(Vec::is_empty),
                    "replacement retained loaded weights"
                );
            }
            RuntimeAction::Load(model) => {
                reqwest::Client::new()
                    .post(format!("http://{}/v1/models/load", self.address))
                    .json(&serde_json::json!({"model": format!("{model:?}")}))
                    .send()
                    .await?
                    .error_for_status()?;
            }
            RuntimeAction::VerifyReady(model) => {
                let value: serde_json::Value =
                    reqwest::get(format!("http://{}/v1/models", self.address))
                        .await?
                        .json()
                        .await?;
                anyhow::ensure!(
                    value["data"].as_array().is_some_and(|items| {
                        items.iter().any(|item| {
                            item["id"] == format!("{model:?}") && item["loaded"] == true
                        })
                    }),
                    "model did not become ready"
                );
            }
            RuntimeAction::Unload(_) => {
                reqwest::Client::new()
                    .post(format!("http://{}/v1/models/unload", self.address))
                    .send()
                    .await?
                    .error_for_status()?;
            }
        }
        Ok(())
    }

    async fn complete_bounded(
        &self,
        _model: ModelClass,
        _messages: Vec<Message>,
        _temperature: f32,
        _max_tokens: u32,
        _format: CompletionFormat,
    ) -> Result<String> {
        let response = reqwest::Client::new()
            .post(format!("http://{}/v1/chat/completions", self.address))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await?;
        anyhow::ensure!(status.is_success(), "{body}");
        Ok(body)
    }
}

#[tokio::test]
async fn speculative_preload() {
    let adapter = Arc::new(RecordingAdapter::default());
    let runtime = ModelRuntime::for_test(adapter.clone());

    runtime
        .enqueue(ModelDemand::speculative(ModelClass::Chat4B))
        .await;
    runtime
        .enqueue(ModelDemand::automation(
            WorkIdentity::new("automation-1"),
            ModelClass::Synthesis35B,
        ))
        .await;
    runtime
        .enqueue(ModelDemand::automation(
            WorkIdentity::new("automation-2"),
            ModelClass::Synthesis35B,
        ))
        .await;
    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("foreground-1"),
            ModelClass::Chat4B,
        ))
        .await;

    let foreground = runtime.dispatch_next().await.expect("foreground demand");
    assert_eq!(foreground.priority(), DemandPriority::Foreground);
    assert!(foreground.lease().is_some());
    foreground.complete().await;

    let automation = runtime.dispatch_next().await.expect("automation demand");
    assert_eq!(automation.priority(), DemandPriority::Automation);
    assert_eq!(automation.lease(), Some(&WorkIdentity::new("automation-1")));
    assert!(automation.lease().is_some());
    automation.complete().await;

    let automation = runtime
        .dispatch_next()
        .await
        .expect("second automation demand");
    assert_eq!(automation.lease(), Some(&WorkIdentity::new("automation-2")));
    automation.complete().await;

    let preload = runtime.dispatch_next().await.expect("preload demand");
    assert_eq!(preload.priority(), DemandPriority::Speculative);
    assert!(preload.lease().is_none(), "preload must never own a lease");
    preload.discard().await;

    assert_eq!(runtime.snapshot().lease_count, 0);
    assert!(!runtime.snapshot().residency_pinned);
    assert!(adapter.recorded_actions().is_empty());

    let adapter = Arc::new(RecordingAdapter::default());
    let runtime = ModelRuntime::production(adapter.clone());
    runtime.initialize().await.expect("clean runtime boundary");
    adapter.clear();
    assert!(runtime
        .speculative_preload(ModelClass::Chat4B)
        .await
        .expect("speculative preload"));
    assert_eq!(runtime.snapshot().lease_count, 0);
    assert!(!runtime.snapshot().residency_pinned);
    let result = runtime
        .complete_bounded(
            ModelDemand::foreground(
                WorkIdentity::new("foreground-yields-preload"),
                ModelClass::Synthesis35B,
            ),
            vec![Message::user("foreground")],
            0.0,
            1,
        )
        .await;
    assert!(
        result.is_err(),
        "recording adapter has no completion provider"
    );
    assert_eq!(runtime.snapshot().lease_count, 0);
    assert!(adapter
        .recorded_actions()
        .contains(&RuntimeAction::Unload(ModelClass::Chat4B)));
    assert!(adapter.recorded_actions().contains(&RuntimeAction::Restart));
    assert_eq!(
        runtime.snapshot().phase,
        RuntimePhase::Ready(ModelClass::Synthesis35B)
    );

    let adapter = BlockingAdapter::new(RuntimeAction::Load(ModelClass::Chat4B));
    let runtime = ModelRuntime::production(adapter.clone());
    runtime.initialize().await.expect("clean runtime boundary");
    let preload_runtime = runtime.clone();
    let preload = tokio::spawn(async move {
        preload_runtime
            .speculative_preload(ModelClass::Chat4B)
            .await
    });
    adapter.entered.wait().await;

    let automation_runtime = runtime.clone();
    let automation = tokio::spawn(async move {
        automation_runtime
            .complete_bounded(
                ModelDemand::automation(
                    WorkIdentity::new("overlapped-automation"),
                    ModelClass::Synthesis35B,
                ),
                vec![Message::user("automation")],
                0.0,
                1,
            )
            .await
    });
    while runtime.snapshot().queued_demand_count < 1 {
        tokio::task::yield_now().await;
    }
    let foreground_runtime = runtime.clone();
    let foreground = tokio::spawn(async move {
        foreground_runtime
            .complete_bounded(
                ModelDemand::foreground(
                    WorkIdentity::new("overlapped-foreground"),
                    ModelClass::Chat4B,
                ),
                vec![Message::user("foreground")],
                0.0,
                1,
            )
            .await
    });
    while runtime.snapshot().queued_demand_count < 2 {
        tokio::task::yield_now().await;
    }
    adapter.release.notify_one();
    assert!(preload.await.expect("preload task").expect("preload"));
    assert!(foreground.await.expect("foreground task").is_err());
    assert!(automation.await.expect("automation task").is_err());
    assert_eq!(
        *adapter.completions.lock().expect("completions lock"),
        vec![ModelClass::Chat4B, ModelClass::Synthesis35B],
        "foreground demand queued behind the transition must still dispatch first"
    );
}

#[tokio::test]
async fn lease_residency() {
    let adapter = Arc::new(RecordingAdapter::default());
    let clock = Arc::new(FakeClock::default());
    let runtime = ModelRuntime::for_test_with_clock(adapter.clone(), clock);

    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("foreground-a"),
            ModelClass::Chat4B,
        ))
        .await;
    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("foreground-b"),
            ModelClass::Chat4B,
        ))
        .await;

    let first = runtime.dispatch_next().await.expect("first lease");
    let second = runtime.dispatch_next().await.expect("second lease");
    assert_eq!(runtime.snapshot().lease_count, 2);

    runtime
        .enqueue(ModelDemand::automation(
            WorkIdentity::new("different-model"),
            ModelClass::Synthesis35B,
        ))
        .await;
    assert!(runtime.dispatch_next().await.is_none());

    runtime.request_retirement(ModelClass::Chat4B).await;
    assert!(adapter.recorded_actions().is_empty());

    second.complete().await;
    assert_eq!(runtime.snapshot().lease_count, 1);
    assert_eq!(runtime.snapshot().retirement_timer_starts, 0);
    assert!(adapter.recorded_actions().is_empty());
    assert!(runtime.dispatch_next().await.is_none());

    first.complete().await;
    assert_eq!(runtime.snapshot().lease_count, 0);
    assert_eq!(runtime.snapshot().retirement_timer_starts, 1);
    assert_eq!(
        runtime.snapshot().retirement_started_at,
        Some(Duration::ZERO)
    );

    runtime.request_retirement(ModelClass::Chat4B).await;
    assert_eq!(runtime.snapshot().retirement_timer_starts, 1);
    assert!(adapter.recorded_actions().is_empty());

    let switched = runtime
        .dispatch_next()
        .await
        .expect("different model after leases drain");
    assert_eq!(
        switched.lease(),
        Some(&WorkIdentity::new("different-model"))
    );
    switched.complete().await;

    let adapter = Arc::new(RecordingAdapter::default());
    let runtime = ModelRuntime::for_test(adapter);
    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("cancellation-blocker"),
            ModelClass::Chat4B,
        ))
        .await;
    let blocker = runtime.dispatch_next().await.expect("blocking lease");
    let cancelled_runtime = runtime.clone();
    let cancelled = tokio::spawn(async move {
        cancelled_runtime
            .complete_bounded(
                ModelDemand::automation(
                    WorkIdentity::new("cancelled-waiter"),
                    ModelClass::Synthesis35B,
                ),
                vec![Message::user("cancelled")],
                0.0,
                1,
            )
            .await
    });
    for _ in 0..100 {
        if runtime.snapshot().queued_demand_count == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(runtime.snapshot().queued_demand_count, 1);
    cancelled.abort();
    assert!(cancelled
        .await
        .expect_err("task must be cancelled")
        .is_cancelled());
    assert_eq!(
        runtime.snapshot().queued_demand_count,
        0,
        "a cancelled acquisition must remove its queued demand"
    );
    blocker.complete().await;
    runtime
        .enqueue(ModelDemand::automation(
            WorkIdentity::new("live-waiter"),
            ModelClass::Synthesis35B,
        ))
        .await;
    let live = runtime
        .dispatch_next()
        .await
        .expect("live waiter dispatches");
    assert_eq!(live.lease(), Some(&WorkIdentity::new("live-waiter")));
    live.complete().await;
}

#[tokio::test]
async fn idle_retirement() {
    let adapter = Arc::new(RecordingAdapter::default());
    let clock = Arc::new(FakeClock::default());
    let runtime = ModelRuntime::for_test_with_clock(adapter.clone(), clock.clone());

    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("foreground-idle"),
            ModelClass::Chat4B,
        ))
        .await;
    runtime
        .dispatch_next()
        .await
        .expect("lease")
        .complete()
        .await;
    assert_eq!(runtime.snapshot().retirement_timer_starts, 1);

    clock.set(Duration::from_secs(20 * 60 - 1));
    runtime.maintain().await.expect("before boundary");
    assert!(adapter.recorded_actions().is_empty());

    clock.set(Duration::from_secs(20 * 60));
    runtime.maintain().await.expect("at boundary");
    assert_eq!(
        adapter.recorded_actions(),
        vec![RuntimeAction::Unload(ModelClass::Chat4B)]
    );

    clock.set(Duration::from_secs(20 * 60 + 1));
    runtime.maintain().await.expect("after boundary");
    assert_eq!(adapter.recorded_actions().len(), 1);
}

#[tokio::test]
async fn retirement_35b() {
    let adapter = Arc::new(RecordingAdapter::default());
    let clock = Arc::new(FakeClock::default());
    let runtime = ModelRuntime::for_test_with_clock(adapter.clone(), clock);

    runtime
        .enqueue(ModelDemand::automation(
            WorkIdentity::new("automation-35b"),
            ModelClass::Synthesis35B,
        ))
        .await;
    let lease = runtime.dispatch_next().await.expect("35B lease");

    runtime.request_retirement(ModelClass::Chat4B).await;
    assert!(!runtime.retire_now().await.expect("wrong-model retirement"));
    assert!(adapter.recorded_actions().is_empty());
    runtime.request_retirement(ModelClass::Synthesis35B).await;
    assert!(!runtime.retire_now().await.expect("leased retirement"));
    assert!(adapter.recorded_actions().is_empty());

    lease.complete().await;
    assert!(runtime.retire_now().await.expect("35B retirement"));
    assert_eq!(
        adapter.recorded_actions(),
        vec![
            RuntimeAction::Unload(ModelClass::Synthesis35B),
            RuntimeAction::Restart,
            RuntimeAction::VerifyHealthyChangedPid,
            RuntimeAction::VerifyZeroLoadedWeights,
        ]
    );
    assert_eq!(runtime.snapshot().generation, 2);
    assert!(runtime.snapshot().clean_changed_pid_boundary);
    assert!(adapter
        .recorded_actions()
        .iter()
        .all(|action| *action != RuntimeAction::Load(ModelClass::Chat4B)));

    for model in [ModelClass::Chat4B, ModelClass::Synthesis35B] {
        let adapter = Arc::new(RecordingAdapter::default());
        let runtime = ModelRuntime::for_test(adapter.clone());
        runtime
            .enqueue(ModelDemand::foreground(
                WorkIdentity::new(format!("shutdown-{model:?}")),
                model,
            ))
            .await;
        runtime
            .dispatch_next()
            .await
            .expect("shutdown residency")
            .complete()
            .await;
        runtime.request_shutdown_retirement().await;
        assert!(runtime.retire_now().await.expect("shutdown retirement"));
        assert_eq!(
            adapter.recorded_actions().first(),
            Some(&RuntimeAction::Unload(model))
        );
        if model == ModelClass::Synthesis35B {
            assert!(adapter.recorded_actions().contains(&RuntimeAction::Restart));
        }
    }

    let adapter = Arc::new(RecordingAdapter::default());
    let runtime = ModelRuntime::for_test(adapter.clone());
    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("active-shutdown"),
            ModelClass::Chat4B,
        ))
        .await;
    let active = runtime
        .dispatch_next()
        .await
        .expect("active shutdown lease");
    let shutdown_runtime = runtime.clone();
    let shutdown = tokio::spawn(async move { shutdown_runtime.shutdown().await });
    while runtime.snapshot().accepting_demand {
        tokio::task::yield_now().await;
    }
    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("rejected-during-shutdown"),
            ModelClass::Chat4B,
        ))
        .await;
    assert!(runtime.dispatch_next().await.is_none());
    active.complete().await;
    shutdown
        .await
        .expect("shutdown task")
        .expect("active lease shutdown proof");
    assert_eq!(
        adapter.recorded_actions(),
        vec![RuntimeAction::Unload(ModelClass::Chat4B)]
    );

    let adapter = BlockingAdapter::new(RuntimeAction::Load(ModelClass::Chat4B));
    let runtime = ModelRuntime::production(adapter.clone());
    runtime.initialize().await.expect("clean runtime boundary");
    let loading_runtime = runtime.clone();
    let loading = tokio::spawn(async move {
        loading_runtime
            .complete_bounded(
                ModelDemand::foreground(
                    WorkIdentity::new("shutdown-during-load"),
                    ModelClass::Chat4B,
                ),
                vec![Message::user("loading")],
                0.0,
                1,
            )
            .await
    });
    adapter.entered.wait().await;
    let shutdown_runtime = runtime.clone();
    let mut shutdown = tokio::spawn(async move { shutdown_runtime.shutdown().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut shutdown)
            .await
            .is_err(),
        "shutdown must wait for an in-flight lifecycle transition"
    );
    adapter.release.notify_one();
    assert!(loading.await.expect("loading task").is_err());
    shutdown
        .await
        .expect("shutdown task")
        .expect("post-transition retirement proof");
    let actions = adapter.recorded_actions();
    let load = actions
        .iter()
        .position(|action| *action == RuntimeAction::Load(ModelClass::Chat4B))
        .expect("load action");
    let unload = actions
        .iter()
        .position(|action| *action == RuntimeAction::Unload(ModelClass::Chat4B))
        .expect("shutdown unload action");
    assert!(load < unload);

    let adapter = Arc::new(RecordingAdapter::default());
    let runtime = ModelRuntime::for_test(adapter.clone());
    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("indeterminate-shutdown"),
            ModelClass::Chat4B,
        ))
        .await;
    runtime
        .dispatch_next()
        .await
        .expect("indeterminate shutdown lease")
        .indeterminate(RuntimeFault::IndeterminateTimeout);
    runtime
        .shutdown()
        .await
        .expect("indeterminate recovery proof");
    assert_eq!(
        adapter.recorded_actions(),
        vec![
            RuntimeAction::Restart,
            RuntimeAction::VerifyHealthyChangedPid,
            RuntimeAction::VerifyZeroLoadedWeights,
        ]
    );
    assert_eq!(runtime.snapshot().generation, 2);
    assert_eq!(runtime.snapshot().lease_count, 0);

    let adapter = Arc::new(RecordingAdapter::default());
    let runtime = ModelRuntime::production(adapter.clone());
    runtime.initialize().await.expect("clean runtime boundary");
    adapter.clear();
    let demand =
        || ModelDemand::automation(WorkIdentity::new("35b-admission"), ModelClass::Synthesis35B);
    adapter.set_headroom(24, 16 * 1024 * 1024 * 1024);
    assert!(runtime
        .complete_bounded(demand(), vec![Message::user("cold")], 0.0, 1)
        .await
        .is_err());
    assert!(!adapter
        .recorded_actions()
        .contains(&RuntimeAction::Load(ModelClass::Synthesis35B)));

    adapter.set_headroom(25, 8 * 1024 * 1024 * 1024 - 1);
    assert!(runtime
        .complete_bounded(demand(), vec![Message::user("cold")], 0.0, 1)
        .await
        .is_err());
    assert_eq!(adapter.memory_checks(), 2);

    adapter.set_headroom(25, 8 * 1024 * 1024 * 1024);
    assert!(runtime
        .complete_bounded(demand(), vec![Message::user("admitted")], 0.0, 1)
        .await
        .is_err());
    assert!(adapter
        .recorded_actions()
        .contains(&RuntimeAction::Load(ModelClass::Synthesis35B)));
    assert_eq!(adapter.memory_checks(), 3);

    adapter.set_headroom(0, 0);
    adapter.clear();
    assert!(runtime
        .complete_bounded(demand(), vec![Message::user("warm")], 0.0, 1)
        .await
        .is_err());
    assert_eq!(
        adapter.memory_checks(),
        3,
        "healthy warm 35B must not reapply cold admission"
    );
    assert!(adapter.recorded_actions().is_empty());
}

#[tokio::test]
async fn poison_changed_pid() {
    let adapter = BlockingAdapter::new(RuntimeAction::Load(ModelClass::Chat4B));
    let runtime = ModelRuntime::production(adapter.clone());
    runtime.initialize().await.expect("clean runtime boundary");
    let load_runtime = runtime.clone();
    let load = tokio::spawn(async move {
        load_runtime
            .complete_bounded(
                ModelDemand::foreground(WorkIdentity::new("cancelled-load"), ModelClass::Chat4B),
                vec![Message::user("load")],
                0.0,
                1,
            )
            .await
    });
    adapter.entered.wait().await;
    assert_eq!(
        runtime.snapshot().phase,
        RuntimePhase::Loading(ModelClass::Chat4B)
    );
    load.abort();
    assert!(load.await.expect_err("load cancellation").is_cancelled());
    assert_eq!(
        runtime.snapshot().phase,
        RuntimePhase::Poisoned(ModelClass::Chat4B)
    );

    let adapter = BlockingAdapter::new(RuntimeAction::Unload(ModelClass::Chat4B));
    let runtime = ModelRuntime::for_test(adapter.clone());
    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("retirement-cancellation"),
            ModelClass::Chat4B,
        ))
        .await;
    runtime
        .dispatch_next()
        .await
        .expect("lease")
        .complete()
        .await;
    runtime.request_retirement(ModelClass::Chat4B).await;
    let retirement_runtime = runtime.clone();
    let retirement = tokio::spawn(async move { retirement_runtime.retire_now().await });
    adapter.entered.wait().await;
    assert_eq!(
        runtime.snapshot().phase,
        RuntimePhase::Retiring(ModelClass::Chat4B)
    );
    retirement.abort();
    assert!(retirement
        .await
        .expect_err("retirement cancellation")
        .is_cancelled());
    assert_eq!(
        runtime.snapshot().phase,
        RuntimePhase::Poisoned(ModelClass::Chat4B)
    );

    let adapter = BlockingAdapter::new(RuntimeAction::Restart);
    let runtime = ModelRuntime::for_test(adapter.clone());
    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("recovery-cancellation"),
            ModelClass::Chat4B,
        ))
        .await;
    runtime
        .dispatch_next()
        .await
        .expect("lease")
        .complete()
        .await;
    runtime.poison(RuntimeFault::Device).await;
    let recovery_runtime = runtime.clone();
    let recovery = tokio::spawn(async move { recovery_runtime.recover().await });
    adapter.entered.wait().await;
    assert_eq!(runtime.snapshot().phase, RuntimePhase::Restarting);
    recovery.abort();
    assert!(recovery
        .await
        .expect_err("recovery cancellation")
        .is_cancelled());
    assert_eq!(
        runtime.snapshot().phase,
        RuntimePhase::Poisoned(ModelClass::Chat4B)
    );

    let _ports = EXACT_PORT_LOCK.lock().await;
    let address = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8082);
    let probe = reserve_isolated_port(address).await;
    drop(probe);
    let adapter = SubprocessAdapter::start(address).await;
    let old_pid = adapter.pid();
    let runtime = ModelRuntime::for_test(adapter.clone());

    let result = runtime
        .complete_bounded(
            ModelDemand::foreground(WorkIdentity::new("poisoned-work"), ModelClass::Chat4B),
            vec![Message::user("controlled fault")],
            0.0,
            1,
        )
        .await;
    assert!(
        result.is_err(),
        "injected device failure must reach Model Runtime"
    );
    assert_eq!(
        runtime.snapshot().phase,
        RuntimePhase::Poisoned(ModelClass::Chat4B)
    );
    assert_eq!(
        runtime.snapshot().lease_count,
        1,
        "indeterminate completion retains its lease until changed-PID recovery"
    );

    runtime
        .enqueue(ModelDemand::foreground(
            WorkIdentity::new("blocked-work"),
            ModelClass::Chat4B,
        ))
        .await;
    assert!(runtime.dispatch_next().await.is_none());

    runtime.recover().await.expect("changed-PID recovery");
    assert_ne!(adapter.pid(), old_pid);
    assert_eq!(runtime.snapshot().generation, 2);
    assert_eq!(runtime.snapshot().phase, RuntimePhase::Unloaded);
    assert_eq!(runtime.snapshot().lease_count, 0);

    let admitted = runtime
        .dispatch_next()
        .await
        .expect("post-recovery admission");
    assert_eq!(admitted.generation(), 2);
    admitted.complete().await;
    assert_eq!(runtime.snapshot().lease_count, 0);
}

#[tokio::test]
async fn port_isolation() {
    let _ports = EXACT_PORT_LOCK.lock().await;
    let protected_before = (protected_listener_pids(8080), protected_listener_pids(8082));
    let sentinel_address = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8080);
    let fixture_address = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8082);
    let sentinel_probe = reserve_isolated_port(sentinel_address).await;
    let fixture_probe = reserve_isolated_port(fixture_address).await;
    drop((sentinel_probe, fixture_probe));

    let sentinel = SentinelProcess::start(sentinel_address).await;
    let sentinel_pid = sentinel.pid();
    let sentinel_before = sentinel.state().await.expect("sentinel baseline");
    let adapter = SubprocessAdapter::start(fixture_address).await;
    let runtime = ModelRuntime::for_test(adapter);

    assert!(runtime
        .speculative_preload(ModelClass::Chat4B)
        .await
        .expect("preload"));
    runtime.request_retirement(ModelClass::Chat4B).await;
    assert!(runtime.retire_now().await.expect("unload"));
    assert!(runtime
        .speculative_preload(ModelClass::Chat4B)
        .await
        .expect("second preload"));
    runtime.poison(RuntimeFault::Metal).await;
    runtime.recover().await.expect("poison restart");

    let sentinel_after = sentinel.state().await.expect("sentinel final");
    assert_eq!(sentinel.pid(), sentinel_pid);
    assert_eq!(sentinel_after, sentinel_before);
    assert_eq!(sentinel_after["request_count"], 0);
    assert_eq!(sentinel_after["state_hash"], "sentinel-state-v1");
    assert_eq!(
        (protected_listener_pids(8080), protected_listener_pids(8082),),
        protected_before
    );
}

#[tokio::test]
#[ignore = "subprocess entrypoint for controlled BaseRT fixture"]
async fn basert_fixture_process() {
    use axum::{
        extract::State,
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    let address: SocketAddr = std::env::var("BAGENT_MODEL_RUNTIME_FIXTURE")
        .expect("fixture address")
        .parse()
        .expect("valid fixture address");
    let loaded = Arc::new(Mutex::new(Vec::<String>::new()));
    let app = Router::new()
        .route("/health", get(|| async { StatusCode::OK }))
        .route(
            "/v1/models",
            get(|State(loaded): State<Arc<Mutex<Vec<String>>>>| async move {
                let data = loaded
                    .lock()
                    .expect("loaded models")
                    .iter()
                    .map(|id| serde_json::json!({"id": id, "loaded": true}))
                    .collect::<Vec<_>>();
                Json(serde_json::json!({"data": data}))
            }),
        )
        .route(
            "/v1/models/load",
            post(
                |State(loaded): State<Arc<Mutex<Vec<String>>>>,
                 Json(body): Json<serde_json::Value>| async move {
                    let model = body["model"].as_str().unwrap_or("unknown").to_string();
                    *loaded.lock().expect("loaded models") = vec![model];
                    StatusCode::OK
                },
            ),
        )
        .route(
            "/v1/models/unload",
            post(|State(loaded): State<Arc<Mutex<Vec<String>>>>| async move {
                loaded.lock().expect("loaded models").clear();
                StatusCode::OK
            }),
        )
        .route(
            "/v1/chat/completions",
            post(|| async {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": {"message": "Metal device lost"}})),
                )
            }),
        )
        .with_state(loaded);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("bind fixture port");
    axum::serve(listener, app).await.expect("serve fixture");
}

#[tokio::test]
#[ignore = "subprocess entrypoint for disposable port-8080 sentinel"]
async fn port_sentinel_process() {
    use axum::{
        extract::State,
        routing::{any, get},
        Json, Router,
    };
    let address: SocketAddr = std::env::var("BAGENT_MODEL_RUNTIME_SENTINEL")
        .expect("sentinel address")
        .parse()
        .expect("valid sentinel address");
    let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/state",
            get(
                |State(requests): State<Arc<std::sync::atomic::AtomicUsize>>| async move {
                    Json(serde_json::json!({
                        "request_count": requests.load(std::sync::atomic::Ordering::SeqCst),
                        "state_hash": "sentinel-state-v1"
                    }))
                },
            ),
        )
        .fallback(any(
            |State(requests): State<Arc<std::sync::atomic::AtomicUsize>>| async move {
                requests.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                axum::http::StatusCode::NO_CONTENT
            },
        ))
        .with_state(requests);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("bind sentinel port");
    axum::serve(listener, app).await.expect("serve sentinel");
}
