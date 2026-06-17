import Combine
import ScreenCaptureKit
import SwiftUI

// MARK: - Attachment types

enum ChatAttachmentKind: String, Sendable {
    case image, pdf, text, other
}

struct ChatAttachment: Identifiable, @unchecked Sendable {
    let id: String          // server-assigned UUID
    let filename: String
    let mime: String
    let kind: ChatAttachmentKind
    /// Local URL where the original file lives (for thumbnail generation).
    let localURL: URL
    let sizeBytes: Int
    /// Base-64 encoded thumbnail (JPEG, max 120×120) for image attachments.
    var thumbnail: NSImage? = nil
}

struct ChatMessage: Identifiable, @unchecked Sendable {
    let id = UUID()
    let role: Role
    var content: String
    var attachments: [ChatAttachment] = []
    /// Set when the assistant's response found a specific mail message.
    /// Drives the "Otvoriť mail" animated button.
    var mailRef: DaemonClient.MailRef? = nil
    /// Set when the assistant's response found a local file (Phase 13A).
    var fileRef: DaemonClient.FileRef? = nil
    /// Set when the assistant's response found an Odoo record (Phase 6).
    var odooRef: DaemonClient.OdooRef? = nil
    /// Set when the assistant's response found a WhatsApp chat (Phase 11).
    var whatsappRef: DaemonClient.WhatsappRef? = nil
    var debugTraceId: String? = nil
    var debugPreview: String? = nil
    var debugPromptChars: Int? = nil
    var debugTokenEstimate: Int? = nil
    var debugMessageCount: Int? = nil
    var debugPayload: String? = nil
    var debugSelectedSkills: [String]? = nil
    var debugSelectedMemoryIds: [String]? = nil
    var debugConversationRecallInjected: Bool? = nil
    /// Codex task complexity rating (Phase 8). Set from `task_rating` SSE event.
    var taskRating: (level: String, score: Int, reasons: [String], privacyRisk: String)? = nil

    enum Role { case user, assistant }
}

enum AgentStatus {
    case ready, thinking, error, awaitingApproval

    var color: Color {
        switch self {
        case .ready:            return Color(red: 0.18, green: 0.80, blue: 0.44)
        case .thinking:         return Color(red: 0.20, green: 0.60, blue: 1.00)
        case .error:            return Color(red: 0.95, green: 0.27, blue: 0.27)
        case .awaitingApproval: return Color(red: 1.00, green: 0.78, blue: 0.15)
        }
    }

    var accessibilityLabel: String {
        switch self {
        case .ready:            return "Pripravený"
        case .thinking:         return "Spracováva"
        case .error:            return "Chyba"
        case .awaitingApproval: return "Čaká na schválenie"
        }
    }
}

@MainActor
final class ChatViewModel: ObservableObject {
    @Published var messages: [ChatMessage] = []
    @Published var inputText: String = ""
    @Published var isThinking = false
    @Published var isExpanded = false
    @Published var hasNotch = false
    @Published var showSettings = false
    @Published var showMemory = false
    @Published var showSkills = false
    @Published var showDebug = false
    @Published var debugConversationPayload: String? = nil
    @Published var isLoadingDebug = false
    @Published var memorySearchQuery: String = ""
    @Published var filteredMemoryItems: [MemoryItem] = []
    @Published var memoryKindFilter: String = ""  // "" = all kinds
    @Published var availableModels: [String] = ["qwen2.5:7b"]
    @Published var visionModelAvailable: Bool = false
    /// Set true when an image is attached and the vision model isn't available —
    /// triggers the one-time pull prompt in the UI.
    @Published var showVisionModelAlert: Bool = false
    @Published var daemonHealth: DaemonHealth?
    @Published var isSyncing = false
    @Published var lastSyncResult: String? = nil
    @Published var streamingChunk: Int = 0
    @Published var lastMemorySavedId: String? = nil
    @Published var memoryItems: [MemoryItem] = []
    @Published var isLoadingMemory = false
    @Published var skills: [SkillItem] = []
    @Published var isLoadingSkills = false
    @Published var pendingApprovals: [ApprovalItem] = []
    /// Files queued to send with the next message.
    @Published var pendingAttachments: [ChatAttachment] = []
    /// True while uploading a file to the daemon.
    @Published var isUploadingAttachment = false
    @Published var usageStats: DaemonClient.UsageStats? = nil
    @Published var isLoadingUsage = false
    @Published var isClearingCache = false

