use bagentd::{
    unified_work::{ExecutionOrigin, UnifiedWorkAuthority, AUTOMATION_AGE_BOUNDARY},
    work_coordinator::{
        ApprovalIdentity, AutomationDefinitionIdentity, AutomationDefinitionRevision,
        AutomationRunIdentity, AutomationSessionIdentity, Command, ConversationTurnIdentity,
        CoordinatorConfig, CoordinatorDependencies, CurrentChatIdentity, DaemonGeneration,
        DeterministicWorkIdentitySource, FixedCoordinatorClock, WorkCoordinator, WorkRevision,
        WorkState,
    },
};
use std::sync::Arc;
use tempfile::TempDir;

fn fixture(identities: &[&str]) -> (TempDir, Arc<UnifiedWorkAuthority>) {
    let dir = tempfile::tempdir().unwrap();
    let generation = DaemonGeneration::new("stage4-generation");
    let coordinator = WorkCoordinator::open_with_dependencies(
        dir.path().join("work.sqlite"),
        CoordinatorConfig::default(),
        generation.clone(),
        CoordinatorDependencies {
            identity_source: Box::new(DeterministicWorkIdentitySource::new(
                identities.iter().copied(),
            )),
            clock: Box::new(FixedCoordinatorClock::new("2026-08-18T18:00:00Z")),
        },
    )
    .unwrap();
    (
        dir,
        Arc::new(UnifiedWorkAuthority::new(Arc::new(coordinator), generation)),
    )
}

fn conversation(authority: &UnifiedWorkAuthority, suffix: usize, now: u64) {
    authority
        .submit_conversation(
            format!("conversation-command-{suffix}"),
            CurrentChatIdentity::new(format!("chat-{suffix}")),
            ConversationTurnIdentity::new(format!("turn-{suffix}")),
            now,
        )
        .unwrap();
}

fn automation(authority: &UnifiedWorkAuthority, suffix: usize, now: u64) {
    authority
        .submit_automation(
            format!("automation-command-{suffix}"),
            AutomationRunIdentity::new(format!("run-{suffix}")),
            AutomationSessionIdentity::new(format!("session-{suffix}")),
            AutomationDefinitionIdentity::new(format!("definition-{suffix}")),
            AutomationDefinitionRevision::new(1),
            now,
        )
        .unwrap();
}

#[test]
fn fairness_foreground() {
    let (_dir, authority) = fixture(&["automation", "fg-1", "fg-2", "fg-3", "fg-4"]);
    automation(&authority, 0, 0);
    for index in 1..=4 {
        conversation(&authority, index, AUTOMATION_AGE_BOUNDARY);
    }

    for expected in ["fg-1", "fg-2", "fg-3"] {
        let grant = authority
            .dispatch_next(AUTOMATION_AGE_BOUNDARY)
            .unwrap()
            .unwrap();
        assert_eq!(grant.origin, ExecutionOrigin::Foreground);
        assert_eq!(grant.work_identity.as_str(), expected);
        authority.release_slot(grant.origin);
    }
    let aged = authority
        .dispatch_next(AUTOMATION_AGE_BOUNDARY)
        .unwrap()
        .unwrap();
    assert_eq!(aged.origin, ExecutionOrigin::Automation);
    assert_eq!(aged.work_identity.as_str(), "automation");
}

#[test]
fn automation_capacity_two() {
    let (_dir, authority) = fixture(&["automation-1", "automation-2", "automation-3"]);
    for index in 1..=3 {
        automation(&authority, index, 0);
    }
    let first = authority.dispatch_next(0).unwrap().unwrap();
    let second = authority.dispatch_next(0).unwrap().unwrap();
    assert_eq!(first.origin, ExecutionOrigin::Automation);
    assert_eq!(second.origin, ExecutionOrigin::Automation);
    assert_ne!(first.work_identity, second.work_identity);
    assert_eq!(authority.capacity(), (0, 2));
    assert!(authority.dispatch_next(0).unwrap().is_none());
    authority.release_slot(first.origin);
    assert!(authority.dispatch_next(0).unwrap().is_some());
    authority.release_slot(second.origin);
    authority.release_slot(ExecutionOrigin::Automation);
    assert_eq!(authority.capacity(), (0, 0));
}

