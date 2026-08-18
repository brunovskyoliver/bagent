use bagentd::unified_work::{ExecutionBoundary, ExecutionBoundaryAdapter, UnifiedWorkAuthority};
use bagentd::work_coordinator::{
    Command, ConversationTurnIdentity, CoordinatorConfig, CoordinatorDependencies,
    CurrentChatIdentity, DaemonGeneration, DeterministicWorkIdentitySource, FailurePoint,
    FixedCoordinatorClock, WorkCoordinator,
};
use std::sync::Arc;

fn coordinator(identities: &[&str]) -> (tempfile::TempDir, WorkCoordinator) {
    let dir = tempfile::tempdir().unwrap();
    let coordinator = WorkCoordinator::open_with_dependencies(
        dir.path().join("failure.sqlite"),
        CoordinatorConfig::default(),
        DaemonGeneration::new("failure-generation"),
        CoordinatorDependencies {
            identity_source: Box::new(DeterministicWorkIdentitySource::new(
                identities.iter().copied(),
            )),
            clock: Box::new(FixedCoordinatorClock::new("2026-08-18T18:30:00Z")),
        },
    )
    .unwrap();
    (dir, coordinator)
}

fn create(identity: &str) -> Command {
    Command::create_conversation(
        format!("command-{identity}"),
        CurrentChatIdentity::new(format!("chat-{identity}")),
        ConversationTurnIdentity::new(format!("turn-{identity}")),
        DaemonGeneration::new("failure-generation"),
    )
}

#[test]
fn every_required_boundary_fails_closed() {
    let boundaries = [
        ("admission", FailurePoint::BeforeTransaction),
        ("persistence", FailurePoint::AfterStateMutation),
        ("outbox", FailurePoint::AfterOutboxInsert),
    ];
    for (name, point) in boundaries {
        let (_dir, coordinator) = coordinator(&[name]);
        let result = coordinator.submit_with_failure(create(name), point);
        assert!(result.is_err(), "{name} failpoint must be observable");
        let snapshot = coordinator.snapshot().unwrap();
        assert!(
            snapshot.works.is_empty(),
            "{name} must not partially admit Work"
        );
        assert!(
            snapshot.approvals.is_empty(),
            "{name} must not leak approval authority"
        );
        assert_eq!(snapshot.cursor.value(), 0, "{name} must not leak an event");
        assert_eq!(coordinator.verify_integrity().unwrap(), "ok");
    }

    let (_dir, committed_coordinator) = coordinator(&["after-response"]);
    let command = create("after-response");
    assert!(committed_coordinator
        .submit_with_failure(command.clone(), FailurePoint::AfterCommitBeforeResponse)
        .is_err());
    let replay = committed_coordinator.submit(command).unwrap();
    assert_eq!(replay.receipt().work_identity.as_str(), "after-response");
    assert_eq!(committed_coordinator.snapshot().unwrap().works.len(), 1);

    struct FailingAdapter {
        failure: ExecutionBoundary,
        crossed: Vec<ExecutionBoundary>,
    }
    impl ExecutionBoundaryAdapter for FailingAdapter {
        fn cross(
            &mut self,
            boundary: ExecutionBoundary,
            _work: &bagentd::work_coordinator::WorkIdentity,
        ) -> Result<(), String> {
            self.crossed.push(boundary);
            if boundary == self.failure {
                Err(format!("injected {boundary:?}"))
            } else {
                Ok(())
            }
        }
    }
    for (index, boundary) in [
        ExecutionBoundary::Runtime,
        ExecutionBoundary::Tool,
        ExecutionBoundary::Approval,
        ExecutionBoundary::Completion,
    ]
    .into_iter()
    .enumerate()
    {
        let (_dir, coordinator) = coordinator(&[&format!("adapter-{index}")]);
        let generation = DaemonGeneration::new("failure-generation");
        let authority = UnifiedWorkAuthority::new(Arc::new(coordinator), generation);
        let work = authority
            .submit_conversation(
                format!("adapter-create-{index}"),
                CurrentChatIdentity::new(format!("adapter-chat-{index}")),
                ConversationTurnIdentity::new(format!("adapter-turn-{index}")),
                0,
            )
            .unwrap();
        let mut adapter = FailingAdapter {
            failure: boundary,
            crossed: Vec::new(),
        };
        assert_eq!(
            authority.execute_with_adapter(work, &mut adapter).unwrap(),
            bagentd::work_coordinator::WorkState::Failed
        );
        assert_eq!(adapter.crossed.last(), Some(&boundary));
        assert!(authority.coordinator().snapshot().unwrap().works[0]
            .state
            .is_terminal());
        assert_eq!(authority.coordinator().verify_integrity().unwrap(), "ok");
    }
}
