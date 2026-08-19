use bagentd::work_coordinator::{
    ApprovalIdentity, AutomationDefinitionIdentity, AutomationDefinitionRevision,
    AutomationRunIdentity, AutomationSessionIdentity, Command, CommandAcknowledgement,
    CommandError, ConversationTurnIdentity, CoordinatorConfig, CoordinatorDependencies,
    CurrentChatIdentity, DaemonGeneration, DeterministicWorkIdentitySource, EventCursor, EventRead,
    FailurePoint, FixedCoordinatorClock, ModelRuntimeGeneration, WorkActivityCategory,
    WorkCoordinator, WorkIdentity, WorkRevision, WorkState,
};
use tempfile::TempDir;

fn dependencies(identities: &[&str]) -> CoordinatorDependencies {
    CoordinatorDependencies {
        identity_source: Box::new(DeterministicWorkIdentitySource::new(
            identities.iter().copied(),
        )),
        clock: Box::new(FixedCoordinatorClock::new("2026-08-18T16:00:00Z")),
    }
}

#[test]
fn notch_projection_snapshot_is_transactional_and_bounded() {
    let (_temp, coordinator) = fixture("daemon-notch", 32, &["work-a", "work-b"]);
    for (suffix, expected_identity) in [("a", "work-a"), ("b", "work-b")] {
        let created = coordinator
            .submit(create_conversation(
                format!("create-{suffix}"),
                format!("chat-{suffix}"),
                format!("turn-{suffix}"),
                "daemon-notch",
            ))
            .unwrap();
        assert_eq!(created.receipt().work_identity.as_str(), expected_identity);
        let work = created.receipt().work_identity.clone();
        coordinator
            .submit(work_transition(
                format!("wait-{suffix}"),
                work.clone(),
                rev(1),
                WorkState::WaitingForModel,
                "daemon-notch",
            ))
            .unwrap();
        coordinator
            .submit(work_transition(
                format!("run-{suffix}"),
                work.clone(),
                rev(2),
                WorkState::Running,
                "daemon-notch",
            ))
            .unwrap();
        coordinator
            .submit(work_transition(
                format!("done-{suffix}"),
                work,
                rev(3),
                WorkState::Completed,
                "daemon-notch",
            ))
            .unwrap();
    }

    let (snapshot, work_count_in_same_transaction) = coordinator
        .projected_snapshot(|connection, snapshot| {
            let count = connection
                .query_row("SELECT COUNT(*) FROM works", [], |row| {
                    row.get::<_, usize>(0)
                })
                .map_err(|error| CommandError::Storage(error.to_string()))?;
            assert_eq!(snapshot.works.len(), 1);
            Ok(count)
        })
        .unwrap();

    assert_eq!(work_count_in_same_transaction, 2);
    assert_eq!(snapshot.works.len(), 1);
    assert!(snapshot.automation_runs.is_empty());
    assert!(snapshot.interruptions.is_empty());
    assert!(matches!(
        coordinator
            .notch_events(snapshot.cursor, &snapshot.daemon_generation)
            .unwrap(),
        EventRead::Events(events) if events.is_empty()
    ));
}

#[test]
fn notch_projection_bounds_high_cardinality_unread_sessions_by_terminal_state() {
    let identities = [
        "work-00", "work-01", "work-02", "work-03", "work-04", "work-05", "work-06", "work-07",
        "work-08", "work-09", "work-10", "work-11",
    ];
    let (_temp, coordinator) = fixture("daemon-unread", 128, &identities);
    for index in 0..identities.len() {
        let terminal_state = match index % 3 {
            0 => WorkState::Completed,
            1 => WorkState::Partial,
            _ => WorkState::Failed,
        };
        let work = coordinator
            .submit(create_automation(
                format!("create-{index}"),
                format!("run-{index}"),
                format!("session-{index}"),
                format!("definition-{index}"),
                1,
                "daemon-unread",
            ))
            .unwrap()
            .receipt()
            .work_identity
            .clone();
        for (revision, state) in [
            (1, WorkState::WaitingForModel),
            (2, WorkState::Running),
            (3, terminal_state),
        ] {
            coordinator
                .submit(work_transition(
                    format!("transition-{index}-{revision}"),
                    work.clone(),
                    rev(revision),
                    state,
                    "daemon-unread",
                ))
                .unwrap();
        }
    }

    let (snapshot, ()) = coordinator.projected_snapshot(|_, _| Ok(())).unwrap();
    assert_eq!(snapshot.works.len(), 3);
    assert_eq!(
        snapshot
            .works
            .iter()
            .map(|work| (work.identity.as_str(), work.state))
            .collect::<Vec<_>>(),
        vec![
            ("work-00", WorkState::Completed),
            ("work-01", WorkState::Partial),
            ("work-02", WorkState::Failed),
        ]
    );
}