    /// Set to true by NotchWindowController before expanding so the pill
    /// animates to its hover state before the chat panel appears.
    @Published var pillHovered = false

    // MARK: - Scroll viewport persistence (Phase 1B)
    /// The id of the message that was topmost-visible when the panel last collapsed.
    /// `nil` means "no saved position" → scroll to bottom on open.
    var savedScrollAnchorId: UUID? = nil
    /// True when the chat was scrolled to (or near) the bottom when last collapsed.
    var savedScrollWasAtBottom: Bool = true

    var agentStatus: AgentStatus {
        if !pendingApprovals.isEmpty { return .awaitingApproval }
        if isThinking { return .thinking }
        if let h = daemonHealth, (!h.daemonUp || !h.ollamaUp) { return .error }
        return .ready
    }

    private var approvalPollTask: Task<Void, Never>?

    @Published var selectedModel: String = UserDefaults.standard.string(forKey: "bagent.model") ?? "qwen2.5:7b" {
        didSet { UserDefaults.standard.set(selectedModel, forKey: "bagent.model") }
    }

    @Published var selectedClassifierModel: String = UserDefaults.standard.string(forKey: "bagent.classifier_model") ?? "qwen3:0.6b" {
        didSet { UserDefaults.standard.set(selectedClassifierModel, forKey: "bagent.classifier_model") }
    }

    // MARK: - Codex (Phase 8)

    /// User-configured path to the `codex` binary. Empty = auto-discover from $PATH.
    @Published var codexBinaryPath: String = UserDefaults.standard.string(forKey: "bagent.codex_path") ?? "" {
        didSet { UserDefaults.standard.set(codexBinaryPath, forKey: "bagent.codex_path") }
    }
    /// Last result from "Testovať Codex" — nil while not tested, true/false after.
    @Published var codexTestResult: String? = nil
    @Published var isTestingCodex: Bool = false

    func testCodex() {
        isTestingCodex = true
        codexTestResult = nil
        Task {
            do {
                let status = try await client.codexStatus()
                await MainActor.run {
                    if status.available {
                        self.codexTestResult = "✓ \(status.version ?? "dostupný")"
                    } else {
                        self.codexTestResult = "✗ \(status.error ?? "nenájdený")"
                    }
                    self.isTestingCodex = false
                }
            } catch {
                await MainActor.run {
                    self.codexTestResult = "✗ \(error.localizedDescription)"
                    self.isTestingCodex = false
                }
            }
        }
    }

    // MARK: - Odoo (Phase 6)

    /// Odoo connection settings — URL, DB, user stored in UserDefaults (not secrets).
    @Published var odooURL:  String = UserDefaults.standard.string(forKey: "bagent.odoo.url")  ?? ""
    @Published var odooDB:   String = UserDefaults.standard.string(forKey: "bagent.odoo.db")   ?? ""
    @Published var odooUser: String = UserDefaults.standard.string(forKey: "bagent.odoo.user") ?? ""
    /// API key is loaded from Keychain; the `@Published` field holds the live session value only.
    @Published var odooAPIKey: String = ""

    @Published var odooTestResult: String? = nil
    @Published var isTestingOdoo: Bool = false

