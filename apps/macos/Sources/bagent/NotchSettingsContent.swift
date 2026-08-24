import AppKit
import SwiftUI

/// The bounded settings presentation inside the existing notch panel.
/// Navigation is owned by `ChatViewModel.compassRailRoute`; this view only
/// projects existing preferences, daemon state, connector state, and permission
/// compatibility summaries.
struct NotchSettingsContent: View {
    @ObservedObject var viewModel: ChatViewModel
    @ObservedObject var browserCoordinator: BrowserCoordinator
    @ObservedObject private var permissions: PermissionsManager
    private let acceptanceState: CompassRailAcceptanceState?
    private let reduceMotionOverride: Bool?
    @AppStorage(NotchWindowController.pasteWheelEnabledKey) private var pasteWheelEnabled = true
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion
    @Environment(\.compassRailHighContrast) private var highContrast
    @FocusState private var focusedControl: CompassRailFocusedControl?
    @State private var focusMemory = CompassRailFocusMemory()

    @State private var showingBrowserProfileConfirmation = false

    init(
        viewModel: ChatViewModel,
        browserCoordinator: BrowserCoordinator = BrowserCoordinator(),
        acceptanceState: CompassRailAcceptanceState? = nil,
        reduceMotionOverride: Bool? = nil
    ) {
        self.viewModel = viewModel
        self.browserCoordinator = browserCoordinator
        self.permissions = viewModel.permissions
        self.acceptanceState = acceptanceState
        self.reduceMotionOverride = reduceMotionOverride
    }

    private var route: CompassRailRoute { viewModel.compassRailRoute }
    private var area: CompassRailArea { route.area }
    private var reduceMotion: Bool { reduceMotionOverride ?? systemReduceMotion }
    private var rulesPolicyDisplay: String {
        acceptanceState == nil ? viewModel.rulesPolicySummary : "Configured by daemon"
    }

    var body: some View {
        return VStack(alignment: .leading, spacing: 7) {
            rail
            Rectangle().fill(Color.white.opacity(0.10)).frame(height: 1).accessibilityHidden(true)
            content
                .id(route)
                .transition(reduceMotion ? .opacity : .asymmetric(
                    insertion: .move(edge: .trailing).combined(with: .opacity),
                    removal: .move(edge: .leading).combined(with: .opacity)
                ))
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .clipped()
        }
        .accessibilityElement(children: .contain)
        .animation(
            reduceMotion ? nil : .easeInOut(duration: 0.22),
            value: route
        )
        .onChange(of: route) { oldRoute, newRoute in
            if oldRoute.isChild, !newRoute.isChild, oldRoute.area == newRoute.area,
               let remembered = focusMemory.openingControl {
                restoreFocus(remembered)
                return
            }
            if !newRoute.isChild, oldRoute.area != newRoute.area {
                DispatchQueue.main.async { focusedControl = .rail(newRoute.area) }
            }
        }
    }