#[test]
fn activity_updates_are_revisioned_ordered_and_current_only() {
    let (_temp, coordinator) = fixture("daemon-activity", 32, &["work-activity"]);
    let created = coordinator
        .submit(create_conversation(
            "create-activity",
            "chat-activity",
            "turn-activity",
            "daemon-activity",
        ))
        .unwrap();
    let work = created.receipt().work_identity.clone();
    coordinator
        .submit(work_transition(
            "wait-activity",
            work.clone(),
            rev(1),
            WorkState::WaitingForModel,
            "daemon-activity",
        ))
        .unwrap();
    coordinator
        .submit(work_transition(
            "run-activity",
            work.clone(),
            rev(2),
            WorkState::Running,
            "daemon-activity",
        ))
        .unwrap();

    coordinator
        .submit(Command::set_activity(
            "mail-activity",
            work.clone(),
            rev(3),
            Some(WorkActivityCategory::Mail),
            DaemonGeneration::new("daemon-activity"),
        ))
        .unwrap();
    coordinator
        .submit(Command::set_activity(
            "web-activity",
            work.clone(),
            rev(4),
            Some(WorkActivityCategory::Web),
            DaemonGeneration::new("daemon-activity"),
        ))
        .unwrap();
    coordinator
        .submit(Command::set_activity(
            "clear-activity",
            work.clone(),
            rev(5),
            None,
            DaemonGeneration::new("daemon-activity"),
        ))
        .unwrap();

    let snapshot = coordinator.snapshot().unwrap();
    assert_eq!(snapshot.works[0].revision, rev(6));
    assert_eq!(snapshot.works[0].activity, None);
    let targeted = coordinator.work(&work).unwrap().unwrap();
    assert_eq!(targeted.revision, rev(6));
    assert_eq!(targeted.activity, None);
    let events = match coordinator
        .events(
            Some(EventCursor::new(3)),
            &DaemonGeneration::new("daemon-activity"),
        )
        .unwrap()
    {
        EventRead::Events(events) => events,
        EventRead::Gap { .. } => panic!("activity events retained"),
    };
    assert_eq!(
        events
            .iter()
            .map(|event| event.activity)
            .collect::<Vec<_>>(),
        vec![
            Some(WorkActivityCategory::Mail),
            Some(WorkActivityCategory::Web),
            None,
        ]
    );
}

#[test]
fn terminal_automation_attention_is_acknowledged_once_at_expected_revision() {
    let (_temp, coordinator) = fixture("daemon-attention", 32, &["work-attention"]);
    let work = coordinator
        .submit(create_automation(
            "create-attention",
            "run-attention",
            "session-attention",
            "definition-attention",
            1,
            "daemon-attention",
        ))
        .unwrap()
        .receipt()
        .work_identity
        .clone();
    for (command, revision, state) in [
        ("wait-attention", 1, WorkState::WaitingForModel),
        ("run-attention", 2, WorkState::Running),
        ("complete-attention", 3, WorkState::Completed),
    ] {
        coordinator
            .submit(work_transition(
                command,
                work.clone(),
                rev(revision),
                state,
                "daemon-attention",
            ))
            .unwrap();
    }

    let acknowledgement = coordinator
        .submit(Command::acknowledge_attention(
            "ack-attention",
            work.clone(),
            rev(4),
            DaemonGeneration::new("daemon-attention"),
        ))
        .unwrap();
    assert_eq!(acknowledgement.receipt().work_revision, rev(5));
    assert_eq!(acknowledgement.receipt().state, WorkState::Completed);

    assert!(matches!(
        coordinator.submit(Command::acknowledge_attention(
            "ack-attention-again",
            work,
            rev(5),
            DaemonGeneration::new("daemon-attention"),
        )),
        Err(CommandError::Conflict { current_revision: Some(revision) }) if revision == rev(5)
    ));
}

