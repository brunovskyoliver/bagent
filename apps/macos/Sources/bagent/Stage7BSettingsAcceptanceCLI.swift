import AppKit
import ApplicationServices
import SwiftUI

@MainActor
private final class Stage7BLiveStateStore: ObservableObject {
    @Published var state = CompassRailAcceptanceState.baseline
}

private struct Stage7BLiveHost: View {
    @ObservedObject var viewModel: ChatViewModel
    @ObservedObject var stateStore: Stage7BLiveStateStore

    var body: some View {
        NotchSettingsContent(
            viewModel: viewModel,
            acceptanceState: stateStore.state,
            reduceMotionOverride: true
        )
    }
}

enum Stage7BSettingsAcceptanceCLI {
    static let environmentKey = "BAGENT_STAGE7B_SETTINGS_FIXTURE"
    static let accessibilityEnvironmentKey = "BAGENT_STAGE7B_SETTINGS_AX_FIXTURE"

    static func run(outputDirectory: URL, variant: String) async -> Int32 {
        guard ProcessInfo.processInfo.environment[environmentKey] == "1" else { return 64 }
        do {
            try await renderCatalog(to: outputDirectory, variant: variant)
            return 0
        } catch {
            fputs("Stage 7B settings fixture failed: \(error)\n", stderr)
            return 1
        }
    }

    static func runLiveAccessibility(outputDirectory: URL) async -> Int32 {
        guard ProcessInfo.processInfo.environment[accessibilityEnvironmentKey] == "1" else { return 64 }
        do {
            return try await liveAccessibilityRun(to: outputDirectory)
        } catch {
            try? writeJSON(
                [
                    "status": "failed",
                    "error": String(describing: error),
                    "route_count": 0,
                    "element_count": 0,
                    "assertion_count": 0,
                    "skipped_count": 0,
                ],
                to: outputDirectory.appendingPathComponent("live-ax.json")
            )
            fputs("Stage 7B live Accessibility fixture failed: \(error)\n", stderr)
            return 1
        }
    }