    /// Save creds to Keychain + UserDefaults and authenticate with the daemon.
    func configureOdoo() {
        // Persist non-secret fields in UserDefaults (URL, DB, user).
        UserDefaults.standard.set(odooURL,  forKey: "bagent.odoo.url")
        UserDefaults.standard.set(odooDB,   forKey: "bagent.odoo.db")
        UserDefaults.standard.set(odooUser, forKey: "bagent.odoo.user")
        // API key goes to Keychain only.
        KeychainStore.saveOdoo(url: odooURL, db: odooDB, user: odooUser, apiKey: odooAPIKey)

        isTestingOdoo = true
        odooTestResult = nil
        Task {
            do {
                let result = try await client.odooConfigure(
                    url: odooURL, db: odooDB, user: odooUser, apiKey: odooAPIKey
                )
                await MainActor.run {
                    if result.ok {
                        self.odooTestResult = "✓ Odoo \(result.version ?? "pripojený"), uid=\(result.uid ?? 0)"
                    } else {
                        self.odooTestResult = "✗ \(result.error ?? "chyba autentifikácie")"
                    }
                    self.isTestingOdoo = false
                }
            } catch {
                await MainActor.run {
                    self.odooTestResult = "✗ \(error.localizedDescription)"
                    self.isTestingOdoo = false
                }
            }
        }
    }

    /// Re-push Odoo credentials from Keychain to the daemon (called on each launch).
    func restoreOdooFromKeychain() {
        guard let creds = KeychainStore.loadOdoo() else { return }
        odooURL  = creds.url
        odooDB   = creds.db
        odooUser = creds.user
        odooAPIKey = creds.apiKey
        Task {
            _ = try? await client.odooConfigure(
                url: creds.url, db: creds.db, user: creds.user, apiKey: creds.apiKey
            )
        }
    }

    /// Open an Odoo record in Safari (called by the "Otvoriť v Safari" button).
    func openOdoo(_ ref: DaemonClient.OdooRef) {
        Task {
            try? await client.odooOpen(url: ref.url)
        }
    }

    // MARK: - WhatsApp (Phase 11)

    @Published var whatsappStatus: DaemonClient.WhatsappStatusResult? = nil
    @Published var whatsappQrString: String? = nil
    @Published var isConnectingWhatsapp: Bool = false
    @Published var whatsappStatusMessage: String? = nil

    func connectWhatsapp() {
        isConnectingWhatsapp = true
        whatsappStatusMessage = nil
        Task {
            do {
                try await client.whatsappStart()
                await pollWhatsappStatus()
            } catch {
                await MainActor.run {
                    self.whatsappStatusMessage = "✗ \(error.localizedDescription)"
                    self.isConnectingWhatsapp = false
                }
            }
        }
    }

    func disconnectWhatsapp() {
        Task {
            try? await client.whatsappStop()
            await pollWhatsappStatus()
        }
    }

    func logoutWhatsapp() {
        Task {
            try? await client.whatsappLogout()
            await pollWhatsappStatus()
            await MainActor.run {
                self.whatsappQrString = nil
            }
        }
    }

    func refreshWhatsappQr() {
        Task {
            if let qr = try? await client.whatsappQr() {
                await MainActor.run {
                    self.whatsappQrString = qr.qr
                }
            }
        }
    }

    @MainActor
    private func pollWhatsappStatus() async {
        // Poll up to 120 s (240 × 500 ms) — Puppeteer/Chromium startup can take ~30 s.
        for _ in 0..<240 {
            if let s = try? await client.whatsappStatus() {
                self.whatsappStatus = s
                self.isConnectingWhatsapp = s.status == "starting"
                if s.needs_qr && self.whatsappQrString == nil {
                    refreshWhatsappQr()
                }
                if s.status == "ready" || s.status == "error" || s.status == "disconnected" {
                    break
                }
            }
            try? await Task.sleep(for: .milliseconds(500))
        }
        self.isConnectingWhatsapp = false
    }

    func refreshWhatsappStatus() {
        Task {
            if let s = try? await client.whatsappStatus() {
                await MainActor.run {
                    self.whatsappStatus = s
                    if s.needs_qr { self.refreshWhatsappQr() }
                }
            }
        }
    }

    /// Preferred microphone name. Empty string = "System default" (no swap).
    /// Persisted across launches; pushed to `speech.preferredInputName` on change and at init.
    @Published var selectedMicrophone: String = UserDefaults.standard.string(forKey: "bagent.microphone") ?? "" {
        didSet {
            UserDefaults.standard.set(selectedMicrophone, forKey: "bagent.microphone")
            speech.preferredInputName = selectedMicrophone.isEmpty ? nil : selectedMicrophone
        }
    }

