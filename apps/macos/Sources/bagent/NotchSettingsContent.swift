import SwiftUI

/// Notch-styled settings surface — black/white minimal pages rendered inside
/// the notch bridge (`/settings`). Pages switch with ←/→ or the header
/// controls; the notch's left-wing icon mirrors the current page.
struct NotchSettingsContent: View {
    @ObservedObject var viewModel: ChatViewModel
    @ObservedObject private var permissions: PermissionsManager
    @AppStorage(NotchWindowController.pasteWheelEnabledKey) private var pasteWheelEnabled = true
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// +1 = forward (push from trailing), -1 = back.
    @State private var pageDirection: CGFloat = 1

    init(viewModel: ChatViewModel) {
        self.viewModel = viewModel
        self.permissions = viewModel.permissions
    }

    private var page: NotchSettingsPage { viewModel.notchSettingsPage }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            header
            Rectangle()
                .fill(Color.white.opacity(0.10))
                .frame(height: 1)
            ZStack(alignment: .topLeading) {
                pageContent
                    .id(page)
                    .transition(reduceMotion ? .opacity : .asymmetric(
                        insertion: .move(edge: pageDirection > 0 ? .trailing : .leading)
                            .combined(with: .opacity),
                        removal: .move(edge: pageDirection > 0 ? .leading : .trailing)
                            .combined(with: .opacity)
                    ))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .clipped()
            Spacer(minLength: 0)
        }
        .animation(reduceMotion ? nil : .spring(response: 0.32, dampingFraction: 0.86),
                   value: page)
        .onChange(of: viewModel.notchSettingsPage) { old, new in
            pageDirection = new.rawValue >= old.rawValue ? 1 : -1
        }
    }

    // MARK: - Header (title + page icons)

    private var header: some View {
        HStack(spacing: 10) {
            Text(page.title)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                .contentTransition(.opacity)
            Spacer()
            HStack(spacing: 8) {
                ForEach(NotchSettingsPage.allCases, id: \.rawValue) { p in
                    Button {
                        viewModel.notchSettingsPage = p
                    } label: {
                        Image(systemName: p.symbolName)
                            .font(.system(size: 13, weight: .medium))
                            .foregroundStyle(p == page
                                ? Color.white.opacity(0.92)
                                : NotchWrapMetrics.notchTextFaint)
                            .frame(width: 26, height: 26)
                            .background(
                                Circle().fill(Color.white.opacity(p == page ? 0.14 : 0))
                            )
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(p.title)
                }
            }
            Text("←→")
                .font(.system(size: 11))
                .foregroundStyle(NotchWrapMetrics.notchTextFaint)
        }
    }

    // MARK: - Pages

    @ViewBuilder
    private var pageContent: some View {
        switch page {
        case .general:     generalPage
        case .permissions: permissionsPage
        case .model:       modelPage
        case .connectors:  connectorsPage
        }
    }

    private var generalPage: some View {
        VStack(alignment: .leading, spacing: 15) {
            toggleRow(
                icon: "waveform.circle",
                title: "Hlasový režim",
                subtitle: viewModel.voiceModeEnabled
                    ? "⌥Space otvorí hlasový vstup"
                    : "⌥Space otvorí textový chat",
                isOn: $viewModel.voiceModeEnabled
            )
            toggleRow(
                icon: "clipboard",
                title: "Koleso schránky",
                subtitle: permissions.hasAccessibility
                    ? "Podržanie pravého ⌘ otvorí 5 posledných položiek"
                    : "Vyžaduje Accessibility povolenie",
                isOn: $pasteWheelEnabled
            )
            toggleRow(
                icon: "bell.badge",
                title: "cmux notifikácie",
                subtitle: "Upozornenia agentov v notchi",
                isOn: $viewModel.cmuxNotificationsEnabled
            )
        }
    }

    private var permissionsPage: some View {
        VStack(alignment: .leading, spacing: 14) {
            permissionRow("Full Disk Access", granted: permissions.hasFullDiskAccess) {
                permissions.openPrivacySettings()
            }
            permissionRow("Mikrofón", granted: permissions.hasMicrophoneAccess) {
                Task {
                    await permissions.requestMicrophoneAccess()
                    if !permissions.hasMicrophoneAccess { permissions.openMicrophoneSettings() }
                }
            }
            permissionRow("Snímanie obrazovky", granted: permissions.hasScreenRecording) {
                permissions.requestScreenRecording()
                if !permissions.hasScreenRecording { permissions.openScreenRecordingSettings() }
            }
            permissionRow("Accessibility", granted: permissions.hasAccessibility) {
                permissions.requestAccessibility()
                if !permissions.hasAccessibility { permissions.openAccessibilitySettings() }
            }
            Text("Po pregenerovaní podpisu aplikácie treba Accessibility udeliť znova.")
                .font(.system(size: 10))
                .foregroundStyle(NotchWrapMetrics.notchTextFaint)
        }
    }

    private var modelPage: some View {
        VStack(alignment: .leading, spacing: 13) {
            Text("Chat model")
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            ForEach(viewModel.availableModels, id: \.self) { model in
                Button {
                    viewModel.selectedModel = model
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: viewModel.selectedModel == model
                              ? "circle.inset.filled" : "circle")
                            .font(.system(size: 11))
                            .foregroundStyle(viewModel.selectedModel == model
                                ? Color.white.opacity(0.9)
                                : NotchWrapMetrics.notchTextFaint)
                        Text(model)
                            .font(.system(size: 13, design: .monospaced))
                            .foregroundStyle(viewModel.selectedModel == model
                                ? NotchWrapMetrics.notchTextPrimary
                                : NotchWrapMetrics.notchTextSecondary)
                        Spacer()
                    }
                }
                .buttonStyle(.plain)
            }
        }
    }

    private var connectorsPage: some View {
        VStack(alignment: .leading, spacing: 14) {
            connectorRow("Apple Mail", up: viewModel.daemonHealth?.mailConnector)
            connectorRow("Apple Poznámky", up: viewModel.daemonHealth?.notesConnector)
            connectorRow("WhatsApp", up: viewModel.daemonHealth?.whatsappConnector)
            connectorRow("Odoo", up: viewModel.daemonHealth?.odooConnector)
            connectorRow("Codex", up: viewModel.daemonHealth?.codexConnector)
            connectorRow("Ollama", up: viewModel.daemonHealth?.ollamaUp)
        }
    }

    // MARK: - Row builders

    private func toggleRow(icon: String, title: String, subtitle: String,
                           isOn: Binding<Bool>) -> some View {
        HStack(spacing: 8) {
            Image(systemName: icon)
                .font(.system(size: 15))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                Text(subtitle)
                    .font(.system(size: 11))
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
            }
            Spacer()
            Toggle("", isOn: isOn)
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
                .tint(Color.white.opacity(0.35))
                .accessibilityLabel(title)
        }
    }

    private func permissionRow(_ title: String, granted: Bool,
                               grant: @escaping () -> Void) -> some View {
        HStack(spacing: 8) {
            Circle()
                .fill(granted ? Color.white.opacity(0.85) : Color.white.opacity(0.25))
                .frame(width: 7, height: 7)
            Text(title)
                .font(.system(size: 13))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
            Spacer()
            if granted {
                Text("aktívne")
                    .font(.system(size: 12))
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
            } else {
                Button("udeliť", action: grant)
                    .buttonStyle(.plain)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundStyle(Color.white.opacity(0.9))
            }
        }
    }

    private func connectorRow(_ title: String, up: Bool?) -> some View {
        HStack(spacing: 8) {
            Circle()
                .fill(up == true ? Color.white.opacity(0.85) : Color.white.opacity(0.25))
                .frame(width: 7, height: 7)
            Text(title)
                .font(.system(size: 13))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
            Spacer()
            Text(up == nil ? "…" : (up == true ? "pripojený" : "nedostupný"))
                .font(.system(size: 12))
                .foregroundStyle(NotchWrapMetrics.notchTextFaint)
        }
    }
}
