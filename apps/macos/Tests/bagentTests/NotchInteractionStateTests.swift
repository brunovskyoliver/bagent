import XCTest
@testable import bagent

final class NotchInteractionStateTests: XCTestCase {
    @MainActor
    func testHiddenThinkingCompletionAlwaysPresentsOutputSurface() {
        let viewModel = ChatViewModel(startMonitoring: false)
        try! viewModel.installThinkingFixture()
        var outputPresentationRequested = false
        viewModel.onFirstAssistantToken = {
            outputPresentationRequested = true
            viewModel.applyNotchIntent(.openOutput)
        }

        viewModel.ensureCompletedTurnOutputPresented()

        XCTAssertTrue(outputPresentationRequested)
        XCTAssertEqual(viewModel.chatSurfaceMode, .outputExpanded)
        XCTAssertEqual(viewModel.notchInteractionMode, .output)
    }

    @MainActor
    func testHiddenThinkingWithVisibleOutputUsesOutputPresentationCallback() {
        let viewModel = ChatViewModel(startMonitoring: false)
        try! viewModel.installThinkingFixture()
        var presented = false
        viewModel.onFirstAssistantToken = {
            presented = true
            viewModel.applyNotchIntent(.openOutput)
        }

        viewModel.ensureCompletedTurnOutputPresented()

        XCTAssertTrue(presented)
        XCTAssertEqual(viewModel.chatSurfaceMode, .outputExpanded)
        XCTAssertEqual(viewModel.notchInteractionMode, .output)
    }

    @MainActor
    func testCompletionReopensOutputAfterThinkingWasCollapsed() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.applyNotchIntent(.collapse)
        var presentationRequests = 0
        viewModel.onFirstAssistantToken = {
            presentationRequests += 1
            viewModel.applyNotchIntent(.openOutput)
        }

        viewModel.ensureCompletedTurnOutputPresented()
        viewModel.ensureCompletedTurnOutputPresented()

        XCTAssertEqual(presentationRequests, 1)
        XCTAssertTrue(viewModel.isExpanded)
        XCTAssertEqual(viewModel.chatSurfaceMode, .outputExpanded)
        XCTAssertEqual(viewModel.notchInteractionMode, .output)
    }
}
