use bagentd::work_coordinator::{
    Command, ConversationTurnIdentity, CoordinatorConfig, CoordinatorDependencies,
    CurrentChatIdentity, DaemonGeneration, DeterministicWorkIdentitySource, EventCursor, EventRead,
    FixedCoordinatorClock, WorkCoordinator, WorkEvent,
};

#[test]
fn work_surfaces() {
    const CANARIES: [&str; 6] = [
        "CANARY_CREDENTIAL",
        "CANARY_RAW_ARGUMENT",
        "CANARY_REASONING",
        "CANARY_EVIDENCE",
        "CANARY_PRIVATE_IDENTITY",
        "CANARY_PROMPT",
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("privacy.sqlite");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE forbidden_source_fixture (
           credential TEXT, raw_argument TEXT, reasoning TEXT,
           evidence_content TEXT, private_identity TEXT, prompt TEXT
         );
         INSERT INTO forbidden_source_fixture VALUES (
           'CANARY_CREDENTIAL','CANARY_RAW_ARGUMENT','CANARY_REASONING',
           'CANARY_EVIDENCE','CANARY_PRIVATE_IDENTITY','CANARY_PROMPT'
         );",
        )
        .unwrap();
    drop(connection);
    let generation = DaemonGeneration::new("privacy-generation");
    let coordinator = WorkCoordinator::open_with_dependencies(
        &path,
        CoordinatorConfig::default(),
        generation.clone(),
        CoordinatorDependencies {
            identity_source: Box::new(DeterministicWorkIdentitySource::new(["opaque-work"])),
            clock: Box::new(FixedCoordinatorClock::new("2026-08-18T20:00:00Z")),
        },
    )
    .unwrap();
    coordinator
        .submit(Command::create_conversation(
            "privacy-command",
            CurrentChatIdentity::new("opaque-chat"),
            ConversationTurnIdentity::new("opaque-turn"),
            generation.clone(),
        ))
        .unwrap();

    let snapshot = coordinator.snapshot().unwrap();
    let events = match coordinator
        .events(Some(EventCursor::new(0)), &generation)
        .unwrap()
    {
        EventRead::Events(events) => events,
        EventRead::Gap { .. } => panic!("retained event expected"),
    };
    let surfaces = [
        serde_json::to_string(&snapshot).unwrap(),
        serde_json::to_string(&events).unwrap(),
        serde_json::to_string(&snapshot.compact_projection()).unwrap(),
        serde_json::to_string(&snapshot.structural_diagnostic()).unwrap(),
        format!("{:?}", snapshot.structural_diagnostic()),
    ]
    .join("\n");
    for canary in CANARIES {
        assert!(!surfaces.contains(canary), "forbidden {canary} leaked");
    }

    let mut unknown_event = serde_json::to_value(&events[0]).unwrap();
    unknown_event.as_object_mut().unwrap().insert(
        "raw_arguments".to_owned(),
        serde_json::Value::String("CANARY_RAW_ARGUMENT".to_owned()),
    );
    assert!(serde_json::from_value::<WorkEvent>(unknown_event).is_err());
    let mut unknown_snapshot = serde_json::to_value(&snapshot).unwrap();
    unknown_snapshot.as_object_mut().unwrap().insert(
        "reasoning".to_owned(),
        serde_json::Value::String("CANARY_REASONING".to_owned()),
    );
    assert!(
        serde_json::from_value::<bagentd::work_coordinator::WorkSnapshot>(unknown_snapshot)
            .is_err()
    );
}
