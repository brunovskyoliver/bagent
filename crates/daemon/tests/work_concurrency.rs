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

/// Proves the real admission entrypoint (`submit_*` + `admit`, driven by a
/// spawned `run_dispatcher`) actually grants Work — not just `dispatch_next`
/// called directly by a test. Before this was wired in, `admit` would hang
/// forever: nothing in production ever called `dispatch_next`.
#[tokio::test]
async fn admission_dispatcher_grants_through_the_real_async_path() {
    let (_dir, authority) = fixture(&["solo-turn"]);
    let dispatcher_authority = authority.clone();
    let dispatcher = tokio::spawn(dispatcher_authority.run_dispatcher(|| 0));

    let identity = authority
        .submit_conversation(
            "solo-command",
            CurrentChatIdentity::new("solo-chat"),
            ConversationTurnIdentity::new("solo-turn"),
            0,
        )
        .unwrap();

    tokio::time::timeout(std::time::Duration::from_secs(2), authority.admit(identity))
        .await
        .expect("admit must return once the dispatcher grants the Work, not hang");
    assert_eq!(authority.capacity(), (1, 0));
    dispatcher.abort();
}

/// Two foreground turns never run concurrently (only one `foreground_running`
/// slot exists) even though a concurrently-queued Automation run — a
/// separate capacity pool — is admitted independently. Proves A18's
/// foreground serialization/priority through the real async admission path,
/// not only against `dispatch_next` in isolation.
#[tokio::test]
async fn admission_dispatcher_serializes_foreground_independent_of_automation() {
    let (_dir, authority) = fixture(&["fg-1", "fg-2", "bg-run"]);
    let dispatcher_authority = authority.clone();
    let dispatcher = tokio::spawn(dispatcher_authority.run_dispatcher(|| 0));

    let automation_identity = authority
        .submit_automation(
            "bg-command",
            AutomationRunIdentity::new("bg-run"),
            AutomationSessionIdentity::new("bg-session"),
            AutomationDefinitionIdentity::new("bg-definition"),
            AutomationDefinitionRevision::new(1),
            0,
        )
        .unwrap();
    let first_turn = authority
        .submit_conversation(
            "fg-command-1",
            CurrentChatIdentity::new("fg-chat-1"),
            ConversationTurnIdentity::new("fg-1"),
            0,
        )
        .unwrap();
    let second_turn = authority
        .submit_conversation(
            "fg-command-2",
            CurrentChatIdentity::new("fg-chat-2"),
            ConversationTurnIdentity::new("fg-2"),
            0,
        )
        .unwrap();

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        authority.admit(first_turn.clone()),
    )
    .await
    .expect("first foreground turn must be admitted");
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        authority.admit(automation_identity),
    )
    .await
    .expect("Automation admits from its own capacity pool, independent of foreground");
    assert_eq!(authority.capacity(), (1, 1));

    let second_admitted = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        authority.admit(second_turn.clone()),
    )
    .await;
    assert!(
        second_admitted.is_err(),
        "second foreground turn must not admit while the first is still running"
    );

    authority.release_slot(&first_turn);
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        authority.admit(second_turn),
    )
    .await
    .expect("second foreground turn admits once the first releases its slot");
    dispatcher.abort();
}

