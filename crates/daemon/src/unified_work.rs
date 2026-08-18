//! Unified admission, capacity, approval and cancellation authority.
//!
//! This module deliberately keeps scheduling policy separate from persistence:
//! every lifecycle mutation is first committed through [`WorkCoordinator`],
//! while this small actor owns the volatile execution slots reconstructed from
//! the authoritative snapshot after restart.

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
}

pub struct UnifiedWorkAuthority {
    coordinator: Arc<WorkCoordinator>,
    generation: DaemonGeneration,
    scheduler: Mutex<SchedulerState>,
    automation_slots: Arc<tokio::sync::Semaphore>,
    held_automation_slots: Mutex<HashMap<WorkIdentity, tokio::sync::OwnedSemaphorePermit>>,
}

impl UnifiedWorkAuthority {
    pub fn new(coordinator: Arc<WorkCoordinator>, generation: DaemonGeneration) -> Self {
        Self {
            coordinator,
            generation,
            scheduler: Mutex::new(SchedulerState::default()),
            automation_slots: Arc::new(tokio::sync::Semaphore::new(AUTOMATION_CAPACITY)),
            held_automation_slots: Mutex::new(HashMap::new()),
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
        self.enqueue(acknowledgement, ExecutionOrigin::Foreground, now)
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
        self.enqueue(acknowledgement, ExecutionOrigin::Automation, now)
    }

    fn enqueue(
        &self,
        acknowledgement: CommandAcknowledgement,
        origin: ExecutionOrigin,
        now: u64,
    ) -> Result<WorkIdentity, CommandError> {
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
        Ok(receipt.work_identity.clone())
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
        Ok(Some(DispatchGrant {
            work_identity: queued.work_identity,
            origin: queued.origin,
        }))
    }

    pub fn release_slot(&self, origin: ExecutionOrigin) {
        let mut state = self.scheduler.lock().expect("scheduler mutex poisoned");
        match origin {
            ExecutionOrigin::Foreground => {
                state.foreground_running = state.foreground_running.saturating_sub(1)
            }
            ExecutionOrigin::Automation => {
                state.automation_running = state.automation_running.saturating_sub(1)
            }
        }
    }

    pub fn capacity(&self) -> (usize, usize) {
        let state = self.scheduler.lock().expect("scheduler mutex poisoned");
        (state.foreground_running, state.automation_running)
    }

    pub async fn acquire_automation_slot(&self, work: WorkIdentity) {
        let permit = self
            .automation_slots
            .clone()
            .acquire_owned()
            .await
            .expect("unified Work automation capacity remains open");
        self.held_automation_slots
            .lock()
            .expect("slot mutex poisoned")
            .insert(work, permit);
    }

    pub fn release_execution_slot(&self, work: &WorkIdentity) {
        self.held_automation_slots
            .lock()
            .expect("slot mutex poisoned")
            .remove(work);
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

    pub fn request_approval(
        &self,
        command: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        approval: ApprovalIdentity,
        category: impl Into<String>,
        origin: ExecutionOrigin,
    ) -> Result<WorkRevision, CommandError> {
        let acknowledgement = self.coordinator.submit(Command::request_approval(
            command,
            work,
            revision,
            approval,
            category,
            self.generation.clone(),
        ))?;
        self.release_slot(origin);
        Ok(acknowledgement.receipt().work_revision)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve_approval(
        &self,
        command: impl Into<CommandIdentity>,
        work: WorkIdentity,
        revision: WorkRevision,
        approval: ApprovalIdentity,
        allow: bool,
        expected_decision_revision: u64,
        origin: ExecutionOrigin,
        now: u64,
    ) -> Result<WorkRevision, CommandError> {
        let acknowledgement = self.coordinator.submit(Command::resolve_approval(
            command,
            work.clone(),
            revision,
            approval,
            allow,
            expected_decision_revision,
            self.generation.clone(),
        ))?;
        let receipt = acknowledgement.receipt();
        if allow {
            self.scheduler
                .lock()
                .expect("scheduler mutex poisoned")
                .queue
                .push_back(Queued {
                    work_identity: work,
                    revision: receipt.work_revision,
                    origin,
                    enqueued_at: now,
                });
        }
        Ok(receipt.work_revision)
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
        self.scheduler
            .lock()
            .expect("scheduler mutex poisoned")
            .queue
            .retain(|queued| queued.work_identity != work);
        Ok(acknowledgement.receipt().work_revision)
    }
}