    private var rail: some View {
        HStack(spacing: 3) {
            ForEach(CompassRailArea.allCases, id: \.self) { peer in
                Button {
                    viewModel.selectCompassRailArea(peer)
                    focusedControl = .rail(peer)
                } label: {
                    Label {
                        Text(peer.title).lineLimit(1).minimumScaleFactor(0.65)
                    } icon: {
                        Image(systemName: peer.symbolName)
                    }
                    .settingsFont(size: 9, weight: .medium)
                        .frame(maxWidth: .infinity).frame(height: 24)
                        .foregroundStyle(peer == area ? NotchWrapMetrics.notchTextPrimary : (highContrast ? Color.white.opacity(CompassRailStateCatalog.highContrastSecondaryOpacity) : NotchWrapMetrics.notchTextFaint))
                        .background(RoundedRectangle(cornerRadius: 5).fill(Color.white.opacity(peer == area ? 0.14 : 0)))
                }
                .buttonStyle(.plain)
                .focusable()
                .focused($focusedControl, equals: .rail(peer))
                .accessibilityLabel(peer.title)
                .accessibilityValue(peer == area ? "Selected" : "Not selected")
                .accessibilityAddTraits(peer == area ? .isSelected : [])
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Compass Rail")
    }

    @ViewBuilder
    private var content: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 6) {
                if route.isChild {
                    Button { goBack() } label: {
                        Image(systemName: "chevron.left").settingsFont(size: 10, weight: .semibold)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(area.backAccessibilityLabel)
                }
                Text(route.child?.title ?? area.title)
                    .settingsFont(size: 14, weight: .semibold)
                    .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 0)
            }
            page
        }
        .onAppear {
            if !route.isChild, let remembered = focusMemory.openingControl {
                restoreFocus(remembered)
            }
        }
    }

    @ViewBuilder
    private var page: some View {
        switch route {
        case .area(.general): generalPage
        case .area(.modelRuntime): modelRuntimePage
        case .area(.integrations): integrationsPage
        case .area(.privacyAndPermissions): privacyPage
        case .child(.whatsapp): whatsappPage
        case .child(.odoo): odooPage
        case .child(.codex): codexPage
        case .child(.fullDiskAccess): fullDiskAccessPage
        case .child(.screenRecording): screenRecordingPage
        case .child(.accessibility): accessibilityPage
        case .child(.rulesAndApprovalPolicy): rulesPage
        }
    }

    private func goBack() {
        guard viewModel.goBackInCompassRail() else { return }
        if let remembered = focusMemory.openingControl {
            restoreFocus(remembered)
        }
    }

    private func restoreFocus(_ control: CompassRailFocusedControl) {
        DispatchQueue.main.async {
            focusedControl = control
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                focusedControl = control
            }
        }
    }


    private var generalPage: some View {
        VStack(alignment: .leading, spacing: 9) {
            ToggleRow(title: "Paste wheel", subtitle: permissions.hasAccessibility ? "Hold the right Command key for recent clips" : "Accessibility permission required", isOn: $pasteWheelEnabled)
            ToggleRow(title: "cmux notifications", subtitle: "Agent notifications in the notch", isOn: $viewModel.cmuxNotificationsEnabled)
            ToggleRow(
                title: "bagent Browser",
                subtitle: "Private WebKit sessions for Codex and Claude",
                isOn: Binding(
                    get: { browserCoordinator.isEnabled },
                    set: { browserCoordinator.setEnabled($0) }
                )
            )
            if browserCoordinator.isEnabled {
                Text("Browser Profile keeps its own website data.").settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextFaint).fixedSize(horizontal: false, vertical: true)
                actionButton("Clear Browser Profile") { showingBrowserProfileConfirmation = true }
            }
            Text("Option-Space opens the notch input.").settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextFaint).fixedSize(horizontal: false, vertical: true)
        }
        .alert("Clear bagent Browser Profile?", isPresented: $showingBrowserProfileConfirmation) {
            Button("Cancel", role: .cancel) {}
            Button("Clear Profile", role: .destructive) {
                Task { await browserCoordinator.clearProfileAfterUserConfirmation() }
            }
        } message: {
            Text("This closes all Browser Sessions and removes their cookies and website data.")
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("General settings")
    }

    private var modelRuntimePage: some View {
        let phase = acceptanceState?.modelPhase ?? viewModel.notchPresentation.snapshot.model
        let waiting = acceptanceState?.automationWaiting ?? viewModel.notchPresentation.snapshot.works.contains { $0.state == .waitingForModel }
        return VStack(alignment: .leading, spacing: 4) {
            SummaryRow(title: "Local chat model", value: acceptanceState?.selectedModel ?? viewModel.daemonHealth?.model ?? "Unavailable")
            SummaryRow(title: "Residency", value: phase.displayName)
            SummaryRow(title: "Preload on input", value: preloadPolicySummary)
            SummaryRow(title: "Shared idle timeout", value: idleTimeoutSummary)
            SummaryRow(title: "Active lease", value: runtimeLeaseSummary)
            SummaryRow(title: "Automation waiting", value: waiting ? "Waiting for residency" : "No work waiting")
            SummaryRow(title: "Changed-PID recovery", value: changedPIDRecoverySummary)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Model and Runtime settings")
    }

    private var runtimeLeaseSummary: String {
        if let acceptanceState { return acceptanceState.activeLease ? "Active — unloading prevented" : "None" }
        guard let runtime = viewModel.daemonHealth?.modelRuntime else { return "Not reported" }
        return runtime.leaseCount == 0 ? "None" : "Active — unloading prevented"
    }

    private var preloadPolicySummary: String {
        if let acceptanceState { return acceptanceState.preloadOnInput ? "On" : "Off" }
        guard let value = viewModel.daemonHealth?.modelRuntime?.preloadOnInput else { return "Not reported" }
        return value ? "On" : "Off"
    }

    private var idleTimeoutSummary: String {
        if let acceptanceState { return "\(acceptanceState.idleTimeoutSeconds) \(String(localized: "seconds", defaultValue: "seconds"))" }
        guard let seconds = viewModel.daemonHealth?.modelRuntime?.sharedIdleTimeoutSeconds else { return "Not reported" }
        return "\(seconds) \(String(localized: "seconds", defaultValue: "seconds"))"
    }

    private var changedPIDRecoverySummary: String {
        if let acceptanceState { return acceptanceState.changedPIDRecovery }
        return switch viewModel.daemonHealth?.modelRuntime?.changedPIDRecovery {
        case "in progress": "In progress"
        case "ready": "Ready"
        default: "Not reported"
        }
    }

    private var integrationsPage: some View {
        VStack(alignment: .leading, spacing: 4) {
            PermissionConnectorRow(title: "Apple Mail", state: localPermissionConnectorState(viewModel.daemonHealth?.mailConnector), focus: $focusedControl, focusValue: .appleMail) { openChild(.fullDiskAccess, opener: .appleMail) }
            PermissionConnectorRow(title: "Apple Notes", state: localPermissionConnectorState(viewModel.daemonHealth?.notesConnector), focus: $focusedControl, focusValue: .appleNotes) { openChild(.fullDiskAccess, opener: .appleNotes) }
            ChildConnectorRow(title: "WhatsApp", state: whatsappOverviewState, focus: $focusedControl, focusValue: .child(.whatsapp)) { openChild(.whatsapp) }
            ChildConnectorRow(title: "Odoo", state: odooOverviewState, focus: $focusedControl, focusValue: .child(.odoo)) { openChild(.odoo) }
            ChildConnectorRow(title: "Codex", state: codexOverviewState, focus: $focusedControl, focusValue: .child(.codex)) { openChild(.codex) }
            SummaryRow(title: "Connector service", value: viewModel.daemonHealth?.daemonUp == true ? "Available" : "Local service unavailable")
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Integrations settings")
    }

    private var privacyPage: some View {
        VStack(alignment: .leading, spacing: 5) {
            ChildConnectorRow(title: "Full Disk Access", state: permissionSummary(permissions.hasFullDiskAccess), focus: $focusedControl, focusValue: .child(.fullDiskAccess)) { openChild(.fullDiskAccess) }
            ChildConnectorRow(title: "Screen Recording", state: permissionSummary(permissions.hasScreenRecording), focus: $focusedControl, focusValue: .child(.screenRecording)) { openChild(.screenRecording) }
            ChildConnectorRow(title: "Accessibility", state: permissionSummary(permissions.hasAccessibility), focus: $focusedControl, focusValue: .child(.accessibility)) { openChild(.accessibility) }
            ChildConnectorRow(title: "Rules and approval policy", state: "Daemon-owned", focus: $focusedControl, focusValue: .child(.rulesAndApprovalPolicy)) { openChild(.rulesAndApprovalPolicy) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Privacy and Permissions settings")
    }

    private var whatsappPage: some View {
        VStack(alignment: .leading, spacing: 6) {
            SummaryRow(title: "Status", value: whatsappOverviewState)
            if let validationState = acceptanceState?.validationState {
                SummaryRow(title: "Validation", value: validationState)
            } else if viewModel.isConnectingWhatsapp {
                SummaryRow(title: "Validation", value: "Testing")
            } else if viewModel.whatsappStatus?.status == "ready" || viewModel.whatsappStatus?.status == "authenticated" {
                SummaryRow(title: "Validation", value: "Valid")
                actionButton("Disconnect") { viewModel.disconnectWhatsapp() }
            } else if viewModel.daemonHealth?.daemonUp != true {
                SummaryRow(title: "Validation", value: "Local service unavailable")
            } else {
                SummaryRow(title: "Validation", value: viewModel.whatsappStatus?.status == "error" ? "Validation failed" : "Needs setup")
                actionButton("Connect") { viewModel.connectWhatsapp() }
            }
            if viewModel.whatsappStatus?.needs_qr == true {
                actionButton("Show pairing QR") { viewModel.showWhatsappPairing = true; viewModel.refreshWhatsappQr() }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("WhatsApp configuration")
    }

    private var odooPage: some View {
        VStack(alignment: .leading, spacing: 4) {
            compactField("URL", text: $viewModel.odooURL, placeholder: "https://company.odoo.com")
            compactField("Database", text: $viewModel.odooDB, placeholder: "company")
            compactField("User", text: $viewModel.odooUser, placeholder: "user@example.com")
            compactField("API key", text: $viewModel.odooAPIKey, placeholder: "Stored in Keychain", secure: true)
            HStack(spacing: 6) {
                SummaryRow(title: "Validation", value: normalizedValidation(viewModel.odooTestResult, isTesting: viewModel.isTestingOdoo, serviceAvailable: viewModel.odooMcpAvailable))
                Spacer(minLength: 0)
                actionButton("Test") { viewModel.configureOdoo() }
                    .disabled(!viewModel.canTestOdoo || viewModel.isTestingOdoo)
            }
            Text("Secrets stay in Keychain and are never displayed.").settingsFont(size: 9).foregroundStyle(NotchWrapMetrics.notchTextFaint)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Odoo configuration")
    }

    private var codexPage: some View {
        VStack(alignment: .leading, spacing: 7) {
            compactField("Binary", text: $viewModel.codexBinaryPath, placeholder: "Automatic from PATH")
            SummaryRow(title: "Validation", value: normalizedValidation(viewModel.codexTestResult, isTesting: viewModel.isTestingCodex, serviceAvailable: viewModel.codexServiceAvailable))
            actionButton(viewModel.isTestingCodex ? "Testing…" : "Test") { viewModel.testCodex() }.disabled(viewModel.isTestingCodex)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Codex configuration")
    }

    private var rulesPage: some View {
        VStack(alignment: .leading, spacing: 7) {
            SummaryRow(title: "Policy source", value: rulesPolicyDisplay)
            Text("Policy details remain authoritative in the daemon.").settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextFaint).fixedSize(horizontal: false, vertical: true)
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Rules and approval policy")
        .onAppear {
            if acceptanceState == nil {
                viewModel.refreshRulesPolicySummary()
            }
        }
    }

    private var fullDiskAccessPage: some View {
        permissionAssistPage(kind: .fullDiskAccess, title: "Full Disk Access", granted: permissionGranted(permissions.hasFullDiskAccess)) { permissions.openPrivacySettings() }
    }

    private var screenRecordingPage: some View {
        permissionAssistPage(kind: .screenRecording, title: "Screen Recording", granted: permissionGranted(permissions.hasScreenRecording)) { permissions.requestScreenRecording() }
    }

    private var accessibilityPage: some View {
        permissionAssistPage(kind: .accessibility, title: "Accessibility", granted: permissionGranted(permissions.hasAccessibility)) { permissions.requestAccessibility() }
    }

    private func permissionAssistPage(kind: PermissionGrantKind, title: LocalizedStringKey, granted: Bool, action: @escaping () -> Void) -> some View {
        let phase = permissions.phase(for: kind)
        return VStack(alignment: .leading, spacing: 7) {
            SummaryRow(title: title, value: granted ? "Active" : "Needs setup")
            if granted && phase == .grantedButUIRelaunchRequired {
                Text("The permission is granted, but this signed UI must be replaced before it can use it. The daemon remains running.")
                    .settingsFont(size: 9)
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                    .fixedSize(horizontal: false, vertical: true)
                actionButton("Relaunch bagent") { viewModel.requestUIOnlyRelaunch(for: kind) }
                if let error = viewModel.uiOnlyRelaunchError {
                    Text(error)
                        .settingsFont(size: 9)
                        .foregroundStyle(.red.opacity(0.9))
                }
            } else if !granted {
                Text("macOS requires your action. bagent cannot grant this permission automatically. Opening a pane does not prove success; returning from System Settings triggers an authoritative recheck. bagent never resets TCC.")
                    .settingsFont(size: 9)
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                    .fixedSize(horizontal: false, vertical: true)
                actionButton("Open", action: action)
                DraggableApplicationAffordance(kind: kind, permissions: permissions)
                Text(permissionHint(for: kind))
                    .settingsFont(size: 9)
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(title)
        .onAppear {
            if acceptanceState == nil {
                permissions.beginAssist(for: kind)
            }
        }
    }

    private func permissionHint(for kind: PermissionGrantKind) -> String {
        switch kind {
        case .fullDiskAccess: "Select Full Disk Access if the exact pane was not confirmed."
        case .screenRecording: "Select Screen Recording if the exact pane was not confirmed."
        case .accessibility: "Select Accessibility if the exact pane was not confirmed."
        }
    }

    private func openChild(_ child: CompassRailChild, opener: CompassRailFocusedControl? = nil) {
        focusMemory.remember(opener ?? .child(child))
        viewModel.openCompassRailChild(child)
    }

    private var whatsappOverviewState: String {
        if let validationState = acceptanceState?.validationState { return validationState }
        if viewModel.daemonHealth?.daemonUp != true { return "Local service unavailable" }
        if viewModel.isConnectingWhatsapp { return "Testing" }
        if viewModel.whatsappStatus?.status == "ready" || viewModel.whatsappStatus?.status == "authenticated" { return "Connected" }
        if viewModel.whatsappStatus?.status == "error" { return "Validation failed" }
        return "Needs setup"
    }

    private func localConnectorState(_ connected: Bool?) -> String {
        guard viewModel.daemonHealth?.daemonUp != false else { return "Local service unavailable" }
        guard let connected else { return "Testing" }
        return connected ? "Connected" : "Needs setup"
    }

    private var odooOverviewState: String {
        connectorOverviewState(
            connected: viewModel.daemonHealth?.odooConnector,
            result: viewModel.odooTestResult,
            isTesting: viewModel.isTestingOdoo,
            serviceAvailable: viewModel.odooMcpAvailable
        )
    }

    private var codexOverviewState: String {
        connectorOverviewState(
            connected: viewModel.daemonHealth?.codexConnector,
            result: viewModel.codexTestResult,
            isTesting: viewModel.isTestingCodex,
            serviceAvailable: viewModel.codexServiceAvailable
        )
    }

    private func connectorOverviewState(connected: Bool?, result: String?, isTesting: Bool, serviceAvailable: Bool?) -> String {
        if isTesting { return "Testing" }
        if serviceAvailable == false { return "Local service unavailable" }
        if let result, !result.isEmpty { return result.hasPrefix("✓") ? "Valid" : "Validation failed" }
        return localConnectorState(connected)
    }

    private func permissionGranted(_ actual: Bool) -> Bool {
        acceptanceState?.permissionGranted ?? actual
    }

    private func permissionSummary(_ actual: Bool) -> String {
        permissionGranted(actual) ? "Active" : "Needs setup"
    }

    private func localPermissionConnectorState(_ connected: Bool?) -> String {
        guard let connected else { return "Testing" }
        // The UI probe is only evidence for this signed UI process. Mail and
        // Notes are read by the daemon, so its connector projection remains
        // the authority for connector readiness.
        return connected ? "Connected" : "Permission required"
    }

    private func normalizedValidation(_ result: String?, isTesting: Bool, serviceAvailable: Bool?) -> String {
        if let validationState = acceptanceState?.validationState { return validationState }
        if viewModel.daemonHealth?.daemonUp != true { return "Local service unavailable" }
        if isTesting { return "Testing" }
        if serviceAvailable == false { return "Local service unavailable" }
        guard let result, !result.isEmpty else { return "Not tested" }
        return result.hasPrefix("✓") ? "Valid" : "Validation failed"
    }

    private func compactField(_ label: LocalizedStringKey, text: Binding<String>, placeholder: LocalizedStringKey, secure: Bool = false) -> some View {
        HStack(spacing: 5) {
            Text(label).settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextFaint).frame(width: 50, alignment: .leading)
            Group {
                if secure {
                    SecureField(placeholder, text: text).focused($focusedControl, equals: .secureField)
                } else {
                    TextField(placeholder, text: text).focused($focusedControl, equals: .textField)
                }
            }
            .textFieldStyle(.plain).settingsFont(size: 11, design: .monospaced).foregroundStyle(NotchWrapMetrics.notchTextPrimary)
            .padding(.horizontal, 6).padding(.vertical, 3).background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 5))
        }
    }

    private func actionButton(_ title: LocalizedStringKey, action: @escaping () -> Void) -> some View {
        Button(title, action: action).buttonStyle(.plain).settingsFont(size: 10, weight: .medium).foregroundStyle(Color.white.opacity(0.9))
            .padding(.horizontal, 8).padding(.vertical, 3).background(Color.white.opacity(0.12), in: RoundedRectangle(cornerRadius: 5))
    }
}

private struct ToggleRow: View {
    let title: LocalizedStringKey
    let subtitle: LocalizedStringKey
    @Binding var isOn: Bool

    var body: some View {
        HStack(spacing: 7) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title).settingsFont(size: 12, weight: .medium).foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                Text(subtitle).settingsFont(size: 9).foregroundStyle(NotchWrapMetrics.notchTextFaint)
            }
            Spacer(minLength: 0)
            Toggle(title, isOn: $isOn).labelsHidden().toggleStyle(.switch).controlSize(.small).tint(Color.white.opacity(0.35))
                .accessibilityLabel(title).accessibilityValue(isOn ? "On" : "Off")
        }
    }
}

private struct SummaryRow: View {
    let title: LocalizedStringKey
    let value: String

    var body: some View {
        HStack(spacing: 5) {
            Text(title).settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextFaint)
            Spacer(minLength: 4)
            Text(localizedValue(value)).settingsFont(size: 10, weight: .medium).foregroundStyle(NotchWrapMetrics.notchTextSecondary).lineLimit(1)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(title))
        .accessibilityValue(Text(value))
    }
}

private struct PermissionConnectorRow: View {
    let title: LocalizedStringKey
    let state: String
    let focus: FocusState<CompassRailFocusedControl?>.Binding
    let focusValue: CompassRailFocusedControl
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Text(title).settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                Spacer(minLength: 4)
                Text(localizedValue(state)).settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                Image(systemName: "chevron.right").settingsFont(size: 8, weight: .semibold).foregroundStyle(NotchWrapMetrics.notchTextFaint).accessibilityHidden(true)
            }
        }
        .buttonStyle(.plain)
        .focusable()
        .focused(focus, equals: focusValue)
        .accessibilityLabel(title)
        .accessibilityValue(localizedValue(state))
        .accessibilityHint("Opens Privacy and Permissions")
    }
}

