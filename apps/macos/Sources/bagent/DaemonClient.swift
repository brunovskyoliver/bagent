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
}

struct MemoryHit: Identifiable, Decodable, Sendable {
    let id: String
    let namespace: String
    let kind: String
    let text: String
    let score: Float
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
                                    classifierModel: "—", mailConnector: false, notesConnector: false)
            }
            struct ConnectorResp: Decodable { let mail: Bool; let notes: Bool }
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
                mailConnector:  h.connectors?.mail  ?? false,
                notesConnector: h.connectors?.notes ?? false
            )
        } catch {
            return DaemonHealth(daemonUp: false, ollamaUp: false, model: "—",
                                classifierModel: "—", mailConnector: false, notesConnector: false)
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

    enum ChatEvent: Sendable {
        case token(String)
        case debugTrace(DebugTraceSummary)
        case memorySaved(id: String)
        case approvalRequested(id: String, tool: String, description: String?)
        case toolBlocked(tool: String)
        case mailAttachments([MailAttachmentRef])
        case mailFound(MailRef)
        case actionTaken(message: String)
        case done(sessionId: String?)
    }

    struct DebugTraceSummary: Decodable, Sendable {
        let prompt_trace_id: String
        let session_id: String?
        let preview: String
        let prompt_chars: Int?
        let prompt_token_estimate: Int?
        let message_count: Int?
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
        attachmentIds: [String] = []
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
                    }
                    req.httpBody = try JSONEncoder().encode(Body(
                        message: text,
                        model: model,
                        session_id: sessionId,
                        attachment_ids: attachmentIds
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
                                    message_count: event.message_count
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
                        case "action_taken":
                            if let msg = event.message {
                                continuation.yield(.actionTaken(message: msg))
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
    // debug_trace event fields
    let prompt_trace_id: String?
    let preview: String?
    let prompt_chars: Int?
    let prompt_token_estimate: Int?
    let message_count: Int?
}
