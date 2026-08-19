import XCTest
@testable import bagent

@MainActor
final class EventConsumerRecoveryTests: XCTestCase {
    func testCursorGapFetchesExactlyOneReplacementSnapshot() async throws {
        let initial = snapshot(cursor: 10, revision: 1, state: .queued)
        let replacement = snapshot(cursor: 20, revision: 3, state: .running)
        let transport = FakeNotchEventTransport(
            snapshots: [initial, replacement],
            batches: [.events([
                NotchWorkEvent(
                    schemaVersion: 1,
                    cursor: 12,
                    daemonGeneration: "daemon-a",
                    work: work(revision: 2, state: .waitingForModel),
                    model: .loading
                )
            ])]
        )
        let consumer = NotchEventConsumer(transport: transport)

        try await consumer.synchronize()
        try await consumer.synchronize()

        let presentation = consumer.presentation
        let snapshotFetchCount = await transport.snapshotFetchCount
        XCTAssertEqual(snapshotFetchCount, 2)
        XCTAssertEqual(presentation.revision.cursor, 20)
        XCTAssertEqual(presentation.rail.selectedStage, .think)
    }

    func testSchemaRevisionAndGenerationDiscontinuitiesEachFetchOneSnapshot() async throws {
        let badEvents: [NotchWorkEvent] = [
            .init(schemaVersion: 2, cursor: 11, daemonGeneration: "daemon-a", work: work(revision: 2, state: .running), model: .ready),
            .init(schemaVersion: 1, cursor: 11, daemonGeneration: "daemon-a", work: work(revision: 3, state: .running), model: .ready),
            .init(schemaVersion: 1, cursor: 11, daemonGeneration: "daemon-b", work: work(revision: 2, state: .running), model: .ready),
        ]

        for badEvent in badEvents {
            let replacement = snapshot(cursor: 20, revision: 4, state: .running)
            let transport = FakeNotchEventTransport(
                snapshots: [snapshot(cursor: 10, revision: 1, state: .queued), replacement],
                batches: [.events([badEvent])]
            )
            let consumer = NotchEventConsumer(transport: transport, consumerFence: "fixed-fence")

            try await consumer.synchronize()
            try await consumer.synchronize()

            let stats = await transport.stats()
            XCTAssertEqual(consumer.presentation.revision.cursor, 20)
            XCTAssertEqual(stats.snapshotFetchCount, 2)
            XCTAssertEqual(stats.uniqueFences, ["fixed-fence"])
        }
    }

    func testServerGapInstallsReplacementWithoutStartingAnotherConsumer() async throws {
        let replacement = snapshot(cursor: 30, revision: 5, state: .running)
        let transport = FakeNotchEventTransport(
            snapshots: [snapshot(cursor: 10, revision: 1, state: .queued)],
            batches: [.gap(replacement)]
        )
        let consumer = NotchEventConsumer(transport: transport, consumerFence: "one-consumer")

        try await consumer.synchronize()
        try await consumer.synchronize()

        let stats = await transport.stats()
        XCTAssertEqual(consumer.presentation.revision.cursor, 30)
        XCTAssertEqual(stats.snapshotFetchCount, 1)
        XCTAssertEqual(stats.uniqueFences, ["one-consumer"])
    }

    func testReconnectInvalidationFetchesOneFreshSnapshotWithSameFence() async throws {
        let transport = FakeNotchEventTransport(
            snapshots: [
                snapshot(cursor: 10, revision: 1, state: .queued),
                snapshot(cursor: 40, revision: 6, state: .running),
            ],
            batches: []
        )
        let consumer = NotchEventConsumer(transport: transport, consumerFence: "reconnect-fence")

        try await consumer.synchronize()
        consumer.invalidateConsumerFence()
        try await consumer.synchronize()

        let stats = await transport.stats()
        XCTAssertEqual(consumer.presentation.revision.cursor, 40)
        XCTAssertEqual(stats.snapshotFetchCount, 2)
        XCTAssertEqual(stats.uniqueFences, ["reconnect-fence"])
    }

    func testConsumerFenceConflictPropagatesWithoutReplacingAuthoritativeState() async throws {
        let initial = snapshot(cursor: 10, revision: 1, state: .queued)
        let transport = FencedNotchEventTransport(snapshot: initial)
        let consumer = NotchEventConsumer(transport: transport, consumerFence: "old-consumer")

        try await consumer.synchronize()
        do {
            try await consumer.synchronize()
            XCTFail("stale consumer must stop")
        } catch {
            XCTAssertEqual(error as? NotchEventTransportError, .consumerFenced)
        }

        XCTAssertEqual(consumer.presentation.revision.cursor, 10)
        let stats = await transport.stats()
        XCTAssertEqual(stats.snapshots, 1)
        XCTAssertEqual(stats.events, 1)
    }

