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
        case .setup:       setupPage
        }
    }

    // Setup is the only page that scrolls — credentials + rules don't fit the notch.
    private var setupPage: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                odooGroup
                codexGroup
                whatsappGroup
                rulesGroup
            }
            .padding(.trailing, 4)
            .padding(.bottom, 8)
        }
        .scrollIndicators(.never)
        .task { await loadRules() }
    }

    private var odooGroup: some View {
        settingsGroup("Odoo") {
            field("URL", text: $viewModel.odooURL, placeholder: "https://firma.odoo.com")
            field("Databáza", text: $viewModel.odooDB, placeholder: "firma")
            field("Používateľ", text: $viewModel.odooUser, placeholder: "user@example.com")
            field("API kľúč", text: $viewModel.odooAPIKey, placeholder: "z Odoo → API Keys", secure: true)
            field("uvx cesta", text: $viewModel.odooUvxPath, placeholder: "voliteľné")
            actionRow(
                busy: viewModel.isTestingOdoo,
                busyLabel: "Testujem… (prvé spustenie môže trvať minútu)",
                result: viewModel.odooTestResult,
                button: "Testovať",
                disabled: viewModel.odooURL.isEmpty || viewModel.odooDB.isEmpty
                    || viewModel.odooAPIKey.isEmpty,
                action: { viewModel.configureOdoo() }
            )
            note("API kľúč sa nikdy nezapíše na disk — iba Keychain a env MCP child procesu.")
        }
    }

    private var codexGroup: some View {
        settingsGroup("Codex") {
            field("Binárka", text: $viewModel.codexBinaryPath, placeholder: "automaticky z $PATH")
            actionRow(
                busy: viewModel.isTestingCodex,
                busyLabel: "Testujem…",
                result: viewModel.codexTestResult,
                button: "Testovať",
                disabled: false,
                action: { viewModel.testCodex() }
            )
        }
    }

    private var whatsappGroup: some View {
        settingsGroup("WhatsApp") {
            let status = viewModel.whatsappStatus?.status ?? "stopped"
            HStack(spacing: 8) {
                Circle()
                    .fill(status == "ready" ? Color.white.opacity(0.85) : Color.white.opacity(0.25))
                    .frame(width: 7, height: 7)
                Text(viewModel.whatsappStatus?.me_name ?? status)
                    .font(.system(size: 12))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                Spacer()
                if viewModel.isConnectingWhatsapp {
                    Text("Spúšťam…")
                        .font(.system(size: 11))
                        .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                } else if status == "ready" || status == "authenticated" {
                    notchButton("Odpojiť") { viewModel.disconnectWhatsapp() }
                } else {
                    notchButton("Pripojiť") { viewModel.connectWhatsapp() }
                }
            }
            if viewModel.whatsappStatus?.needs_qr == true {
                notchButton("Zobraziť QR") {
                    viewModel.showWhatsappPairing = true
                    viewModel.refreshWhatsappQr()
                }
            }
            if let msg = viewModel.whatsappStatusMessage {
                note(msg)
            }
            note("Neoficiálny WhatsApp Web bridge. Odoslanie správy vždy vyžaduje schválenie.")
        }
    }

    private var rulesGroup: some View {
        settingsGroup("Pravidlá (rules.yaml)") {
            TextEditor(text: $rulesYaml)
                .font(.system(size: 11, design: .monospaced))
                .scrollContentBackground(.hidden)
                .background(Color.white.opacity(0.06))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                .frame(height: 140)
                .clipShape(RoundedRectangle(cornerRadius: 6))
                .disabled(isLoadingRules)
            HStack(spacing: 8) {
                if let err = rulesError {
                    Text(err)
                        .font(.system(size: 11))
                        .foregroundStyle(Color.red.opacity(0.9))
                        .lineLimit(2)
                } else if rulesSaved {
                    Text("Uložené")
                        .font(.system(size: 11))
                        .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                }
                Spacer()
                notchButton("Uložiť") { Task { await saveRules() } }
                    .disabled(isLoadingRules || rulesYaml.isEmpty)
            }
        }
    }

    // MARK: - Rules state

    @State private var rulesYaml: String = ""
    @State private var rulesError: String?
    @State private var rulesSaved = false
    @State private var isLoadingRules = false

    private func loadRules() async {
        guard rulesYaml.isEmpty else { return }
        isLoadingRules = true
        if let yaml = try? await DaemonClient().rulesYaml() { rulesYaml = yaml }
        isLoadingRules = false
    }

    private func saveRules() async {
        rulesError = nil
        rulesSaved = false
        do {
            try await DaemonClient().saveRules(yaml: rulesYaml)
            rulesSaved = true
            try? await Task.sleep(for: .seconds(3))
            rulesSaved = false
        } catch DaemonError.serverError(let msg) {
            rulesError = msg
        } catch {
            rulesError = error.localizedDescription
        }
    }

    // MARK: - Setup row builders

    private func settingsGroup<C: View>(_ title: String,
                                        @ViewBuilder content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            content()
        }
    }

    private func field(_ label: String, text: Binding<String>,
                       placeholder: String, secure: Bool = false) -> some View {
        HStack(spacing: 8) {
            Text(label)
                .font(.system(size: 11))
                .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                .frame(width: 78, alignment: .leading)
            Group {
                if secure {
                    SecureField(placeholder, text: text)
                } else {
                    TextField(placeholder, text: text)
                }
            }
            .textFieldStyle(.plain)
            .font(.system(size: 12, design: .monospaced))
            .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
            .padding(.horizontal, 7)
            .padding(.vertical, 4)
            .background(Color.white.opacity(0.06),
                        in: RoundedRectangle(cornerRadius: 5))
        }
    }

    @ViewBuilder
    private func actionRow(busy: Bool, busyLabel: String, result: String?,
                           button: String, disabled: Bool,
                           action: @escaping () -> Void) -> some View {
        HStack(spacing: 8) {
            if busy {
                Text(busyLabel)
                    .font(.system(size: 11))
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
            } else if let result {
                Text(result)
                    .font(.system(size: 11))
                    .foregroundStyle(result.hasPrefix("✓")
                        ? NotchWrapMetrics.notchTextSecondary
                        : Color.red.opacity(0.9))
                    .lineLimit(2)
            }
            Spacer()
            notchButton(button, action: action)
                .disabled(busy || disabled)
        }
    }

    private func notchButton(_ title: String, action: @escaping () -> Void) -> some View {
        Button(title, action: action)
            .buttonStyle(.plain)
            .font(.system(size: 11, weight: .medium))
            .foregroundStyle(Color.white.opacity(0.9))
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(Color.white.opacity(0.12),
                        in: RoundedRectangle(cornerRadius: 5))
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.system(size: 10))
            .foregroundStyle(NotchWrapMetrics.notchTextFaint)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var generalPage: some View {
        VStack(alignment: .leading, spacing: 15) {
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
