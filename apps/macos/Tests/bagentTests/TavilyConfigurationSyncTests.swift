import XCTest
@testable import bagent

@MainActor
final class TavilyConfigurationSyncTests: XCTestCase {
    @MainActor
    private final class Recorder {
        var attempts = 0
        var failFirstAttempt = false

        func configure(_ credential: String?) async throws -> DaemonClient.TavilyConfigurationStatus {
            attempts += 1
            if failFirstAttempt && attempts == 1 {
                throw DaemonError.notReady
            }
            return credential == nil ? .absent : .configured
        }
    }

    private func health(
        ready: Bool = true,
        processID: Int = 101,
        tavily: DaemonClient.TavilyConfigurationStatus = .pending
    ) -> DaemonHealth {
        DaemonHealth(
            daemonUp: ready,
            processID: ready ? processID : nil,
            tavilyConfiguration: tavily,
            baseRTUp: false,
            model: "model",
            classifierModel: "classifier",
            mailConnector: false,
            notesConnector: false,
            codexConnector: false,
            odooConnector: false,
            whatsappConnector: false
        )
    }

    func testCredentialPresentBeforeDelayedDaemonReadinessConfiguresOnceReady() async {
        let sync = TavilyConfigurationSynchronizer()
        let recorder = Recorder()
        let credential = String(repeating: "k", count: 32)

        let waiting = await sync.synchronize(
            health: health(ready: false),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )
        XCTAssertEqual(waiting, .pending)
        XCTAssertEqual(recorder.attempts, 0)

        let configured = await sync.synchronize(
            health: health(),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )
        XCTAssertEqual(configured, .configured)
        XCTAssertEqual(recorder.attempts, 1)
    }

    func testFailedFirstConfigurationHasOneBoundedSuccessfulRetry() async {
        let sync = TavilyConfigurationSynchronizer(maxAttemptsPerDaemon: 2)
        let recorder = Recorder()
        recorder.failFirstAttempt = true
        let credential = String(repeating: "k", count: 32)

        let first = await sync.synchronize(
            health: health(),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )
        let second = await sync.synchronize(
            health: health(),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )
        let third = await sync.synchronize(
            health: health(tavily: .configured),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )

        XCTAssertEqual(first, .configurationFailed)
        XCTAssertEqual(second, .configured)
        XCTAssertEqual(third, .configured)
        XCTAssertEqual(recorder.attempts, 2)
    }

    func testChangedDaemonPIDAndPendingStatusRequireReconfiguration() async {
        let sync = TavilyConfigurationSynchronizer()
        let recorder = Recorder()
        let credential = String(repeating: "k", count: 32)

        _ = await sync.synchronize(
            health: health(processID: 101),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )
        _ = await sync.synchronize(
            health: health(processID: 101, tavily: .configured),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )
        _ = await sync.synchronize(
            health: health(processID: 202),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )

        XCTAssertEqual(recorder.attempts, 2)
    }

    func testClientReconnectRepairsPendingStateForSameDaemon() async {
        let sync = TavilyConfigurationSynchronizer()
        let recorder = Recorder()
        let credential = String(repeating: "k", count: 32)

        _ = await sync.synchronize(
            health: health(),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )
        _ = await sync.synchronize(
            health: health(tavily: .configured),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )
        _ = await sync.synchronize(
            health: health(tavily: .pending),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )

        XCTAssertEqual(recorder.attempts, 2)
    }

    func testAbsentCredentialIsExplicitlySynchronized() async {
        let sync = TavilyConfigurationSynchronizer()
        let recorder = Recorder()

        let status = await sync.synchronize(
            health: health(),
            loadCredential: { .absent },
            configure: recorder.configure
        )

        XCTAssertEqual(status, .absent)
        XCTAssertEqual(recorder.attempts, 1)
    }

    func testAppRelaunchResendsCredentialToExistingConfiguredDaemonOnce() async {
        let sync = TavilyConfigurationSynchronizer()
        let recorder = Recorder()
        let credential = String(repeating: "k", count: 32)

        let status = await sync.synchronize(
            health: health(tavily: .configured),
            loadCredential: { .present(credential) },
            configure: recorder.configure
        )

        XCTAssertEqual(status, .configured)
        XCTAssertEqual(recorder.attempts, 1)
    }

    func testRepeatedConfigurationFailuresStayBoundedForOneDaemon() async {
        let sync = TavilyConfigurationSynchronizer(maxAttemptsPerDaemon: 2)
        let credential = String(repeating: "k", count: 32)
        var attempts = 0

        for _ in 0..<4 {
            _ = await sync.synchronize(
                health: health(),
                loadCredential: { .present(credential) },
                configure: { _ in
                    attempts += 1
                    throw DaemonError.notReady
                }
            )
        }

        XCTAssertEqual(attempts, 2)
    }

    func testKeychainReadFailureIsNotTreatedAsCredentialAbsence() async {
        let sync = TavilyConfigurationSynchronizer(maxAttemptsPerDaemon: 2)
        let recorder = Recorder()

        let first = await sync.synchronize(
            health: health(tavily: .configured),
            loadCredential: { .failed },
            configure: recorder.configure
        )
        let second = await sync.synchronize(
            health: health(tavily: .configured),
            loadCredential: { .failed },
            configure: recorder.configure
        )

        XCTAssertEqual(first, .configurationFailed)
        XCTAssertEqual(second, .configurationFailed)
        XCTAssertEqual(recorder.attempts, 0)
    }

    func testFailedFirstNormalizedStatusReportHasOneBoundedRetry() {
        let reporter = TavilyConfigurationFailureReporter(maxAttemptsPerDaemon: 2)

        XCTAssertTrue(reporter.shouldAttempt(processID: 101, status: .configurationFailed))
        reporter.didAttempt(accepted: false)
        XCTAssertTrue(reporter.shouldAttempt(processID: 101, status: .configurationFailed))
        reporter.didAttempt(accepted: true)
        XCTAssertFalse(reporter.shouldAttempt(processID: 101, status: .configurationFailed))

        XCTAssertTrue(reporter.shouldAttempt(processID: 202, status: .configurationFailed))
    }

    func testUnavailableHealthDoesNotResetFailureReportBudgetForSamePID() {
        let reporter = TavilyConfigurationFailureReporter(maxAttemptsPerDaemon: 2)

        XCTAssertTrue(reporter.shouldAttempt(processID: 101, status: .configurationFailed))
        reporter.didAttempt(accepted: false)
        XCTAssertFalse(reporter.shouldAttempt(processID: nil, status: .pending))
        XCTAssertTrue(reporter.shouldAttempt(processID: 101, status: .configurationFailed))
        reporter.didAttempt(accepted: false)
        XCTAssertFalse(reporter.shouldAttempt(processID: 101, status: .configurationFailed))
    }
}