    @Published var chatWindowW: CGFloat = ChatViewModel.savedSize("bagent.chat.w", 400) {
        didSet { UserDefaults.standard.set(Double(chatWindowW), forKey: "bagent.chat.w") }
    }
    @Published var chatWindowH: CGFloat = ChatViewModel.savedSize("bagent.chat.h", 520) {
        didSet { UserDefaults.standard.set(Double(chatWindowH), forKey: "bagent.chat.h") }
    }

    private static func savedSize(_ key: String, _ fallback: CGFloat) -> CGFloat {
        let v = UserDefaults.standard.double(forKey: key)
        return CGFloat(v > 0 ? v : Double(fallback))
    }

    private let client = DaemonClient()
    let permissions = PermissionsManager()

    // MARK: - Voice input
    /// On-device Whisper STT. Shared by the inline mic button and the voice overlay.
    let speech = SpeechController()
    /// True while the inline (chat-input) mic is recording into `inputText`.
    @Published var isVoiceRecording = false
    private var speechCancellables: Set<AnyCancellable> = []
    /// Set by the voice overlay handoff: marks the next turn as voice-initiated so
    /// the hands-free loop re-opens voice mode once the assistant replies.
    var voiceTurnActive = false
    /// Invoked after a voice-initiated turn finishes streaming (re-presents voice).
    var onVoiceTurnComplete: (() -> Void)?
    /// Drives notch expansion into voice mode (no separate panel).
    @Published var isVoiceNotchActive: Bool = false
    /// Brief confirmation message shown in the notch after a silent background action.
    @Published var voiceActionMessage: String? = nil
    /// Called when the daemon executed a background action instead of streaming LLM.
    var onVoiceActionTaken: ((String) -> Void)?

    // Session ID persisted in UserDefaults so it survives app restarts
    private var sessionId: String? {
        get { UserDefaults.standard.string(forKey: "bagent.session_id") }
        set { UserDefaults.standard.set(newValue, forKey: "bagent.session_id") }
    }

    var currentSessionId: String? { sessionId }

    // MARK: - Init

    init() {
        // Push the persisted mic preference into the speech controller so it is
        // available the first time a voice session starts (didSet doesn't fire for
        // inline stored-property initializers).
        let savedMic = UserDefaults.standard.string(forKey: "bagent.microphone") ?? ""
        speech.preferredInputName = savedMic.isEmpty ? nil : savedMic

        // Restore Odoo credentials from Keychain so the in-memory daemon connector is
        // populated without requiring the user to re-enter creds after restart.
        restoreOdooFromKeychain()
    }

    // MARK: - Actions

    func clear() {
        messages = []
        inputText = ""
        isThinking = false
        showSettings = false
        showDebug = false
        pendingAttachments = []
        savedScrollAnchorId = nil
        savedScrollWasAtBottom = true
        // Start a new session on explicit clear
        sessionId = nil
        Task { await startNewSession() }
    }

    func loadModels() async {
        do {
            let fetched = try await client.models()
            if !fetched.isEmpty {
                availableModels = fetched
                if !fetched.contains(selectedModel) {
                    selectedModel = fetched.first ?? "qwen2.5:7b"
                }
                if !fetched.contains(selectedClassifierModel) {
                    selectedClassifierModel = fetched.contains("qwen3:0.6b") ? "qwen3:0.6b" : (fetched.first ?? "qwen2.5:0.5b")
                }
                // Check whether the vision model is installed
                visionModelAvailable = fetched.contains(where: {
                    $0.hasPrefix("qwen2.5vl") || $0.hasPrefix("qwen2.5-vl")
                })
            }
        } catch {}
    }

    func loadUsage() async {
        isLoadingUsage = true
        do {
            usageStats = try await client.usage()
        } catch {}
        isLoadingUsage = false
    }

    func clearMailCache() async {
        isClearingCache = true
        do {
            try await client.clearMailCache()
            await loadUsage()
        } catch {}
        isClearingCache = false
    }

