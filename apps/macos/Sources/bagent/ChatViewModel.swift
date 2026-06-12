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

    // Session ID persisted in UserDefaults so it survives app restarts
    private var sessionId: String? {
        get { UserDefaults.standard.string(forKey: "bagent.session_id") }
        set { UserDefaults.standard.set(newValue, forKey: "bagent.session_id") }
    }

    // MARK: - Actions

    func clear() {
        messages = []
        inputText = ""
        isThinking = false
        showSettings = false
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
        if sessionId == nil { await startNewSession() }
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
                               language: "und", text: h.text, source_ref: nil, created_at: "", use_count: 0)
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

    func toggleMemoryPanel() {
        if showMemory {
            showMemory = false
        } else {
            showSettings = false
            showMemory = true
            Task { await loadMemoryItems() }
        }
    }

    func toggleSettingsPanel() {
        if showSettings {
            showSettings = false
        } else {
            showMemory = false
            showSettings = true
        }
    }

    func send() {
        let text = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !pendingAttachments.isEmpty else { return }
        guard !isThinking else { return }
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

        Task {
            let assistantMsg = ChatMessage(role: .assistant, content: "")
            messages.append(assistantMsg)
            let idx = messages.count - 1

            do {
                let stream = client.chatStream(text: text, sessionId: sid, model: model, attachmentIds: attachmentIds)
                var first = true
                var didAutoOpen = false
                for try await event in stream {
                    switch event {
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
                    case .done(let returnedSessionId):
                        if let sid = returnedSessionId { sessionId = sid }
                        if first { isThinking = false }
                    }
                }
                if first { isThinking = false }
            } catch {
                isThinking = false
                messages[idx].content = "Chyba: \(error.localizedDescription)"
            }
        }
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

    // MARK: - Vision model check

    func isVisionModelAvailable() async -> Bool {
        let models = (try? await client.models()) ?? []
        return models.contains(where: { $0.hasPrefix("qwen2.5vl") || $0.hasPrefix("qwen2.5-vl") })
    }

    // MARK: - Private

    private func startNewSession() async {
        guard sessionId == nil else { return }
        do {
            sessionId = try await client.createSession()
        } catch {}
    }
}