private struct ChildConnectorRow: View {
    let title: LocalizedStringKey
    let state: String
    let focus: FocusState<CompassRailFocusedControl?>.Binding
    let focusValue: CompassRailFocusedControl
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Text(title).settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                Spacer(minLength: 4)
                Text(localizedValue(state)).settingsFont(size: 10).foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                Image(systemName: "chevron.right").settingsFont(size: 8, weight: .semibold).foregroundStyle(NotchWrapMetrics.notchTextFaint).accessibilityHidden(true)
            }
        }
        .buttonStyle(.plain)
        .focusable()
        .focused(focus, equals: focusValue)
        .accessibilityLabel(title).accessibilityValue(localizedValue(state))
    }
}

private func localizedValue(_ value: String) -> String {
    String(localized: String.LocalizationValue(value))
}

private struct SettingsFontModifier: ViewModifier {
    let size: CGFloat
    let weight: Font.Weight
    let design: Font.Design
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    func body(content: Content) -> some View {
        let scale: CGFloat = dynamicTypeSize.isAccessibilitySize ? 1.15 : 1
        content.font(.system(size: size * scale, weight: weight, design: design))
    }
}

private extension View {
    func settingsFont(size: CGFloat, weight: Font.Weight = .regular, design: Font.Design = .default) -> some View {
        modifier(SettingsFontModifier(size: size, weight: weight, design: design))
    }
}

