import Foundation
import OSLog

@MainActor
final class TavilyConfigurationSynchronizer {
    private let maxAttemptsPerDaemon: Int
    private var processID: Int?
    private var attempts = 0

    init(maxAttemptsPerDaemon: Int = 2) {
        self.maxAttemptsPerDaemon = max(1, maxAttemptsPerDaemon)
    }

    func synchronize(
        health: DaemonHealth,
        loadCredential: () -> String?,
        configure: (String?) async throws -> DaemonClient.TavilyConfigurationStatus
    ) async -> DaemonClient.TavilyConfigurationStatus {
        guard health.daemonUp, let currentProcessID = health.processID else {
            return .pending
        }
        if processID != currentProcessID {
            processID = currentProcessID
            attempts = 0
        }

        let credential = loadCredential().flatMap { $0.isEmpty ? nil : $0 }
        let desiredStatus: DaemonClient.TavilyConfigurationStatus = credential == nil
            ? .absent
            : .configured
        if health.tavilyConfiguration == desiredStatus {
            attempts = 0
            return desiredStatus
        }
        guard attempts < maxAttemptsPerDaemon else {
            return .configurationFailed
        }

        attempts += 1
        do {
            let status = try await configure(credential)
            if status == desiredStatus {
                attempts = 0
            }
            return status
        } catch {
            return .configurationFailed
        }
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
                    loadCredential: KeychainStore.loadTavilyAPIKey,
                    configure: client.configureTavily
                )
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
