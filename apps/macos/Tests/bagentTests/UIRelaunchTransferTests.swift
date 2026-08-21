import XCTest
@testable import bagent

final class UIRelaunchTransferTests: XCTestCase {
    func testHappyPathKeepsExactlyOneVisibleInteractiveConsumer() throws {
        var machine = UIRelaunchTransferMachine()
        assertPresentation(machine.state, oldVisible: true, oldInteractive: true, oldConsumes: true, replacementVisible: false, replacementInteractive: false, replacementConsumes: false)

        try machine.apply(.replacementLaunched)
        assertPresentation(machine.state, oldVisible: true, oldInteractive: true, oldConsumes: true, replacementVisible: false, replacementInteractive: false, replacementConsumes: false)
        try machine.apply(.handoffConsumed)
        try machine.apply(.authoritativeStateRefetched)
        try machine.apply(.successorAuthorityReserved)
        try machine.apply(.replacementReady)
        try machine.apply(.oldUIFenced)
        assertPresentation(machine.state, oldVisible: false, oldInteractive: false, oldConsumes: false, replacementVisible: false, replacementInteractive: false, replacementConsumes: false)
        try machine.apply(.successorActivated)
        assertPresentation(machine.state, oldVisible: false, oldInteractive: false, oldConsumes: false, replacementVisible: true, replacementInteractive: true, replacementConsumes: true)
        try machine.apply(.activePresentationAcknowledged)
        XCTAssertEqual(machine.state.phase, .acknowledged)
    }

    func testRejectedTakeoversLeaveOldConsumerAuthoritative() {
        for event in [
            UIRelaunchTransferEvent.staleConsumerRejected,
            .duplicateReplacementRejected,
            .tokenReplayRejected,
            .replacementCrashed,
            .failedReadiness,
            .failedActivationAcknowledgement,
        ] {
            var machine = UIRelaunchTransferMachine()
            XCTAssertThrowsError(try machine.apply(event))
            assertPresentation(machine.state, oldVisible: true, oldInteractive: true, oldConsumes: true, replacementVisible: false, replacementInteractive: false, replacementConsumes: false)
        }
    }

    func testTimeoutRollsBackAndLateReplacementStaysHidden() throws {
        var machine = UIRelaunchTransferMachine()
        try machine.apply(.replacementLaunched)
        try machine.apply(.handoffConsumed)
        try machine.apply(.authoritativeStateRefetched)
        try machine.apply(.successorAuthorityReserved)
        try machine.apply(.replacementReady)
        try machine.apply(.oldUIFenced)
        try machine.apply(.takeoverTimedOut)
        XCTAssertEqual(machine.state.phase, .rolledBack)
        assertPresentation(machine.state, oldVisible: true, oldInteractive: true, oldConsumes: true, replacementVisible: false, replacementInteractive: false, replacementConsumes: false)
        try machine.apply(.lateReplacementDetected)
        XCTAssertEqual(machine.state.phase, .lateReplacementExited)
        assertPresentation(machine.state, oldVisible: true, oldInteractive: true, oldConsumes: true, replacementVisible: false, replacementInteractive: false, replacementConsumes: false)
    }

    func testDaemonAvailabilityDoesNotChangeOwnership() throws {
        var machine = UIRelaunchTransferMachine()
        try machine.apply(.replacementLaunched)
        try machine.apply(.daemonUnavailable)
        XCTAssertEqual(machine.state.phase, .replacementHidden)
        try machine.apply(.daemonAvailable)
        XCTAssertEqual(machine.state.phase, .replacementHidden)
        try machine.apply(.handoffConsumed)
        try machine.apply(.authoritativeStateRefetched)
        try machine.apply(.successorAuthorityReserved)
        try machine.apply(.replacementReady)
        try machine.apply(.oldUIFenced)
        try machine.apply(.successorActivated)
        try machine.apply(.daemonUnavailable)
        XCTAssertEqual(machine.state.phase, .successorActive)
    }

    func testTransferTimeoutIsTenSecondsAndUIOnlyOwnershipForbidsDaemonMutation() {
        XCTAssertEqual(UIRelaunchTransferMachine.timeout, 10)
        XCTAssertTrue(UIOnlyRelaunchOwnership.forbiddenActions.contains(.launchDaemon))
        XCTAssertTrue(UIOnlyRelaunchOwnership.forbiddenActions.contains(.restartBaseRT))
        XCTAssertTrue(UIOnlyRelaunchOwnership.forbiddenActions.contains(.mutateAutomationWork))
    }

    private func assertPresentation(
        _ state: UIRelaunchTransferState,
        oldVisible: Bool,
        oldInteractive: Bool,
        oldConsumes: Bool,
        replacementVisible: Bool,
        replacementInteractive: Bool,
        replacementConsumes: Bool,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        XCTAssertEqual(state.oldVisible, oldVisible, file: file, line: line)
        XCTAssertEqual(state.oldInteractive, oldInteractive, file: file, line: line)
        XCTAssertEqual(state.oldConsumes, oldConsumes, file: file, line: line)
        XCTAssertEqual(state.replacementVisible, replacementVisible, file: file, line: line)
        XCTAssertEqual(state.replacementInteractive, replacementInteractive, file: file, line: line)
        XCTAssertEqual(state.replacementConsumes, replacementConsumes, file: file, line: line)
        XCTAssertLessThanOrEqual(
            [state.oldVisible && state.oldInteractive, state.replacementVisible && state.replacementInteractive].filter { $0 }.count,
            1,
            file: file,
            line: line
        )
    }
}