#[test]
fn approval_restart_capacity() {
    let (dir, authority) = fixture(&["approval-work", "other-work"]);
    automation(&authority, 1, 0);
    let grant = authority.dispatch_next(0).unwrap().unwrap();
    let running = authority
        .coordinator()
        .submit(Command::transition(
            "approval-running",
            grant.work_identity.clone(),
            WorkRevision::new(2),
            WorkState::Running,
            DaemonGeneration::new("stage4-generation"),
        ))
        .unwrap();
    let approval_revision = authority
        .request_approval(
            "approval-request",
            grant.work_identity.clone(),
            running.receipt().work_revision,
            ApprovalIdentity::new("approval-stable"),
            "filesystem.write",
            grant.origin,
        )
        .unwrap();
    assert_eq!(authority.capacity(), (0, 0));

    automation(&authority, 2, 1);
    assert!(
        authority.dispatch_next(1).unwrap().is_some(),
        "approval wait releases capacity"
    );
    authority.release_slot(ExecutionOrigin::Automation);
    let before = authority.coordinator().snapshot().unwrap();
    assert_eq!(before.approvals[0].identity.as_str(), "approval-stable");

    drop(authority);
    let restarted_generation = DaemonGeneration::new("stage4-restarted");
    let restarted = Arc::new(UnifiedWorkAuthority::new(
        Arc::new(
            WorkCoordinator::open(
                dir.path().join("work.sqlite"),
                CoordinatorConfig::default(),
                restarted_generation.clone(),
            )
            .unwrap(),
        ),
        restarted_generation,
    ));
    let after = restarted.coordinator().snapshot().unwrap();
    assert_eq!(after.approvals[0].identity.as_str(), "approval-stable");
    assert_eq!(
        after.approvals[0].state,
        bagentd::work_coordinator::ApprovalState::Pending
    );

    let resumed = restarted
        .resolve_approval(
            "approval-resolve",
            grant.work_identity.clone(),
            approval_revision,
            ApprovalIdentity::new("approval-stable"),
            true,
            0,
            ExecutionOrigin::Automation,
            2,
        )
        .unwrap();
    assert_eq!(resumed, WorkRevision::new(5));
    let stale = restarted.resolve_approval(
        "approval-stale",
        grant.work_identity,
        approval_revision,
        ApprovalIdentity::new("approval-stable"),
        true,
        0,
        ExecutionOrigin::Automation,
        2,
    );
    assert!(stale.is_err(), "one stale decision cannot resume twice");
}

#[test]
fn cancellation_races() {
    let (_dir, authority) = fixture(&["queued", "loading", "executing", "approval", "completion"]);
    for index in 0..5 {
        conversation(&authority, index, 0);
    }
    let generation = DaemonGeneration::new("stage4-generation");

    let queued_revision = authority
        .cancel("cancel-queued", "queued".into(), WorkRevision::new(1))
        .unwrap();
    let queued_replay = authority
        .cancel("cancel-queued", "queued".into(), WorkRevision::new(1))
        .unwrap();
    assert_eq!(queued_revision, queued_replay);
    authority
        .coordinator()
        .submit(Command::transition(
            "terminal-queued",
            "queued",
            queued_revision,
            WorkState::Cancelled,
            generation.clone(),
        ))
        .unwrap();

    authority
        .coordinator()
        .submit(Command::transition(
            "loading-start",
            "loading",
            WorkRevision::new(1),
            WorkState::WaitingForModel,
            generation.clone(),
        ))
        .unwrap();
    let loading_revision = authority
        .cancel("cancel-loading", "loading".into(), WorkRevision::new(2))
        .unwrap();
    authority
        .coordinator()
        .submit(Command::transition(
            "terminal-loading",
            "loading",
            loading_revision,
            WorkState::Cancelled,
            generation.clone(),
        ))
        .unwrap();

    for (work, prefix) in [
        ("executing", "executing"),
        ("approval", "approval"),
        ("completion", "completion"),
    ] {
        authority
            .coordinator()
            .submit(Command::transition(
                format!("{prefix}-loading"),
                work,
                WorkRevision::new(1),
                WorkState::WaitingForModel,
                generation.clone(),
            ))
            .unwrap();
        authority
            .coordinator()
            .submit(Command::transition(
                format!("{prefix}-running"),
                work,
                WorkRevision::new(2),
                WorkState::Running,
                generation.clone(),
            ))
            .unwrap();
    }

    let executing_revision = authority
        .cancel("cancel-executing", "executing".into(), WorkRevision::new(3))
        .unwrap();
    authority
        .coordinator()
        .submit(Command::transition(
            "terminal-executing",
            "executing",
            executing_revision,
            WorkState::Cancelled,
            generation.clone(),
        ))
        .unwrap();

    let approval_revision = authority
        .request_approval(
            "approval-race-request",
            "approval".into(),
            WorkRevision::new(3),
            ApprovalIdentity::new("approval-race"),
            "side-effect",
            ExecutionOrigin::Foreground,
        )
        .unwrap();
    let cancelling_approval = authority
        .cancel("cancel-approval", "approval".into(), approval_revision)
        .unwrap();
    authority
        .coordinator()
        .submit(Command::transition(
            "terminal-approval",
            "approval",
            cancelling_approval,
            WorkState::Cancelled,
            generation.clone(),
        ))
        .unwrap();

    authority
        .coordinator()
        .submit(Command::transition(
            "completion-wins",
            "completion",
            WorkRevision::new(3),
            WorkState::Completed,
            generation,
        ))
        .unwrap();
    assert!(authority
        .cancel("late-cancel", "completion".into(), WorkRevision::new(3))
        .is_err());

    let snapshot = authority.coordinator().snapshot().unwrap();
    assert!(snapshot.works.iter().all(|work| work.state.is_terminal()));
    assert_eq!(
        snapshot.approvals[0].state,
        bagentd::work_coordinator::ApprovalState::Withdrawn
    );
    assert_eq!(authority.capacity(), (0, 0));
    let mut cursors = authority
        .coordinator()
        .events(
            Some(bagentd::work_coordinator::EventCursor::new(0)),
            &DaemonGeneration::new("stage4-generation"),
        )
        .unwrap();
    let events = match &mut cursors {
        bagentd::work_coordinator::EventRead::Events(events) => events,
        _ => panic!("events retained"),
    };
    let terminal_events = events
        .iter()
        .filter(|event| event.state.is_terminal())
        .count();
    assert_eq!(terminal_events, 5, "one ordered terminal outcome per Work");
}
