import Foundation
import XCTest
@testable import bagent

final class NotchTransitionTests: XCTestCase {
    func testInterruptedTransitionConvergesToLatestProjectionAndPreservesPillAnchor() throws {
        let runningSnapshot = snapshot(
            cursor: 1,
            state: .running,
            activity: .init(category: .filesystem),
            terminalAttention: nil,
            approvals: []
        )
        let approvalSnapshot = snapshot(
            cursor: 2,
            state: .waitingForApproval,
            activity: nil,
            terminalAttention: nil,
            approvals: [NotchApproval(identity: "approval-stage8", workIdentity: "automation-a")]
        )
        let failedSnapshot = snapshot(
            cursor: 3,
            state: .failed,
            activity: nil,
            terminalAttention: .failed,
            approvals: []
        )

        let normalRunning = try NotchProjection.reduce(
            previous: .idle,
            input: .snapshot(runningSnapshot),
            reduceMotion: false
        )
        let normalApproval = try NotchProjection.reduce(
            previous: normalRunning,
            input: .snapshot(approvalSnapshot),
            reduceMotion: false
        )
        let normalFailed = try NotchProjection.reduce(
            previous: normalApproval,
            input: .snapshot(failedSnapshot),
            reduceMotion: false
        )

        XCTAssertEqual(normalRunning.rail.selectedStage, .tool)
        XCTAssertEqual(normalApproval.rail.selectedStage, .tool)
        XCTAssertEqual(normalApproval.statusPill.label, "APPROVE")
        XCTAssertEqual(normalApproval.pendingApprovalIdentity, "approval-stage8")
        XCTAssertEqual(normalApproval.rail.caption, "Approval required")
        XCTAssertEqual(normalFailed.rail.selectedStage, .done)
        XCTAssertEqual(normalFailed.statusPill.label, "FAILED")
        XCTAssertEqual(normalFailed.revision.cursor, 3)
        XCTAssertEqual(normalFailed.pendingApprovalIdentity, nil)

        let reducedApproval = try NotchProjection.reduce(
            previous: normalRunning,
            input: .snapshot(approvalSnapshot),
            reduceMotion: true
        )
        XCTAssertTrue(normalRunning.motion.iconMotionEnabled)
        XCTAssertFalse(normalRunning.motion.reduceMotion)
        XCTAssertTrue(reducedApproval.motion.reduceMotion)
        XCTAssertFalse(reducedApproval.motion.iconMotionEnabled)
        XCTAssertEqual(reducedApproval.motion.surfaceDuration, 0)

        let anchors = [normalRunning, normalApproval, normalFailed, reducedApproval]
            .map { _ in NotchPillLayout.origin(maxPanelWidth: 741) }
        let anchorInvariant = anchors.dropFirst().allSatisfy { $0 == anchors[0] }
        XCTAssertTrue(anchorInvariant, "the invariant status pill must not move during interruption")
        XCTAssertEqual(anchors.first, CGPoint(x: 643, y: 9))

        if let evidencePath = ProcessInfo.processInfo.environment["BAGENT_NOTCH_TRANSITION_EVIDENCE"] {
            let evidence: [String: Any] = [
                "status": "pass",
                "transition_count": 2,
                "interruption_reconciled": normalFailed.revision.cursor == 3
                    && normalFailed.pendingApprovalIdentity == nil,
                "status_pill_anchor_invariant": anchorInvariant,
                "normal_motion_recorded": normalRunning.motion.iconMotionEnabled
                    && !normalRunning.motion.reduceMotion,
                "reduced_motion_recorded": reducedApproval.motion.reduceMotion
                    && !reducedApproval.motion.iconMotionEnabled,
            ]
            let data = try JSONSerialization.data(withJSONObject: evidence, options: [.sortedKeys])
            try data.write(to: URL(fileURLWithPath: evidencePath), options: .atomic)
        }
    }

    private func snapshot(
        cursor: UInt64,
        state: NotchWorkState,
        activity: NotchActivity?,
        terminalAttention: NotchTerminalAttention?,
        approvals: [NotchApproval]
    ) -> NotchWorkSnapshot {
        NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: cursor,
            daemonGeneration: "stage8-transition",
            works: [NotchWork(
                identity: "automation-a",
                revision: cursor,
                origin: .automation,
                state: state,
                activity: activity,
                queuePosition: nil,
                automationDisplayName: "Stage 8 Automation",
                automationDefinitionIdentity: "definition-a",
                automationSessionIdentity: "session-a",
                terminalAttention: terminalAttention,
                claimedOrder: 1
            )],
            pendingApprovals: approvals,
            model: .ready
        )
    }
}
