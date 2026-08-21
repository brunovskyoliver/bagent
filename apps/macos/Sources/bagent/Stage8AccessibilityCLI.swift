import AppKit
import ApplicationServices
import SwiftUI

/// Signed, disposable macOS 26 Accessibility evidence for the projection
/// surface. It never grants or changes TCC state and emits only public labels,
/// values, counts, and the final verdict.
@MainActor
enum Stage8AccessibilityCLI {
    static let environmentKey = "BAGENT_STAGE8_ACCESSIBILITY_FIXTURE"

    static func run(outputURL: URL) async -> Int32 {
        guard ProcessInfo.processInfo.environment[environmentKey] == "1" else { return 64 }
        do {
            try await runFixture(outputURL: outputURL)
            return 0
        } catch {
            try? writeEvidence(
                [
                    "status": "failed",
                    "accessibility_available": AXIsProcessTrusted(),
                    "active_element_count": 0,
                    "approval_element_count": 0,
                    "assertion_count": 0,
                    "skipped_count": 0,
                    "error": String(describing: error),
                ],
                to: outputURL
            )
            fputs("Stage 8 live notch Accessibility fixture failed: \(error)\n", stderr)
            return 1
        }
    }

    private enum FixtureError: Error, CustomStringConvertible {
        case unavailable
        case assertion(String)

        var description: String {
            switch self {
            case .unavailable: "Accessibility API is unavailable"
            case .assertion(let message): message
            }
        }
    }

    private static func runFixture(outputURL: URL) async throws {
        guard AXIsProcessTrusted() else {
            throw FixtureError.unavailable
        }

        NSApplication.shared.setActivationPolicy(.accessory)
        NSApplication.shared.finishLaunching()
        let viewModel = ChatViewModel(startMonitoring: false)
        try viewModel.applyAuthoritativeSnapshot(
            .init(
                schemaVersion: 1,
                cursor: 1,
                daemonGeneration: "stage8-accessibility",
                works: [
                    automation("automation-a", order: 1, category: .web),
                    automation("automation-b", order: 2, category: .mail),
                ],
                pendingApprovals: [],
                model: .ready
            )
        )
        viewModel.setNotchReduceMotion(true)

        let host = NSHostingView(
            rootView: NotchWrapView(
                notchWidth: 221,
                notchHeight: 38,
                viewModel: viewModel,
                onTap: {},
                onHoverChanged: { _ in },
                acceptanceReduceMotionOverride: true
            )
        )
        host.setAccessibilityElement(false)
        host.frame = CGRect(x: 0, y: 0, width: 741, height: 318)
        let panel = NSPanel(
            contentRect: host.frame,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.isReleasedWhenClosed = false
        panel.contentView = host
        panel.orderFrontRegardless()
        NSApplication.shared.activate(ignoringOtherApps: true)
        defer { panel.close() }

        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(500))
        host.layoutSubtreeIfNeeded()
        let applicationElement = AXUIElementCreateApplication(getpid())
        let activeElements = accessibilityTree(from: applicationElement)
        let activeAssertions = try verifyActiveProjection(activeElements)

        viewModel.pendingApprovals = [
            ApprovalItem(
                id: "approval-stage8",
                toolName: "filesystem_write",
                description: "Approval required for filesystem_write",
                expiresAt: "",
                createdAt: "",
                origin: nil
            )
        ]
        try viewModel.applyAuthoritativeSnapshot(
            .init(
                schemaVersion: 1,
                cursor: 2,
                daemonGeneration: "stage8-accessibility",
                works: [
                    automation("automation-a", order: 1, category: .web, state: .waitingForApproval),
                    automation("automation-b", order: 2, category: .mail),
                ],
                pendingApprovals: [
                    NotchApproval(identity: "approval-stage8", workIdentity: "automation-a")
                ],
                model: .ready
            )
        )
        try await Task.sleep(for: .milliseconds(300))
        host.layoutSubtreeIfNeeded()
        let approvalElements = accessibilityTree(from: applicationElement)
        let approvalAssertions = try verifyApprovalProjection(approvalElements)

        try writeEvidence(
            [
                "status": "pass",
                "accessibility_available": true,
                "active_element_count": activeElements.count,
                "approval_element_count": approvalElements.count,
                "assertion_count": activeAssertions + approvalAssertions,
                "skipped_count": 0,
                "states": ["active", "approval"],
                "reduced_motion": true,
            ],
            to: outputURL
        )
    }

