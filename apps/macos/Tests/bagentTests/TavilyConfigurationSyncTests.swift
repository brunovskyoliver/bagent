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
            loadCredential: { credential },
            configure: recorder.configure
        )
        XCTAssertEqual(waiting, .pending)
        XCTAssertEqual(recorder.attempts, 0)

        let configured = await sync.synchronize(
            health: health(),
            loadCredential: { credential },
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
            loadCredential: { credential },
            configure: recorder.configure
        )
        let second = await sync.synchronize(
            health: health(),
            loadCredential: { credential },
            configure: recorder.configure
        )
        let third = await sync.synchronize(
            health: health(tavily: .configured),
            loadCredential: { credential },
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
            loadCredential: { credential },
            configure: recorder.configure
        )
        _ = await sync.synchronize(
            health: health(processID: 101, tavily: .configured),
            loadCredential: { credential },
            configure: recorder.configure
        )
        _ = await sync.synchronize(
            health: health(processID: 202),
            loadCredential: { credential },
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
            loadCredential: { credential },
            configure: recorder.configure
        )
        _ = await sync.synchronize(
            health: health(tavily: .configured),
            loadCredential: { credential },
            configure: recorder.configure
        )
        _ = await sync.synchronize(
            health: health(tavily: .pending),
            loadCredential: { credential },
            configure: recorder.configure
        )

        XCTAssertEqual(recorder.attempts, 2)
    }

    func testAbsentCredentialIsExplicitlySynchronized() async {
        let sync = TavilyConfigurationSynchronizer()
        let recorder = Recorder()

        let status = await sync.synchronize(
            health: health(),
            loadCredential: { nil },
            configure: recorder.configure
        )

        XCTAssertEqual(status, .absent)
        XCTAssertEqual(recorder.attempts, 1)
    }
}
