import SwiftUI

struct ChatMessage: Identifiable, @unchecked Sendable {
    let id = UUID()
    let role: Role
    var content: String

    enum Role { case user, assistant }
}

@MainActor
final class ChatViewModel: ObservableObject {
    @Published var messages: [ChatMessage] = []
    @Published var inputText: String = ""
    @Published var isThinking = false
    @Published var isExpanded = false
    @Published var hasNotch = false
    @Published var showSettings = false
    @Published var availableModels: [String] = ["qwen2.5:7b"]
    @Published var daemonHealth: DaemonHealth?
    @Published var isSyncing = false
    @Published var lastSyncResult: String? = nil
    @Published var streamingChunk: Int = 0
    @Published var lastMemorySavedId: String? = nil
    @Published var memoryItems: [MemoryItem] = []
    @Published var isLoadingMemory = false
    @Published var pendingApprovals: [ApprovalItem] = []

    private var approvalPollTask: Task<Void, Never>?

    @Published var selectedModel: String = UserDefaults.standard.string(forKey: "bagent.model") ?? "qwen2.5:7b" {
        didSet { UserDefaults.standard.set(selectedModel, forKey: "bagent.model") }
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
            }
        } catch {}
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
        } catch {}
        isLoadingMemory = false
    }

    func deleteMemoryItem(id: String) async {
        do {
            try await client.memoryDelete(id: id)
            memoryItems.removeAll { $0.id == id }
        } catch {}
    }

    func send() {
        let text = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !isThinking else { return }
        inputText = ""
        let model = selectedModel
        let sid = sessionId
        messages.append(ChatMessage(role: .user, content: text))
        isThinking = true

        Task {
            let assistantMsg = ChatMessage(role: .assistant, content: "")
            messages.append(assistantMsg)
            let idx = messages.count - 1

            do {
                let stream = client.chatStream(text: text, sessionId: sid, model: model)
                var first = true
                for try await event in stream {
                    switch event {
                    case .token(let t):
                        if first { isThinking = false; first = false }
                        messages[idx].content += t
                        streamingChunk += 1
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

    // MARK: - Private

    private func startNewSession() async {
        guard sessionId == nil else { return }
        do {
            sessionId = try await client.createSession()
        } catch {}
    }
}
