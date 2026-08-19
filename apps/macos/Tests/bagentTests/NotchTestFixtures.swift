@testable import bagent

@MainActor
extension ChatViewModel {
    func installThinkingFixture() throws {
        try applyAuthoritativeSnapshot(NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 1,
            daemonGeneration: "test-daemon",
            works: [NotchWork(
                identity: "test-conversation-work",
                revision: 1,
                origin: .conversation,
                state: .running,
                activity: nil,
                queuePosition: nil,
                automationDisplayName: nil,
                terminalAttention: nil
            )],
            pendingApprovals: [],
            model: .ready
        ))
    }

    func installCompletedFixture() throws {
        try applyAuthoritativeSnapshot(NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 2,
            daemonGeneration: "test-daemon",
            works: [NotchWork(
                identity: "test-conversation-work",
                revision: 2,
                origin: .conversation,
                state: .completed,
                activity: nil,
                queuePosition: nil,
                automationDisplayName: nil,
                terminalAttention: nil
            )],
            pendingApprovals: [],
            model: .ready
        ))
    }
}