    private static func verifyActiveProjection(_ elements: [AXUIElement]) throws -> Int {
        let labels = Set(elements.compactMap { stringAttribute($0, kAXDescriptionAttribute) }
            + elements.compactMap { stringAttribute($0, kAXTitleAttribute) })
        guard labels.contains("Activity"), labels.contains("Status") else {
            throw FixtureError.assertion("active projection labels are incomplete")
        }
        let buttons = elements.filter { stringAttribute($0, kAXRoleAttribute) == kAXButtonRole }
        let buttonLabels = Set(buttons.compactMap { stringAttribute($0, kAXDescriptionAttribute) }
            + buttons.compactMap { stringAttribute($0, kAXTitleAttribute) })
        guard buttonLabels.contains("Activity"), buttonLabels.contains("Status") else {
            throw FixtureError.assertion("active projection buttons are incomplete")
        }
        let values = elements.compactMap { displayedValue($0) }.joined(separator: " ")
        guard values.contains("2 active"), values.contains("Automation Runs") else {
            throw FixtureError.assertion("active projection values are incomplete")
        }
        return 3
    }

    private static func verifyApprovalProjection(_ elements: [AXUIElement]) throws -> Int {
        let labels = Set(elements.compactMap { stringAttribute($0, kAXDescriptionAttribute) }
            + elements.compactMap { stringAttribute($0, kAXTitleAttribute) })
        let values = elements.compactMap { displayedValue($0) }
        let strings = Set(labels).union(values)
        guard strings.contains("Schválenie akcie"), strings.contains("Schváliť"), strings.contains("Zamietnuť") else {
            throw FixtureError.assertion(
                "approval projection labels are incomplete: \((strings).sorted().joined(separator: " | "))"
            )
        }
        let valueText = values.joined(separator: " ")
        guard valueText.contains("Approval required for filesystem_write") else {
            throw FixtureError.assertion(
                "approval projection value is incomplete: \(valueText)"
            )
        }
        return 2
    }

    private static func automation(
        _ identity: String,
        order: UInt64,
        category: NotchActivityCategory,
        state: NotchWorkState = .running
    ) -> NotchWork {
        NotchWork(
            identity: identity,
            revision: state == .waitingForApproval ? 2 : 1,
            origin: .automation,
            state: state,
            activity: .init(category: category),
            queuePosition: nil,
            automationDisplayName: "Controlled Automation",
            automationDefinitionIdentity: "definition-\(identity)",
            automationSessionIdentity: "session-\(identity)",
            terminalAttention: nil,
            claimedOrder: order
        )
    }

    private static func accessibilityTree(from root: AXUIElement) -> [AXUIElement] {
        var result: [AXUIElement] = []
        var pending = [root]
        var visited = Set<CFHashCode>()
        while let element = pending.popLast() {
            let identifier = CFHash(element)
            guard visited.insert(identifier).inserted else { continue }
            result.append(element)
            if let children = valueAttribute(element, kAXChildrenAttribute) as? [AXUIElement] {
                pending.append(contentsOf: children)
            }
        }
        return result
    }

    private static func valueAttribute(_ element: AXUIElement, _ attribute: String) -> Any? {
        var value: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success else { return nil }
        return value
    }

    private static func stringAttribute(_ element: AXUIElement, _ attribute: String) -> String? {
        valueAttribute(element, attribute) as? String
    }

    private static func displayedValue(_ element: AXUIElement) -> String? {
        stringAttribute(element, kAXValueAttribute) ?? stringAttribute(element, kAXValueDescriptionAttribute)
    }

    private static func writeEvidence(_ object: [String: Any], to url: URL) throws {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: url, options: .atomic)
    }
}