    func syncMail() async {
        guard !isSyncing else { return }
        isSyncing = true
        lastSyncResult = nil
        do {
            let (synced, total) = try await client.syncMail()
            lastSyncResult = "Synchronizované: \(synced) nových, \(total) spolu"
        } catch {
            lastSyncResult = "Chyba: \(error.localizedDescription)"
        }
        isSyncing = false
    }

    func refreshHealth() async {
        daemonHealth = await client.healthStatus()
        permissions.refresh()
        if messages.isEmpty {
            await startFreshSession()
        } else if sessionId == nil {
            await startNewSession()
        }
    }

    func loadMemoryItems() async {
        isLoadingMemory = true
        do {
            memoryItems = try await client.memoryItems()
            applyMemoryFilter()
        } catch {}
        isLoadingMemory = false
    }

    func deleteMemoryItem(id: String) async {
        do {
            try await client.memoryDelete(id: id)
            memoryItems.removeAll { $0.id == id }
            filteredMemoryItems.removeAll { $0.id == id }
        } catch {}
    }

    func searchMemory(query: String) async {
        isLoadingMemory = true
        do {
            if query.trimmingCharacters(in: .whitespaces).isEmpty {
                memoryItems = try await client.memoryItems()
                applyMemoryFilter()
            } else {
                let hits = try await client.memorySearch(query: query,
                                                         namespace: memoryKindFilter.isEmpty ? nil : memoryKindFilter)
                // MemoryHit is flat (id, namespace, kind, text, score) — map to MemoryItem shape
                filteredMemoryItems = hits.map { h in
                    MemoryItem(id: h.id, namespace: h.namespace, kind: h.kind,
                               language: "und", text: h.text, source_ref: nil, created_at: "", use_count: 0,
                               status: nil, source: nil, confidence: nil, importance: nil, sensitivity: nil)
                }
            }
        } catch {}
        isLoadingMemory = false
    }

    func applyMemoryFilter() {
        if memoryKindFilter.isEmpty {
            filteredMemoryItems = memoryItems
        } else {
            filteredMemoryItems = memoryItems.filter { $0.kind == memoryKindFilter }
        }
    }

    func loadSkills() async {
        isLoadingSkills = true
        do {
            skills = try await client.skills()
        } catch {}
        isLoadingSkills = false
    }

    func toggleMemoryPanel() {
        if showMemory {
            showMemory = false
        } else {
            showSettings = false
            showSkills = false
            showDebug = false
            showMemory = true
            Task { await loadMemoryItems() }
        }
    }

    func toggleSkillsPanel() {
        if showSkills {
            showSkills = false
        } else {
            showSettings = false
            showMemory = false
            showDebug = false
            showSkills = true
            Task { await loadSkills() }
        }
    }

    func toggleSettingsPanel() {
        if showSettings {
            showSettings = false
        } else {
            showMemory = false
            showSkills = false
            showDebug = false
            showSettings = true
        }
    }

    func toggleDebugPanel() {
        if showDebug {
            showDebug = false
        } else {
            showSettings = false
            showMemory = false
            showSkills = false
            showDebug = true
            Task { await loadDebugConversation() }
        }
    }

    func loadDebugConversation() async {
        guard let sessionId else {
            debugConversationPayload = "No conversation id yet."
            return
        }
        isLoadingDebug = true
        do {
            debugConversationPayload = try await client.debugConversation(id: sessionId)
        } catch {
            debugConversationPayload = "Chyba: \(error.localizedDescription)"
        }
        isLoadingDebug = false
    }

    func loadDebugTrace(for messageId: UUID) async {
        guard let idx = messages.firstIndex(where: { $0.id == messageId }),
              let traceId = messages[idx].debugTraceId
        else { return }
        let currentPayload = messages[idx].debugPayload ?? ""
        guard currentPayload.isEmpty || currentPayload.contains("trace not found") || currentPayload.hasPrefix("Chyba:") else {
            return
        }
        messages[idx].debugPayload = "Načítavam trace…"
        for attempt in 0..<6 {
            do {
                messages[idx].debugPayload = try await client.debugTrace(id: traceId)
                return
            } catch {
                if attempt == 5 {
                    messages[idx].debugPayload = "Chyba: \(error.localizedDescription)"
                } else {
                    try? await Task.sleep(for: .seconds(1))
                }
            }
        }
    }

