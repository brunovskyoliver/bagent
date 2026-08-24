import XCTest
@testable import bagent

@MainActor
final class PermissionRecheckTests: XCTestCase {
    func testAllTwelvePermissionPhasesAreDistinct() {
        XCTAssertEqual(PermissionGrantAssistPhase.allCases.count, 12)
        XCTAssertEqual(Set(PermissionGrantAssistPhase.allCases).count, 12)
    }

    func testAllTwelvePermissionPhasesHaveExplicitAcceptedTransitions() {
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .unknown, event: .probe(.denied)),
            .deniedOrMissing
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .deniedOrMissing, event: .exactPaneOpening),
            .openingExactPane
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .openingExactPane, event: .paneOpened),
            .readyToDrag
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .openingExactPane, event: .paneFailed),
            .exactPaneFailureRootFallback
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .exactPaneFailureRootFallback, event: .paneOpened),
            .readyToDrag
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .readyToDrag, event: .dragBegan),
            .draggingApplication
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .draggingApplication, event: .dragEnded),
            .waitingForSystemSettings
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .waitingForSystemSettings, event: .activation),
            .authoritativeRecheck
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .authoritativeRecheck, event: .probe(.granted)),
            .grantedAndActive
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .grantedAndActive, event: .relaunchRequested),
            .daemonPreservingRelaunchHandoff
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .daemonPreservingRelaunchHandoff, event: .replacementReady),
            .authoritativeRecheck
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .authoritativeRecheck, event: .relaunchProbe(.granted)),
            .relaunchCompletedAndPermissionRechecked
        )
    }

    func testAuthoritativeProbeMapsEachPermissionIndependently() {
        let result = PermissionProbeSnapshot(
            fullDiskAccess: [
                .mail: .granted,
                .notes: .denied,
            ],
            screenRecording: .granted,
            accessibility: .denied
        )

        XCTAssertEqual(result.fullDiskAccess[.mail], .granted)
        XCTAssertEqual(result.fullDiskAccess[.notes], .denied)
        XCTAssertEqual(result.permissionState(for: .fullDiskAccess), .denied)
        XCTAssertEqual(result.permissionState(for: .screenRecording), .granted)
        XCTAssertEqual(result.permissionState(for: .accessibility), .denied)
    }

    func testFullDiskAccessUsesSeparateDaemonResultsWithoutAUIAuthority() async {
        let probe = SystemPermissionProbe(
            daemon: StubDaemonFullDiskAccessProbe(result: .init(mail: .granted, notes: .denied)),
            screenRecording: { true },
            accessibility: { true }
        )

        let result = await probe.probe()

        XCTAssertEqual(result.fullDiskAccess[.mail], .granted)
        XCTAssertEqual(result.fullDiskAccess[.notes], .denied)
        XCTAssertEqual(result.permissionState(for: .fullDiskAccess), .denied)
    }

    func testProductionMappingDoesNotTreatAProbeCallbackAsRelaunchCompletion() {
        let result = PermissionProbeSnapshot(
            fullDiskAccess: [.mail: .granted, .notes: .granted],
            screenRecording: .granted,
            accessibility: .granted,
            relaunchRequired: [.screenRecording]
        )

        XCTAssertEqual(
            PermissionGrantAssistMachine.phase(
                after: result.permissionState(for: .screenRecording),
                uiRequiresRelaunch: result.requiresUIRelaunch(for: .screenRecording)
            ),
            .grantedButUIRelaunchRequired
        )
    }

    func testProductionLifecycleRequiresReplacementAfterProcessScopedGrant() {
        let lifecycle = PermissionProbeLifecycle()
        let denied = PermissionProbeSnapshot(
            fullDiskAccess: [.mail: .granted, .notes: .granted],
            screenRecording: .denied,
            accessibility: .granted
        )
        let granted = PermissionProbeSnapshot(
            fullDiskAccess: [.mail: .granted, .notes: .granted],
            screenRecording: .granted,
            accessibility: .granted
        )

        XCTAssertTrue(lifecycle.relaunchRequirements(for: denied).isEmpty)
        XCTAssertEqual(
            lifecycle.relaunchRequirements(for: granted),
            [.screenRecording]
        )
        XCTAssertEqual(
            lifecycle.relaunchRequirements(for: granted),
            [.screenRecording]
        )
    }

    func testOpeningAndDragNeverAdvanceTheAuthoritativePhase() {
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .deniedOrMissing, event: .exactPaneOpening),
            .openingExactPane
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .openingExactPane, event: .paneOpened),
            .readyToDrag
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.next(after: .readyToDrag, event: .probe(.denied)),
            .deniedOrMissing
        )
    }

    func testActivationBurstProducesOneLatestProbeAndSuppressesStaleGeneration() async {
        let probe = ControlledPermissionProbe(result: .allGranted)
        let coordinator = PermissionRecheckCoordinator(
            probe: probe,
            debounce: .milliseconds(10)
        )

        coordinator.didBecomeActive()
        coordinator.didBecomeActive()
        coordinator.didBecomeActive()
        try? await Task.sleep(for: .milliseconds(40))

        let count = await probe.count()
        XCTAssertEqual(count, 1)
        let state = coordinator.phase
        XCTAssertEqual(state, .grantedAndActive)
    }

    func testActivationGenerationCoalescesOlderDebouncedProbe() async {
        let probe = SequencedPermissionProbe(results: [.allDenied, .allGranted, .allDenied])
        let coordinator = PermissionRecheckCoordinator(probe: probe, debounce: .zero)

        coordinator.didBecomeActive()
        coordinator.didBecomeActive()
        try? await Task.sleep(for: .milliseconds(450))

        let phase = coordinator.phase
        let count = await probe.count()
        XCTAssertEqual(phase, .grantedAndActive)
        XCTAssertEqual(count, 2)
    }

    func testDelayedDeniedProbeConvergesToGrantedWithoutAnotherActivation() async {
        let probe = SequencedPermissionProbe(results: [.allDenied, .allGranted])
        let coordinator = PermissionRecheckCoordinator(
            probe: probe,
            debounce: .zero
        )

        coordinator.didBecomeActive()
        try? await Task.sleep(for: .milliseconds(450))

        let count = await probe.count()
        XCTAssertEqual(coordinator.phase, .grantedAndActive)
        XCTAssertEqual(count, 2)
    }

    func testGrantRevocationAndRelaunchRequiredMapping() {
        XCTAssertEqual(
            PermissionGrantAssistMachine.phase(after: .granted, uiRequiresRelaunch: false),
            .grantedAndActive
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.phase(after: .granted, uiRequiresRelaunch: true),
            .grantedButUIRelaunchRequired
        )
        XCTAssertEqual(
            PermissionGrantAssistMachine.phase(after: .denied, uiRequiresRelaunch: false),
            .deniedOrMissing
        )
    }

    func testPermissionOwnerCleanupCancelsPendingWork() async {
        let probe = ControlledPermissionProbe(result: .allGranted)
        let coordinator = PermissionRecheckCoordinator(probe: probe, debounce: .seconds(1))
        coordinator.didBecomeActive()
        coordinator.stop()
        try? await Task.sleep(for: .milliseconds(20))

        let count = await probe.count()
        XCTAssertEqual(count, 0)
        XCTAssertEqual(coordinator.phase, .unknown)
    }
}