    func testWorkEventPreservesLiveReduceMotionPreference() async throws {
        let transport = FakeNotchEventTransport(
            snapshots: [snapshot(cursor: 10, revision: 1, state: .queued)],
            batches: [.events([.init(
                schemaVersion: 1,
                cursor: 11,
                daemonGeneration: "daemon-a",
                work: work(revision: 2, state: .running),
                model: .ready
            )])]
        )
        let consumer = NotchEventConsumer(transport: transport)

        try await consumer.synchronize()
        try consumer.setReduceMotion(true)
        try await consumer.synchronize()

        XCTAssertFalse(consumer.presentation.motion.iconMotionEnabled)
        XCTAssertEqual(consumer.presentation.motion.surfaceDuration, 0)
    }

    func testAttentionAcknowledgementClassifiesAllTransportOutcomes() throws {
        XCTAssertEqual(
            try DaemonClient.decodeWorkAttentionAcknowledgement(statusCode: 200, data: Data()),
            .acknowledged
        )
        XCTAssertEqual(
            try DaemonClient.decodeWorkAttentionAcknowledgement(
                statusCode: 409,
                data: Data(#"{"code":"work_conflict","error":"revision conflict"}"#.utf8)
            ),
            .authoritativeConflict
        )
        XCTAssertThrowsError(try DaemonClient.decodeWorkAttentionAcknowledgement(
            statusCode: 409,
            data: Data(#"{"code":"stale_consumer_fence","error":"changed wording"}"#.utf8)
        )) { error in
            XCTAssertEqual(error as? NotchEventTransportError, .consumerFenced)
        }
        XCTAssertThrowsError(try DaemonClient.decodeWorkAttentionAcknowledgement(
            statusCode: 503,
            data: Data()
        ))
        XCTAssertThrowsError(try DaemonClient.decodeWorkAttentionAcknowledgement(
            statusCode: 409,
            data: Data(#"{"code":"unknown_conflict"}"#.utf8)
        ))
        XCTAssertThrowsError(try DaemonClient.decodeWorkAttentionAcknowledgement(
            statusCode: 409,
            data: Data("not-json".utf8)
        ))
    }

    private func snapshot(cursor: UInt64, revision: UInt64, state: NotchWorkState) -> NotchWorkSnapshot {
        NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: cursor,
            daemonGeneration: "daemon-a",
            works: [work(revision: revision, state: state)],
            pendingApprovals: [],
            model: .ready
        )
    }

    private func work(revision: UInt64, state: NotchWorkState) -> NotchWork {
        NotchWork(
            identity: "work-a",
            revision: revision,
            origin: .automation,
            state: state,
            activity: nil,
            queuePosition: nil,
            automationDisplayName: "Morning check",
            terminalAttention: nil
        )
    }
}

private actor FencedNotchEventTransport: NotchEventTransport {
    let snapshot: NotchWorkSnapshot
    private var snapshotCount = 0
    private var eventCount = 0

    init(snapshot: NotchWorkSnapshot) {
        self.snapshot = snapshot
    }

    func fetchSnapshot(consumerFence _: String) async throws -> NotchWorkSnapshot {
        snapshotCount += 1
        return snapshot
    }

    func fetchEvents(
        after _: UInt64,
        daemonGeneration _: String,
        consumerFence _: String
    ) async throws -> NotchEventBatch {
        eventCount += 1
        throw NotchEventTransportError.consumerFenced
    }

    func stats() -> (snapshots: Int, events: Int) {
        (snapshotCount, eventCount)
    }
}

private actor FakeNotchEventTransport: NotchEventTransport {
    private var snapshots: [NotchWorkSnapshot]
    private var batches: [NotchEventBatch]
    private(set) var snapshotFetchCount = 0
    private var fences: [String] = []

    init(snapshots: [NotchWorkSnapshot], batches: [NotchEventBatch]) {
        self.snapshots = snapshots
        self.batches = batches
    }

    func stats() -> (snapshotFetchCount: Int, uniqueFences: Set<String>) {
        (snapshotFetchCount, Set(fences))
    }

    func fetchSnapshot(consumerFence: String) async throws -> NotchWorkSnapshot {
        snapshotFetchCount += 1
        fences.append(consumerFence)
        return snapshots.removeFirst()
    }

    func fetchEvents(
        after _: UInt64,
        daemonGeneration _: String,
        consumerFence: String
    ) async throws -> NotchEventBatch {
        fences.append(consumerFence)
        return batches.removeFirst()
    }
}