    func send() {
        let text = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !pendingAttachments.isEmpty else { return }
        guard !isThinking else { return }
        if messages.isEmpty {
            sessionId = nil
        }
        inputText = ""
        let model = selectedModel
        let sid = sessionId
        let attachments = pendingAttachments
        pendingAttachments = []
        let attachmentIds = attachments.map { $0.id }
        var userMsg = ChatMessage(role: .user, content: text)
        userMsg.attachments = attachments
        messages.append(userMsg)
        isThinking = true

        // Only the turn immediately following a voice handoff loops back to voice.
        let wasVoiceTurn = voiceTurnActive
        voiceTurnActive = false

        Task {
            let assistantMsg = ChatMessage(role: .assistant, content: "")
            messages.append(assistantMsg)
            let idx = messages.count - 1

            do {
                // ── Screen context gate (Phase 7) ─────────────────────────────
                // 1. Cheap local keyword pre-gate (avoids LLM call on every turn)
                // 2. If pre-gate passes → authoritative LLM classifier via /screen/intent
                // 3. Capture according to classifier flags
                var screenCtx: ScreenContextFields? = nil
                if permissions.hasScreenRecording && Self.looksLikeScreenTurn(text) {
                    let intent = await client.screenIntent(message: text)
                    if intent.wants_screen || intent.wants_selection {
                        let raw = await ScreenContextProvider.shared.capture(
                            wantsScreen: intent.wants_screen,
                            wantsOCR: intent.wants_ocr,
                            wantsSelection: intent.wants_selection
                        )
                        screenCtx = ScreenContextFields(
                            imagePNGBase64: raw.imagePNGBase64,
                            ocrText: raw.ocrText,
                            activeApp: raw.activeApp,
                            selectedText: raw.selectedText
                        )
                    }
                }

                let stream = client.chatStream(text: text, sessionId: sid, model: model, attachmentIds: attachmentIds, screenContext: screenCtx)
                var first = true
                var didAutoOpen = false
                for try await event in stream {
                    switch event {
                    case .debugTrace(let trace):
                        messages[idx].debugTraceId = trace.prompt_trace_id
                        messages[idx].debugPreview = trace.preview
                        messages[idx].debugPromptChars = trace.prompt_chars
                        messages[idx].debugTokenEstimate = trace.prompt_token_estimate
                        messages[idx].debugMessageCount = trace.message_count
                        messages[idx].debugSelectedSkills = trace.selected_skill_names
                        messages[idx].debugSelectedMemoryIds = trace.selected_memory_ids
                        messages[idx].debugConversationRecallInjected = trace.conversation_recall_injected
                        if let sid = trace.session_id { sessionId = sid }
                    case .token(let t):
                        if first { isThinking = false; first = false }
                        messages[idx].content += t
                        streamingChunk += 1
                        // Auto-open Mail after the first sentence has appeared in the response.
                        if !didAutoOpen,
                           let ref = messages[idx].mailRef,
                           ref.auto_open {
                            let content = messages[idx].content
                            if content.contains("\n") || content.count > 80 {
                                didAutoOpen = true
                                openMail(ref)
                            }
                        }
                    case .memorySaved(let id):
                        lastMemorySavedId = id
                        Task {
                            try? await Task.sleep(for: .seconds(3))
                            if lastMemorySavedId == id { lastMemorySavedId = nil }
                        }
                    case .approvalRequested(let id, let tool, let desc):
                        let item = ApprovalItem(
                            id: id, toolName: tool, description: desc,
                            expiresAt: "", createdAt: ""
                        )
                        pendingApprovals.append(item)
                    case .toolBlocked:
                        break
                    case .mailAttachments(let refs):
                        let chips = refs.map { ref in
                            ChatAttachment(
                                id: UUID().uuidString,
                                filename: ref.filename,
                                mime: "application/pdf",
                                kind: .pdf,
                                localURL: URL(fileURLWithPath: ref.path),
                                sizeBytes: ref.size
                            )
                        }
                        messages[idx].attachments.append(contentsOf: chips)
                    case .mailFound(let ref):
                        messages[idx].mailRef = ref
                    case .fileFound(let ref):
                        messages[idx].fileRef = ref
                    case .odooFound(let ref):
                        messages[idx].odooRef = ref
                    case .whatsappFound(let ref):
                        messages[idx].whatsappRef = ref
                    case .fileOpened:
                        break // no UI action for now; daemon already opened the file
                    case .actionTaken(let message):
                        isThinking = false
                        voiceActionMessage = message
                        onVoiceActionTaken?(message)
                    case .taskRating(let level, let score, let reasons, let privacyRisk):
                        messages[idx].taskRating = (level: level, score: score, reasons: reasons, privacyRisk: privacyRisk)
                    case .done(let returnedSessionId):
                        if let sid = returnedSessionId { sessionId = sid }
                        if first { isThinking = false }
                        Task { await loadDebugTrace(for: messages[idx].id) }
                        // Hands-free loop: re-open voice mode after a voice-initiated reply.
                        if wasVoiceTurn { onVoiceTurnComplete?() }
                    }
                }
                if first { isThinking = false }
            } catch {
                isThinking = false
                messages[idx].content = "Chyba: \(error.localizedDescription)"
            }
        }
    }