fn rev(value: u64) -> WorkRevision {
    WorkRevision::new(value)
}

fn create_conversation(
    command: impl Into<bagentd::work_coordinator::CommandIdentity>,
    chat: impl Into<String>,
    turn: impl Into<String>,
    generation: impl Into<String>,
) -> Command {
    Command::create_conversation(
        command,
        CurrentChatIdentity::new(chat),
        ConversationTurnIdentity::new(turn),
        DaemonGeneration::new(generation),
    )
}

fn create_automation(
    command: impl Into<bagentd::work_coordinator::CommandIdentity>,
    run: impl Into<String>,
    session: impl Into<String>,
    definition: impl Into<String>,
    revision: u64,
    generation: impl Into<String>,
) -> Command {
    Command::create_automation(
        command,
        AutomationRunIdentity::new(run),
        AutomationSessionIdentity::new(session),
        AutomationDefinitionIdentity::new(definition),
        AutomationDefinitionRevision::new(revision),
        DaemonGeneration::new(generation),
    )
}

fn work_transition(
    command: impl Into<bagentd::work_coordinator::CommandIdentity>,
    work: impl Into<WorkIdentity>,
    revision: WorkRevision,
    state: WorkState,
    generation: impl Into<String>,
) -> Command {
    Command::transition(
        command,
        work,
        revision,
        state,
        DaemonGeneration::new(generation),
    )
}

fn transition_with_model_runtime(
    command: impl Into<bagentd::work_coordinator::CommandIdentity>,
    work: impl Into<WorkIdentity>,
    revision: WorkRevision,
    state: WorkState,
    model_generation: impl Into<String>,
    daemon_generation: impl Into<String>,
) -> Command {
    Command::transition_with_model_runtime(
        command,
        work,
        revision,
        state,
        ModelRuntimeGeneration::new(model_generation),
        DaemonGeneration::new(daemon_generation),
    )
}

fn request_approval(
    command: impl Into<bagentd::work_coordinator::CommandIdentity>,
    work: impl Into<WorkIdentity>,
    revision: WorkRevision,
    approval: impl Into<String>,
    category: impl Into<String>,
    generation: impl Into<String>,
) -> Command {
    Command::request_approval(
        command,
        work,
        revision,
        ApprovalIdentity::new(approval),
        category,
        DaemonGeneration::new(generation),
    )
}

fn fixture(generation: &str, max_events: usize, identities: &[&str]) -> (TempDir, WorkCoordinator) {
    let temp = tempfile::tempdir().expect("temporary database directory");
    let coordinator = WorkCoordinator::open_with_dependencies(
        temp.path().join("work.sqlite3"),
        CoordinatorConfig { max_events },
        DaemonGeneration::new(generation),
        dependencies(identities),
    )
    .expect("open coordinator");
    (temp, coordinator)
}

