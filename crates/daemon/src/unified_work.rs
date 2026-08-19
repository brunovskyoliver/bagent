//! Unified admission, capacity, approval and cancellation authority.
//!
//! This module deliberately keeps scheduling policy separate from persistence:
//! every lifecycle mutation is first committed through [`WorkCoordinator`],
//! while this small actor owns the volatile execution slots reconstructed from
//! the authoritative snapshot after restart.
//!
//! Admission is notify-driven rather than polled: [`Self::enqueue`] wakes a
//! single shared [`tokio::sync::Notify`] on every state change (new arrival,
//! grant, release, cancel), [`Self::run_dispatcher`] drains [`Self::dispatch_next`]
//! whenever woken (plus a 1s fallback tick for the Automation aging boundary),
//! and [`Self::admit`] blocks the caller until its own Work leaves the queue.
//! This is the only path that grants foreground/Automation execution capacity;
//! no caller may construct a competing semaphore or bypass it.

use crate::work_coordinator::{
    ApprovalIdentity, AutomationDefinitionIdentity, AutomationDefinitionRevision,
    AutomationRunIdentity, AutomationSessionIdentity, Command, CommandAcknowledgement,
    CommandError, CommandIdentity, ConversationTurnIdentity, CurrentChatIdentity, DaemonGeneration,
    WorkCoordinator, WorkIdentity, WorkRevision, WorkState,
};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

