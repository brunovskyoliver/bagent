import AppKit
import SwiftUI
import XCTest
@testable import bagent

final class ActivityTranscriptHostingTests: XCTestCase {
    @MainActor
    func testControllerEndToEndThinkingOutputHoverCollapseAndReopen() throws {
        _ = NSApplication.shared
        let viewModel = ChatViewModel(startMonitoring: false)
        let controller = NotchWindowController(chatViewModel: viewModel)

        controller.presentInputOnly()
        drainMainRunLoop()
        XCTAssertEqual(viewModel.notchInteractionMode, .input)
        XCTAssertTrue(controller.isNotchInteractionShowing)

        viewModel.onInputOnlySubmitted?()
        try viewModel.installThinkingFixture()
        drainMainRunLoop()
        XCTAssertEqual(viewModel.notchInteractionMode, .thinking)

        var message = evidenceMessage()
        message.content = ""
        message.displayedContent = ""
        viewModel.messages = [message]
        viewModel.streamingAssistantMessageId = message.id
        // Match the live failure exactly: the user leaves the activity rows
        // expanded while the blue thinking state is active, and they remain
        // expanded as the answer starts streaming and finalizes.
        viewModel.isActivityTranscriptExpanded = true
        drainMainRunLoop()
        XCTAssertTrue(viewModel.isActivityTranscriptExpanded)

        let panel = controller.statusPanelForTesting
        for x in stride(from: panel.frame.minX + 8, through: panel.frame.maxX - 8, by: 16) {
            let location = panel.convertPoint(fromScreen: NSPoint(x: x, y: panel.frame.midY))
            let event = try XCTUnwrap(NSEvent.mouseEvent(
                with: .mouseMoved,
                location: location,
                modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: panel.windowNumber,
                context: nil,
                eventNumber: 0,
                clickCount: 0,
                pressure: 0
            ))
            panel.sendEvent(event)
        }

        viewModel.messages[0].content = "Verified fixture answer"
        viewModel.messages[0].displayedContent = "Verified fixture answer"
        viewModel.ensureCompletedTurnOutputPresented()
        for index in 0..<40 {
            let chunk = " final-\(index)"
            viewModel.messages[0].content += chunk
            viewModel.messages[0].displayedContent += chunk
            viewModel.streamingChunk += 1
            drainMainRunLoop()
        }
        viewModel.streamingAssistantMessageId = nil
        try viewModel.installCompletedFixture()
        drainMainRunLoop()

        XCTAssertEqual(viewModel.notchInteractionMode, .output)
        XCTAssertEqual(viewModel.chatSurfaceMode, .outputExpanded)
        XCTAssertTrue(viewModel.isActivityTranscriptExpanded)
        XCTAssertTrue(panel.isVisible)
        XCTAssertTrue(controller.isNotchInteractionShowing)

        controller.collapse()
        drainMainRunLoop()
        XCTAssertEqual(viewModel.notchInteractionMode, .collapsed)

        controller.presentInputOnly()
        drainMainRunLoop()
        XCTAssertEqual(viewModel.notchInteractionMode, .input)
        XCTAssertTrue(controller.isNotchInteractionShowing)

        controller.collapse()
        panel.orderOut(nil)
    }

    @MainActor
    func testThinkingCompletionBuildsOutputSurfaceBeforeFurtherHoverEvents() throws {
        let viewModel = ChatViewModel(startMonitoring: false)
        var message = evidenceMessage()
        message.content = ""
        message.displayedContent = ""
        viewModel.messages = [message]
        try viewModel.installThinkingFixture()
        viewModel.streamingAssistantMessageId = message.id

        let hosted = NSHostingView(rootView: NotchWrapView(
            notchWidth: 221,
            notchHeight: 39,
            viewModel: viewModel,
            onTap: {},
            onHoverChanged: { _ in }
        ))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 741, height: 319),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.contentView = hosted
        window.orderFront(nil)
        drainMainRunLoop()

        let thinkingHover = try XCTUnwrap(NSEvent.mouseEvent(
            with: .mouseMoved,
            location: NSPoint(x: 370, y: 300),
            modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: window.windowNumber,
            context: nil,
            eventNumber: 0,
            clickCount: 0,
            pressure: 0
        ))
        window.sendEvent(thinkingHover)
        drainMainRunLoop()

        // Reproduce click-away winning while the thinking surface is delayed.
        // The completed turn must still reopen the answer surface.
        viewModel.applyNotchIntent(.collapse)
        drainMainRunLoop()

        viewModel.messages[0].content = "Verified answer"
        viewModel.messages[0].displayedContent = "Verified answer"
        viewModel.ensureCompletedTurnOutputPresented()
        drainMainRunLoop()

