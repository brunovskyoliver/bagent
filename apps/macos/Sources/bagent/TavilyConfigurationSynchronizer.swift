import Foundation

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
        loadCredential: @MainActor () -> String?,
        configure: @MainActor (String?) async throws -> DaemonClient.TavilyConfigurationStatus
    ) async -> DaemonClient.TavilyConfigurationStatus {
        guard health.daemonUp, let currentProcessID = health.processID else {
            return .pending
        }

        if processID != currentProcessID {
            processID = currentProcessID
            attempts = 0
        }

        if health.tavilyConfiguration == .configured {
            attempts = 0
            return .configured
        }

        guard attempts < maxAttemptsPerDaemon else {
            return .configurationFailed
        }

        let credential = loadCredential().flatMap { value in
            value.isEmpty ? nil : value
        }
        let desiredStatus: DaemonClient.TavilyConfigurationStatus = credential == nil
            ? .absent
            : .configured
        if health.tavilyConfiguration == desiredStatus {
            attempts = 0
            return desiredStatus
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