pub const AUTOMATION_CAPACITY: usize = 2;
pub const FOREGROUND_BURST_LIMIT: usize = 3;
pub const AUTOMATION_AGE_BOUNDARY: u64 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionOrigin {
    Foreground,
    Automation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionBoundary {
    Runtime,
    Tool,
    Approval,
    Completion,
}

pub trait ExecutionBoundaryAdapter {
    fn cross(&mut self, boundary: ExecutionBoundary, work: &WorkIdentity) -> Result<(), String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchGrant {
    pub work_identity: WorkIdentity,
    pub origin: ExecutionOrigin,
}

#[derive(Clone, Debug)]
struct Queued {
    work_identity: WorkIdentity,
    revision: WorkRevision,
    origin: ExecutionOrigin,
    enqueued_at: u64,
}

#[derive(Default)]
struct SchedulerState {
    queue: VecDeque<Queued>,
    foreground_running: usize,
    automation_running: usize,
    consecutive_foreground: usize,
    running_origin: HashMap<WorkIdentity, ExecutionOrigin>,
}

impl SchedulerState {
    fn release(&mut self, work: &WorkIdentity) {
        if let Some(origin) = self.running_origin.remove(work) {
            match origin {
                ExecutionOrigin::Foreground => {
                    self.foreground_running = self.foreground_running.saturating_sub(1)
                }
                ExecutionOrigin::Automation => {
                    self.automation_running = self.automation_running.saturating_sub(1)
                }
            }
        }
    }
}

pub struct UnifiedWorkAuthority {
    coordinator: Arc<WorkCoordinator>,
    generation: DaemonGeneration,
    scheduler: Mutex<SchedulerState>,
    dispatch_notify: tokio::sync::Notify,
}

impl UnifiedWorkAuthority {
    pub fn new(coordinator: Arc<WorkCoordinator>, generation: DaemonGeneration) -> Self {
        Self {
            coordinator,
            generation,
            scheduler: Mutex::new(SchedulerState::default()),
            dispatch_notify: tokio::sync::Notify::new(),
        }
    }

    pub fn coordinator(&self) -> &Arc<WorkCoordinator> {
        &self.coordinator
    }
    pub fn generation(&self) -> &DaemonGeneration {
        &self.generation
    }
    pub fn model_runtime(
        &self,
        runtime: Arc<crate::model_runtime::ModelRuntime>,
        work: WorkIdentity,
        origin: ExecutionOrigin,
    ) -> crate::model_runtime::CoordinatedModelRuntime {
        crate::model_runtime::CoordinatedModelRuntime::new(
            runtime,
            match origin {
                ExecutionOrigin::Foreground => crate::model_runtime::WorkDemandOrigin::Foreground,
                ExecutionOrigin::Automation => crate::model_runtime::WorkDemandOrigin::Automation,
            },
            work,
        )
    }
    pub fn current(
        &self,
        work: &WorkIdentity,
    ) -> Result<Option<crate::work_coordinator::WorkRecord>, CommandError> {
        Ok(self
            .coordinator
            .snapshot()?
            .works
            .into_iter()
            .find(|record| &record.identity == work))
    }

    pub fn submit_conversation(
        &self,
        command: impl Into<CommandIdentity>,
        chat: CurrentChatIdentity,
        turn: ConversationTurnIdentity,
        now: u64,
    ) -> Result<WorkIdentity, CommandError> {
        let acknowledgement = self.coordinator.submit(Command::create_conversation(
            command,
            chat,
            turn,
            self.generation.clone(),
        ))?;
        Ok(self.enqueue(acknowledgement, ExecutionOrigin::Foreground, now))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_automation(
        &self,
        command: impl Into<CommandIdentity>,
        run: AutomationRunIdentity,
        session: AutomationSessionIdentity,
        definition: AutomationDefinitionIdentity,
        definition_revision: AutomationDefinitionRevision,
        now: u64,
    ) -> Result<WorkIdentity, CommandError> {
        let acknowledgement = self.coordinator.submit(Command::create_automation(
            command,
            run,
            session,
            definition,
            definition_revision,
            self.generation.clone(),
        ))?;
        Ok(self.enqueue(acknowledgement, ExecutionOrigin::Automation, now))
    }

    fn enqueue(
        &self,
        acknowledgement: CommandAcknowledgement,
        origin: ExecutionOrigin,
        now: u64,
    ) -> WorkIdentity {
        let receipt = acknowledgement.receipt();
        let queued = Queued {
            work_identity: receipt.work_identity.clone(),
            revision: receipt.work_revision,
            origin,
            enqueued_at: now,
        };
        self.scheduler
            .lock()
            .expect("scheduler mutex poisoned")
            .queue
            .push_back(queued);
        self.dispatch_notify.notify_waiters();
        receipt.work_identity.clone()
    }

    /// Waits until `work` leaves the admission queue, i.e. until the
    /// dispatcher has granted it execution capacity (or it was cancelled out
    /// from under it, which the caller's subsequent `transition` call will
    /// surface as a `CommandError`).
    pub async fn admit(&self, work: WorkIdentity) {
        loop {
            let notified = self.dispatch_notify.notified();
            let still_queued = self
                .scheduler
                .lock()
                .expect("scheduler mutex poisoned")
                .queue
                .iter()
                .any(|queued| queued.work_identity == work);
            if !still_queued {
                return;
            }
            notified.await;
        }
    }

    /// Background loop, spawned once per daemon: drains grantable Work
    /// whenever the queue changes, and re-checks periodically so the
    /// Automation aging boundary fires even without new arrivals.
    pub async fn run_dispatcher(self: Arc<Self>, clock: impl Fn() -> u64 + Send + Sync + 'static) {
        loop {
            let notified = self.dispatch_notify.notified();
            while matches!(self.dispatch_next(clock()), Ok(Some(_))) {}
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
            }
        }
    }

    pub fn dispatch_next(&self, now: u64) -> Result<Option<DispatchGrant>, CommandError> {
        let mut state = self.scheduler.lock().expect("scheduler mutex poisoned");
        let aged_automation = state.queue.iter().position(|queued| {
            queued.origin == ExecutionOrigin::Automation
                && now.saturating_sub(queued.enqueued_at) >= AUTOMATION_AGE_BOUNDARY
                && state.automation_running < AUTOMATION_CAPACITY
        });
        let foreground = state.queue.iter().position(|queued| {
            queued.origin == ExecutionOrigin::Foreground && state.foreground_running == 0
        });
        let automation = state.queue.iter().position(|queued| {
            queued.origin == ExecutionOrigin::Automation
                && state.automation_running < AUTOMATION_CAPACITY
        });
        let index = if aged_automation.is_some()
            && state.consecutive_foreground >= FOREGROUND_BURST_LIMIT
        {
            aged_automation
        } else {
            foreground.or(automation)
        };
        let Some(index) = index else {
            return Ok(None);
        };
        let queued = state
            .queue
            .remove(index)
            .expect("selected queue entry exists");
        match queued.origin {
            ExecutionOrigin::Foreground => {
                state.foreground_running += 1;
                state.consecutive_foreground += 1;
            }
            ExecutionOrigin::Automation => {
                state.automation_running += 1;
                state.consecutive_foreground = 0;
            }
        }
        state
            .running_origin
            .insert(queued.work_identity.clone(), queued.origin);
        drop(state);
        self.coordinator.submit(Command::transition(
            format!(
                "dispatch-model-{}-{}",
                queued.work_identity,
                queued.revision.value()
            ),
            queued.work_identity.clone(),
            queued.revision,
            WorkState::WaitingForModel,
            self.generation.clone(),
        ))?;
        self.dispatch_notify.notify_waiters();
        Ok(Some(DispatchGrant {
            work_identity: queued.work_identity,
            origin: queued.origin,
        }))
    }

    /// Releases the execution slot held by `work`, if any. Safe to call on
    /// Work that never held a slot (approval-wait, cancel of a queued item).
    pub fn release_slot(&self, work: &WorkIdentity) {
        let mut state = self.scheduler.lock().expect("scheduler mutex poisoned");
        state.release(work);
        drop(state);
        self.dispatch_notify.notify_waiters();
    }

    pub fn capacity(&self) -> (usize, usize) {
        let state = self.scheduler.lock().expect("scheduler mutex poisoned");
        (state.foreground_running, state.automation_running)
    }

    /// Re-admits Work whose approval was just resolved (the coordinator
    /// already moved it back to `Running`); waits for capacity without
    /// re-entering the Queued/WaitingForModel transition.
    pub async fn resume(&self, work: WorkIdentity, origin: ExecutionOrigin) {
        loop {
            let notified = self.dispatch_notify.notified();
            {
                let mut state = self.scheduler.lock().expect("scheduler mutex poisoned");
                let has_capacity = match origin {
                    ExecutionOrigin::Foreground => true,
                    ExecutionOrigin::Automation => state.automation_running < AUTOMATION_CAPACITY,
                };
                if has_capacity {
                    match origin {
                        ExecutionOrigin::Foreground => state.foreground_running += 1,
                        ExecutionOrigin::Automation => state.automation_running += 1,
                    }
                    state.running_origin.insert(work, origin);
                    return;
                }
            }
            notified.await;
        }
    }

    pub fn transition(
        &self,
        command: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        next: WorkState,
    ) -> Result<WorkRevision, CommandError> {
        if next != WorkState::Queued {
            self.scheduler
                .lock()
                .expect("scheduler mutex poisoned")
                .queue
                .retain(|queued| queued.work_identity != work);
        }
        let revision = self
            .coordinator
            .submit(Command::transition(
                command,
                work,
                revision,
                next,
                self.generation.clone(),
            ))?
            .receipt()
            .work_revision;
        Ok(revision)
    }

    pub fn execute_with_adapter(
        &self,
        work: WorkIdentity,
        adapter: &mut dyn ExecutionBoundaryAdapter,
    ) -> Result<WorkState, CommandError> {
        let mut revision = WorkRevision::new(1);
        if adapter.cross(ExecutionBoundary::Runtime, &work).is_err() {
            self.transition("adapter-runtime-failed", work, revision, WorkState::Failed)?;
            return Ok(WorkState::Failed);
        }
        revision = self.transition(
            "adapter-waiting-model",
            work.clone(),
            revision,
            WorkState::WaitingForModel,
        )?;
        revision = self.transition(
            "adapter-running",
            work.clone(),
            revision,
            WorkState::Running,
        )?;
        for (boundary, command) in [
            (ExecutionBoundary::Tool, "adapter-tool-failed"),
            (ExecutionBoundary::Approval, "adapter-approval-failed"),
            (ExecutionBoundary::Completion, "adapter-completion-failed"),
        ] {
            if adapter.cross(boundary, &work).is_err() {
                self.transition(command, work, revision, WorkState::Failed)?;
                return Ok(WorkState::Failed);
            }
        }
        self.transition("adapter-completed", work, revision, WorkState::Completed)?;
        Ok(WorkState::Completed)
    }

    /// Requests a durable approval and releases the requesting Work's
    /// execution slot for the duration of the wait (capacity must not be
    /// held while blocked on a human decision).
    pub fn request_approval(
        &self,
        command: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        approval: ApprovalIdentity,
        category: impl Into<String>,
    ) -> Result<WorkRevision, CommandError> {
        let acknowledgement = self.coordinator.submit(Command::request_approval(
            command,
            work.clone(),
            revision,
            approval,
            category,
            self.generation.clone(),
        ))?;
        self.release_slot(&work);
        Ok(acknowledgement.receipt().work_revision)
    }

    pub fn resolve_approval(
        &self,
        command: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        approval: ApprovalIdentity,
        allow: bool,
        expected_decision_revision: u64,
    ) -> Result<WorkRevision, CommandError> {
        let acknowledgement = self.coordinator.submit(Command::resolve_approval(
            command,
            work,
            revision,
            approval,
            allow,
            expected_decision_revision,
            self.generation.clone(),
        ))?;
        Ok(acknowledgement.receipt().work_revision)
    }

    pub fn cancel(
        &self,
        command: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
    ) -> Result<WorkRevision, CommandError> {
        let acknowledgement = self.coordinator.submit(Command::cancel(
            command,
            work.clone(),
            revision,
            self.generation.clone(),
        ))?;
        let mut state = self.scheduler.lock().expect("scheduler mutex poisoned");
        state.queue.retain(|queued| queued.work_identity != work);
        state.release(&work);
        drop(state);
        self.dispatch_notify.notify_waiters();
        Ok(acknowledgement.receipt().work_revision)
    }
}