#[test]
fn persistence_atomicity() {
    for failure in [
        FailurePoint::BeforeTransaction,
        FailurePoint::AfterStateMutation,
        FailurePoint::AfterOutboxInsert,
        FailurePoint::AtCommit,
    ] {
        let (_temp, coordinator) = fixture("daemon-a05", 32, &["work-a05", "work-a05"]);
        let created = coordinator
            .submit(create_conversation(
                "create-a05",
                "chat-a05",
                "turn-a05",
                "daemon-a05",
            ))
            .expect("create work");
        assert!(matches!(created, CommandAcknowledgement::Committed(_)));

        let transition = work_transition(
            "transition-a05",
            "work-a05",
            rev(1),
            WorkState::WaitingForModel,
            "daemon-a05",
        );
        assert!(matches!(
            coordinator.submit_with_failure(transition.clone(), failure),
            Err(CommandError::InjectedFailure(point)) if point == failure
        ));

        let unchanged = coordinator.snapshot().expect("snapshot after rollback");
        assert_eq!(unchanged.cursor.value(), 1);
        assert_eq!(unchanged.works.len(), 1);
        assert_eq!(unchanged.works[0].revision.value(), 1);
        assert_eq!(unchanged.works[0].state, WorkState::Queued);
        assert_eq!(coordinator.verify_integrity().unwrap(), "ok");

        assert!(matches!(
            coordinator.submit(transition),
            Ok(CommandAcknowledgement::Committed(_))
        ));
        let committed = coordinator.snapshot().expect("snapshot after retry");
        assert_eq!(committed.cursor.value(), 2);
        assert_eq!(committed.works[0].revision.value(), 2);
        assert_eq!(committed.works[0].state, WorkState::WaitingForModel);
        assert_eq!(coordinator.verify_integrity().unwrap(), "ok");
    }

    let (_temp, coordinator) = fixture(
        "daemon-a05-constraints",
        32,
        &["work-a05-automation-one", "work-a05-automation-two"],
    );
    coordinator
        .submit(create_automation(
            "create-a05-automation-one",
            "run-a05-one",
            "session-a05-one",
            "definition-a05",
            1,
            "daemon-a05-constraints",
        ))
        .unwrap();
    assert!(coordinator
        .submit(create_automation(
            "create-a05-automation-two",
            "run-a05-two",
            "session-a05-two",
            "definition-a05",
            1,
            "daemon-a05-constraints",
        ))
        .is_err());
    let constrained = coordinator.snapshot().unwrap();
    assert_eq!(constrained.works.len(), 1);
    assert_eq!(constrained.cursor.value(), 1);
    assert_eq!(coordinator.verify_integrity().unwrap(), "ok");
}

#[test]
fn revision_conflicts() {
    let temp = tempfile::tempdir().expect("temporary database directory");
    let path = temp.path().join("work.sqlite3");
    let first = WorkCoordinator::open_with_dependencies(
        &path,
        CoordinatorConfig { max_events: 32 },
        DaemonGeneration::new("daemon-a06"),
        dependencies(&["work-a06"]),
    )
    .expect("first coordinator handle");
    first
        .submit(create_conversation(
            "create-a06",
            "chat-a06",
            "turn-a06",
            "daemon-a06",
        ))
        .expect("create work");
    let second = WorkCoordinator::open(
        &path,
        CoordinatorConfig { max_events: 32 },
        DaemonGeneration::new("daemon-a06"),
    )
    .expect("second coordinator handle");

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let first_barrier = barrier.clone();
    let first_writer = std::thread::spawn(move || {
        first_barrier.wait();
        first.submit(work_transition(
            "writer-a06-first",
            "work-a06",
            rev(1),
            WorkState::WaitingForModel,
            "daemon-a06",
        ))
    });
    let second_barrier = barrier.clone();
    let second_writer = std::thread::spawn(move || {
        second_barrier.wait();
        second.submit(work_transition(
            "writer-a06-second",
            "work-a06",
            rev(1),
            WorkState::Cancelling,
            "daemon-a06",
        ))
    });
    barrier.wait();
    let outcomes = [first_writer.join().unwrap(), second_writer.join().unwrap()];

    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(CommandAcknowledgement::Committed(_))))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(
                outcome,
                Err(CommandError::Conflict {
                    current_revision: Some(revision)
                }) if revision == &WorkRevision::new(2)
            ))
            .count(),
        1
    );

    let inspector = WorkCoordinator::open(
        &path,
        CoordinatorConfig { max_events: 32 },
        DaemonGeneration::new("daemon-a06"),
    )
    .unwrap();
    let after_race = inspector.snapshot().unwrap();
    assert_eq!(after_race.cursor.value(), 2);
    assert_eq!(after_race.works[0].revision.value(), 2);
    let next = match after_race.works[0].state {
        WorkState::WaitingForModel => WorkState::Running,
        WorkState::Cancelling => WorkState::Cancelled,
        other => panic!("unexpected winning state: {other:?}"),
    };
    let follow_up = inspector
        .submit(work_transition(
            "follow-up-a06",
            "work-a06",
            rev(2),
            next,
            "daemon-a06",
        ))
        .unwrap();
    assert_eq!(follow_up.receipt().work_revision.value(), 3);
    assert_eq!(follow_up.receipt().event_cursor.value(), 3);
}