    @MainActor
    private static func renderCatalog(to outputDirectory: URL, variant: String) async throws {
        try FileManager.default.createDirectory(at: outputDirectory, withIntermediateDirectories: true)
        try validateInteractionContract()
        try validateContrastContract()
        setenv("BAGENT_DATA_DIR", outputDirectory.path, 1)
        let viewModel = ChatViewModel(startMonitoring: false, settingsFixture: true)
        viewModel.openNotchSettings()
        var renderedRoutes: [String] = []
        for panelWidth in CompassRailStateCatalog.syntheticPanelWidths {
            for route in CompassRailStateCatalog.routes {
                viewModel.selectCompassRailArea(route.area)
                if let child = route.child {
                    viewModel.openCompassRailChild(child)
                }
                for state in states(for: route) {
                    let root = ZStack {
                        Color.black
                        NotchSettingsContent(
                            viewModel: viewModel,
                            acceptanceState: state,
                            reduceMotionOverride: variant == "reduce-motion"
                        )
                        .padding(.horizontal, 18)
                        .padding(.vertical, 12)
                    }
                    .frame(width: CGFloat(panelWidth), height: 318)
                    .environment(\.dynamicTypeSize, variant == "large-text" ? DynamicTypeSize.accessibility2 : DynamicTypeSize.large)
                    // macOS SwiftUI exposes colorSchemeContrast as read-only. The
                    // disposable fixture injects the equivalent contrast state
                    // without changing the user's display preference.
                    .environment(\.compassRailHighContrast, variant == "high-contrast")
                    .contrast(variant == "high-contrast" ? 1.5 : 1)
                    let themedRoot: AnyView
                    switch variant {
                    case "light": themedRoot = AnyView(root.environment(\.colorScheme, .light))
                    case "dark": themedRoot = AnyView(root.environment(\.colorScheme, .dark))
                    default: themedRoot = AnyView(root)
                    }
                    let host = NSHostingView(rootView: themedRoot)
                    host.frame = CGRect(x: 0, y: 0, width: CGFloat(panelWidth), height: 318)
                    host.layoutSubtreeIfNeeded()
                    try await Task.sleep(for: .milliseconds(40))
                    host.layoutSubtreeIfNeeded()
                    let fittingSize = host.fittingSize
                    guard fittingSize.width.isFinite, fittingSize.height.isFinite,
                          fittingSize.width <= host.bounds.width + 1,
                          fittingSize.height <= host.bounds.height + 1 else {
                        throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 4)
                    }
                    guard let bitmap = host.bitmapImageRepForCachingDisplay(in: host.bounds) else {
                        throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 2)
                    }
                    guard bitmap.pixelsWide > 0, bitmap.pixelsHigh > 0 else {
                        throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 5)
                    }
                    host.cacheDisplay(in: host.bounds, to: bitmap)
                    try validateBitmapInset(bitmap)
                    guard let data = bitmap.representation(using: .png, properties: [:]) else {
                        throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 3)
                    }
                    let name = "w\(Int(panelWidth))-\(route.identifier)-\(stateIdentifier(state))"
                    try data.write(to: outputDirectory.appendingPathComponent("\(name).png"), options: Data.WritingOptions.atomic)
                    renderedRoutes.append(name)
                }
            }
        }

        let evidence: [String: Any] = [
            "variant": variant,
            "color_scheme": ["light", "dark"].contains(variant) ? variant : "system",
            "route_count": CompassRailStateCatalog.routes.count,
            "rendered_image_count": renderedRoutes.count,
            "routes": renderedRoutes,
            "panel_widths": CompassRailStateCatalog.syntheticPanelWidths,
            "pixel_height": 318,
            "model_runtime_state_count": CompassRailStateCatalog.modelRuntimeStates.count,
            "model_runtime_fixture_count": CompassRailStateCatalog.modelRuntimeFixtureStates.count,
            "validation_state_count": CompassRailStateCatalog.validationStates.count,
            "permission_state_count": CompassRailStateCatalog.permissionStates.count,
            "state_render_count_per_width": renderedRoutes.count / CompassRailStateCatalog.syntheticPanelWidths.count,
            "reduce_motion": variant == "reduce-motion",
            "signed_fixture_pid": ProcessInfo.processInfo.processIdentifier,
        ]
        let evidenceData = try JSONSerialization.data(withJSONObject: evidence, options: [.prettyPrinted, .sortedKeys])
        try evidenceData.write(to: outputDirectory.appendingPathComponent("catalog.json"), options: .atomic)
    }

    @MainActor
    private static func liveAccessibilityRun(to outputDirectory: URL) async throws -> Int32 {
        try FileManager.default.createDirectory(at: outputDirectory, withIntermediateDirectories: true)

        guard accessibilityProbeIsAvailable() else {
            try writeJSON(
                [
                    "status": "needs_accessibility_grant",
                    "accessibility_available": false,
                    "route_count": 0,
                    "element_count": 0,
                    "assertion_count": 0,
                    "skipped_count": 0,
                ],
                to: outputDirectory.appendingPathComponent("live-ax.json")
            )
            fputs("Stage 7B live Accessibility fixture: Accessibility unavailable\n", stderr)
            return 78
        }

        NSApplication.shared.setActivationPolicy(.accessory)
        NSApplication.shared.finishLaunching()

        let viewModel = ChatViewModel(startMonitoring: false, settingsFixture: true)
        let stateStore = Stage7BLiveStateStore()
        let host = NSHostingView(rootView: Stage7BLiveHost(viewModel: viewModel, stateStore: stateStore))
        host.frame = CGRect(x: 0, y: 0, width: 520, height: 500)
        host.setAccessibilityElement(false)
        func makePanel(for host: NSHostingView<Stage7BLiveHost>) -> NSPanel {
            let panel = NSPanel(
                contentRect: host.frame,
                styleMask: [.titled],
                backing: .buffered,
                defer: false
            )
            panel.title = "Stage 7B Accessibility Fixture"
            panel.isReleasedWhenClosed = false
            panel.contentView = host
            return panel
        }
        let panel = makePanel(for: host)
        panel.orderFrontRegardless()
        panel.makeKeyAndOrderFront(nil)
        NSApplication.shared.activate(ignoringOtherApps: true)
        defer { panel.close() }

        let applicationElement = AXUIElementCreateApplication(getpid())
        _ = try await waitForPeerRail(applicationElement, host: host, panel: panel, context: "initial")
        var routeEvidence: [[String: Any]] = []
        var totalElements = 0
        var totalAssertions = 0
        let skipped = 0

        for route in CompassRailStateCatalog.routes {
            for state in states(for: route) {
                viewModel.openNotchSettings()
                viewModel.selectCompassRailArea(route.area)
                if let child = route.child { viewModel.openCompassRailChild(child) }
                stateStore.state = state
                panel.orderFrontRegardless()
                panel.makeKeyAndOrderFront(nil)
                NSApplication.shared.activate(ignoringOtherApps: true)
                try await settle(host: host)
                let elements = try await waitForPeerRail(applicationElement, host: host, panel: panel, context: route.identifier)
                let result = try verify(route: route, state: state, elements: elements)
                totalElements += elements.count
                totalAssertions += result.assertions
                routeEvidence.append([
                    "route": route.identifier,
                    "state": stateIdentifier(state),
                    "element_count": elements.count,
                    "assertion_count": result.assertions,
                    "peer_order": result.peerOrder,
                    "value_descriptions": result.valueDescriptions,
                    "status": "pass",
                ])
            }
        }

        totalAssertions += try await verifyKeyboardAndFocus(
            viewModel: viewModel,
            host: host,
            stateStore: stateStore,
            applicationElement: applicationElement,
            panel: panel
        )

        guard !routeEvidence.isEmpty, totalElements > 0, totalAssertions > 0, skipped == 0 else {
            throw LiveAccessibilityError.invalidEvidence
        }
        try writeJSON(
            [
                "status": "pass",
                "accessibility_available": true,
                "route_count": routeEvidence.count,
                "element_count": totalElements,
                "assertion_count": totalAssertions,
                "skipped_count": skipped,
                "routes": routeEvidence,
                "fixture_pid": ProcessInfo.processInfo.processIdentifier,
            ],
            to: outputDirectory.appendingPathComponent("live-ax.json")
        )
        return 0
    }

    private static func accessibilityProbeIsAvailable() -> Bool {
        guard AXIsProcessTrusted() else { return false }
        let candidates = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.finder")
            .filter { $0.processIdentifier != ProcessInfo.processInfo.processIdentifier }
        for candidate in candidates {
            let element = AXUIElementCreateApplication(candidate.processIdentifier)
            var role: CFTypeRef?
            if AXUIElementCopyAttributeValue(element, kAXRoleAttribute as CFString, &role) == .success,
               role != nil {
                return true
            }
        }
        return false
    }

    @MainActor
    private static func settle(host: NSView) async throws {
        host.layoutSubtreeIfNeeded()
        try await Task.sleep(for: .milliseconds(180))
        host.layoutSubtreeIfNeeded()
        NSApp.keyWindow?.displayIfNeeded()
    }

    @MainActor
    private static func waitForPeerRail(_ application: AXUIElement, host: NSView, panel: NSPanel, context: String) async throws -> [AXUIElement] {
        for _ in 0..<40 {
            host.layoutSubtreeIfNeeded()
            host.displayIfNeeded()
            panel.orderFrontRegardless()
            panel.makeKeyAndOrderFront(nil)
            NSApplication.shared.activate(ignoringOtherApps: true)
            let elements = accessibilityTree(from: AXUIElementCreateApplication(getpid()))
            let peerCount = CompassRailArea.allCases.reduce(into: 0) { count, peer in
                if button(named: peer.title, in: elements) != nil { count += 1 }
            }
            if peerCount == CompassRailArea.allCases.count { return elements }
            try await Task.sleep(for: .milliseconds(50))
        }
        throw LiveAccessibilityError.assertion("live Compass Rail did not become ready for \(context)")
    }

    private struct AccessibilityVerificationResult {
        let assertions: Int
        let peerOrder: [String]
        let valueDescriptions: [String]
    }

    private enum LiveAccessibilityError: Error, CustomStringConvertible {
        case assertion(String)
        case invalidEvidence

        var description: String {
            switch self {
            case .assertion(let message): message
            case .invalidEvidence: "live Accessibility evidence was empty or skipped"
            }
        }
    }

    private static func verify(
        route: CompassRailRoute,
        state: CompassRailAcceptanceState,
        elements: [AXUIElement]
    ) throws -> AccessibilityVerificationResult {
        var assertions = 0
        let roles = elements.compactMap { stringAttribute($0, kAXRoleAttribute) }
        let scrollRoles = [kAXScrollAreaRole, kAXScrollBarRole, "AXScrollView"]
        let hasScrollAttribute = elements.contains { element in
            valueAttribute(element, "AXHorizontalScrollBar") != nil
                || valueAttribute(element, "AXVerticalScrollBar") != nil
                || boolAttribute(element, "AXScrollable")
        }
        guard !roles.contains(where: scrollRoles.contains), !hasScrollAttribute else {
            throw LiveAccessibilityError.assertion("scroll semantics exposed for \(route.identifier)")
        }
        assertions += 1

        let buttons = elements.compactMap { element -> (element: AXUIElement, label: String, value: String?)? in
            guard stringAttribute(element, kAXRoleAttribute) == kAXButtonRole else { return nil }
            guard let label = stringAttribute(element, kAXDescriptionAttribute) ?? stringAttribute(element, kAXTitleAttribute) else { return nil }
            return (element, label, displayedValue(element))
        }
        let peerTitles = CompassRailArea.allCases.map(\.title)
        let peerButtons = buttons.filter { peerTitles.contains($0.label) }
        let peerOrder = peerButtons.map(\.label)
        guard peerOrder == peerTitles else {
            throw LiveAccessibilityError.assertion("peer button order mismatch for \(route.identifier): \(peerOrder)")
        }
        assertions += 1
        guard peerButtons.filter({ $0.value?.contains("Selected") == true }).map(\.label) == [route.area.title] else {
            throw LiveAccessibilityError.assertion("selected peer state mismatch for \(route.identifier)")
        }
        assertions += 1
        guard peerButtons.allSatisfy({ ($0.value ?? "").isEmpty == false }) else {
            throw LiveAccessibilityError.assertion("peer button value missing for \(route.identifier)")
        }
        assertions += 1

        let expectedHeading = route.child?.title ?? route.area.title
        let headings = elements.compactMap { element -> String? in
            guard stringAttribute(element, kAXRoleAttribute) == "AXHeading" else { return nil }
            return stringAttribute(element, kAXDescriptionAttribute) ?? stringAttribute(element, kAXTitleAttribute)
        }
        guard headings.contains(expectedHeading) else { throw LiveAccessibilityError.assertion("heading missing for \(route.identifier)") }
        assertions += 1

        if route.isChild {
            let backLabel = route.area.backAccessibilityLabel
            guard buttons.contains(where: { $0.label == backLabel }) else {
                throw LiveAccessibilityError.assertion("Back label missing for \(route.identifier)")
            }
            assertions += 1
        }

        let valueElements = elements.filter { displayedValue($0)?.isEmpty == false }
        guard !valueElements.isEmpty else { throw LiveAccessibilityError.assertion("no AX values for \(route.identifier)") }
        assertions += 1

        let values = valueElements.compactMap(displayedValue)
        let expected = expectedLabelledValues(for: route, state: state)
        let labelledValues = elements.reduce(into: [String: String]()) { valuesByLabel, element in
            guard let label = stringAttribute(element, kAXDescriptionAttribute) ?? stringAttribute(element, kAXTitleAttribute),
                  let value = displayedValue(element),
                  !value.isEmpty,
                  !(stringAttribute(element, kAXRoleAttribute) == kAXButtonRole && CompassRailArea.allCases.map(\.title).contains(label)) else { return }
            valuesByLabel[label] = value
        }
        guard labelledValues == expected else {
            throw LiveAccessibilityError.assertion("AX value set mismatch for \(route.identifier): actual=\(labelledValues), expected=\(expected)")
        }
        assertions += expected.count
        if expected.isEmpty {
            assertions += values.isEmpty ? 0 : 1
        }
        return AccessibilityVerificationResult(assertions: assertions, peerOrder: peerOrder, valueDescriptions: values)
    }

    private static func expectedLabelledValues(
        for route: CompassRailRoute,
        state: CompassRailAcceptanceState
    ) -> [String: String] {
        switch route {
        case .area(.general):
            return ["Paste wheel": "On", "cmux notifications": "On"]
        case .area(.modelRuntime):
            return [
                "Local chat model": state.selectedModel,
                "Residency": modelPhaseDisplayName(state.modelPhase),
                "Preload on input": state.preloadOnInput ? "On" : "Off",
                "Shared idle timeout": "\(state.idleTimeoutSeconds) seconds",
                "Active lease": state.activeLease ? "Active — unloading prevented" : "None",
                "Automation waiting": state.automationWaiting ? "Waiting for residency" : "No work waiting",
                "Changed-PID recovery": state.changedPIDRecovery,
            ]
        case .area(.integrations):
            return ["Apple Mail": "Testing", "Apple Notes": "Testing", "WhatsApp": "Not tested", "Odoo": "Testing", "Codex": "Testing", "Connector service": "Local service unavailable"]
        case .area(.privacyAndPermissions):
            return ["Full Disk Access": "Needs setup", "Screen Recording": "Needs setup", "Accessibility": "Needs setup", "Rules and approval policy": "Daemon-owned"]
        case .child(.whatsapp), .child(.odoo), .child(.codex):
            switch route {
            case .child(.whatsapp): return ["Status": state.validationState, "Validation": state.validationState]
            case .child(.odoo), .child(.codex): return ["Validation": state.validationState]
            default: return [:]
            }
        case .child(.fullDiskAccess), .child(.screenRecording), .child(.accessibility):
            return [route.child?.title ?? "": state.permissionGranted ? "Active" : "Needs setup"]
        case .child(.rulesAndApprovalPolicy):
            return ["Policy source": "Configured by daemon"]
        }
    }

    private static func modelPhaseDisplayName(_ phase: NotchModelPhase) -> String {
        switch phase {
        case .unavailable, .poisoned: "Unavailable"
        case .unloaded: "Unloaded"
        case .loading: "Loading"
        case .loadedNotReady: "Loaded"
        case .ready: "Ready"
        case .retiring: "Unloading"
        case .restarting: "Loading"
        }
    }

    @MainActor
    private static func verifyKeyboardAndFocus(
        viewModel: ChatViewModel,
        host: NSHostingView<Stage7BLiveHost>,
        stateStore: Stage7BLiveStateStore,
        applicationElement: AXUIElement,
        panel: NSPanel
    ) async throws -> Int {
        var assertions = 0
        let compassMonitor = NotchWindowController.installCompassRailKeyMonitor(
            on: panel,
            viewModel: viewModel,
            focusedControl: { NotchWindowController.compassRailFocusedControl(in: panel) }
        )
        defer {
            if let compassMonitor {
                NSEvent.removeMonitor(compassMonitor)
            }
        }

        viewModel.openNotchSettings()
        viewModel.selectCompassRailArea(.general)
        stateStore.state = .baseline
        try await settle(host: host)
        let generalElements = accessibilityTree(from: applicationElement)
        let peerButtons = CompassRailArea.allCases.compactMap { peer in
            button(named: peer.title, in: generalElements)
        }
        guard peerButtons.count == CompassRailArea.allCases.count else {
            throw LiveAccessibilityError.assertion("live peer focus order is incomplete")
        }
        guard AXUIElementSetAttributeValue(peerButtons[0], kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success else {
            throw LiveAccessibilityError.assertion("AX could not focus the first peer")
        }
        try await settle(host: host)
        for peer in CompassRailArea.allCases.dropFirst() {
            guard let tab = keyEvent(keyCode: 48, characters: "\t", windowNumber: panel.windowNumber) else {
                throw LiveAccessibilityError.assertion("could not create Tab event")
            }
            NSApp.sendEvent(tab)
            try await settle(host: host)
            let focused = focusedElement(from: applicationElement)
            guard let expected = button(named: peer.title, in: accessibilityTree(from: applicationElement)),
                  focused.map({ sameElement($0, expected) }) == true || boolAttribute(expected, kAXFocusedAttribute) else {
                throw LiveAccessibilityError.assertion("live Tab focus order failed at \(peer.title)")
            }
            assertions += 1
        }

        viewModel.selectCompassRailArea(.general)
        guard AXUIElementSetAttributeValue(peerButtons[0], kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success,
              let rightArrow = keyEvent(keyCode: 124, characters: "\u{F703}", windowNumber: panel.windowNumber) else {
            throw LiveAccessibilityError.assertion("production right-arrow rail navigation failed")
        }
        NSApp.sendEvent(rightArrow)
        guard viewModel.compassRailRoute == .area(.modelRuntime) else {
            throw LiveAccessibilityError.assertion("production right-arrow rail navigation failed")
        }
        assertions += 1

        viewModel.selectCompassRailArea(.integrations)
        stateStore.state = .baseline
        try await settle(host: host)
        let before = accessibilityTree(from: applicationElement)
        guard let codex = button(named: CompassRailChild.codex.title, in: before) else {
            let labels = before.filter { stringAttribute($0, kAXRoleAttribute) == kAXButtonRole }.map {
                stringAttribute($0, kAXDescriptionAttribute) ?? stringAttribute($0, kAXTitleAttribute) ?? ""
            }
            throw LiveAccessibilityError.assertion("Codex opener missing: \(labels)")
        }
        guard AXUIElementSetAttributeValue(codex, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success else {
            throw LiveAccessibilityError.assertion("AX could not focus the Codex opener")
        }
        try await settle(host: host)
        guard boolAttribute(codex, kAXFocusedAttribute) else {
            throw LiveAccessibilityError.assertion("AX focus did not land on the Codex opener")
        }
        guard AXUIElementPerformAction(codex, kAXPressAction as CFString) == .success else {
            throw LiveAccessibilityError.assertion("AX press could not open Codex")
        }
        try await settle(host: host)
        guard viewModel.compassRailRoute == .child(.codex) else {
            throw LiveAccessibilityError.assertion("AX child navigation failed")
        }
        assertions += 1

        guard let back = button(named: CompassRailArea.integrations.backAccessibilityLabel, in: accessibilityTree(from: applicationElement)) else {
            throw LiveAccessibilityError.assertion("AX Back button missing")
        }
        guard AXUIElementPerformAction(back, kAXPressAction as CFString) == .success else {
            throw LiveAccessibilityError.assertion("AX press could not go Back")
        }
        try await settle(host: host)
        let afterBack = accessibilityTree(from: applicationElement)
        let restored = button(named: CompassRailChild.codex.title, in: afterBack)
        guard viewModel.compassRailRoute == .area(.integrations),
              let restored,
              focusedElement(from: applicationElement).map({ sameElement($0, restored) }) == true
                || boolAttribute(restored, kAXFocusedAttribute) else {
            throw LiveAccessibilityError.assertion("focus did not return to the Codex opener after Back")
        }
        assertions += 1

        viewModel.openCompassRailChild(.odoo)
        stateStore.state = .baseline
        try await settle(host: host)
        guard let editable = elements(withRole: kAXTextFieldRole, in: accessibilityTree(from: applicationElement)).first,
              AXUIElementSetAttributeValue(editable, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success else {
            throw LiveAccessibilityError.assertion("editable AX control missing")
        }
        try await settle(host: host)
        guard focusedElement(from: applicationElement).map({ sameElement($0, editable) }) == true
                || boolAttribute(editable, kAXFocusedAttribute) else {
            throw LiveAccessibilityError.assertion("AX could not focus editable control")
        }
        guard let leftArrow = keyEvent(keyCode: 123, characters: "\u{F702}", windowNumber: panel.windowNumber) else {
            throw LiveAccessibilityError.assertion("could not create left-arrow event")
        }
        NSApp.sendEvent(leftArrow)
        try await settle(host: host)
        guard viewModel.compassRailRoute == .child(.odoo),
              focusedElement(from: applicationElement).map({ sameElement($0, editable) }) == true
                || boolAttribute(editable, kAXFocusedAttribute) else {
            throw LiveAccessibilityError.assertion("editable control did not retain left-arrow ownership")
        }
        assertions += 1
        _ = panel
        return assertions
    }

    private static func keyEvent(keyCode: UInt16, characters: String, windowNumber: Int) -> NSEvent? {
        NSEvent.keyEvent(
            with: .keyDown,
            location: .zero,
            modifierFlags: [],
            timestamp: 0,
            windowNumber: windowNumber,
            context: nil,
            characters: characters,
            charactersIgnoringModifiers: characters,
            isARepeat: false,
            keyCode: keyCode
        )
    }

    private static func accessibilityTree(from root: AXUIElement) -> [AXUIElement] {
        var result: [AXUIElement] = []
        var pending = [root]
        var visited = Set<CFHashCode>()
        while let element = pending.popLast() {
            guard visited.insert(CFHash(element)).inserted else { continue }
            result.append(element)
            if let children = valueAttribute(element, kAXChildrenAttribute) as? [AXUIElement] {
                pending.append(contentsOf: children.reversed())
            }
        }
        return result
    }

    private static func elements(withRole role: String, in elements: [AXUIElement]) -> [AXUIElement] {
        elements.filter { stringAttribute($0, kAXRoleAttribute) == role }
    }

    private static func button(named name: String, in elements: [AXUIElement]) -> AXUIElement? {
        elements.first {
            stringAttribute($0, kAXRoleAttribute) == kAXButtonRole
                && (stringAttribute($0, kAXDescriptionAttribute) == name || stringAttribute($0, kAXTitleAttribute) == name)
        }
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

    private static func boolAttribute(_ element: AXUIElement, _ attribute: String) -> Bool {
        (valueAttribute(element, attribute) as? Bool) == true
            || (valueAttribute(element, attribute) as? NSNumber)?.boolValue == true
    }

    private static func focusedElement(from application: AXUIElement) -> AXUIElement? {
        guard let value = valueAttribute(application, kAXFocusedUIElementAttribute) else { return nil }
        return unsafeBitCast(value as AnyObject, to: AXUIElement.self)
    }

    private static func sameElement(_ lhs: AXUIElement, _ rhs: AXUIElement) -> Bool {
        CFEqual(lhs, rhs)
    }

    private static func writeJSON(_ object: [String: Any], to url: URL) throws {
        let data = try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys])
        try data.write(to: url, options: .atomic)
    }

    private static func states(for route: CompassRailRoute) -> [CompassRailAcceptanceState] {
        switch route {
        case .area(.modelRuntime):
            return CompassRailStateCatalog.modelRuntimeStates.flatMap { phase in
                CompassRailStateCatalog.modelRuntimeFixtureStates.map { fixtureState in
                    CompassRailAcceptanceState(
                        modelPhase: phase,
                        validationState: "Not tested",
                        permissionGranted: false,
                        preloadOnInput: false,
                        idleTimeoutSeconds: 1200,
                        activeLease: fixtureState == "active-lease",
                        automationWaiting: fixtureState == "waiting-for-residency",
                        changedPIDRecovery: fixtureState == "changed-pid-recovery" ? "In progress" : "Not reported"
                    )
                }
            }
        case .child(.whatsapp), .child(.odoo), .child(.codex):
            return CompassRailStateCatalog.validationStates.map {
                CompassRailAcceptanceState(modelPhase: .ready, validationState: $0, permissionGranted: false, preloadOnInput: false, idleTimeoutSeconds: 1200)
            }
        case .child(.fullDiskAccess), .child(.screenRecording), .child(.accessibility):
            return CompassRailStateCatalog.permissionStates.map {
                CompassRailAcceptanceState(modelPhase: .ready, validationState: "Not tested", permissionGranted: $0 == "Active", preloadOnInput: false, idleTimeoutSeconds: 1200)
            }
        default:
            return [.baseline]
        }
    }

    private static func stateIdentifier(_ state: CompassRailAcceptanceState) -> String {
        if state.activeLease { return state.modelPhase.rawValue + "-active-lease" }
        if state.automationWaiting { return state.modelPhase.rawValue + "-waiting-for-residency" }
        if state.changedPIDRecovery == "In progress" { return state.modelPhase.rawValue + "-changed-pid-recovery" }
        if state.modelPhase != .ready {
            return state.modelPhase.rawValue
        }
        if state.validationState != "Not tested" { return state.validationState.lowercased().replacingOccurrences(of: " ", with: "-") }
        return state.permissionGranted ? "active" : "needs-setup"
    }

    @MainActor
    private static func validateInteractionContract() throws {
        let viewModel = ChatViewModel(startMonitoring: false, settingsFixture: true)
        viewModel.openNotchSettings()
        guard viewModel.handleCompassRailKey(.right, focusedControl: nil) == .select(.modelRuntime),
              viewModel.compassRailRoute == .area(.modelRuntime) else {
            throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 10)
        }
        viewModel.openCompassRailChild(.codex)
        guard viewModel.handleCompassRailKey(.escape, focusedControl: nil) == .back,
              viewModel.compassRailRoute == .area(.integrations) else {
            throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 11)
        }
        guard CompassRailKeyboard.route(.right, route: .area(.general), focusedControl: nil) == .select(.modelRuntime),
              CompassRailKeyboard.route(.left, route: .area(.integrations), focusedControl: .textField) == nil,
              CompassRailKeyboard.route(.escape, route: .child(.codex), focusedControl: nil) == .back else {
            throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 6)
        }
        var focus = CompassRailFocusMemory()
        focus.remember(.child(.odoo))
        guard focus.controlToRestore(afterOpening: .odoo) == .child(.odoo) else {
            throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 7)
        }
    }

    private static func validateContrastContract() throws {
        let standard = contrastRatio(foregroundWhiteOpacity: 0.55, backgroundWhiteOpacity: 0)
        let high = contrastRatio(foregroundWhiteOpacity: CompassRailStateCatalog.highContrastSecondaryOpacity, backgroundWhiteOpacity: 0)
        guard standard >= 4.5, high >= 4.5, high > standard else {
            throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 8)
        }
    }

    private static func contrastRatio(foregroundWhiteOpacity: Double, backgroundWhiteOpacity: Double) -> Double {
        let foreground = relativeLuminance(sRGB: foregroundWhiteOpacity)
        let background = relativeLuminance(sRGB: backgroundWhiteOpacity)
        return (max(foreground, background) + 0.05) / (min(foreground, background) + 0.05)
    }

    private static func relativeLuminance(sRGB: Double) -> Double {
        let linear = sRGB <= 0.04045 ? sRGB / 12.92 : pow((sRGB + 0.055) / 1.055, 2.4)
        return 0.2126 * linear + 0.7152 * linear + 0.0722 * linear
    }

    private static func validateBitmapInset(_ bitmap: NSBitmapImageRep) throws {
        let maxX = max(0, bitmap.pixelsWide - 1)
        let maxY = max(0, bitmap.pixelsHigh - 1)
        for point in [
            (0, 0), (maxX, 0), (0, maxY), (maxX, maxY),
            (maxX / 2, 0), (maxX / 2, maxY), (0, maxY / 2), (maxX, maxY / 2)
        ] {
            guard let color = bitmap.colorAt(x: point.0, y: point.1)?.usingColorSpace(.deviceRGB) else { continue }
            var red = CGFloat.zero
            var green = CGFloat.zero
            var blue = CGFloat.zero
            var alpha = CGFloat.zero
            color.getRed(&red, green: &green, blue: &blue, alpha: &alpha)
            guard alpha < 0.05 || max(red, green, blue) < 0.12 else {
                throw NSError(domain: "Stage7BSettingsAcceptanceCLI", code: 9)
            }
        }
    }
}