/// Automation capacity of two is enforced through the real async path: a
/// third Automation Run does not admit until a slot is released via
/// `release_slot`, the same call production terminal handlers make.
#[tokio::test]
async fn admission_dispatcher_enforces_automation_capacity_of_two() {
    let (_dir, authority) = fixture(&["run-1", "run-2", "run-3"]);
    let dispatcher_authority = authority.clone();
    let dispatcher = tokio::spawn(dispatcher_authority.run_dispatcher(|| 0));

    let mut identities = Vec::new();
    for suffix in 1..=3 {
        let identity = authority
            .submit_automation(
                format!("run-command-{suffix}"),
                AutomationRunIdentity::new(format!("run-{suffix}")),
                AutomationSessionIdentity::new(format!("run-session-{suffix}")),
                AutomationDefinitionIdentity::new(format!("run-definition-{suffix}")),
                AutomationDefinitionRevision::new(1),
                0,
            )
            .unwrap();
        identities.push(identity);
    }

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        authority.admit(identities[0].clone()),
    )
    .await
    .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        authority.admit(identities[1].clone()),
    )
    .await
    .unwrap();
    assert_eq!(authority.capacity(), (0, 2));

    let third_admitted = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        authority.admit(identities[2].clone()),
    )
    .await;
    assert!(
        third_admitted.is_err(),
        "third Automation Run must not admit while two are already running"
    );

    authority.release_slot(&identities[0]);
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        authority.admit(identities[2].clone()),
    )
    .await
    .expect("third Automation Run admits once a slot is released");
    dispatcher.abort();
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
        authority.release_slot(&grant.work_identity);
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
    authority.release_slot(&first.work_identity);
    let third = authority.dispatch_next(0).unwrap().unwrap();
    authority.release_slot(&second.work_identity);
    authority.release_slot(&third.work_identity);
    // Releasing an unknown/already-released Work is a safe no-op.
    authority.release_slot(&third.work_identity);
    assert_eq!(authority.capacity(), (0, 0));
}

#[test]
fn crash_recovery() {
    let (dir, authority) = fixture(&["crash-work"]);
    conversation(&authority, 0, 0);
    let identity = authority.dispatch_next(0).unwrap().unwrap().work_identity;
    drop(authority);

    let generation = DaemonGeneration::new("crash-recovered-generation");
    let reopened = WorkCoordinator::open(
        dir.path().join("work.sqlite"),
        CoordinatorConfig::default(),
        generation,
    )
    .unwrap();
    let snapshot = reopened.snapshot().unwrap();
    let work = snapshot
        .works
        .iter()
        .find(|work| work.identity == identity)
        .expect("crashed Work remains addressable");
    assert!(
        work.state.is_terminal(),
        "crashed Work must be terminalized"
    );
    assert_eq!(work.state, WorkState::Abandoned);
    assert_eq!(
        snapshot
            .works
            .iter()
            .filter(|candidate| candidate.identity == identity)
            .count(),
        1,
        "recovery must not duplicate Work"
    );
    let connection = rusqlite::Connection::open(dir.path().join("work.sqlite")).unwrap();
    connection
        .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
        .map(|value| assert_eq!(value, "ok"))
        .unwrap();
    let obsolete_placeholder: i64 = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='automation_session_pending_approvals')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        obsolete_placeholder, 0,
        "bootstrap must not resurrect removed authority"
    );
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
    let _approval_revision = authority
        .request_approval(
            "approval-request",
            grant.work_identity.clone(),
            running.receipt().work_revision,
            ApprovalIdentity::new("approval-stable"),
            "filesystem.write",
        )
        .unwrap();
    assert_eq!(authority.capacity(), (0, 0));

    automation(&authority, 2, 1);
    let backfill = authority
        .dispatch_next(1)
        .unwrap()
        .expect("approval wait releases capacity");
    authority.release_slot(&backfill.work_identity);
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
        bagentd::work_coordinator::ApprovalState::Abandoned
    );
    let abandoned_revision = after
        .works
        .iter()
        .find(|work| work.identity == grant.work_identity)
        .unwrap()
        .revision;

    let abandoned_decision = restarted
        .resolve_approval(
            "approval-resolve",
            grant.work_identity.clone(),
            abandoned_revision,
            ApprovalIdentity::new("approval-stable"),
            true,
            0,
        )
        .unwrap_err();
    assert!(matches!(
        abandoned_decision,
        bagentd::work_coordinator::CommandError::IllegalTransition {
            from: WorkState::Abandoned,
            to: WorkState::Running
        }
    ));
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
