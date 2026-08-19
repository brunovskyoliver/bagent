import XCTest
@testable import bagent

final class NotchInteractionStateTests: XCTestCase {
    @MainActor
    func testAuthoritativeForegroundCompletionPresentsOutputAfterCollapse() throws {
        let viewModel = ChatViewModel(startMonitoring: false)
        try viewModel.installThinkingFixture()
        viewModel.applyNotchIntent(.collapse)

        try viewModel.installCompletedFixture()

        XCTAssertEqual(viewModel.notchInteractionMode, .output)
    }

    @MainActor
    func testTransportContentDeliveryCannotChangeInteractionMode() throws {
        let viewModel = ChatViewModel(startMonitoring: false)
        try viewModel.installThinkingFixture()
        let mode = viewModel.notchInteractionMode

        viewModel.messages = [ChatMessage(role: .assistant, content: "delivered token")]
        viewModel.streamingChunk += 1

        XCTAssertEqual(viewModel.notchInteractionMode, mode)
    }

    @MainActor
    func testExplicitUserIntentCanOpenForegroundOutput() throws {
        let viewModel = ChatViewModel(startMonitoring: false)
        try viewModel.installThinkingFixture()

        viewModel.applyNotchIntent(.openOutput)

        XCTAssertEqual(viewModel.notchInteractionMode, .output)
    }

    @MainActor
    func testApprovalPayloadCannotAuthorizeUIWithoutMatchingWorkProjection() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.pendingApprovals = [ApprovalItem(
            id: "stale-approval",
            toolName: "private-tool",
            description: "stale payload",
            expiresAt: "",
            createdAt: "",
            origin: nil
        )]

        XCTAssertNil(viewModel.authoritativePendingApproval)
        if case .awaitingApproval = viewModel.agentStatus {
            XCTFail("payload-only approval must not authorize the approval surface")
        }
    }
}