    // MARK: - Voice input

    /// Estimates how long the user needs to read the last assistant response
    /// before voice re-entry. Based on ≈150 WPM, clamped 1.5–7 s.
    func voiceTurnResumeDelay() -> TimeInterval {
        let text = messages.last(where: { $0.role == .assistant })?.content ?? ""
        let wordCount = text.split(separator: " ").count
        let reading = Double(wordCount) / 150.0 * 60.0
        return max(1.5, min(7.0, reading))
    }

    /// Feed a finalized voice transcript through the normal chat pipeline.
    func submitTranscript(_ text: String) {
        let t = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        inputText = t
        send()
    }

    /// Inline mic: toggle recording into the text field (does not open the overlay).
    func toggleInlineVoice() {
        if speech.isRunning {
            speech.finalize()
        } else {
            startInlineVoice()
        }
    }

    private func startInlineVoice() {
        speechCancellables.removeAll()
        isVoiceRecording = true

        // Final transcript lands in the field, editable like typed text.
        speech.onFinalTranscript = { [weak self] text in
            guard let self else { return }
            self.inputText = text
            self.isVoiceRecording = false
        }

        // Live-fill the field with the running transcript.
        speech.$partialText
            .receive(on: RunLoop.main)
            .sink { [weak self] text in
                guard let self, self.isVoiceRecording, !text.isEmpty else { return }
                self.inputText = text
            }
            .store(in: &speechCancellables)

        // Clear the recording flag when the session ends or errors.
        speech.$state
            .receive(on: RunLoop.main)
            .sink { [weak self] st in
                guard let self else { return }
                switch st {
                case .done, .idle, .error: self.isVoiceRecording = false
                default: break
                }
            }
            .store(in: &speechCancellables)

        Task { await speech.startSession(mode: .inline) }
    }

    func cancelInlineVoice() {
        speech.cancel()
        isVoiceRecording = false
        speechCancellables.removeAll()
    }

    // MARK: - Approvals