private extension NotchModelPhase {
    var displayName: String {
        switch self {
        case .unavailable, .poisoned: "Unavailable"
        case .unloaded: "Unloaded"
        case .loading, .restarting: "Loading"
        case .loadedNotReady: "Loaded"
        case .ready: "Ready"
        case .retiring: "Unloading"
        }
    }
}

private struct DraggableApplicationAffordance: View {
    let kind: PermissionGrantKind
    @ObservedObject var permissions: PermissionsManager
    private let bundleURL = Bundle.main.bundleURL
    @State private var hasValidSignedBundle = false

    var body: some View {
        Group {
            if hasValidSignedBundle {
                Label("Drag bagent.app to the selected pane", systemImage: "app.badge")
                    .settingsFont(size: 9, weight: .medium)
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                    .onDrag {
                        permissions.dragBegan(for: kind)
                        return NSItemProvider(object: ApplicationDragRepresentation.writer(for: bundleURL))
                    } preview: {
                        Image(nsImage: NSWorkspace.shared.icon(forFile: bundleURL.path))
                            .resizable()
                            .frame(width: 64, height: 64)
                    }
            } else {
                Text("Use System Settings' native Add flow for the signed running application.")
                    .settingsFont(size: 9)
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
            }
        }
        .accessibilityLabel(ApplicationDragRepresentation.accessibilityLabel)
        .accessibilityHint(ApplicationDragRepresentation.accessibilityHint)
        .task(id: bundleURL.path) {
            let valid = await Task.detached {
                ApplicationDragRepresentation.hasValidSignedBundle(at: bundleURL)
            }.value
            guard !Task.isCancelled else { return }
            hasValidSignedBundle = valid
        }
    }
}
