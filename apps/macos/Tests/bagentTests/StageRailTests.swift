import XCTest
@testable import bagent

final class StageRailTests: XCTestCase {
    func testTwoConcurrentAutomationsUseSharedRailAndInvariantCountPill() throws {
        let snapshot = NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 4,
            daemonGeneration: "daemon-a",
            works: [
                automation("older", order: 1, category: .web),
                automation("newer", order: 2, category: .filesystem),
            ],
            pendingApprovals: [],
            model: .ready
        )

        let presentation = try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot))

        XCTAssertEqual(presentation.focusedWorkIdentity, "older")
        XCTAssertEqual(presentation.activeAutomationCount, 2)
        XCTAssertEqual(presentation.runPosition, 1)
        XCTAssertEqual(presentation.rail.selectedStage, .tool)
        XCTAssertEqual(presentation.statusPill.label, "2 ACTIVE")
        XCTAssertFalse(presentation.isThinking, "background work must not block Current Chat input")
        XCTAssertEqual(presentation.geometry, .init(wingWidth: 248, bridgeHeight: 150))
        XCTAssertEqual(NotchPillLayout.size, .init(width: 74, height: 18))
        XCTAssertEqual(NotchPillLayout.origin(maxPanelWidth: 741), .init(x: 643, y: 9))
    }

    func testFocusPriorityAndStableFIFOAutomationCycling() throws {
        let snapshot = NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 9,
            daemonGeneration: "daemon-a",
            works: [
                automation("older", order: 1, category: .web),
                automation("newer", order: 2, category: .filesystem),
                work("foreground", origin: .conversation, state: .running, activity: .init(category: .mail)),
                work("approval", origin: .automation, state: .waitingForApproval),
            ],
            pendingApprovals: [.init(identity: "approval-a", workIdentity: "approval")],
            model: .ready
        )

        var presentation = try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot))
        XCTAssertEqual(presentation.focusedWorkIdentity, "approval")
        XCTAssertEqual(presentation.statusPill.label, "APPROVE")
        XCTAssertFalse(presentation.statusPill.opensAutomations)

        var withoutApproval = snapshot
        withoutApproval.pendingApprovals = []
        withoutApproval.works.removeAll { $0.identity == "approval" }
        presentation = try NotchProjection.reduce(previous: presentation, input: .snapshot(withoutApproval))
        XCTAssertEqual(presentation.focusedWorkIdentity, "foreground")
        XCTAssertEqual(presentation.rail.activityCategory, .mail)
        XCTAssertEqual(presentation.statusPill.label, "2 ACTIVE")
        XCTAssertTrue(presentation.statusPill.opensAutomations)

        var automationOnly = withoutApproval
        automationOnly.works.removeAll { $0.origin == .conversation || $0.identity == "approval" }
        presentation = try NotchProjection.reduce(previous: presentation, input: .snapshot(automationOnly))
        XCTAssertEqual(presentation.focusedWorkIdentity, "older")
        XCTAssertEqual(presentation.runPosition, 1)

        presentation = try NotchProjection.reduce(previous: presentation, input: .localIntent(.cycleAutomation))
        XCTAssertEqual(presentation.focusedWorkIdentity, "newer")
        XCTAssertEqual(presentation.runPosition, 2)

        presentation = try NotchProjection.reduce(previous: presentation, input: .localIntent(.cycleAutomation))
        XCTAssertEqual(presentation.focusedWorkIdentity, "older")
        XCTAssertEqual(presentation.runPosition, 1)
    }

    func testNewestUnacknowledgedTerminalAutomationReceivesFocus() throws {
        let snapshot = NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 12,
            daemonGeneration: "daemon-a",
            works: [
                terminal("older-failed", order: 4, state: .failed, attention: .failed),
                terminal("newer-unread", order: 5, state: .completed, attention: .unread),
            ],
            pendingApprovals: [],
            model: .unloaded
        )

        let presentation = try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot))

        XCTAssertEqual(presentation.focusedWorkIdentity, "newer-unread")
        XCTAssertEqual(presentation.rail.selectedStage, .done)
        XCTAssertEqual(presentation.statusPill.label, "FAILED", "pill priority remains independent of focus")
    }

    func testEveryAcceptedBridgeHeightAndInvariantPillAnchor() throws {
        let cases: [(NotchWorkSnapshot, CGFloat)] = [
            (snapshot(works: [], model: .unloaded), 0),
            (snapshot(works: [], model: .loading), 78),
            (snapshot(works: [work("thinking", origin: .conversation, state: .running)]), 78),
            (snapshot(works: [work("tool", origin: .automation, state: .running, activity: .init(category: .web))]), 78),
            (snapshot(works: [terminal("unread", order: 1, state: .completed, attention: .unread)]), 98),
            (snapshot(works: [
                work("foreground", origin: .conversation, state: .running),
                automation("background", order: 1, category: .automation),
            ]), 126),
            (snapshot(works: [
                automation("one", order: 1, category: .automation),
                automation("two", order: 2, category: .automation),
            ]), 150),
            (NotchWorkSnapshot(
                schemaVersion: 1,
                cursor: 1,
                daemonGeneration: "daemon-a",
                works: [work("approval", origin: .conversation, state: .waitingForApproval)],
                pendingApprovals: [.init(identity: "approval", workIdentity: "approval")],
                model: .ready
            ), 176),
        ]

        for (snapshot, height) in cases {
            let presentation = try NotchProjection.reduce(previous: .idle, input: .snapshot(snapshot))
            XCTAssertEqual(presentation.geometry.bridgeHeight, height)
            XCTAssertEqual(NotchPillLayout.size, .init(width: 74, height: 18))
            XCTAssertEqual(NotchPillLayout.origin(maxPanelWidth: 741), .init(x: 643, y: 9))
        }
    }

    func testPillPriorityAndReducedMotionContract() throws {
        let works = [
            terminal("failed", order: 1, state: .failed, attention: .failed),
            terminal("partial", order: 2, state: .partial, attention: .partial),
            terminal("unread", order: 3, state: .completed, attention: .unread),
        ]
        var presentation = try NotchProjection.reduce(
            previous: .idle,
            input: .snapshot(snapshot(works: works, model: .loading)),
            reduceMotion: true
        )
        XCTAssertEqual(presentation.statusPill.label, "LOADING")
        XCTAssertEqual(presentation.motion, .accepted(reduceMotion: true))
        XCTAssertEqual(presentation.motion.surfaceDuration, 0)
        XCTAssertEqual(presentation.motion.contentRevealDuration, 0.12)
        XCTAssertFalse(presentation.motion.iconMotionEnabled)

        presentation = try NotchProjection.reduce(
            previous: presentation,
            input: .snapshot(snapshot(works: works, model: .unloaded))
        )
        XCTAssertEqual(presentation.statusPill.label, "FAILED")
        XCTAssertEqual(presentation.motion.surfaceDuration, 0.58)
        XCTAssertEqual(presentation.motion.contentRevealDelay, 0.36)
        XCTAssertEqual(presentation.motion.pillCrossfadeDuration, 0.16)
        XCTAssertEqual(presentation.motion.completionRevealDuration, 0.24)
    }

    private func automation(
        _ identity: String,
        order: UInt64,
        category: NotchActivityCategory
    ) -> NotchWork {
        NotchWork(
            identity: identity,
            revision: 1,
            origin: .automation,
            state: .running,
            activity: .init(category: category),
            queuePosition: nil,
            automationDisplayName: "Saved automation",
            terminalAttention: nil,
            claimedOrder: order
        )
    }

    private func terminal(
        _ identity: String,
        order: UInt64,
        state: NotchWorkState,
        attention: NotchTerminalAttention
    ) -> NotchWork {
        NotchWork(
            identity: identity,
            revision: 1,
            origin: .automation,
            state: state,
            activity: nil,
            queuePosition: nil,
            automationDisplayName: "Saved automation",
            automationDefinitionIdentity: "definition-\(identity)",
            automationSessionIdentity: "session-\(identity)",
            terminalAttention: attention,
            terminalOrder: order,
            claimedOrder: order
        )
    }

    private func work(
        _ identity: String,
        origin: NotchWorkOrigin,
        state: NotchWorkState,
        activity: NotchActivity? = nil
    ) -> NotchWork {
        NotchWork(
            identity: identity,
            revision: 1,
            origin: origin,
            state: state,
            activity: activity,
            queuePosition: nil,
            automationDisplayName: nil,
            terminalAttention: nil
        )
    }

    private func snapshot(
        works: [NotchWork],
        model: NotchModelPhase = .ready
    ) -> NotchWorkSnapshot {
        NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 1,
            daemonGeneration: "daemon-a",
            works: works,
            pendingApprovals: [],
            model: model
        )
    }
}
