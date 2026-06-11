import SwiftUI

struct SettingsView: View {
    @ObservedObject var viewModel: ChatViewModel
    @ObservedObject private var permissions: PermissionsManager

    init(viewModel: ChatViewModel) {
        self.viewModel = viewModel
        self.permissions = viewModel.permissions
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                statusSection
                permissionsSection
                connectorsSection
                ollamaSection
                memorySection
                rulesSection
                shortcutsSection
            }
            .padding(16)
        }
        .task {
            await viewModel.loadModels()
            await viewModel.refreshHealth()
            await viewModel.loadMemoryItems()
        }
    }

    // MARK: - Permissions

    private var permissionsSection: some View {
        SettingsSection(title: "Oprávnenia") {
            VStack(spacing: 6) {
                HStack(spacing: 8) {
                    Circle()
                        .fill(permissions.hasFullDiskAccess ? Color.green : Color.orange)
                        .frame(width: 7, height: 7)
                    Text("Full Disk Access")
                        .font(.system(size: 12))
                    Spacer()
                    if !permissions.hasFullDiskAccess {
                        Button("Udeliť") { permissions.openPrivacySettings() }
                            .font(.system(size: 11))
                            .buttonStyle(.plain)
                            .foregroundStyle(Color.accentColor)
                    } else {
                        Text("aktívne").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                if !permissions.hasFullDiskAccess {
                    Text("Potrebné pre prístup k Mail a Poznámkam.")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    // MARK: - Connectors

    private var connectorsSection: some View {
        SettingsSection(title: "Konektory") {
            VStack(spacing: 8) {
                ConnectorRow(
                    label: "Apple Mail",
                    icon: "envelope",
                    accessible: viewModel.daemonHealth?.mailConnector
                )
                ConnectorRow(
                    label: "Apple Poznámky",
                    icon: "note.text",
                    accessible: viewModel.daemonHealth?.notesConnector
                )
                Divider()
                HStack {
                    if viewModel.isSyncing {
                        ProgressView().scaleEffect(0.65)
                        Text("Synchronizujem…")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    } else if let result = viewModel.lastSyncResult {
                        Text(result)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                            .lineLimit(2)
                    } else {
                        Text("Mail nebol ešte synchronizovaný")
                            .font(.system(size: 11))
                            .foregroundStyle(.tertiary)
                    }
                    Spacer()
                    Button {
                        Task { await viewModel.syncMail() }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "arrow.clockwise")
                                .font(.system(size: 10))
                            Text("Sync")
                                .font(.system(size: 11))
                        }
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(Color.accentColor)
                    .disabled(viewModel.isSyncing || viewModel.daemonHealth?.mailConnector != true)
                }
            }
        }
    }

    // MARK: - Status

    private var statusSection: some View {
        SettingsSection(title: "Stav") {
            VStack(spacing: 6) {
                StatusRow(label: "Daemon", up: viewModel.daemonHealth?.daemonUp)
                StatusRow(label: "Ollama", up: viewModel.daemonHealth?.ollamaUp)
                if let model = viewModel.daemonHealth?.model, model != "—" {
                    HStack {
                        Text("Model daemon").font(.system(size: 12)).foregroundStyle(.secondary)
                        Spacer()
                        Text(model).font(.system(size: 11)).foregroundStyle(.tertiary)
                    }
                }
            }
        }
    }

    // MARK: - Ollama

    private var ollamaSection: some View {
        SettingsSection(title: "Ollama") {
            VStack(alignment: .leading, spacing: 8) {
                Text("Model").font(.system(size: 11)).foregroundStyle(.secondary)
                Picker("", selection: $viewModel.selectedModel) {
                    ForEach(viewModel.availableModels, id: \.self) { m in
                        Text(m).tag(m)
                    }
                }
                .labelsHidden()
                .frame(maxWidth: .infinity, alignment: .leading)
                Text("Odporúčaný pre SK/EN: qwen2.5:7b")
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
            }
        }
    }

    // MARK: - Memory

    private var memorySection: some View {
        SettingsSection(title: "Pamäť / Naučené preferencie") {
            VStack(alignment: .leading, spacing: 6) {
                if viewModel.isLoadingMemory {
                    HStack {
                        ProgressView().scaleEffect(0.65)
                        Text("Načítavam…").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                } else if viewModel.memoryItems.isEmpty {
                    Text("Žiadne uložené preferencie.")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                } else {
                    ForEach(viewModel.memoryItems) { item in
                        HStack(alignment: .top, spacing: 6) {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(item.text)
                                    .font(.system(size: 11))
                                    .lineLimit(2)
                                HStack(spacing: 4) {
                                    Text(memoryKindLabel(item.kind))
                                        .font(.system(size: 9))
                                        .padding(.horizontal, 5)
                                        .padding(.vertical, 1)
                                        .background(memoryKindColor(item.kind).opacity(0.15))
                                        .foregroundStyle(memoryKindColor(item.kind))
                                        .clipShape(RoundedRectangle(cornerRadius: 3))
                                    Text(item.namespace)
                                        .font(.system(size: 9))
                                        .foregroundStyle(.tertiary)
                                }
                            }
                            Spacer()
                            Button {
                                Task { await viewModel.deleteMemoryItem(id: item.id) }
                            } label: {
                                Image(systemName: "trash")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.secondary)
                            }
                            .buttonStyle(.plain)
                        }
                        .padding(.vertical, 3)
                        Divider()
                    }
                }
                Button {
                    Task { await viewModel.loadMemoryItems() }
                } label: {
                    HStack(spacing: 4) {
                        Image(systemName: "arrow.clockwise").font(.system(size: 10))
                        Text("Obnoviť").font(.system(size: 11))
                    }
                }
                .buttonStyle(.plain)
                .foregroundStyle(Color.accentColor)
                .padding(.top, 4)
            }
        }
    }

    private func memoryKindLabel(_ kind: String) -> String {
        switch kind {
        case "preference":    return "preferencia"
        case "correction":    return "oprava"
        case "sk_glossary":   return "glosár SK"
        case "style_profile": return "štýl"
        case "fact":          return "fakt"
        default:              return kind
        }
    }

    private func memoryKindColor(_ kind: String) -> Color {
        switch kind {
        case "preference":    return .blue
        case "correction":    return .orange
        case "sk_glossary":   return .purple
        case "style_profile": return .green
        default:              return .gray
        }
    }

    // MARK: - Rules

    @State private var rulesYaml: String = ""
    @State private var rulesError: String? = nil
    @State private var rulesSaved: Bool = false
    @State private var isLoadingRules: Bool = false

    private var rulesSection: some View {
        SettingsSection(title: "Pravidlá (rules.yaml)") {
            VStack(alignment: .leading, spacing: 8) {
                Text("Pravidlá určujú, ktoré nástroje vyžadujú schválenie.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                TextEditor(text: $rulesYaml)
                    .font(.system(size: 11, design: .monospaced))
                    .frame(height: 160)
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(Color(nsColor: .separatorColor), lineWidth: 1)
                    )
                    .disabled(isLoadingRules)
                if let err = rulesError {
                    Text("Chyba: \(err)")
                        .font(.system(size: 11))
                        .foregroundStyle(.red)
                }
                HStack {
                    if rulesSaved {
                        Label("Uložené", systemImage: "checkmark.circle.fill")
                            .font(.system(size: 11))
                            .foregroundStyle(.green)
                    }
                    Spacer()
                    Button("Uložiť") {
                        Task { await saveRules() }
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(isLoadingRules || rulesYaml.isEmpty)
                }
            }
            .task { await loadRules() }
        }
    }

    private func loadRules() async {
        isLoadingRules = true
        if let yaml = try? await DaemonClient().rulesYaml() {
            rulesYaml = yaml
        }
        isLoadingRules = false
    }

    private func saveRules() async {
        rulesError = nil
        rulesSaved = false
        do {
            try await DaemonClient().saveRules(yaml: rulesYaml)
            rulesSaved = true
            Task {
                try? await Task.sleep(for: .seconds(3))
                rulesSaved = false
            }
        } catch DaemonError.serverError(let msg) {
            rulesError = msg
        } catch {
            rulesError = error.localizedDescription
        }
    }

    // MARK: - Shortcuts

    private var shortcutsSection: some View {
        SettingsSection(title: "Skratky") {
            VStack(spacing: 4) {
                ShortcutRow(label: "Otvoriť / zatvoriť",  key: "⌥Space")
                ShortcutRow(label: "Odoslať správu",      key: "⌘↩")
                ShortcutRow(label: "Zatvoriť panel",      key: "Esc")
                ShortcutRow(label: "Vymazať konverzáciu", key: "🗑 trash")
            }
        }
    }
}

// MARK: - Helpers

private struct SettingsSection<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
            content
        }
    }
}

private struct StatusRow: View {
    let label: String
    let up: Bool?

    var body: some View {
        HStack(spacing: 8) {
            Text(label).font(.system(size: 12))
            Spacer()
            Group {
                if let up {
                    Circle()
                        .fill(up ? Color.green : Color.red)
                        .frame(width: 7, height: 7)
                    Text(up ? "online" : "offline")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                } else {
                    ProgressView().scaleEffect(0.6)
                }
            }
        }
    }
}

private struct ConnectorRow: View {
    let label: String
    let icon: String
    let accessible: Bool?

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: icon)
                .font(.system(size: 11))
                .frame(width: 14)
                .foregroundStyle(.secondary)
            Text(label).font(.system(size: 12))
            Spacer()
            Group {
                if let accessible {
                    Circle()
                        .fill(accessible ? Color.green : Color.orange)
                        .frame(width: 7, height: 7)
                    Text(accessible ? "dostupné" : "FDA chýba")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                } else {
                    ProgressView().scaleEffect(0.6)
                }
            }
        }
    }
}

private struct ShortcutRow: View {
    let label: String
    let key: String

    var body: some View {
        HStack {
            Text(label).font(.system(size: 12))
            Spacer()
            Text(key)
                .font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Color(nsColor: .controlBackgroundColor))
                .clipShape(RoundedRectangle(cornerRadius: 4))
        }
    }
}
