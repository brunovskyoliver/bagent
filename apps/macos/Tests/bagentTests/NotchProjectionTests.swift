import XCTest
@testable import bagent

final class NotchProjectionTests: XCTestCase {
    func testEveryWorkStateHasAnExactDeterministicProjection() throws {
        let expectations: [(NotchWorkState, NotchTerminalAttention?, StageRailStage?, String?, CGFloat)] = [
            (.queued, nil, .think, "ACTIVE", 78),
            (.waitingForModel, nil, .model, "ACTIVE", 78),
            (.running, nil, .think, "ACTIVE", 78),
            (.waitingForApproval, nil, .think, "ACTIVE", 78),
            (.cancelling, nil, .think, "ACTIVE", 78),
            (.completed, .unread, .done, "UNREAD", 98),
            (.partial, .partial, .done, "PARTIAL", 98),
            (.failed, .failed, .done, "FAILED", 98),
            (.cancelled, nil, nil, nil, 0),
            (.abandoned, nil, nil, nil, 0),
        ]

        for (state, attention, stage, pill, height) in expectations {
            let snapshot = NotchWorkSnapshot(
                schemaVersion: 1,
                cursor: 42,
                daemonGeneration: "daemon-fixture",
                works: [work(
                    identity: "work-\(state.rawValue)",
                    revision: 3,
                    origin: .automation,
                    state: state,
                    terminalAttention: attention
                )],
                pendingApprovals: [],
                model: .unloaded
            )

            let first = try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot))
            let second = try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot))

            XCTAssertEqual(first, second, "projection must be pure for \(state)")
            XCTAssertEqual(first.revision, .init(cursor: 42, daemonGeneration: "daemon-fixture"))
            XCTAssertEqual(first.rail.selectedStage, stage, "stage for \(state)")
            XCTAssertEqual(first.statusPill.label, pill, "pill for \(state)")
            XCTAssertEqual(first.geometry.bridgeHeight, height, "height for \(state)")
        }
    }

    func testWaitingForModelSnapshotSelectsModelWithoutTransportInference() throws {
        let snapshot = NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 7,
            daemonGeneration: "daemon-a",
            works: [
                NotchWork(
                    identity: "work-chat",
                    revision: 2,
                    origin: .conversation,
                    state: .waitingForModel,
                    activity: nil,
                    queuePosition: nil,
                    automationDisplayName: nil,
                    terminalAttention: nil
                )
            ],
            pendingApprovals: [],
            model: .unloaded
        )

        let presentation = try NotchProjection.reduce(
            previous: .idle,
            input: .snapshot(snapshot)
        )

        XCTAssertEqual(presentation.revision, .init(cursor: 7, daemonGeneration: "daemon-a"))
        XCTAssertEqual(presentation.interactionMode, .thinking)
        XCTAssertEqual(presentation.rail.selectedStage, .model)
        XCTAssertEqual(presentation.statusPill.label, "ACTIVE")
        XCTAssertEqual(presentation.geometry, .init(wingWidth: 248, bridgeHeight: 78))
    }

    func testOrderedEventAdvancesRevisionAndDuplicateHasNoEffect() throws {
        let initial = NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 10,
            daemonGeneration: "daemon-a",
            works: [work(identity: "automation-a", revision: 1, origin: .automation, state: .queued)],
            pendingApprovals: [],
            model: .ready
        )
        let queued = try NotchProjection.reduce(previous: .idle, input: .snapshot(initial))
        let runningWork = work(
            identity: "automation-a",
            revision: 2,
            origin: .automation,
            state: .running,
            activity: .init(category: .mail)
        )
        let event = NotchWorkEvent(
            schemaVersion: 1,
            cursor: 11,
            daemonGeneration: "daemon-a",
            work: runningWork,
            model: .ready
        )

        let running = try NotchProjection.reduce(previous: queued, input: .event(event))
        let duplicate = try NotchProjection.reduce(previous: running, input: .event(event))

        XCTAssertEqual(running.revision.cursor, 11)
        XCTAssertEqual(running.rail.selectedStage, .tool)
        XCTAssertEqual(running.rail.activityCategory, .mail)
        XCTAssertEqual(duplicate, running)
    }

    func testOutOfOrderAndRevisionSkippingEventsAreRejected() throws {
        let snapshot = NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 5,
            daemonGeneration: "daemon-a",
            works: [work(identity: "automation-a", revision: 1, origin: .automation, state: .queued)],
            pendingApprovals: [],
            model: .ready
        )
        let initial = try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot))

        XCTAssertThrowsError(try NotchProjection.reduce(
            previous: initial,
            input: .event(.init(
                schemaVersion: 1,
                cursor: 7,
                daemonGeneration: "daemon-a",
                work: work(identity: "automation-a", revision: 2, origin: .automation, state: .running),
                model: .ready
            ))
        )) { error in
            XCTAssertEqual(error as? NotchProjectionError, .cursorGap(expected: 6, actual: 7))
        }

        XCTAssertThrowsError(try NotchProjection.reduce(
            previous: initial,
            input: .event(.init(
                schemaVersion: 1,
                cursor: 6,
                daemonGeneration: "daemon-a",
                work: work(identity: "automation-a", revision: 3, origin: .automation, state: .running),
                model: .ready
            ))
        )) { error in
            XCTAssertEqual(
                error as? NotchProjectionError,
                .revisionMismatch(workIdentity: "automation-a", expected: 2, actual: 3)
            )
        }
    }

    private func work(
        identity: String,
        revision: UInt64,
        origin: NotchWorkOrigin,
        state: NotchWorkState,
        activity: NotchActivity? = nil,
        terminalAttention: NotchTerminalAttention? = nil
    ) -> NotchWork {
        NotchWork(
            identity: identity,
            revision: revision,
            origin: origin,
            state: state,
            activity: activity,
            queuePosition: nil,
            automationDisplayName: nil,
            terminalAttention: terminalAttention
        )
    }
}
