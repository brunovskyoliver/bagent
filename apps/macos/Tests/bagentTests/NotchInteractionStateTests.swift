import XCTest
@testable import bagent

final class NotchInteractionStateTests: XCTestCase {
    @MainActor
    func testHiddenThinkingCompletionAlwaysPresentsOutputSurface() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.isThinking = true
        viewModel.chatSurfaceMode = .thinkingHidden
        viewModel.notchInteractionMode = .thinking
        var outputPresentationRequested = false
        viewModel.onFirstAssistantToken = {
            outputPresentationRequested = true
            viewModel.chatSurfaceMode = .outputExpanded
            viewModel.notchInteractionMode = .output
        }

        viewModel.isThinking = false
        viewModel.ensureCompletedTurnOutputPresented()

        XCTAssertTrue(outputPresentationRequested)
        XCTAssertEqual(viewModel.chatSurfaceMode, .outputExpanded)
        XCTAssertEqual(viewModel.notchInteractionMode, .output)
    }

    @MainActor
    func testHiddenThinkingWithVisibleOutputUsesOutputPresentationCallback() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.chatSurfaceMode = .thinkingHidden
        viewModel.notchInteractionMode = .thinking
        var presented = false
        viewModel.onFirstAssistantToken = {
            presented = true
            viewModel.chatSurfaceMode = .outputExpanded
            viewModel.notchInteractionMode = .output
        }

        viewModel.ensureCompletedTurnOutputPresented()

        XCTAssertTrue(presented)
        XCTAssertEqual(viewModel.chatSurfaceMode, .outputExpanded)
        XCTAssertEqual(viewModel.notchInteractionMode, .output)
    }

    @MainActor
    func testCompletionReopensOutputAfterThinkingWasCollapsed() {
        let viewModel = ChatViewModel(startMonitoring: false)
        viewModel.isThinking = false
        viewModel.isExpanded = false
        viewModel.chatSurfaceMode = .collapsed
        viewModel.notchInteractionMode = .collapsed
        var presentationRequests = 0
        viewModel.onFirstAssistantToken = {
            presentationRequests += 1
            viewModel.isExpanded = true
            viewModel.chatSurfaceMode = .outputExpanded
            viewModel.notchInteractionMode = .output
        }

        viewModel.ensureCompletedTurnOutputPresented()
        viewModel.ensureCompletedTurnOutputPresented()

        XCTAssertEqual(presentationRequests, 1)
        XCTAssertTrue(viewModel.isExpanded)
        XCTAssertEqual(viewModel.chatSurfaceMode, .outputExpanded)
        XCTAssertEqual(viewModel.notchInteractionMode, .output)
    }
}
