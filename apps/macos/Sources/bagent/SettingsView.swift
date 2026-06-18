import SwiftUI
import AppKit

struct SettingsView: View {
    @ObservedObject var viewModel: ChatViewModel
    @ObservedObject private var permissions: PermissionsManager
    @ObservedObject private var speech: SpeechController

    init(viewModel: ChatViewModel) {
        self.viewModel = viewModel
        self.permissions = viewModel.permissions
        self.speech = viewModel.speech
    }

    @State private var availableMics: [String] = []
    @State private var selectedCategory: SettingsCategory = .overview
    @State private var showWhatsappDiagnostics = false

    var body: some View {
        VStack(spacing: 0) {
            SettingsCategoryBar(selection: $selectedCategory)
                .padding(.horizontal, 12)
                .padding(.top, 12)
                .padding(.bottom, 8)
            Divider()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    switch selectedCategory {
                    case .overview:
                        statusSection
                        usageSection
                    case .access:
                        permissionsSection
                    case .connectors:
                        connectorsSection
                        whatsappSection
                        odooSection
                        codexSection
                    case .ai:
                        ollamaSection
                    case .advanced:
                        rulesSection
                        shortcutsSection
                    }
                }
                .padding(16)
            }
        }
        .task {
            availableMics = SpeechController.availableInputDeviceNames()
            await viewModel.refreshHealth()
            await viewModel.loadModels()
            await viewModel.loadUsage()
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
                    Text("Potrebné pre prístup k Mail, Poznámkam a vyhľadávanie lokálnych súborov (Dokumenty, Plocha, Stiahnuté, iCloud Drive).")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                Divider().padding(.vertical, 2)

                HStack(spacing: 8) {
                    Circle()
                        .fill(permissions.hasMicrophoneAccess ? Color.green : Color.orange)
                        .frame(width: 7, height: 7)
                    Image(systemName: "mic.fill").font(.system(size: 10)).foregroundStyle(.secondary)
                    Text("Mikrofón")
                        .font(.system(size: 12))
                    Spacer()
                    if !permissions.hasMicrophoneAccess {
                        Button("Udeliť") {
                            Task {
                                await permissions.requestMicrophoneAccess()
                                if !permissions.hasMicrophoneAccess { permissions.openMicrophoneSettings() }
                            }
                        }
                        .font(.system(size: 11))
                        .buttonStyle(.plain)
                        .foregroundStyle(Color.accentColor)
                    } else {
                        Text("aktívne").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                if !permissions.hasMicrophoneAccess {
                    Text("Potrebné pre hlasový vstup (Whisper, lokálne na zariadení).")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                Divider().padding(.vertical, 2)

                HStack(spacing: 8) {
                    Image(systemName: viewModel.voiceModeEnabled ? "waveform.circle.fill" : "waveform.circle")
                        .font(.system(size: 13))
                        .foregroundStyle(viewModel.voiceModeEnabled ? Color.accentColor : Color.secondary)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Hlasový režim")
                            .font(.system(size: 12))
                        Text(viewModel.voiceModeEnabled ? "⌥Space otvorí hlasový vstup." : "⌥Space otvorí normálny chat. Whisper sa nenačíta.")
                            .font(.system(size: 10))
                            .foregroundStyle(.tertiary)
                    }
                    Spacer()
                    Toggle("", isOn: $viewModel.voiceModeEnabled)
                        .labelsHidden()
                        .toggleStyle(.switch)
                        .accessibilityLabel("Hlasový režim")
                }

                Divider().padding(.vertical, 2)

                // Screen Recording (Phase 7)
                HStack(spacing: 8) {
                    Circle()
                        .fill(permissions.hasScreenRecording ? Color.green : Color.orange)
                        .frame(width: 7, height: 7)
                    Image(systemName: "rectangle.dashed").font(.system(size: 10)).foregroundStyle(.secondary)
                    Text("Snímanie obrazovky")
                        .font(.system(size: 12))
                    Spacer()
                    if !permissions.hasScreenRecording {
                        Button("Udeliť") {
                            permissions.requestScreenRecording()
                            if !permissions.hasScreenRecording { permissions.openScreenRecordingSettings() }
                        }
                        .font(.system(size: 11))
                        .buttonStyle(.plain)
                        .foregroundStyle(Color.accentColor)
                    } else {
                        Text("aktívne").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                if !permissions.hasScreenRecording {
                    Text("Potrebné pre analýzu obrazovky. Snímky sa nikdy neukladajú na disk.")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                Divider().padding(.vertical, 2)

                // Accessibility (Phase 7 — selected text)
                HStack(spacing: 8) {
                    Circle()
                        .fill(permissions.hasAccessibility ? Color.green : Color.orange)
                        .frame(width: 7, height: 7)
                    Image(systemName: "accessibility").font(.system(size: 10)).foregroundStyle(.secondary)
                    Text("Accessibility")
                        .font(.system(size: 12))
                    Spacer()
                    if !permissions.hasAccessibility {
                        Button("Udeliť") {
                            permissions.requestAccessibility()
                            if !permissions.hasAccessibility { permissions.openAccessibilitySettings() }
                        }
                        .font(.system(size: 11))
                        .buttonStyle(.plain)
                        .foregroundStyle(Color.accentColor)
                    } else {
                        Text("aktívne").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                }
                if !permissions.hasAccessibility {
                    Text("Potrebné pre čítanie vybraného textu. Heslové polia sú vždy vynechané.")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }

                HStack(spacing: 8) {
                    if speech.state == .loadingModel {
                        ProgressView().scaleEffect(0.5).frame(width: 7, height: 7)
                    } else {
                        Circle()
                            .fill(speech.isModelLoaded ? Color.green : Color.gray)
                            .frame(width: 7, height: 7)
                    }
                    Image(systemName: "waveform").font(.system(size: 10)).foregroundStyle(.secondary)
                    Text("Whisper model")
                        .font(.system(size: 12))
                    Spacer()
                    Text(!viewModel.voiceModeEnabled ? "vypnutý"
                         : (speech.state == .loadingModel ? "sťahuje sa…"
                            : (speech.isModelLoaded ? "pripravený" : "stiahne sa pri prvom použití")))
                        .font(.system(size: 11)).foregroundStyle(.secondary)
                }
                .opacity(viewModel.voiceModeEnabled ? 1 : 0.55)

                Divider().padding(.vertical, 2)

                Text("Mikrofón").font(.system(size: 11)).foregroundStyle(.secondary)
                Picker("", selection: $viewModel.selectedMicrophone) {
                    Text("Predvolený systémom").tag("")
                    ForEach(availableMics, id: \.self) { name in
                        Text(name).tag(name)
                    }
                }
                .labelsHidden()
                .frame(maxWidth: .infinity, alignment: .leading)
                .disabled(!viewModel.voiceModeEnabled)
                .opacity(viewModel.voiceModeEnabled ? 1 : 0.55)
                Text("Ak vybraný mikrofón nie je dostupný, použije sa predvolený.")
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
                    .frame(maxWidth: .infinity, alignment: .leading)
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
                        Text("Model chat").font(.system(size: 12)).foregroundStyle(.secondary)
                        Spacer()
                        Text(model).font(.system(size: 11)).foregroundStyle(.tertiary)
                    }
                }
                if let cm = viewModel.daemonHealth?.classifierModel, cm != "—" {
                    HStack {
                        Text("Model klasifikátor").font(.system(size: 12)).foregroundStyle(.secondary)
                        Spacer()
                        Text(cm).font(.system(size: 11)).foregroundStyle(.tertiary)
                    }
                }
                HStack {
                    Spacer()
                    Button {
                        Task { await viewModel.refreshHealth() }
                    } label: {
                        Label("Obnoviť stav", systemImage: "arrow.clockwise")
                    }
                    .font(.system(size: 11))
                    .buttonStyle(.plain)
                    .foregroundStyle(Color.accentColor)
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

                Text("Model klasifikátora").font(.system(size: 11)).foregroundStyle(.secondary)
                Picker("", selection: $viewModel.selectedClassifierModel) {
                    ForEach(viewModel.availableModels, id: \.self) { m in
                        Text(m).tag(m)
                    }
                }
                .labelsHidden()
                .frame(maxWidth: .infinity, alignment: .leading)
                Text("Odporúčaný pre intent: qwen3:0.6b. Zmena sa použije po reštarte daemona.")
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)

                Divider()

                // Vision model status
                HStack(spacing: 6) {
                    Circle()
                        .fill(viewModel.visionModelAvailable ? Color.green : Color.orange)
                        .frame(width: 7, height: 7)
                    Text("Vision model (qwen2.5vl:7b)")
                        .font(.system(size: 12))
                    Spacer()
                    Text(viewModel.visionModelAvailable ? "nainštalovaný" : "chýba")
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                }
                if !viewModel.visionModelAvailable {
                    Text("Pre prikladanie obrázkov spusti: ollama pull qwen2.5vl:7b")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .textSelection(.enabled)
                }
            }
        }
    }

    // MARK: - Rules

    @State private var rulesYaml: String = ""
    @State private var rulesError: String? = nil
    @State private var rulesSaved: Bool = false
    @State private var isLoadingRules: Bool = false

    // MARK: - Codex State (Phase 8)
    @State private var codexStatusMessage: String? = nil

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

    // MARK: - Codex (Phase 8)

    private var codexSection: some View {
        SettingsSection(title: "Codex (pokročilé úlohy)") {
            VStack(alignment: .leading, spacing: 8) {
                Text("Codex slúži ako externý harness pre zložité cross-source úlohy. Spúšťa sa iba po schválení kontextového paketu.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)

                // Binary path
                HStack(spacing: 6) {
                    Text("Cesta k binárke")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .frame(width: 110, alignment: .leading)
                    TextField("(automaticky z $PATH)", text: $viewModel.codexBinaryPath)
                        .font(.system(size: 12, design: .monospaced))
                        .textFieldStyle(.roundedBorder)
                }

                // Status / test button row
                HStack(spacing: 8) {
                    if viewModel.isTestingCodex {
                        ProgressView().scaleEffect(0.65)
                        Text("Testujem…")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    } else if let msg = viewModel.codexTestResult {
                        Image(systemName: msg.hasPrefix("✓") ? "checkmark.circle.fill" : "xmark.circle.fill")
                            .foregroundStyle(msg.hasPrefix("✓") ? Color.green : Color.red)
                            .font(.system(size: 11))
                        Text(msg)
                            .font(.system(size: 11))
                            .foregroundStyle(msg.hasPrefix("✓") ? Color.primary : Color.red)
                            .lineLimit(1)
                    } else {
                        ConnectorRow(
                            label: "Codex",
                            icon: "cpu",
                            accessible: viewModel.daemonHealth?.codexConnector
                        )
                    }
                    Spacer()
                    Button("Testovať Codex") {
                        viewModel.testCodex()
                    }
                    .buttonStyle(.bordered)
                    .disabled(viewModel.isTestingCodex)
                }

                if viewModel.codexTestResult != nil {
                    Button("Vymazať výsledok") {
                        viewModel.codexTestResult = nil
                    }
                    .font(.system(size: 11))
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                }

                Divider()
                Text("Codex nikdy nezíska prístup k súborom, mailov, databázam ani heslám priamo — dostáva iba schválený kontextový paket od daemona.")
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
            }
        }
    }

    // MARK: - Odoo (Phase 6)

    private var odooSection: some View {
        SettingsSection(title: "Odoo (CRM / Helpdesk)") {
            VStack(alignment: .leading, spacing: 8) {
                Text("Pripojenie k Odoo 18 — len na čítanie (tikety, faktúry, partneri). Credentials sa ukladajú do Keychain.")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)

                // URL field
                HStack(spacing: 6) {
                    Text("URL")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .frame(width: 80, alignment: .leading)
                    TextField("https://mycompany.odoo.com", text: $viewModel.odooURL)
                        .font(.system(size: 12, design: .monospaced))
                        .textFieldStyle(.roundedBorder)
                }

                // Database
                HStack(spacing: 6) {
                    Text("Databáza")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .frame(width: 80, alignment: .leading)
                    TextField("mycompany", text: $viewModel.odooDB)
                        .font(.system(size: 12, design: .monospaced))
                        .textFieldStyle(.roundedBorder)
                }

                // Username
                HStack(spacing: 6) {
                    Text("Používateľ")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .frame(width: 80, alignment: .leading)
                    TextField("user@example.com", text: $viewModel.odooUser)
                        .font(.system(size: 12, design: .monospaced))
                        .textFieldStyle(.roundedBorder)
                }

                // API Key
                HStack(spacing: 6) {
                    Text("API kľúč")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .frame(width: 80, alignment: .leading)
                    SecureField("(z Odoo → Nastavenia → API Keys)", text: $viewModel.odooAPIKey)
                        .font(.system(size: 12, design: .monospaced))
                        .textFieldStyle(.roundedBorder)
                    Button {
                        viewModel.loadSavedOdooAPIKey()
                    } label: {
                        Image(systemName: "key")
                            .font(.system(size: 12))
                    }
                    .buttonStyle(.plain)
                    .help("Načítať uložený API kľúč z Keychain")
                }

                // Status / test button row
                HStack(spacing: 8) {
                    if viewModel.isTestingOdoo {
                        ProgressView().scaleEffect(0.65)
                        Text("Testujem…")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    } else if let msg = viewModel.odooTestResult {
                        Image(systemName: msg.hasPrefix("✓") ? "checkmark.circle.fill" : "xmark.circle.fill")
                            .foregroundStyle(msg.hasPrefix("✓") ? Color.green : Color.red)
                            .font(.system(size: 11))
                        Text(msg)
                            .font(.system(size: 11))
                            .foregroundStyle(msg.hasPrefix("✓") ? Color.primary : Color.red)
                            .lineLimit(1)
                    } else {
                        ConnectorRow(
                            label: "Odoo",
                            icon: "building.2",
                            accessible: viewModel.daemonHealth?.odooConnector
                        )
                    }
                    Spacer()
                    Button("Testovať Odoo") {
                        viewModel.configureOdoo()
                    }
                    .buttonStyle(.bordered)
                    .disabled(viewModel.isTestingOdoo
                              || viewModel.odooURL.isEmpty
                              || viewModel.odooDB.isEmpty
                              || viewModel.odooAPIKey.isEmpty)
                }

                if viewModel.odooTestResult != nil {
                    Button("Vymazať výsledok") {
                        viewModel.odooTestResult = nil
                    }
                    .font(.system(size: 11))
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                }

                Divider()
                Text("API kľúč sa nikdy nezapíše na disk — uchováva sa iba v Keychain a v pamäti daemona.")
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
            }
        }
    }

    // MARK: - WhatsApp (Phase 11)

    private var whatsappSection: some View {
        SettingsSection(title: "WhatsApp") {
            VStack(alignment: .leading, spacing: 10) {

                // Warning box
                HStack(alignment: .top, spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(.orange)
                        .font(.system(size: 11))
                    Text("Používa neoficiálny WhatsApp Web bridge (whatsapp-web.js). Odoslanie správy vždy vyžaduje tvoje schválenie. Nikdy sa neposielajú hromadné správy, média ani automatické odpovede.")
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(8)
                .background(Color.orange.opacity(0.08))
                .cornerRadius(6)

                // Status row
                HStack(spacing: 8) {
                    let st = viewModel.whatsappStatus?.status ?? "stopped"
                    Circle()
                        .fill(whatsappStatusColor(st))
                        .frame(width: 8, height: 8)
                    Text(whatsappStatusLabel(st))
                        .font(.system(size: 12))
                    if let me = viewModel.whatsappStatus?.me_name {
                        Text("(\(me))")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Obnoviť") { viewModel.refreshWhatsappStatus() }
                        .buttonStyle(.plain)
                        .font(.system(size: 11))
                        .foregroundStyle(Color.accentColor)
                }

                if viewModel.whatsappStatus?.needs_qr == true {
                    HStack(spacing: 8) {
                        Image(systemName: "qrcode.viewfinder")
                            .foregroundStyle(.secondary)
                        Text("QR párovanie je pripravené na samostatnej obrazovke.")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                        Spacer()
                        Button("Otvoriť QR") {
                            withAnimation(.spring(response: 0.28, dampingFraction: 0.78)) {
                                viewModel.showSettings = false
                                viewModel.showWhatsappPairing = true
                            }
                            viewModel.refreshWhatsappQr()
                        }
                        .buttonStyle(.bordered)
                        .font(.system(size: 11))
                    }
                }

                // Action buttons
                HStack(spacing: 8) {
                    if viewModel.isConnectingWhatsapp {
                        ProgressView().scaleEffect(0.65)
                        Text("Spúšťam bridge…")
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    } else {
                        let st = viewModel.whatsappStatus?.status ?? "stopped"
                        if st == "stopped" || st == "error" || st == "disconnected" || st == "missing_node" || st == "bridge_not_installed" {
                            Button("Pripojiť WhatsApp") { viewModel.connectWhatsapp() }
                                .buttonStyle(.borderedProminent)
                                .font(.system(size: 12))
                        } else {
                            Button("Odpojiť") { viewModel.disconnectWhatsapp() }
                                .buttonStyle(.bordered)
                                .font(.system(size: 12))
                        }
                        Button("Odhlásiť a vymazať reláciu") { viewModel.logoutWhatsapp() }
                            .buttonStyle(.plain)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                    }
                    Spacer()
                }

                if let msg = viewModel.whatsappStatusMessage {
                    Text(msg)
                        .font(.system(size: 11))
                        .foregroundStyle(msg.hasPrefix("✓") ? Color.primary : Color.red)
                }

                Button {
                    withAnimation(.easeInOut(duration: 0.16)) {
                        showWhatsappDiagnostics.toggle()
                    }
                    if showWhatsappDiagnostics {
                        Task { await viewModel.loadWhatsappDebug() }
                    }
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: showWhatsappDiagnostics ? "chevron.down" : "chevron.right")
                            .font(.system(size: 9, weight: .semibold))
                            .frame(width: 12)
                        Text("Diagnostika")
                            .font(.system(size: 11, weight: .medium))
                        Spacer()
                        if viewModel.isLoadingWhatsappDebug {
                            ProgressView().scaleEffect(0.55)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                if showWhatsappDiagnostics {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 8) {
                            Button {
                                Task { await viewModel.loadWhatsappDebug() }
                            } label: {
                                Label("Obnoviť", systemImage: "arrow.clockwise")
                            }
                            .buttonStyle(.plain)
                            Button {
                                copySettingsText(viewModel.whatsappDebugPayload ?? "")
                            } label: {
                                Label("Kopírovať JSON", systemImage: "doc.on.doc")
                            }
                            .buttonStyle(.plain)
                            .disabled((viewModel.whatsappDebugPayload ?? "").isEmpty)
                        }
                        .font(.system(size: 10))
                        .foregroundStyle(Color.accentColor)

                        ScrollView {
                            Text(viewModel.whatsappDebugPayload ?? "Žiadne diagnostické dáta.")
                                .font(.system(size: 10, design: .monospaced))
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(8)
                        }
                        .frame(maxHeight: 120)
                        .background(Color.black.opacity(0.06))
                        .clipShape(RoundedRectangle(cornerRadius: 6))
                    }
                    .transition(.opacity.combined(with: .move(edge: .top)))
                }

                Divider()
                Text("Vyžaduje Node.js ≥18. Spusti `make whatsapp-bridge-install` pred prvým pripojením. Token ani session path sa nikdy nezobrazujú.")
                    .font(.system(size: 10))
                    .foregroundStyle(.tertiary)
            }
        }
        .task { viewModel.refreshWhatsappStatus() }
    }

    private func whatsappStatusColor(_ status: String) -> Color {
        switch status {
        case "ready":          return .green
        case "qr", "starting", "authenticated", "authenticated_waiting_for_ready": return .yellow
        case "disconnected", "error": return .red
        default:               return .gray
        }
    }

    private func whatsappStatusLabel(_ status: String) -> String {
        switch status {
        case "stopped":             return "Nezačaté"
        case "starting":            return "Spúšťam…"
        case "qr":                  return "Čakám na QR"
        case "authenticated":       return "Autentifikovaný"
        case "authenticated_waiting_for_ready": return "Čakám na načítanie"
        case "ready":               return "Pripojený"
        case "disconnected":        return "Odpojený"
        case "error":               return "Chyba"
        case "missing_node":        return "Node.js nenájdený"
        case "bridge_not_installed": return "Bridge neinštalovaný"
        default:                    return status
        }
    }

    // MARK: - Shortcuts

    private var shortcutsSection: some View {
        SettingsSection(title: "Skratky") {
            VStack(spacing: 4) {
                ShortcutRow(label: "Otvoriť / zatvoriť",  key: "⌥Space")
                ShortcutRow(label: "Odoslať správu",      key: "⌘↩")
                ShortcutRow(label: "Zatvoriť panel",      key: "Esc")
                ShortcutRow(label: "Vymazať konverzáciu", key: "⌘⌫")
            }
        }
    }

    // MARK: - Disk Usage

    private var usageSection: some View {
        SettingsSection(title: "Využitie disku") {
            VStack(alignment: .leading, spacing: 6) {
                if viewModel.isLoadingUsage {
                    HStack {
                        ProgressView().scaleEffect(0.65)
                        Text("Načítavam…").font(.system(size: 11)).foregroundStyle(.secondary)
                    }
                } else if let stats = viewModel.usageStats {
                    UsageRow(label: "Databáza",         value: stats.dbFormatted)
                    UsageRow(label: "Prílohy",          value: stats.attachmentsFormatted)
                    UsageRow(label: "Položky pamäti",   value: "\(stats.memory_items_count)")
                    UsageRow(label: "Správy konverzácií", value: "\(stats.chat_turns_count)")
                    UsageRow(label: "Emaily v cache",   value: "\(stats.mail_cache_count)")
                    UsageRow(label: "Embeddingy",       value: "\(stats.embeddings_count)")
                    Divider()
                    HStack {
                        Text("Celkovo")
                            .font(.system(size: 11, weight: .semibold))
                        Spacer()
                        Text(stats.totalFormatted)
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.primary)
                    }
                } else {
                    Text("Nedostupné")
                        .font(.system(size: 11))
                        .foregroundStyle(.tertiary)
                }

                HStack(spacing: 8) {
                    Button {
                        Task { await viewModel.loadUsage() }
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: "arrow.clockwise").font(.system(size: 10))
                            Text("Obnoviť").font(.system(size: 11))
                        }
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(Color.accentColor)

                    Button {
                        Task { await viewModel.clearMailCache() }
                    } label: {
                        HStack(spacing: 4) {
                            if viewModel.isClearingCache {
                                ProgressView().scaleEffect(0.55)
                            } else {
                                Image(systemName: "trash").font(.system(size: 10))
                            }
                            Text("Vymazať starú mail cache").font(.system(size: 11))
                        }
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
                    .disabled(viewModel.isClearingCache)
                }
                .padding(.top, 4)
            }
        }
    }
}

// MARK: - Helpers

private func copySettingsText(_ text: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
    NotificationCenter.default.post(name: .bagentCodeCopied, object: nil)
}

private enum SettingsCategory: String, CaseIterable, Identifiable {
    case overview = "Overview"
    case access = "Access"
    case connectors = "Connectors"
    case ai = "AI"
    case advanced = "Advanced"

    var id: String { rawValue }

    var icon: String {
        switch self {
        case .overview: return "gauge"
        case .access: return "lock.shield"
        case .connectors: return "point.3.connected.trianglepath.dotted"
        case .ai: return "cpu"
        case .advanced: return "slider.horizontal.3"
        }
    }
}

private struct SettingsCategoryBar: View {
    @Binding var selection: SettingsCategory

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                ForEach(SettingsCategory.allCases) { category in
                    Button {
                        withAnimation(.easeInOut(duration: 0.16)) {
                            selection = category
                        }
                    } label: {
                        HStack(spacing: 5) {
                            Image(systemName: category.icon)
                                .font(.system(size: 10, weight: .semibold))
                            Text(category.rawValue)
                                .font(.system(size: 11, weight: .medium))
                                .lineLimit(1)
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 6)
                        .frame(minHeight: 28)
                        .foregroundStyle(selection == category ? Color.primary : Color.secondary)
                        .background(selection == category ? Color.secondary.opacity(0.16) : Color.clear)
                        .clipShape(RoundedRectangle(cornerRadius: 7))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help(category.rawValue)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct UsageRow: View {
    let label: String
    let value: String

    var body: some View {
        HStack {
            Text(label)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
            Spacer()
            Text(value)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.primary)
        }
    }
}

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
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color.secondary.opacity(0.07))
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(Color.secondary.opacity(0.10), lineWidth: 1)
        )
        .clipShape(RoundedRectangle(cornerRadius: 8))
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