#[test]
fn command_idempotency_restart() {
    let temp = tempfile::tempdir().expect("temporary database directory");
    let path = temp.path().join("work.sqlite3");
    let coordinator = WorkCoordinator::open_with_dependencies(
        &path,
        CoordinatorConfig { max_events: 32 },
        DaemonGeneration::new("daemon-a07"),
        dependencies(&["work-a07"]),
    )
    .unwrap();
    coordinator
        .submit(create_automation(
            "create-a07",
            "run-a07",
            "session-a07",
            "automation-a07",
            7,
            "daemon-a07",
        ))
        .unwrap();
    let transition = work_transition(
        "stable-command-a07",
        "work-a07",
        rev(1),
        WorkState::WaitingForModel,
        "daemon-a07",
    );
    assert!(matches!(
        coordinator
            .submit_with_failure(transition.clone(), FailurePoint::AfterCommitBeforeResponse),
        Err(CommandError::InjectedFailure(
            FailurePoint::AfterCommitBeforeResponse
        ))
    ));
    drop(coordinator);

    let restarted_client = WorkCoordinator::open(
        &path,
        CoordinatorConfig { max_events: 32 },
        DaemonGeneration::new("daemon-a07-restarted"),
    )
    .unwrap();
    let replay = restarted_client.submit(transition.clone()).unwrap();
    assert!(matches!(replay, CommandAcknowledgement::Committed(_)));
    assert_eq!(replay.receipt().work_revision.value(), 2);
    assert_eq!(replay.receipt().event_cursor.value(), 2);
    assert_eq!(restarted_client.submit(transition).unwrap(), replay);

    let snapshot = restarted_client.snapshot().unwrap();
    assert_eq!(snapshot.works[0].revision.value(), 3);
    assert_eq!(snapshot.works[0].state, WorkState::Abandoned);
    assert_eq!(snapshot.cursor.value(), 3);
    assert!(matches!(
        restarted_client.submit(work_transition(
            "stable-command-a07",
            "work-a07",
            rev(2),
            WorkState::Running,
            "daemon-a07-restarted",
        )),
        Err(CommandError::CommandIdentityConflict)
    ));
}

