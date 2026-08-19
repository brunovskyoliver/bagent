import Foundation
import Combine

enum NotchEventBatch: Equatable, Sendable {
    case events([NotchWorkEvent])
    case gap(NotchWorkSnapshot)
}

enum NotchEventTransportError: Error, Equatable {
    case consumerFenced
}

protocol NotchEventTransport: Sendable {
    func fetchSnapshot(consumerFence: String) async throws -> NotchWorkSnapshot
    func fetchEvents(
        after cursor: UInt64,
        daemonGeneration: String,
        consumerFence: String
    ) async throws -> NotchEventBatch
}

@MainActor
final class NotchEventConsumer: ObservableObject {
    private let transport: any NotchEventTransport
    private let consumerFence: String
    @Published private(set) var presentation: NotchPresentation = .idle
    private var hasAuthoritativeSnapshot = false

    init(
        transport: any NotchEventTransport,
        consumerFence: String = UUID().uuidString
    ) {
        self.transport = transport
        self.consumerFence = consumerFence
    }

    func synchronize(reduceMotion: Bool = false) async throws {
        guard hasAuthoritativeSnapshot else {
            try await replaceFromSnapshot(reduceMotion: reduceMotion)
            return
        }

        let batch = try await transport.fetchEvents(
            after: presentation.revision.cursor,
            daemonGeneration: presentation.revision.daemonGeneration,
            consumerFence: consumerFence
        )
        switch batch {
        case .gap(let snapshot):
            try install(snapshot, reduceMotion: reduceMotion)
        case .events(let events):
            do {
                for event in events {
                    presentation = try NotchProjection.reduce(
                        previous: presentation,
                        input: .event(event),
                        reduceMotion: reduceMotion
                    )
                }
            } catch let error as NotchProjectionError {
                switch error {
                case .unsupportedSchema, .cursorGap, .daemonGenerationChanged, .revisionMismatch,
                     .missingSnapshot:
                    try await replaceFromSnapshot(reduceMotion: reduceMotion)
                }
            }
        }
    }

    func applyLocalIntent(
        _ intent: NotchLocalIntent,
        reduceMotion: Bool = false
    ) throws {
        presentation = try NotchProjection.reduce(
            previous: presentation,
            input: .localIntent(intent),
            reduceMotion: reduceMotion
        )
    }

    func invalidateConsumerFence() {
        hasAuthoritativeSnapshot = false
    }

    func reconcileSnapshot(reduceMotion: Bool = false) async throws {
        try await replaceFromSnapshot(reduceMotion: reduceMotion)
    }

    func replace(with snapshot: NotchWorkSnapshot, reduceMotion: Bool = false) throws {
        try install(snapshot, reduceMotion: reduceMotion)
    }

    private func replaceFromSnapshot(reduceMotion: Bool) async throws {
        let snapshot = try await transport.fetchSnapshot(consumerFence: consumerFence)
        try install(snapshot, reduceMotion: reduceMotion)
    }

    private func install(_ snapshot: NotchWorkSnapshot, reduceMotion: Bool) throws {
        presentation = try NotchProjection.reduce(
            previous: presentation,
            input: .snapshot(snapshot),
            reduceMotion: reduceMotion
        )
        hasAuthoritativeSnapshot = true
    }
}