        XCTAssertEqual(viewModel.notchInteractionMode, .output)
        XCTAssertEqual(viewModel.chatSurfaceMode, .outputExpanded)
        XCTAssertEqual(descendants(of: hosted).filter { $0 is NSScrollView }.count, 1)

        // Stress the real output-sizing path. Each chunk used to advance a
        // @State ratchet from a layout getter, recursively invalidating the
        // hosting view at finalization.
        for index in 0..<40 {
            let chunk = " streamed-\(index)"
            viewModel.messages[0].content += chunk
            viewModel.messages[0].displayedContent += chunk
            viewModel.streamingChunk += 1
            drainMainRunLoop()
        }
        viewModel.streamingAssistantMessageId = nil
        drainMainRunLoop()

        for x in stride(from: 270.0, through: 470.0, by: 10.0) {
            let event = try XCTUnwrap(NSEvent.mouseEvent(
                with: .mouseMoved,
                location: NSPoint(x: x, y: 245),
                modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber,
                context: nil,
                eventNumber: 0,
                clickCount: 0,
                pressure: 0
            ))
            window.sendEvent(event)
            drainMainRunLoop()
        }

        XCTAssertTrue(window.contentView === hosted)
        window.orderOut(nil)
    }

    @MainActor
    func testExpandedTranscriptsSurviveFinalizationAndPointerTraversalInHostingView() throws {
        for message in [legacyMessage(), evidenceMessage()] {
            let viewModel = ChatViewModel(startMonitoring: false)
            viewModel.messages = [message]
            viewModel.applyNotchIntent(.openOutput)
            viewModel.streamingAssistantMessageId = message.id

            let hosted = NSHostingView(rootView: NotchWrapView(
                notchWidth: 221,
                notchHeight: 39,
                viewModel: viewModel,
                onTap: {},
                onHoverChanged: { _ in }
            ))
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 741, height: 319),
                styleMask: [.borderless],
                backing: .buffered,
                defer: false
            )
            window.contentView = hosted
            window.orderFront(nil)
            drainMainRunLoop()

            viewModel.isActivityTranscriptExpanded = true
            drainMainRunLoop()
            XCTAssertEqual(
                descendants(of: hosted).filter { $0 is NSScrollView }.count,
                1,
                "The answer is the only scrolling surface; transcript rows must remain stable during hover dispatch"
            )
            viewModel.streamingAssistantMessageId = nil
            drainMainRunLoop()

            for x in stride(from: 270.0, through: 470.0, by: 10.0) {
                let event = try XCTUnwrap(NSEvent.mouseEvent(
                    with: .mouseMoved,
                    location: NSPoint(x: x, y: 245),
                    modifierFlags: [],
                    timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber,
                    context: nil,
                    eventNumber: 0,
                    clickCount: 0,
                    pressure: 0
                ))
                window.sendEvent(event)
                drainMainRunLoop()
            }

            XCTAssertTrue(window.contentView === hosted)
            window.orderOut(nil)
        }
    }

    @MainActor
    private func legacyMessage() -> ChatMessage {
        var message = ChatMessage(role: .assistant, content: "Finished legacy answer")
        message.displayedContent = message.content
        message.activities = (1...6).map { index in
            TurnActivity(
                id: "legacy-\(index)",
                kind: "tool",
                tool: "mail",
                title: "Read message \(index)",
                detail: "Completed",
                status: "completed",
                durationMs: 10
            )
        }
        return message
    }

    @MainActor
    private func evidenceMessage() -> ChatMessage {
        var message = ChatMessage(role: .assistant, content: "Read 3 of 3 emails")
        message.displayedContent = message.content
        message.evidencePhase = .init(
            turnId: "turn-hosting",
            phase: .reading,
            completed: 3,
            total: 3
        )
        message.evidenceOutcome = .init(
            turnId: "turn-hosting",
            state: .verified,
            kind: .mail,
            acquired: 3,
            requested: 3,
            sourceCount: 0,
            message: "Read 3 of 3 emails"
        )
        message.evidenceActivities = (1...6).map { index in
            EvidenceLogicalActivity(event: .init(
                turnId: "turn-hosting",
                activityId: "evidence-\(index)",
                normalizedOperation: "mail.read",
                argumentHash: "hash-\(index)",
                executionStatus: .succeeded,
                contribution: .satisfied,
                evidenceCount: 1,
                sourceDomains: [],
                durationMs: 10,
                attemptCount: 1,
                retries: 0,
                duplicatesSuppressed: 0,
                failureReason: nil
            ))
        }
        return message
    }

    @MainActor
    private func drainMainRunLoop() {
        RunLoop.main.run(until: Date().addingTimeInterval(0.01))
    }

    @MainActor
    private func descendants(of view: NSView) -> [NSView] {
        view.subviews + view.subviews.flatMap(descendants(of:))
    }
}