#[test]
fn event_ordering_cursor_gap() {
    let temp = tempfile::tempdir().expect("temporary database directory");
    let path = temp.path().join("work.sqlite3");
    let first = WorkCoordinator::open_with_dependencies(
        &path,
        CoordinatorConfig { max_events: 2 },
        DaemonGeneration::new("daemon-a08"),
        dependencies(&["work-a08-conversation"]),
    )
    .unwrap();
    let second = WorkCoordinator::open_with_dependencies(
        &path,
        CoordinatorConfig { max_events: 2 },
        DaemonGeneration::new("daemon-a08"),
        dependencies(&["work-a08-automation"]),
    )
    .unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let first_barrier = barrier.clone();
    let first_create = std::thread::spawn(move || {
        first_barrier.wait();
        first.submit(create_conversation(
            "create-a08-conversation",
            "chat-a08",
            "turn-a08",
            "daemon-a08",
        ))
    });
    let second_barrier = barrier.clone();
    let second_create = std::thread::spawn(move || {
        second_barrier.wait();
        second.submit(create_automation(
            "create-a08-automation",
            "run-a08",
            "session-a08",
            "automation-a08",
            8,
            "daemon-a08",
        ))
    });
    barrier.wait();
    first_create.join().unwrap().unwrap();
    second_create.join().unwrap().unwrap();

    let reader = WorkCoordinator::open(
        &path,
        CoordinatorConfig { max_events: 2 },
        DaemonGeneration::new("daemon-a08"),
    )
    .unwrap();
    assert!(matches!(
        reader
            .events(None, &DaemonGeneration::new("daemon-a08"))
            .unwrap(),
        EventRead::Gap { snapshot } if snapshot.cursor.value() == 2
    ));
    let initial = match reader
        .events(
            Some(EventCursor::new(0)),
            &DaemonGeneration::new("daemon-a08"),
        )
        .unwrap()
    {
        EventRead::Events(events) => events,
        EventRead::Gap { .. } => panic!("initial retained range must be continuous"),
    };
    assert_eq!(
        initial
            .iter()
            .map(|event| event.event_cursor.value())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_ne!(initial[0].work_identity, initial[1].work_identity);

    for event in initial {
        reader
            .submit(work_transition(
                format!("transition-a08-{}", event.work_identity),
                event.work_identity,
                rev(1),
                WorkState::WaitingForModel,
                "daemon-a08",
            ))
            .unwrap();
    }
    let retained = match reader
        .events(
            Some(EventCursor::new(2)),
            &DaemonGeneration::new("daemon-a08"),
        )
        .unwrap()
    {
        EventRead::Events(events) => events,
        EventRead::Gap { .. } => panic!("cursor two is inside retention"),
    };
    assert_eq!(
        retained
            .iter()
            .map(|event| event.event_cursor.value())
            .collect::<Vec<_>>(),
        vec![3, 4]
    );
    assert!(matches!(
        reader
            .events(
                Some(EventCursor::new(1)),
                &DaemonGeneration::new("daemon-a08"),
            )
            .unwrap(),
        EventRead::Gap { snapshot } if snapshot.cursor.value() == 4
    ));
}

fn reduce_events(
    mut snapshot: bagentd::work_coordinator::WorkSnapshot,
    events: &[bagentd::work_coordinator::WorkEvent],
) -> bagentd::work_coordinator::WorkSnapshot {
    for event in events {
        let work = snapshot
            .works
            .iter_mut()
            .find(|work| work.identity == event.work_identity)
            .expect("event Work exists in starting snapshot");
        if event.work_revision > work.revision {
            work.revision = event.work_revision;
            work.state = event.state;
        }
        snapshot.cursor = snapshot.cursor.max(event.event_cursor);
    }
    snapshot
}

#[test]
fn snapshot_reconnect() {
    let (_temp, coordinator) = fixture("daemon-a09", 2, &["work-a09-one", "work-a09-two"]);
    for suffix in ["one", "two"] {
        coordinator
            .submit(create_conversation(
                format!("create-a09-{suffix}"),
                "chat-a09",
                format!("turn-a09-{suffix}"),
                "daemon-a09",
            ))
            .unwrap();
    }
    let at_revision_n = coordinator.snapshot().unwrap();
    assert_eq!(at_revision_n.cursor.value(), 2);

    for suffix in ["one", "two"] {
        coordinator
            .submit(work_transition(
                format!("waiting-a09-{suffix}"),
                format!("work-a09-{suffix}"),
                rev(1),
                WorkState::WaitingForModel,
                "daemon-a09",
            ))
            .unwrap();
    }
    let within_retention = match coordinator
        .events(
            Some(EventCursor::new(2)),
            &DaemonGeneration::new("daemon-a09"),
        )
        .unwrap()
    {
        EventRead::Events(events) => events,
        EventRead::Gap { .. } => panic!("cursor two remains within retention"),
    };
    let mut duplicated_delivery = within_retention.clone();
    duplicated_delivery.extend(within_retention);
    assert_eq!(
        reduce_events(at_revision_n, &duplicated_delivery),
        coordinator.snapshot().unwrap()
    );

    let before_expiry = coordinator.snapshot().unwrap();
    coordinator
        .submit(work_transition(
            "running-a09-one",
            "work-a09-one",
            rev(2),
            WorkState::Running,
            "daemon-a09",
        ))
        .unwrap();
    coordinator
        .submit(request_approval(
            "approval-a09-one",
            "work-a09-one",
            rev(3),
            "approval-record-a09",
            "filesystem_write",
            "daemon-a09",
        ))
        .unwrap();
    coordinator
        .submit(work_transition(
            "running-a09-two",
            "work-a09-two",
            rev(2),
            WorkState::Running,
            "daemon-a09",
        ))
        .unwrap();
    let recovered = match coordinator
        .events(
            Some(before_expiry.cursor),
            &DaemonGeneration::new("daemon-a09"),
        )
        .unwrap()
    {
        EventRead::Gap { snapshot } => snapshot,
        EventRead::Events(_) => panic!("expired cursor must require a snapshot"),
    };
    assert_eq!(recovered, coordinator.snapshot().unwrap());
}

#[test]
fn daemon_restart_recovery() {
    let temp = tempfile::tempdir().expect("temporary database directory");
    let path = temp.path().join("work.sqlite3");
    let coordinator = WorkCoordinator::open_with_dependencies(
        &path,
        CoordinatorConfig { max_events: 64 },
        DaemonGeneration::new("daemon-a10-before"),
        dependencies(&[
            "work-a10-queued",
            "work-a10-waiting",
            "work-a10-running",
            "work-a10-approval",
            "work-a10-cancelling",
            "work-a10-completed",
            "work-a10-automation",
        ]),
    )
    .unwrap();
    for suffix in [
        "queued",
        "waiting",
        "running",
        "approval",
        "cancelling",
        "completed",
    ] {
        coordinator
            .submit(create_conversation(
                format!("create-a10-{suffix}"),
                "chat-a10",
                format!("turn-a10-{suffix}"),
                "daemon-a10-before",
            ))
            .unwrap();
    }
    coordinator
        .submit(create_automation(
            "create-a10-automation",
            "run-a10-active",
            "session-a10-active",
            "automation-a10-active",
            10,
            "daemon-a10-before",
        ))
        .unwrap();
    for suffix in ["waiting", "running", "approval", "completed"] {
        coordinator
            .submit(work_transition(
                format!("waiting-a10-{suffix}"),
                format!("work-a10-{suffix}"),
                rev(1),
                WorkState::WaitingForModel,
                "daemon-a10-before",
            ))
            .unwrap();
    }
    for suffix in ["running", "approval", "completed"] {
        let command = if suffix == "running" {
            transition_with_model_runtime(
                "running-a10-running",
                "work-a10-running",
                rev(2),
                WorkState::Running,
                "model-runtime-a10-before",
                "daemon-a10-before",
            )
        } else {
            work_transition(
                format!("running-a10-{suffix}"),
                format!("work-a10-{suffix}"),
                rev(2),
                WorkState::Running,
                "daemon-a10-before",
            )
        };
        coordinator.submit(command).unwrap();
    }
    coordinator
        .submit(request_approval(
            "approval-a10",
            "work-a10-approval",
            rev(3),
            "approval-record-a10",
            "filesystem_write",
            "daemon-a10-before",
        ))
        .unwrap();
    coordinator
        .submit(work_transition(
            "cancelling-a10",
            "work-a10-cancelling",
            rev(1),
            WorkState::Cancelling,
            "daemon-a10-before",
        ))
        .unwrap();
    coordinator
        .submit(work_transition(
            "completed-a10",
            "work-a10-completed",
            rev(3),
            WorkState::Completed,
            "daemon-a10-before",
        ))
        .unwrap();
    let before_restart = coordinator.snapshot().unwrap();
    assert_eq!(before_restart.cursor.value(), 17);
    assert_eq!(before_restart.automation_runs.len(), 1);
    assert!(before_restart.automation_runs[0].active);
    assert_eq!(before_restart.approvals.len(), 1);
    assert_eq!(
        before_restart.approvals[0].state,
        bagentd::work_coordinator::ApprovalState::Pending
    );
    assert!(before_restart.model_runtime_trusted);
    assert_eq!(
        before_restart
            .model_runtime_generation
            .as_ref()
            .map(ModelRuntimeGeneration::as_str),
        Some("model-runtime-a10-before")
    );
    drop(coordinator);

    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--ignored")
        .arg("--exact")
        .arg("daemon_restart_fixture_process")
        .env("BAGENT_STAGE2_RESTART_DB", &path)
        .status()
        .expect("launch isolated recovery subprocess");
    assert!(child.success());
    let recovered = WorkCoordinator::open(
        &path,
        CoordinatorConfig { max_events: 64 },
        DaemonGeneration::new("daemon-a10-after"),
    )
    .unwrap();
    let after_restart = recovered.snapshot().unwrap();
    assert_eq!(
        after_restart.daemon_generation,
        DaemonGeneration::new("daemon-a10-after")
    );
    assert_eq!(after_restart.cursor.value(), 22);
    assert_eq!(after_restart.automation_runs.len(), 1);
    assert!(!after_restart.automation_runs[0].active);
    assert_eq!(after_restart.approvals.len(), 1);
    assert_eq!(
        after_restart.approvals[0].state,
        bagentd::work_coordinator::ApprovalState::Pending
    );
    assert_eq!(after_restart.interruptions.len(), 4);
    assert!(!after_restart.model_runtime_trusted);
    assert_eq!(after_restart.model_runtime_generation, None);
    for work in &after_restart.works {
        if work.identity.as_str() == "work-a10-completed" {
            assert_eq!(work.state, WorkState::Completed);
            assert_eq!(work.revision.value(), 4);
        } else if work.identity.as_str() == "work-a10-approval" {
            assert_eq!(work.state, WorkState::WaitingForApproval);
            let prior = before_restart
                .works
                .iter()
                .find(|prior| prior.identity == work.identity)
                .unwrap();
            assert_eq!(work.revision, prior.revision);
        } else {
            assert_eq!(work.state, WorkState::Abandoned);
            let prior = before_restart
                .works
                .iter()
                .find(|prior| prior.identity == work.identity)
                .unwrap();
            assert_eq!(work.revision.value(), prior.revision.value() + 1);
        }
    }
    let recovery_events = match recovered
        .events(
            Some(before_restart.cursor),
            &DaemonGeneration::new("daemon-a10-after"),
        )
        .unwrap()
    {
        EventRead::Events(events) => events,
        EventRead::Gap { .. } => panic!("recovery events remain retained"),
    };
    assert_eq!(
        recovery_events
            .iter()
            .map(|event| event.event_cursor.value())
            .collect::<Vec<_>>(),
        vec![18, 19, 20, 21, 22]
    );
    assert!(recovery_events.iter().all(|event| {
        event.state == WorkState::Abandoned
            && event.daemon_generation == DaemonGeneration::new("daemon-a10-after")
    }));
    assert!(matches!(
        recovered
            .events(
                Some(before_restart.cursor),
                &DaemonGeneration::new("daemon-a10-before")
            )
            .unwrap(),
        EventRead::Gap { snapshot } if snapshot == after_restart
    ));
    assert!(matches!(
        recovered.submit(create_conversation(
            "stale-a10",
            "chat-a10",
            "turn-a10-stale",
            "daemon-a10-before",
        )),
        Err(CommandError::StaleDaemonGeneration { current })
            if current == DaemonGeneration::new("daemon-a10-after")
    ));
}

#[test]
#[ignore = "subprocess entrypoint invoked only by daemon_restart_recovery"]
fn daemon_restart_fixture_process() {
    let path = std::env::var_os("BAGENT_STAGE2_RESTART_DB")
        .expect("subprocess database path supplied by parent");
    let coordinator = WorkCoordinator::open(
        path,
        CoordinatorConfig { max_events: 64 },
        DaemonGeneration::new("daemon-a10-after"),
    )
    .expect("recover coordinator in isolated subprocess");
    let snapshot = coordinator.snapshot().unwrap();
    assert_eq!(
        snapshot.daemon_generation,
        DaemonGeneration::new("daemon-a10-after")
    );
    assert!(snapshot.works.iter().all(|work| {
        work.state == WorkState::Abandoned
            || work.state == WorkState::Completed
            || work.state == WorkState::WaitingForApproval
    }));
    assert!(snapshot
        .approvals
        .iter()
        .all(|approval| approval.state == bagentd::work_coordinator::ApprovalState::Pending));
    assert!(!snapshot.model_runtime_trusted);
}
