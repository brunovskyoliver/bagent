import AppKit
import ApplicationServices
import SwiftUI
import XCTest
@testable import bagent

final class StageRailAccessibilityTests: XCTestCase {
    @MainActor
    func testHostedNotchPreservesRailAndPillButtonsInAccessibilityTree() throws {
        let viewModel = ChatViewModel(startMonitoring: false)
        try viewModel.applyAuthoritativeSnapshot(.init(
            schemaVersion: 1,
            cursor: 1,
            daemonGeneration: "accessibility-host",
            works: [automation("automation-a", order: 1, category: .web)],
            pendingApprovals: [],
            model: .ready
        ))
        let host = NSHostingView(rootView: NotchWrapView(
            notchWidth: 221,
            notchHeight: 38,
            viewModel: viewModel,
            onTap: {},
            onHoverChanged: { _ in }
        ))
        host.setAccessibilityElement(false)
        host.frame = CGRect(x: 0, y: 0, width: 741, height: 318)
        let panel = NSPanel(
            contentRect: host.frame,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.contentView = host
        panel.orderFrontRegardless()
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.8))
        host.layoutSubtreeIfNeeded()
        defer { panel.close() }

        let applicationElement = AXUIElementCreateApplication(getpid())
        guard accessibilityString(applicationElement, attribute: kAXRoleAttribute) != nil else {
            throw XCTSkip("The test runner has no Accessibility API permission")
        }
        let elements = accessibilityTree(from: applicationElement)
        let labels = elements.compactMap { accessibilityString($0, attribute: kAXDescriptionAttribute) }
            + elements.compactMap { accessibilityString($0, attribute: kAXTitleAttribute) }
        let buttonLabels = elements.compactMap { element -> String? in
            guard accessibilityString(element, attribute: kAXRoleAttribute) == kAXButtonRole else {
                return nil
            }
            return accessibilityString(element, attribute: kAXDescriptionAttribute)
                ?? accessibilityString(element, attribute: kAXTitleAttribute)
        }
        let description = elements.map {
            "\(accessibilityString($0, attribute: kAXRoleAttribute) ?? "nil"): "
                + "\(accessibilityString($0, attribute: kAXDescriptionAttribute) ?? "nil")"
        }.joined(separator: ", ")

        XCTAssertTrue(labels.contains("Activity"), "Stage Rail must survive the hosting hierarchy: \(description)")
        XCTAssertTrue(labels.contains("Status"), "status pill must survive the hosting hierarchy: \(description)")
        XCTAssertTrue(buttonLabels.contains("Activity"), "the rail must expose its Return-capable Button: \(description)")
        XCTAssertTrue(buttonLabels.contains("Status"), "the active-count pill must expose its Button: \(description)")
    }

    func testRailAndPillExposeCompleteStableAccessibilityValues() throws {
        let snapshot = NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 1,
            daemonGeneration: "accessibility-fixture",
            works: [
                automation("older", order: 1, category: .mail),
                automation("newer", order: 2, category: .web),
            ],
            pendingApprovals: [],
            model: .ready
        )
        var presentation = try NotchProjection.reduce(
            previous: .idle,
            input: .snapshot(snapshot),
            reduceMotion: true
        )

        XCTAssertEqual(presentation.statusPill.accessibilityLabel, "Status")
        XCTAssertEqual(presentation.statusPill.accessibilityValue, "2 active Automation Runs")
        XCTAssertTrue(presentation.rail.accessibilityValue.contains("Tool"))
        XCTAssertTrue(presentation.rail.accessibilityValue.contains("Checking Mail"))
        XCTAssertTrue(presentation.rail.accessibilityValue.contains("background"))
        XCTAssertTrue(presentation.rail.accessibilityValue.contains("run 1 of 2"))
        XCTAssertTrue(presentation.rail.accessibilityValue.contains("2 active"))

        let revision = presentation.revision
        presentation = try NotchProjection.reduce(
            previous: presentation,
            input: .localIntent(.cycleAutomation),
            reduceMotion: true
        )
        XCTAssertEqual(presentation.revision, revision, "navigation cannot mutate Work")
        XCTAssertTrue(presentation.rail.accessibilityValue.contains("run 2 of 2"))
        XCTAssertTrue(presentation.motion.reduceMotion)
        XCTAssertFalse(presentation.motion.iconMotionEnabled)
        XCTAssertEqual(presentation.motion.contentRevealDuration, 0.12)
    }

    func testAcceptedTextTokensMeetSmallTextContrastAgainstBlack() throws {
        let primary = try XCTUnwrap(NSColor(NotchWrapMetrics.notchTextPrimary).usingColorSpace(.sRGB))
        let secondary = try XCTUnwrap(NSColor(NotchWrapMetrics.notchTextSecondary).usingColorSpace(.sRGB))

        XCTAssertGreaterThanOrEqual(contrastRatio(sRGB: primary.redComponent), 4.5)
        XCTAssertGreaterThanOrEqual(
            contrastRatio(sRGB: secondary.redComponent * 0.86),
            4.5,
            "inactive rail labels remain readable at their rendered opacity"
        )
    }

    private func automation(
        _ identity: String,
        order: UInt64,
        category: NotchActivityCategory
    ) -> NotchWork {
        NotchWork(
            identity: identity,
            revision: 1,
            origin: .automation,
            state: .running,
            activity: .init(category: category),
            queuePosition: nil,
            automationDisplayName: "Saved Automation",
            automationDefinitionIdentity: "definition-\(identity)",
            automationSessionIdentity: "session-\(identity)",
            terminalAttention: nil,
            claimedOrder: order
        )
    }

    private func contrastRatio(sRGB component: CGFloat) -> CGFloat {
        let linear = component <= 0.04045
            ? component / 12.92
            : pow((component + 0.055) / 1.055, 2.4)
        return (linear + 0.05) / 0.05
    }

    private func accessibilityTree(from root: AXUIElement) -> [AXUIElement] {
        var result: [AXUIElement] = []
        var pending = [root]
        var visited = Set<CFHashCode>()
        while let element = pending.popLast() {
            let identifier = CFHash(element)
            guard visited.insert(identifier).inserted else { continue }
            result.append(element)
            if let children = accessibilityValue(element, attribute: kAXChildrenAttribute)
                as? [AXUIElement] {
                pending.append(contentsOf: children)
            }
        }
        return result
    }

    private func accessibilityString(_ element: AXUIElement, attribute: String) -> String? {
        accessibilityValue(element, attribute: attribute) as? String
    }

    private func accessibilityValue(_ element: AXUIElement, attribute: String) -> Any? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else {
            return nil
        }
        return value
    }
}