private actor ControlledPermissionProbe: PermissionProbeAdapter {
    let result: PermissionProbeSnapshot
    private(set) var callCount = 0

    init(result: PermissionProbeSnapshot) {
        self.result = result
    }

    func probe() async -> PermissionProbeSnapshot {
        callCount += 1
        return result
    }

    func count() -> Int { callCount }
}

private actor SequencedPermissionProbe: PermissionProbeAdapter {
    let results: [PermissionProbeSnapshot]
    private(set) var callCount = 0

    init(results: [PermissionProbeSnapshot]) {
        self.results = results
    }

    func probe() async -> PermissionProbeSnapshot {
        let result = results[min(callCount, results.count - 1)]
        callCount += 1
        return result
    }

    func count() -> Int { callCount }
}

private struct StubDaemonFullDiskAccessProbe: DaemonFullDiskAccessProbeAdapter {
    let result: DaemonFullDiskAccessSnapshot

    func probe() async -> DaemonFullDiskAccessSnapshot { result }
}

private extension PermissionProbeSnapshot {
    static let allGranted = Self(
        fullDiskAccess: [.mail: .granted, .notes: .granted],
        screenRecording: .granted,
        accessibility: .granted
    )

    static let allDenied = Self(
        fullDiskAccess: [.mail: .denied, .notes: .denied],
        screenRecording: .denied,
        accessibility: .denied
    )

    init(result: PermissionProbeOutcome) {
        self.init(
            fullDiskAccess: [.mail: result, .notes: result],
            screenRecording: result,
            accessibility: result
        )
    }
}
