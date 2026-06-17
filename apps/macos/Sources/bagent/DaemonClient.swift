import Foundation
import AppKit

// MARK: - Health

struct DaemonHealth: Sendable {
    let daemonUp: Bool
    let ollamaUp: Bool
    let model: String
    let classifierModel: String
    let mailConnector: Bool
    let notesConnector: Bool
    let codexConnector: Bool
    let odooConnector: Bool
    let whatsappConnector: Bool
}

// MARK: - Memory

struct MemoryItem: Identifiable, Decodable, Sendable {
    let id: String
    let namespace: String
    let kind: String
    let language: String
    let text: String
    let source_ref: String?
    let created_at: String
    let use_count: Int
    // V11 ledger fields (optional — absent when constructed from search hits)
    let status: String?
    let source: String?
    let confidence: Double?
    let importance: Double?
    let sensitivity: String?
}

struct MemoryHit: Identifiable, Decodable, Sendable {
    let id: String
    let namespace: String
    let kind: String
    let text: String
    let score: Float
}

// MARK: - Skills

struct SkillItem: Identifiable, Decodable, Sendable {
    let name: String
    let description: String
    let version: Int
    let risk: String
    let tags: [String]
    let allowed_tools: [String]
    var body: String?

    var id: String { name }
}

// MARK: - Screen context (Phase 7)

/// Ephemeral screen context collected by ScreenContextProvider and forwarded to
/// the daemon in the `/chat` request body. Never persisted to disk on either side.
struct ScreenContextFields: Sendable {
    var imagePNGBase64: String?
    var ocrText: String
    var activeApp: String?
    var selectedText: String?
}

struct ScreenIntentResponse: Decodable, Sendable {
    let action: String
    let wants_screen: Bool
    let wants_ocr: Bool
    let wants_selection: Bool
}

// MARK: - Client

struct DaemonClient: Sendable {

    private static let dataDir = FileManager.default
        .urls(for: .applicationSupportDirectory, in: .userDomainMask)
        .first!
        .appendingPathComponent("bagent")

    private struct Creds {
        let port: Int
        let token: String
    }

