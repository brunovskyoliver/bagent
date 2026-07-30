import Foundation
import OSLog

@MainActor
final class TavilyConfigurationSynchronizer {
    private let maxAttemptsPerDaemon: Int
    private var processID: Int?
    private var attempts = 0
    private var synchronizedCurrentDaemon = false

    init(maxAttemptsPerDaemon: Int = 2) {
        self.maxAttemptsPerDaemon = max(1, maxAttemptsPerDaemon)
    }

    func synchronize(
        health: DaemonHealth,
        loadCredential: () -> KeychainStore.TavilyCredentialRead,
        configure: (String?) async throws -> DaemonClient.TavilyConfigurationStatus
    ) async -> DaemonClient.TavilyConfigurationStatus {
        guard health.daemonUp, let currentProcessID = health.processID else {
            return .pending
        }
        if processID != currentProcessID {
            processID = currentProcessID
            attempts = 0
            synchronizedCurrentDaemon = false
        }
        guard attempts < maxAttemptsPerDaemon else {
            return .configurationFailed
        }

        let credential: String?
        switch loadCredential() {
        case .present(let value):
            credential = value.isEmpty ? nil : value
        case .absent:
            credential = nil
        case .failed:
            attempts += 1
            return .configurationFailed
        }
        let desiredStatus: DaemonClient.TavilyConfigurationStatus = credential == nil
            ? .absent
            : .configured
        if synchronizedCurrentDaemon && health.tavilyConfiguration == desiredStatus {
            attempts = 0
            return desiredStatus
        }
        attempts += 1
        do {
            let status = try await configure(credential)
            if status == desiredStatus {
                attempts = 0
                synchronizedCurrentDaemon = true
            }
            return status
        } catch {
            return .configurationFailed
        }
    }
}

@MainActor
final class TavilyConfigurationFailureReporter {
    private let maxAttemptsPerDaemon: Int
    private var processID: Int?
    private var attempts = 0
    private var delivered = false

    init(maxAttemptsPerDaemon: Int = 2) {
        self.maxAttemptsPerDaemon = max(1, maxAttemptsPerDaemon)
    }

    func shouldAttempt(
        processID currentProcessID: Int?,
        status: DaemonClient.TavilyConfigurationStatus
    ) -> Bool {
        if processID != currentProcessID {
            processID = currentProcessID
            attempts = 0
            delivered = false
        }
        guard status == .configurationFailed else {
            attempts = 0
            delivered = false
            return false
        }
        return currentProcessID != nil && !delivered && attempts < maxAttemptsPerDaemon
    }

    func didAttempt(accepted: Bool) {
        attempts += 1
        delivered = accepted
    }
}

@MainActor
final class TavilyConfigurationManager {
    private static let logger = Logger(
        subsystem: "sk.bagent.app",
        category: "tavily-configuration"
    )
    private let client: DaemonClient
    private let synchronizer: TavilyConfigurationSynchronizer
    private let failureReporter = TavilyConfigurationFailureReporter()
    private var monitorTask: Task<Void, Never>?
    private var lastRecordedStatus: DaemonClient.TavilyConfigurationStatus?

    init(client: DaemonClient = DaemonClient()) {
        self.client = client
        synchronizer = TavilyConfigurationSynchronizer()
    }

    func start() {
        monitorTask?.cancel()
        monitorTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                let health = await client.healthStatus()
                let status = await synchronizer.synchronize(
                    health: health,
                    loadCredential: KeychainStore.readTavilyCredential,
                    configure: client.configureTavily
                )
                if failureReporter.shouldAttempt(
                    processID: health.processID,
                    status: status
                ) {
                    let accepted = await client.recordTavilyConfigurationFailure()
                    failureReporter.didAttempt(accepted: accepted)
                }
                record(status)
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    func stop() {
        monitorTask?.cancel()
        monitorTask = nil
    }

    private func record(_ status: DaemonClient.TavilyConfigurationStatus) {
        guard status != lastRecordedStatus else { return }
        lastRecordedStatus = status
        Self.logger.notice(
            "Tavily configuration status: \(status.rawValue, privacy: .public)"
        )
    }
}