    func startApprovalPolling() {
        approvalPollTask?.cancel()
        approvalPollTask = Task {
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                guard !Task.isCancelled else { break }
                if let items = try? await client.pendingApprovals() {
                    pendingApprovals = items
                }
            }
        }
    }

    func stopApprovalPolling() {
        approvalPollTask?.cancel()
        approvalPollTask = nil
    }

    func decideApproval(_ item: ApprovalItem, allow: Bool) {
        pendingApprovals.removeAll { $0.id == item.id }
        Task {
            try? await client.decideApproval(id: item.id, allow: allow)
        }
    }

    // MARK: - Attachments

    func addAttachments(urls: [URL]) {
        guard !urls.isEmpty else { return }
        // Cap at 5 total
        let remaining = max(0, 5 - pendingAttachments.count)
        let toAdd = Array(urls.prefix(remaining))
        guard !toAdd.isEmpty else { return }
        isUploadingAttachment = true
        Task {
            var added: [ChatAttachment] = []
            for url in toAdd {
                do {
                    let att = try await client.uploadAttachment(url: url)
                    added.append(att)
                    // One-time vision model alert
                    if att.kind == .image && !visionModelAvailable {
                        showVisionModelAlert = true
                    }
                } catch {
                    // silently skip failed uploads
                }
            }
            pendingAttachments.append(contentsOf: added)
            isUploadingAttachment = false
        }
    }

    func removeAttachment(id: String) {
        pendingAttachments.removeAll { $0.id == id }
    }

    // MARK: - Image paste (Part B — Phase 7)

    /// Tries to read an image from the general pasteboard.
    /// Returns true and inserts `[image #n]` into `inputText` when an image is found.
    @discardableResult
    func pasteImageFromClipboard() -> Bool {
        let pb = NSPasteboard.general
        guard let image = NSImage(pasteboard: pb) else { return false }

        // Count how many images have been pasted this compose session
        let n = pendingAttachments.filter { $0.kind == .image }.count + 1

        // Write to a temp PNG file — uploadAttachment requires a file URL
        guard let cgImage = image.cgImage(forProposedRect: nil, context: nil, hints: nil),
              let pngData = NSBitmapImageRep(cgImage: cgImage)
                                .representation(using: .png, properties: [:]) else { return false }

        let tmpURL = FileManager.default.temporaryDirectory
            .appendingPathComponent("paste_\(UUID().uuidString).png")
        do {
            try pngData.write(to: tmpURL)
        } catch {
            return false
        }

        addAttachments(urls: [tmpURL])
        inputText += inputText.isEmpty ? "[image #\(n)]" : " [image #\(n)]"
        return true
    }

    // MARK: - Mail open (Phase 5E)

    func openMail(_ ref: DaemonClient.MailRef) {
        Task {
            try? await client.openMail(
                rowid: ref.rowid,
                messageId: ref.message_id,
                subject: ref.subject,
                sender: ref.sender
            )
        }
    }

    // MARK: - Screen context (Phase 7)

    /// Cheap local pre-gate: returns true when the message contains keywords that
    /// suggest the user wants the agent to look at the screen. Avoids a daemon round-
    /// trip on every turn. The authoritative check is the LLM classifier via /screen/intent.
    static func looksLikeScreenTurn(_ message: String) -> Bool {
        let low = message.lowercased()
        let keywords = [
            // Slovak
            "obrazovk", "vidíš", "čo vidíš", "pozri na", "pozri sem",
            "analyzuj toto", "analyzuj to", "prečítaj toto", "prečítaj to",
            "čo tam píše", "čo sa zobrazuje", "nájdi na obrazovke",
            "prečítaj výber", "vybraný text", "tento výber",
            // English
            "what's on screen", "what's on my screen", "what is on screen",
            "what can you see", "on the screen",
            "look at my screen", "look at the screen", "what do you see",
            "analyze this", "analyse this", "read this", "read the screen",
            "what does it say", "what does this say", "find on screen",
            "find the button", "locate on screen", "read selection", "selected text",
        ]
        return keywords.contains { low.contains($0) }
    }

    // MARK: - Vision model check

    func isVisionModelAvailable() async -> Bool {
        let models = (try? await client.models()) ?? []
        return models.contains(where: { $0.hasPrefix("qwen2.5vl") || $0.hasPrefix("qwen2.5-vl") })
    }

    // MARK: - Private

    private func startFreshSession() async {
        sessionId = nil
        await startNewSession()
    }

    private func startNewSession() async {
        guard sessionId == nil else { return }
        do {
            sessionId = try await client.createSession()
        } catch {}
    }
}