    private func loadCreds() async throws -> Creds {
        let portURL  = Self.dataDir.appendingPathComponent("daemon.port")
        let tokenURL = Self.dataDir.appendingPathComponent("daemon.token")
        for _ in 0..<40 {
            if let portStr = try? String(contentsOf: portURL, encoding: .utf8),
               let port = Int(portStr.trimmingCharacters(in: .whitespacesAndNewlines)),
               let token = try? String(contentsOf: tokenURL, encoding: .utf8) {
                return Creds(port: port, token: token.trimmingCharacters(in: .whitespacesAndNewlines))
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        throw DaemonError.notReady
    }

    private func authedRequest(_ path: String, creds: Creds) -> URLRequest {
        var req = URLRequest(url: URL(string: "http://127.0.0.1:\(creds.port)\(path)")!)
        req.setValue("Bearer \(creds.token)", forHTTPHeaderField: "Authorization")
        return req
    }

    // MARK: Health

    func healthStatus() async -> DaemonHealth {
        do {
            let c = try await loadCreds()
            var req = authedRequest("/health", creds: c)
            req.timeoutInterval = 3
            let (data, response) = try await URLSession.shared.data(for: req)
            guard (response as? HTTPURLResponse)?.statusCode == 200 else {
                return DaemonHealth(daemonUp: false, ollamaUp: false, model: "—",
                                    classifierModel: "—", mailConnector: false, notesConnector: false,
                                    codexConnector: false, odooConnector: false, whatsappConnector: false)
            }
            struct ConnectorResp: Decodable {
                let mail: Bool; let notes: Bool; let codex: Bool?; let odoo: Bool?
                let whatsapp: Bool?
            }
            struct HealthResp: Decodable {
                let status: String; let ollama: Bool; let model: String
                let classifier_model: String?
                let connectors: ConnectorResp?
            }
            let h = try JSONDecoder().decode(HealthResp.self, from: data)
            return DaemonHealth(
                daemonUp: h.status == "ok",
                ollamaUp: h.ollama,
                model: h.model,
                classifierModel: h.classifier_model ?? "qwen2.5:0.5b",
                mailConnector:      h.connectors?.mail      ?? false,
                notesConnector:     h.connectors?.notes     ?? false,
                codexConnector:     h.connectors?.codex     ?? false,
                odooConnector:      h.connectors?.odoo      ?? false,
                whatsappConnector:  h.connectors?.whatsapp  ?? false
            )
        } catch {
            return DaemonHealth(daemonUp: false, ollamaUp: false, model: "—",
                                classifierModel: "—", mailConnector: false, notesConnector: false,
                                codexConnector: false, odooConnector: false, whatsappConnector: false)
        }
    }

    // MARK: Mail sync

    func syncMail() async throws -> (synced: Int, total: Int) {
        let c = try await loadCreds()
        var req = authedRequest("/mail/sync", creds: c)
        req.httpMethod = "POST"
        req.timeoutInterval = 60
        let (data, _) = try await URLSession.shared.data(for: req)
        struct Resp: Decodable { let synced: Int; let total_cached: Int }
        let r = try JSONDecoder().decode(Resp.self, from: data)
        return (r.synced, r.total_cached)
    }

    // MARK: Models

    func models() async throws -> [String] {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/models", creds: c))
        struct Resp: Decodable { let models: [String] }
        let resp = try JSONDecoder().decode(Resp.self, from: data)
        return resp.models
    }

    // MARK: Chat (SSE streaming)

    struct MailAttachmentRef: Decodable, Sendable {
        let filename: String
        let path: String
        let size: Int
    }

    /// Stable reference to a specific mail message found by the assistant.
    /// Used to populate the "Otvoriť mail" button in the UI.
    struct MailRef: Decodable, Sendable {
        let rowid: Int
        let message_id: String?
        let subject: String
        let sender: String
        /// When true the client auto-opens Mail.app after the first sentence streams in.
        let auto_open: Bool
    }

    // MARK: - Filesystem types (Phase 13A)

    struct FileRef: Decodable, Sendable {
        let path: String
        let display_name: String
        let kind: String
    }

    struct FileSearchRequest: Encodable {
        let query: String
        let roots: [String]?
        let search_names: Bool
        let search_contents: Bool
        let extensions: [String]?
        let include_hidden: Bool
        let max_results: Int
        let max_depth: Int?
    }

    struct FileSearchResult: Decodable, Sendable {
        let path: String
        let display_name: String
        let parent: String?
        let kind: String
        let mime: String?
        let size_bytes: Int?
        let modified_at: String?
        let match_type: String
        let matched_line: String?
        let line_number: Int?
        let score: Double
    }

    struct FileSearchResponse: Decodable, Sendable {
        let query: String
        let results: [FileSearchResult]
        let truncated: Bool
    }

    enum ChatEvent: Sendable {
        case token(String)
        case debugTrace(DebugTraceSummary)
        case memorySaved(id: String)
        case approvalRequested(id: String, tool: String, description: String?)
        case toolBlocked(tool: String)
        case mailAttachments([MailAttachmentRef])
        case mailFound(MailRef)
        case fileFound(FileRef)
        case fileOpened(path: String, success: Bool)
        case actionTaken(message: String)
        /// Task complexity rating emitted after context planning (Phase 8).
        /// Only emitted when level ≥ CodexCandidate.
        case taskRating(level: String, score: Int, reasons: [String], privacyRisk: String)
        /// Phase 6: Odoo record found — shown as "Otvoriť v Safari" button.
        case odooFound(OdooRef)
        /// Phase 11: WhatsApp chat found.
        case whatsappFound(WhatsappRef)
        case done(sessionId: String?)
    }

    /// Stable reference to a found Odoo record. Analogue of `MailRef` / `FileRef`.
    struct OdooRef: Decodable, Sendable {
        let model: String
        let id: Int
        let name: String
        let url: String
    }

    /// Phase 11: Reference to a WhatsApp chat found during tool context.
    struct WhatsappRef: Decodable, Sendable {
        let chat_id: String
        let contact_name: String?
        let snippet: String?
    }

    struct DebugTraceSummary: Decodable, Sendable {
        let prompt_trace_id: String
        let session_id: String?
        let preview: String
        let prompt_chars: Int?
        let prompt_token_estimate: Int?
        let message_count: Int?
        let selected_skill_names: [String]?
        let selected_memory_ids: [String]?
        let conversation_recall_injected: Bool?
    }

    /// Upload a local file to `POST /attachments` and return a `ChatAttachment`.
    func uploadAttachment(url: URL) async throws -> ChatAttachment {
        let c = try await loadCreds()
        var req = authedRequest("/attachments", creds: c)
        req.httpMethod = "POST"
        req.timeoutInterval = 60

        let filename = url.lastPathComponent
        let mime = mimeType(for: url)
        let boundary = UUID().uuidString
        req.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")

        let fileData = try Data(contentsOf: url)
        var body = Data()
        let crlf = "\r\n"
        func s(_ str: String) { body.append(str.data(using: .utf8)!) }
        s("--\(boundary)\(crlf)")
        s("Content-Disposition: form-data; name=\"file\"; filename=\"\(filename)\"\(crlf)")
        s("Content-Type: \(mime)\(crlf)")
        s(crlf)
        body.append(fileData)
        s(crlf)
        s("--\(boundary)--\(crlf)")
        req.httpBody = body

        let (data, response) = try await URLSession.shared.data(for: req)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw DaemonError.badStatus
        }
        struct Resp: Decodable {
            let attachment_id: String
            let filename: String
            let mime: String
            let kind: String
            let size: Int
        }
        let resp = try JSONDecoder().decode(Resp.self, from: data)
        let kind: ChatAttachmentKind = {
            switch resp.kind {
            case "image": return .image
            case "pdf":   return .pdf
            case "text":  return .text
            default:      return .other
            }
        }()
        // Generate a thumbnail for images
        var thumbnail: NSImage? = nil
        if kind == .image { thumbnail = NSImage(contentsOf: url) }
        return ChatAttachment(
            id: resp.attachment_id,
            filename: resp.filename,
            mime: resp.mime,
            kind: kind,
            localURL: url,
            sizeBytes: resp.size,
            thumbnail: thumbnail
        )
    }

    private func mimeType(for url: URL) -> String {
        let ext = url.pathExtension.lowercased()
        switch ext {
        case "jpg", "jpeg": return "image/jpeg"
        case "png":         return "image/png"
        case "gif":         return "image/gif"
        case "webp":        return "image/webp"
        case "heic", "heif": return "image/heic"
        case "pdf":         return "application/pdf"
        case "txt":         return "text/plain"
        case "md":          return "text/markdown"
        case "html":        return "text/html"
        case "csv":         return "text/csv"
        case "json":        return "application/json"
        default:            return "application/octet-stream"
        }
    }

    func chatStream(
        text: String,
        sessionId: String?,
        model: String,
        attachmentIds: [String] = [],
        screenContext: ScreenContextFields? = nil
    ) -> AsyncThrowingStream<ChatEvent, Error> {
        AsyncThrowingStream { continuation in
            Task {
                do {
                    let c = try await loadCreds()
                    var req = authedRequest("/chat", creds: c)
                    req.httpMethod = "POST"
                    req.setValue("application/json", forHTTPHeaderField: "Content-Type")
                    req.timeoutInterval = 120

                    struct Body: Encodable {
                        let message: String
                        let model: String
                        let session_id: String?
                        let attachment_ids: [String]
                        // Screen context (Phase 7) — ephemeral, never persisted
                        let screen_image_b64: String?
                        let screen_ocr_text: String?
                        let active_app: String?
                        let selected_text: String?
                    }
                    req.httpBody = try JSONEncoder().encode(Body(
                        message: text,
                        model: model,
                        session_id: sessionId,
                        attachment_ids: attachmentIds,
                        screen_image_b64: screenContext?.imagePNGBase64,
                        screen_ocr_text: screenContext?.ocrText.isEmpty == false ? screenContext?.ocrText : nil,
                        active_app: screenContext?.activeApp,
                        selected_text: screenContext?.selectedText
                    ))

                    let (bytes, response) = try await URLSession.shared.bytes(for: req)
                    guard (response as? HTTPURLResponse)?.statusCode == 200 else {
                        throw DaemonError.badStatus
                    }

                    for try await line in bytes.lines {
                        guard line.hasPrefix("data: ") else { continue }
                        let json = String(line.dropFirst(6))
                        guard let data = json.data(using: .utf8),
                              let event = try? JSONDecoder().decode(SSEEvent.self, from: data)
                        else { continue }

                        switch event.type {
                        case "debug_trace":
                            if let id = event.prompt_trace_id {
                                continuation.yield(.debugTrace(DebugTraceSummary(
                                    prompt_trace_id: id,
                                    session_id: event.session_id,
                                    preview: event.preview ?? "",
                                    prompt_chars: event.prompt_chars,
                                    prompt_token_estimate: event.prompt_token_estimate,
                                    message_count: event.message_count,
                                    selected_skill_names: event.selected_skill_names,
                                    selected_memory_ids: event.selected_memory_ids,
                                    conversation_recall_injected: event.conversation_recall_injected
                                )))
                            }
                        case "token":
                            if let content = event.content {
                                continuation.yield(.token(content))
                            }
                        case "memory_saved":
                            if let id = event.id {
                                continuation.yield(.memorySaved(id: id))
                            }
                        case "approval_requested":
                            if let id = event.id, let tool = event.tool {
                                continuation.yield(.approvalRequested(
                                    id: id, tool: tool, description: event.description
                                ))
                            }
                        case "tool_blocked":
                            if let tool = event.tool {
                                continuation.yield(.toolBlocked(tool: tool))
                            }
                        case "mail_attachments":
                            if let atts = event.attachments, !atts.isEmpty {
                                continuation.yield(.mailAttachments(atts))
                            }
                        case "mail_found":
                            if let rowid = event.rowid,
                               let subject = event.subject,
                               let sender = event.sender {
                                let ref_ = MailRef(
                                    rowid: rowid,
                                    message_id: event.message_id,
                                    subject: subject,
                                    sender: sender,
                                    auto_open: event.auto_open ?? false
                                )
                                continuation.yield(.mailFound(ref_))
                            }
                        case "file_found":
                            if let path = event.path,
                               let name = event.display_name,
                               let kind = event.kind {
                                continuation.yield(.fileFound(FileRef(
                                    path: path,
                                    display_name: name,
                                    kind: kind
                                )))
                            }
                        case "file_opened":
                            if let path = event.path {
                                continuation.yield(.fileOpened(
                                    path: path,
                                    success: event.success ?? true
                                ))
                            }
                        case "odoo_found":
                            if let model = event.model,
                               let recordId = event.record_id,
                               let name = event.name,
                               let url = event.url {
                                continuation.yield(.odooFound(OdooRef(
                                    model: model,
                                    id: recordId,
                                    name: name,
                                    url: url
                                )))
                            }
                        case "whatsapp_found":
                            if let chatId = event.chat_id {
                                continuation.yield(.whatsappFound(WhatsappRef(
                                    chat_id: chatId,
                                    contact_name: event.contact_name,
                                    snippet: event.snippet
                                )))
                            }
                        case "action_taken":
                            if let msg = event.message {
                                continuation.yield(.actionTaken(message: msg))
                            }
                        case "task_rating":
                            // Phase 8: Codex task complexity hint (only emitted when ≥ CodexCandidate)
                            if let level = event.level, let score = event.score {
                                continuation.yield(.taskRating(
                                    level: level,
                                    score: score,
                                    reasons: event.reasons ?? [],
                                    privacyRisk: event.privacy_risk ?? "unknown"
                                ))
                            }
                        case "done":
                            continuation.yield(.done(sessionId: event.session_id))
                            continuation.finish(); return
                        case "error":
                            throw DaemonError.serverError(event.message ?? "unknown")
                        default: break
                        }
                    }
                    continuation.finish()
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }

    // MARK: Sessions

    func createSession() async throws -> String {
        let c = try await loadCreds()
        var req = authedRequest("/sessions", creds: c)
        req.httpMethod = "POST"
        let (data, _) = try await URLSession.shared.data(for: req)
        struct Resp: Decodable { let session_id: String }
        return try JSONDecoder().decode(Resp.self, from: data).session_id
    }

    // MARK: Memory

    func memoryItems(namespace: String? = nil) async throws -> [MemoryItem] {
        let c = try await loadCreds()
        var path = "/memory"
        if let ns = namespace, !ns.isEmpty { path += "?namespace=\(ns)" }
        let (data, _) = try await URLSession.shared.data(for: authedRequest(path, creds: c))
        struct Resp: Decodable { let items: [MemoryItem] }
        return try JSONDecoder().decode(Resp.self, from: data).items
    }

    func memorySearch(query: String, namespace: String? = nil) async throws -> [MemoryHit] {
        let c = try await loadCreds()
        var comps = URLComponents(string: "http://127.0.0.1:\(c.port)/memory/search")!
        comps.queryItems = [URLQueryItem(name: "q", value: query)]
        if let ns = namespace { comps.queryItems?.append(URLQueryItem(name: "namespace", value: ns)) }
        var req = URLRequest(url: comps.url!)
        req.setValue("Bearer \(c.token)", forHTTPHeaderField: "Authorization")
        let (data, _) = try await URLSession.shared.data(for: req)
        struct Resp: Decodable { let hits: [MemoryHit] }
        return try JSONDecoder().decode(Resp.self, from: data).hits
    }

    func memoryDelete(id: String) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/memory/\(id)", creds: c)
        req.httpMethod = "DELETE"
        _ = try await URLSession.shared.data(for: req)
    }

    // MARK: - Skills

    func skills() async throws -> [SkillItem] {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/skills", creds: c))
        struct Resp: Decodable { let skills: [SkillItem] }
        return try JSONDecoder().decode(Resp.self, from: data).skills
    }

    func skill(name: String) async throws -> SkillItem {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(for: authedRequest("/skills/\(name)", creds: c))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw DaemonError.badStatus }
        return try JSONDecoder().decode(SkillItem.self, from: data)
    }

    func debugContextPlan(message: String) async throws -> String {
        let c = try await loadCreds()
        var req = authedRequest("/debug/context-plan", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let message: String }
        req.httpBody = try JSONEncoder().encode(Body(message: message))
        let (data, response) = try await URLSession.shared.data(for: req)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw DaemonError.serverError(String(decoding: data, as: UTF8.self))
        }
        return prettyJSONString(data)
    }

    // MARK: - Approvals

    func pendingApprovals() async throws -> [ApprovalItem] {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/approvals/pending", creds: c))
        struct Resp: Decodable { let approvals: [ApprovalItem] }
        return try JSONDecoder().decode(Resp.self, from: data).approvals
    }

    func decideApproval(id: String, allow: Bool) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/approvals/\(id)/decide", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let allow: Bool }
        req.httpBody = try JSONEncoder().encode(Body(allow: allow))
        _ = try await URLSession.shared.data(for: req)
    }

