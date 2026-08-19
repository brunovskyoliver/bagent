import AppKit
import SwiftUI

@MainActor
enum Stage5AcceptanceFixture {
    static func run(variant: String) -> Never {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        let viewModel = ChatViewModel(startMonitoring: false)
        try? viewModel.applyAuthoritativeSnapshot(.init(
            schemaVersion: 1,
            cursor: 7,
            daemonGeneration: "stage5-fixture",
            works: [
                automation("fixture-run-a", definition: "fixture-definition-a", order: 1, category: .mail),
                automation("fixture-run-b", definition: "fixture-definition-b", order: 2, category: .web),
                terminalFailure(),
            ],
            pendingApprovals: [],
            model: .ready
        ))
        let notchWidth: CGFloat = 221
        let notchHeight: CGFloat = 38
        let size = CGSize(
            width: 2 * NotchWrapMetrics.maxWingWidth + notchWidth,
            height: notchHeight + NotchWrapMetrics.maxBridgeHeight
        )
        let panel = BagentPanel(
            contentRect: CGRect(origin: .zero, size: size),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.level = .floating
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary]
        panel.contentView = NSHostingView(
            rootView: NotchWrapView(
                notchWidth: notchWidth,
                notchHeight: notchHeight,
                viewModel: viewModel,
                onTap: {},
                onHoverChanged: { _ in },
                acceptanceReduceMotionOverride: variant == "reduce-motion"
            )
            .environment(
                \.dynamicTypeSize,
                variant == "large-text" ? DynamicTypeSize.accessibility2 : DynamicTypeSize.large
            )
        )
        if let screen = NSScreen.main {
            panel.setFrameOrigin(CGPoint(
                x: screen.frame.midX - size.width / 2,
                y: screen.frame.maxY - size.height
            ))
        }
        panel.makeKeyAndOrderFront(nil)
        application.activate(ignoringOtherApps: true)
        withExtendedLifetime((panel, viewModel)) {
            application.run()
        }
        fatalError("NSApplication run loop returned")
    }

    private static func automation(
        _ identity: String,
        definition: String,
        order: UInt64,
        category: NotchActivityCategory
    ) -> NotchWork {
        NotchWork(
            identity: identity,
            revision: 3,
            origin: .automation,
            state: .running,
            activity: .init(category: category),
            queuePosition: nil,
            automationDisplayName: order == 1 ? "Morning Mail" : "Web Monitor",
            automationDefinitionIdentity: definition,
            automationSessionIdentity: "fixture-session-\(order)",
            terminalAttention: nil,
            claimedOrder: order
        )
    }

    private static func terminalFailure() -> NotchWork {
        NotchWork(
            identity: "fixture-terminal",
            revision: 5,
            origin: .automation,
            state: .failed,
            activity: nil,
            queuePosition: nil,
            automationDisplayName: "Nightly Report",
            automationDefinitionIdentity: "fixture-definition-terminal",
            automationSessionIdentity: "fixture-session-terminal",
            terminalAttention: .failed,
            terminalOrder: 6,
            claimedOrder: 3
        )
    }
}