    // MARK: - Rules

    func rulesYaml() async throws -> String {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/rules", creds: c))
        return String(decoding: data, as: UTF8.self)
    }

    func saveRules(yaml: String) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/rules", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let yaml: String }
        req.httpBody = try JSONEncoder().encode(Body(yaml: yaml))
        let (data, resp) = try await URLSession.shared.data(for: req)
        guard (resp as? HTTPURLResponse)?.statusCode == 200 else {
            struct ErrResp: Decodable { let error: String }
            if let e = try? JSONDecoder().decode(ErrResp.self, from: data) {
                throw DaemonError.serverError(e.error)
            }
            throw DaemonError.badStatus
        }
    }

    // MARK: - Mail open (Phase 5E)

    /// Ask the daemon to open a specific email in Apple Mail.app.
    func openMail(rowid: Int?, messageId: String?, subject: String, sender: String) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/mail/open", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable {
            let rowid: Int?
            let message_id: String?
            let subject: String
            let sender: String
        }
        req.httpBody = try JSONEncoder().encode(
            Body(rowid: rowid, message_id: messageId, subject: subject, sender: sender)
        )
        _ = try await URLSession.shared.data(for: req)
    }

    // MARK: - Filesystem (Phase 13A)

    func searchFiles(_ request: FileSearchRequest) async throws -> FileSearchResponse {
        let c = try await loadCreds()
        var req = authedRequest("/filesystem/search", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONEncoder().encode(request)
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode(FileSearchResponse.self, from: data)
    }

    func revealInFinder(path: String) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/filesystem/reveal", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let path: String }
        req.httpBody = try JSONEncoder().encode(Body(path: path))
        _ = try await URLSession.shared.data(for: req)
    }

    func openFolder(path: String) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/filesystem/open-folder", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let path: String }
        req.httpBody = try JSONEncoder().encode(Body(path: path))
        _ = try await URLSession.shared.data(for: req)
    }

    func openFile(path: String) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/filesystem/open", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let path: String }
        req.httpBody = try JSONEncoder().encode(Body(path: path))
        _ = try await URLSession.shared.data(for: req)
    }

    func openFileWith(path: String, app: String) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/filesystem/open-with", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let path: String; let app: String }
        req.httpBody = try JSONEncoder().encode(Body(path: path, app: app))
        _ = try await URLSession.shared.data(for: req)
    }

    // MARK: - Codex (Phase 8)

    struct CodexStatus: Decodable, Sendable {
        let available: Bool
        let binaryPath: String?
        let version: String?
        let configuredPath: String?
        let error: String?

        enum CodingKeys: String, CodingKey {
            case available
            case binaryPath    = "binary_path"
            case version
            case configuredPath = "configured_path"
            case error
        }
    }

    struct CodexTaskRating: Decodable, Sendable {
        let level: String
        let score: Int
        let codexRecommended: Bool
        let requiresApproval: Bool
        let privacyRisk: String
        let suggestedContextScope: String
        let reasons: [String]

        enum CodingKeys: String, CodingKey {
            case level, score, reasons
            case codexRecommended    = "codex_recommended"
            case requiresApproval    = "requires_approval"
            case privacyRisk         = "privacy_risk"
            case suggestedContextScope = "suggested_context_scope"
        }
    }

    struct CodexFinding: Decodable, Sendable {
        let claim: String
        let sourceRefs: [String]
        let confidence: Double?

        enum CodingKeys: String, CodingKey {
            case claim, confidence
            case sourceRefs = "source_refs"
        }
    }

    struct CodexConflict: Decodable, Sendable {
        let description: String
        let sourceRefs: [String]

        enum CodingKeys: String, CodingKey {
            case description
            case sourceRefs = "source_refs"
        }
    }

    struct CodexProposedAction: Decodable, Sendable {
        let kind: String
        let description: String
        let requiresUserApproval: Bool
        let targetRef: String?

        enum CodingKeys: String, CodingKey {
            case kind, description
            case requiresUserApproval = "requires_user_approval"
            case targetRef            = "target_ref"
        }
    }

    struct CodexDraft: Decodable, Sendable {
        let channel: String
        let language: String
        let body: String
    }

    struct CodexRunResult: Decodable, Sendable {
        let ran: Bool
        let reason: String?
        let error: String?
        let message: String?
        let taskId: String?
        let summary: String?
        let findings: [CodexFinding]?
        let conflicts: [CodexConflict]?
        let proposedActions: [CodexProposedAction]?
        let drafts: [CodexDraft]?
        let questionsForUser: [String]?
        let stdoutSnippet: String?
        let stderrSnippet: String?
        let exitCode: Int?
        let timedOut: Bool?
        let outputHash: String?
        let rating: CodexTaskRating?

        enum CodingKeys: String, CodingKey {
            case ran, reason, error, message, summary, findings, conflicts, drafts, rating
            case taskId           = "task_id"
            case proposedActions  = "proposed_actions"
            case questionsForUser = "questions_for_user"
            case stdoutSnippet    = "stdout_snippet"
            case stderrSnippet    = "stderr_snippet"
            case exitCode         = "exit_code"
            case timedOut         = "timed_out"
            case outputHash       = "output_hash"
        }
    }

    func codexStatus() async throws -> CodexStatus {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/codex/status", creds: c))
        return try JSONDecoder().decode(CodexStatus.self, from: data)
    }

    func rateCodexTask(description: String, contextSources: [String]) async throws -> CodexTaskRating {
        let c = try await loadCreds()
        var req = authedRequest("/codex/rate-task", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let description: String; let context_sources: [String] }
        req.httpBody = try JSONEncoder().encode(Body(description: description, context_sources: contextSources))
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode(CodexTaskRating.self, from: data)
    }

    func runCodexTask(
        description: String,
        contextSources: [String],
        contextRefs: [String],
        forceCodex: Bool
    ) async throws -> CodexRunResult {
        let c = try await loadCreds()
        var req = authedRequest("/codex/run-task", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.timeoutInterval = 180 // Codex may take up to 2 min
        struct Body: Encodable {
            let description: String
            let context_sources: [String]
            let context_refs: [String]
            let force_codex: Bool
        }
        req.httpBody = try JSONEncoder().encode(Body(
            description: description,
            context_sources: contextSources,
            context_refs: contextRefs,
            force_codex: forceCodex
        ))
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode(CodexRunResult.self, from: data)
    }

    // MARK: - Odoo (Phase 6)

    struct OdooConfigResult: Decodable, Sendable {
        let ok: Bool
        let version: String?
        let uid: Int?
        let error: String?
    }

    struct OdooStatusResult: Decodable, Sendable {
        let configured: Bool
        let connected: Bool
        let version: String?
        let uid: Int?
        let error: String?
    }

    /// Authenticate and store the connector in-memory.
    /// Also used as the Settings "Testovať Odoo" action — returns version on success.
    func odooConfigure(url: String, db: String, user: String, apiKey: String) async throws -> OdooConfigResult {
        let c = try await loadCreds()
        var req = authedRequest("/odoo/config", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable {
            let base_url: String; let db: String; let username: String; let api_key: String
        }
        req.httpBody = try JSONEncoder().encode(Body(base_url: url, db: db, username: user, api_key: apiKey))
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode(OdooConfigResult.self, from: data)
    }

    func odooStatus() async throws -> OdooStatusResult {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/odoo/status", creds: c))
        return try JSONDecoder().decode(OdooStatusResult.self, from: data)
    }

    func odooOpen(url: String) async throws {
        let c = try await loadCreds()
        var req = authedRequest("/odoo/open", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let url: String }
        req.httpBody = try JSONEncoder().encode(Body(url: url))
        let (_, _) = try await URLSession.shared.data(for: req)
    }

    // MARK: - WhatsApp (Phase 11)

    struct WhatsappStatusResult: Decodable, Sendable {
        let status: String          // "stopped" | "starting" | "qr" | "authenticated" | "ready" | "disconnected" | "error" | "missing_node" | "bridge_not_installed"
        let connected: Bool
        let needs_qr: Bool
        let error: String?
        let me_name: String?
        let me_phone: String?
    }

    struct WhatsappQrResult: Decodable, Sendable {
        let qr: String?
        let status: String?
    }

    struct WhatsappContact: Identifiable, Decodable, Sendable {
        let id: String
        let name: String?
        let phone: String?
    }

    struct WhatsappChat: Identifiable, Decodable, Sendable {
        let id: String
        let name: String?
        let is_group: Bool
        let unread_count: Int
        let last_message_preview: String?
    }

    struct WhatsappMessage: Identifiable, Decodable, Sendable {
        let id: String
        let from: String
        let body: String
        let timestamp: Int
        let from_me: Bool
    }

    func whatsappStatus() async throws -> WhatsappStatusResult {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/whatsapp/status", creds: c))
        return try JSONDecoder().decode(WhatsappStatusResult.self, from: data)
    }

    func whatsappStart() async throws {
        let c = try await loadCreds()
        var req = authedRequest("/whatsapp/start", creds: c)
        req.httpMethod = "POST"
        let (_, _) = try await URLSession.shared.data(for: req)
    }

    func whatsappStop() async throws {
        let c = try await loadCreds()
        var req = authedRequest("/whatsapp/stop", creds: c)
        req.httpMethod = "POST"
        let (_, _) = try await URLSession.shared.data(for: req)
    }

    func whatsappQr() async throws -> WhatsappQrResult {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/whatsapp/qr", creds: c))
        return try JSONDecoder().decode(WhatsappQrResult.self, from: data)
    }

    func whatsappLogout() async throws {
        let c = try await loadCreds()
        var req = authedRequest("/whatsapp/logout", creds: c)
        req.httpMethod = "POST"
        let (_, _) = try await URLSession.shared.data(for: req)
    }

    func whatsappContacts(limit: Int = 50) async throws -> [WhatsappContact] {
        let c = try await loadCreds()
        var req = authedRequest("/whatsapp/contacts?limit=\(limit)", creds: c)
        req.timeoutInterval = 10
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode([WhatsappContact].self, from: data)
    }

    func whatsappChats(limit: Int = 20) async throws -> [WhatsappChat] {
        let c = try await loadCreds()
        var req = authedRequest("/whatsapp/chats?limit=\(limit)", creds: c)
        req.timeoutInterval = 10
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode([WhatsappChat].self, from: data)
    }

    func whatsappMessages(chatId: String, limit: Int = 20) async throws -> [WhatsappMessage] {
        let c = try await loadCreds()
        let enc = chatId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? chatId
        var req = authedRequest("/whatsapp/chats/\(enc)/messages?limit=\(limit)", creds: c)
        req.timeoutInterval = 10
        let (data, _) = try await URLSession.shared.data(for: req)
        return try JSONDecoder().decode([WhatsappMessage].self, from: data)
    }

    // MARK: - Disk usage (Phase 4G)

    struct UsageStats: Decodable, Sendable {
        let db_bytes: Int
        let attachments_bytes: Int
        let memory_items_count: Int
        let chat_turns_count: Int
        let mail_cache_count: Int
        let embeddings_count: Int
        let total_bytes: Int

        var totalFormatted: String { formatBytes(total_bytes) }
        var dbFormatted: String { formatBytes(db_bytes) }
        var attachmentsFormatted: String { formatBytes(attachments_bytes) }

        private func formatBytes(_ n: Int) -> String {
            let d = Double(n)
            if n < 1024 { return "\(n) B" }
            if n < 1024 * 1024 { return String(format: "%.1f KB", d / 1024) }
            if n < 1024 * 1024 * 1024 { return String(format: "%.1f MB", d / (1024 * 1024)) }
            return String(format: "%.2f GB", d / (1024 * 1024 * 1024))
        }
    }

    func usage() async throws -> UsageStats {
        let c = try await loadCreds()
        let (data, _) = try await URLSession.shared.data(for: authedRequest("/usage", creds: c))
        return try JSONDecoder().decode(UsageStats.self, from: data)
    }

    func debugTrace(id: String) async throws -> String {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(for: authedRequest("/debug/traces/\(id)", creds: c))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw DaemonError.serverError(String(decoding: data, as: UTF8.self))
        }
        return prettyJSONString(data)
    }

    func debugConversation(id: String) async throws -> String {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(for: authedRequest("/debug/conversations/\(id)", creds: c))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw DaemonError.serverError(String(decoding: data, as: UTF8.self))
        }
        return prettyJSONString(data)
    }

    func clearMailCache() async throws {
        let c = try await loadCreds()
        var req = authedRequest("/mail/cache/clear", creds: c)
        req.httpMethod = "POST"
        _ = try await URLSession.shared.data(for: req)
    }

    // MARK: - Screen intent (Phase 7)

    /// Classify whether a user message requires screen context.
    /// Returns `ScreenIntentResponse` on success; gracefully returns a "none" default on failure.
    func screenIntent(message: String) async -> ScreenIntentResponse {
        guard let c = try? await loadCreds() else {
            return ScreenIntentResponse(action: "none", wants_screen: false, wants_ocr: false, wants_selection: false)
        }
        var req = authedRequest("/screen/intent", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let message: String }
        req.httpBody = try? JSONEncoder().encode(Body(message: message))

        guard let (data, _) = try? await URLSession.shared.data(for: req),
              let intent = try? JSONDecoder().decode(ScreenIntentResponse.self, from: data)
        else {
            return ScreenIntentResponse(action: "none", wants_screen: false, wants_ocr: false, wants_selection: false)
        }
        return intent
    }

    private func prettyJSONString(_ data: Data) -> String {
        guard
            let obj = try? JSONSerialization.jsonObject(with: data),
            let pretty = try? JSONSerialization.data(withJSONObject: obj, options: [.prettyPrinted, .sortedKeys])
        else {
            return String(decoding: data, as: UTF8.self)
        }
        return String(decoding: pretty, as: UTF8.self)
    }
}

// MARK: - Types

struct ApprovalItem: Identifiable, Decodable, Sendable {
    let id: String
    let toolName: String
    let description: String?
    let expiresAt: String
    let createdAt: String

    enum CodingKeys: String, CodingKey {
        case id, description
        case toolName  = "tool_name"
        case expiresAt = "expires_at"
        case createdAt = "created_at"
    }
}

enum DaemonError: LocalizedError {
    case notReady
    case badStatus
    case serverError(String)

    var errorDescription: String? {
        switch self {
        case .notReady:           return "Daemon sa nespustil včas"
        case .badStatus:          return "Neplatná odpoveď od daemona"
        case .serverError(let m): return "Chyba servera: \(m)"
        }
    }
}

private struct SSEEvent: Decodable {
    let type: String
    let content: String?
    let message: String?
    let id: String?
    let session_id: String?
    let tool: String?
    let description: String?
    let attachments: [DaemonClient.MailAttachmentRef]?
    // mail_found event fields
    let rowid: Int?
    let message_id: String?
    let subject: String?
    let sender: String?
    let auto_open: Bool?
    // file_found / file_opened event fields
    let path: String?
    let display_name: String?
    let kind: String?
    let success: Bool?
    // odoo_found event fields
    let model: String?
    let record_id: Int?
    let name: String?
    let url: String?
    // debug_trace event fields
    let prompt_trace_id: String?
    let preview: String?
    let prompt_chars: Int?
    let prompt_token_estimate: Int?
    let message_count: Int?
    let selected_skill_names: [String]?
    let selected_memory_ids: [String]?
    let conversation_recall_injected: Bool?
    // task_rating event fields (Phase 8)
    let level: String?
    let score: Int?
    let reasons: [String]?
    let privacy_risk: String?
    // whatsapp_found event fields (Phase 11)
    let chat_id: String?
    let contact_name: String?
    let snippet: String?
}
