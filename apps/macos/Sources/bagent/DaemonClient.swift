import Foundation
import AppKit

// MARK: - Health

struct DaemonHealth: Sendable {
    let daemonUp: Bool
    let processID: Int?
    let tavilyConfiguration: DaemonClient.TavilyConfigurationStatus
    let baseRTUp: Bool
    let model: String
    let classifierModel: String
    let mailConnector: Bool
    let notesConnector: Bool
    let codexConnector: Bool
    let odooConnector: Bool
    let whatsappConnector: Bool

    init(
        daemonUp: Bool,
        processID: Int? = nil,
        tavilyConfiguration: DaemonClient.TavilyConfigurationStatus = .pending,
        baseRTUp: Bool,
        model: String,
        classifierModel: String,
        mailConnector: Bool,
        notesConnector: Bool,
        codexConnector: Bool,
        odooConnector: Bool,
        whatsappConnector: Bool
    ) {
        self.daemonUp = daemonUp
        self.processID = processID
        self.tavilyConfiguration = tavilyConfiguration
        self.baseRTUp = baseRTUp
        self.model = model
        self.classifierModel = classifierModel
        self.mailConnector = mailConnector
        self.notesConnector = notesConnector
        self.codexConnector = codexConnector
        self.odooConnector = odooConnector
        self.whatsappConnector = whatsappConnector
    }
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

struct DaemonClient: Sendable, NotchEventTransport {
    enum CurrentChatMutationError: Error {
        case conflict
        case bound
        case unavailable
    }

    enum TavilyConfigurationStatus: String, Codable, Sendable, Equatable {
        case pending
        case absent
        case configured
        case configurationFailed = "configuration_failed"
    }

    private static let dataDir: URL = {
        if ProcessInfo.processInfo.environment["BAGENT_STAGE7A_ACCEPTANCE_FIXTURE"] == "1",
           let path = ProcessInfo.processInfo.environment["BAGENT_DATA_DIR"] {
            return URL(fileURLWithPath: path, isDirectory: true)
        }
        return FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first!
            .appendingPathComponent("bagent")
    }()

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

    private func validateOK(data: Data, response: URLResponse) throws {
        guard let http = response as? HTTPURLResponse else {
            throw DaemonError.badStatus
        }
        guard (200..<300).contains(http.statusCode) else {
            throw DaemonError.serverError(serverErrorMessage(from: data))
        }
    }

    private func serverErrorMessage(from data: Data) -> String {
        if let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
            if let detail = obj["detail"] as? String { return detail }
            if let error = obj["error"] as? String { return error }
        }
        let raw = String(decoding: data, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
        return raw.isEmpty ? "Neznáma chyba" : raw
    }

    // MARK: Health

    func healthStatus() async -> DaemonHealth {
        do {
            let c = try await loadCreds()
            var req = authedRequest("/health", creds: c)
            req.timeoutInterval = 3
            let (data, response) = try await URLSession.shared.data(for: req)
            guard (response as? HTTPURLResponse)?.statusCode == 200 else {
                return DaemonHealth(daemonUp: false, baseRTUp: false, model: "—",
                                    classifierModel: "—", mailConnector: false, notesConnector: false,
                                    codexConnector: false, odooConnector: false, whatsappConnector: false)
            }
            enum WhatsappConnectorResp: Decodable {
                case bool(Bool)
                case object(connected: Bool)

                init(from decoder: Decoder) throws {
                    let single = try decoder.singleValueContainer()
                    if let value = try? single.decode(Bool.self) {
                        self = .bool(value)
                        return
                    }
                    let keyed = try decoder.container(keyedBy: CodingKeys.self)
                    self = .object(connected: try keyed.decodeIfPresent(Bool.self, forKey: .connected) ?? false)
                }

                private enum CodingKeys: String, CodingKey {
                    case connected
                }

                var isConnected: Bool {
                    switch self {
                    case .bool(let value): return value
                    case .object(let connected): return connected
                    }
                }
            }
            struct ConnectorResp: Decodable {
                let mail: Bool; let notes: Bool; let codex: Bool?; let odoo: Bool?
                let whatsapp: WhatsappConnectorResp?
            }
            struct HealthResp: Decodable {
                let status: String; let basert: Bool; let model: String
                let classifier_model: String?
                let process_id: Int?
                let tavily_configuration: TavilyConfigurationStatus?
                let connectors: ConnectorResp?
            }
            let h = try JSONDecoder().decode(HealthResp.self, from: data)
            return DaemonHealth(
                daemonUp: h.status == "ok",
                processID: h.process_id,
                tavilyConfiguration: h.tavily_configuration ?? .pending,
                baseRTUp: h.basert,
                model: h.model,
                classifierModel: h.classifier_model ?? ModelRuntimeConfiguration.model,
                mailConnector:      h.connectors?.mail      ?? false,
                notesConnector:     h.connectors?.notes     ?? false,
                codexConnector:     h.connectors?.codex     ?? false,
                odooConnector:      h.connectors?.odoo      ?? false,
                whatsappConnector:  h.connectors?.whatsapp?.isConnected  ?? false
            )
        } catch {
            return DaemonHealth(daemonUp: false, baseRTUp: false, model: "—",
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

    struct ActivityEvent: Sendable {
        let id: String
        let kind: String
        let tool: String?
        let title: String
        let detail: String?
        let status: String?
        let durationMs: Int?
    }

    enum EvidencePhase: String, Sendable, Equatable {
        case findingMail = "finding_mail"
        case reading
        case searching
        case verifying
        case loadingSynthesisModel = "loading_synthesis_model"
        case preparingAnswer = "preparing_answer"
        case repairing
        case fallingBack = "falling_back"
        case validating
        case deterministicRendering = "deterministic_rendering"
    }

    enum EvidenceExecutionStatus: String, Sendable, Equatable {
        case inProgress = "in_progress"
        case succeeded
        case failed
        case denied
        case timedOut = "timed_out"
    }

    enum EvidenceContribution: String, Sendable, Equatable {
        case satisfied
        case partial
        case empty
        case duplicate
        case irrelevant
    }

    enum EvidenceOutcomeState: String, Sendable, Equatable {
        case verified
        case conflict
        case partial
        case empty
        case unavailable
        case denied
        case verificationShortfall = "verification_shortfall"
    }

    enum EvidenceOutcomeKind: String, Sendable, Equatable {
        case mail
        case web
    }

    enum EvidencePolishStatus: String, Sendable, Equatable {
        case skipped, accepted, rejected
        case timedOut = "timed_out"
        case unavailable
        case memoryIneligible = "memory_ineligible"
    }

    struct EvidencePolishEvent: Sendable, Equatable {
        let turnId: String
        let status: EvidencePolishStatus
    }

    struct EvidencePhaseEvent: Sendable, Equatable {
        let turnId: String
        let phase: EvidencePhase
        let completed: Int?
        let total: Int?
    }

    struct LogicalActivityEvent: Sendable, Equatable {
        let turnId: String
        let activityId: String
        let normalizedOperation: String
        let argumentHash: String
        let executionStatus: EvidenceExecutionStatus
        let contribution: EvidenceContribution
        let evidenceCount: Int
        let sourceDomains: [String]
        let durationMs: Int
        let attemptCount: Int
        let retries: Int
        let duplicatesSuppressed: Int
        let failureReason: String?
    }

    struct EvidenceOutcomeEvent: Sendable, Equatable {
        let turnId: String
        let state: EvidenceOutcomeState
        let kind: EvidenceOutcomeKind
        let acquired: Int
        let requested: Int
        let sourceCount: Int
        let message: String
    }

    struct EvidenceAcquisitionDiagnostic: Sendable, Equatable {
        let status: String
        let provider: String?
        let providerStatus: String?
    }

    struct TranscriptSource: Identifiable, Sendable, Equatable {
        let id: String
        let title: String
        let url: URL
        let domain: String
    }

    enum ChatEvent: Sendable {
        case token(String)
        case activityStarted(ActivityEvent)
        case activityCompleted(ActivityEvent)
        case evidencePhase(EvidencePhaseEvent)
        case logicalActivityStarted(LogicalActivityEvent)
        case logicalActivityCompleted(LogicalActivityEvent)
        case evidencePolish(EvidencePolishEvent)
        case evidenceOutcome(EvidenceOutcomeEvent)
        case evidenceAcquisitionDiagnostic(EvidenceAcquisitionDiagnostic)
        case sourceDiscovered(TranscriptSource)
        case debugTrace(DebugTraceSummary)
        case memorySaved(id: String)
        case approvalRequested(id: String, tool: String, description: String?)
        case toolBlocked(tool: String)
        /// The agent loop is executing a tool this turn (transient status).
        case toolCall(tool: String)
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
        case done
    }

    static func evidenceChatEvent(from event: SSEEvent) -> ChatEvent? {
        switch event.type {
        case "evidence_phase":
            guard let turnId = event.turn_id,
                  let rawPhase = event.phase,
                  let phase = EvidencePhase(rawValue: rawPhase)
            else { return nil }
            return .evidencePhase(.init(
                turnId: turnId,
                phase: phase,
                completed: event.completed,
                total: event.total
            ))
        case "logical_activity_started", "logical_activity_completed":
            guard let turnId = event.turn_id,
                  let activityId = event.activity_id,
                  let operation = event.normalized_operation,
                  let argumentHash = event.argument_hash,
                  let rawExecution = event.execution_status,
                  let execution = EvidenceExecutionStatus(rawValue: rawExecution),
                  let rawContribution = event.contribution,
                  let contribution = EvidenceContribution(rawValue: rawContribution)
            else { return nil }
            let activity = LogicalActivityEvent(
                turnId: turnId,
                activityId: activityId,
                normalizedOperation: operation,
                argumentHash: argumentHash,
                executionStatus: execution,
                contribution: contribution,
                evidenceCount: event.evidence_count ?? 0,
                sourceDomains: event.source_domains ?? [],
                durationMs: event.duration_ms ?? 0,
                attemptCount: event.attempt_count ?? 0,
                retries: event.retries ?? 0,
                duplicatesSuppressed: event.duplicates_suppressed ?? 0,
                failureReason: event.failure_reason
            )
            return event.type == "logical_activity_started"
                ? .logicalActivityStarted(activity)
                : .logicalActivityCompleted(activity)
        case "evidence_outcome":
            guard let turnId = event.turn_id,
                  let rawState = event.state,
                  let state = EvidenceOutcomeState(rawValue: rawState),
                  let rawKind = event.outcome_kind,
                  let kind = EvidenceOutcomeKind(rawValue: rawKind),
                  let message = event.message
            else { return nil }
            return .evidenceOutcome(.init(
                turnId: turnId,
                state: state,
                kind: kind,
                acquired: event.acquired ?? 0,
                requested: event.requested ?? 0,
                sourceCount: event.source_count ?? 0,
                message: message
            ))
        case "evidence_polish":
            guard let turnId = event.turn_id,
                  let rawStatus = event.status,
                  let status = EvidencePolishStatus(rawValue: rawStatus)
            else { return nil }
            return .evidencePolish(.init(turnId: turnId, status: status))
        case "evidence_acquisition_diagnostic":
            guard let status = event.status else { return nil }
            return .evidenceAcquisitionDiagnostic(.init(
                status: status,
                provider: event.provider,
                providerStatus: event.provider_status
            ))
        default:
            return nil
        }
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
            availability: .available,
            thumbnail: thumbnail
        )
    }

    /// Refetch daemon-owned metadata for a pending attachment reference.
    func attachment(id: String) async throws -> ChatAttachment {
        let c = try await loadCreds()
        let encodedID = id.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? id
        let (data, response) = try await URLSession.shared.data(
            for: authedRequest("/attachments/\(encodedID)", creds: c))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw DaemonError.badStatus
        }
        struct Response: Decodable {
            let id: String
            let filename: String
            let mime: String
            let size: Int
        }
        let metadata = try JSONDecoder().decode(Response.self, from: data)
        return ChatAttachment(
            id: metadata.id,
            filename: metadata.filename,
            mime: metadata.mime,
            kind: Self.attachmentKind(for: metadata.mime),
            localURL: nil,
            sizeBytes: metadata.size,
            availability: .available)
    }

    private static func attachmentKind(for mime: String) -> ChatAttachmentKind {
        if mime.hasPrefix("image/") { return .image }
        if mime == "application/pdf" { return .pdf }
        if mime.hasPrefix("text/") { return .text }
        return .other
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

    struct CurrentChatSnapshot: Codable, Sendable, Equatable {
        let identity: String
        let revision: UInt64
        let turnCount: UInt64
        let contentBytes: UInt64
        let turns: [CurrentChatTurn]
        let draft: CurrentChatDraft?
        let continuation: CurrentChatContinuation?
        let submittedAttachments: [SubmittedAttachment]
        let validatedSources: [CurrentChatAvailability]
        let connectorReferences: [CurrentChatAvailability]
        let completedApprovalPresentations: [CompletedApprovalPresentation]

        enum CodingKeys: String, CodingKey {
            case identity, revision, turns, draft, continuation
            case turnCount = "turn_count"
            case contentBytes = "content_bytes"
            case submittedAttachments = "submitted_attachments"
            case validatedSources = "validated_sources"
            case connectorReferences = "connector_references"
            case completedApprovalPresentations = "completed_approval_presentations"
        }
    }

    struct CurrentChatTurn: Codable, Sendable, Equatable {
        let identity: String
        let userMessage: String
        let assistantOutput: String?
        let state: String
        let interruptionReason: String?
        let submittedAt: String
        let completedAt: String?

        enum CodingKeys: String, CodingKey {
            case identity, state
            case userMessage = "user_message"
            case assistantOutput = "assistant_output"
            case interruptionReason = "interruption_reason"
            case submittedAt = "submitted_at"
            case completedAt = "completed_at"
        }
    }

    struct CurrentChatDraft: Codable, Sendable, Equatable {
        let text: String
        let editedAt: String
        let pendingAttachmentReferences: [String]

        enum CodingKeys: String, CodingKey {
            case text
            case editedAt = "edited_at"
            case pendingAttachmentReferences = "pending_attachment_references"
        }
    }

    struct CurrentChatContinuation: Codable, Sendable, Equatable {
        let identity: String
        let sourceAutomationSessionIdentity: String
        let seed: String
        let sourceDeleted: Bool

        enum CodingKeys: String, CodingKey {
            case identity, seed
            case sourceAutomationSessionIdentity = "source_automation_session_identity"
            case sourceDeleted = "source_deleted"
        }
    }

    struct SubmittedAttachment: Codable, Sendable, Equatable {
        let conversationTurnIdentity: String
        let identity: String
        let filename: String
        let mime: String
        let sizeBytes: UInt64
        let available: Bool

        enum CodingKeys: String, CodingKey {
            case identity, filename, mime, available
            case conversationTurnIdentity = "conversation_turn_identity"
            case sizeBytes = "size_bytes"
        }
    }

    struct CurrentChatAvailability: Codable, Sendable, Equatable {
        let identity: String
        let label: String
        let availability: String
    }

    struct CompletedApprovalPresentation: Codable, Sendable, Equatable {
        let identity: String
        let category: String
        let outcome: String
    }

    func configureStage8Acceptance(acquisition: String?, polish: String?) async throws {
        guard ProcessInfo.processInfo.environment[Stage8AcceptanceCLI.environmentKey] == "1" else {
            throw DaemonError.badStatus
        }
        let c = try await loadCreds()
        var req = authedRequest("/acceptance/stage8/fixture", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Selection: Encodable {
            let acquisition: String
            let polish: String
        }
        struct Body: Encodable {
            let selection: Selection?
        }
        req.httpBody = try JSONEncoder().encode(Body(selection: acquisition.map {
            Selection(acquisition: $0, polish: polish ?? "unavailable")
        }))
        let (data, response) = try await URLSession.shared.data(for: req)
        try validateOK(data: data, response: response)
    }

    func chatStream(
        text: String,
        currentChatIdentity: String,
        expectedRevision: UInt64,
        model: String,
        attachmentIds: [String] = [],
        screenContext: ScreenContextFields? = nil,
        sourceMode: SourceMode? = nil
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
                        let current_chat_identity: String
                        let expected_revision: UInt64
                        let attachment_ids: [String]
                        // OCR/selection context — ephemeral, never persisted
                        let screen_ocr_text: String?
                        let active_app: String?
                        let selected_text: String?
                        let source_mode: String?
                    }
                    req.httpBody = try JSONEncoder().encode(Body(
                        message: text,
                        model: model,
                        current_chat_identity: currentChatIdentity,
                        expected_revision: expectedRevision,
                        attachment_ids: attachmentIds,
                        screen_ocr_text: screenContext?.ocrText.isEmpty == false ? screenContext?.ocrText : nil,
                        active_app: screenContext?.activeApp,
                        selected_text: screenContext?.selectedText,
                        source_mode: sourceMode?.rawValue
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

                        if let evidenceEvent = Self.evidenceChatEvent(from: event) {
                            continuation.yield(evidenceEvent)
                            continue
                        }

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
                        case "activity_started", "activity_completed":
                            if let id = event.id, let kind = event.kind,
                               let title = event.title {
                                let activity = ActivityEvent(
                                    id: id,
                                    kind: kind,
                                    tool: event.tool,
                                    title: title,
                                    detail: event.detail,
                                    status: event.status,
                                    durationMs: event.duration_ms
                                )
                                continuation.yield(event.type == "activity_started"
                                    ? .activityStarted(activity)
                                    : .activityCompleted(activity))
                            }
                        case "source_discovered":
                            if let id = event.id, let title = event.title,
                               let rawURL = event.url, let url = URL(string: rawURL),
                               ["http", "https"].contains(url.scheme?.lowercased() ?? "") {
                                continuation.yield(.sourceDiscovered(TranscriptSource(
                                    id: id,
                                    title: title,
                                    url: url,
                                    domain: event.domain ?? url.host() ?? ""
                                )))
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
                        case "tool_call":
                            if let tool = event.tool {
                                continuation.yield(.toolCall(tool: tool))
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
                            continuation.yield(.done)
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

    // MARK: Current Chat

    func currentChat() async throws -> CurrentChatSnapshot {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(
            for: authedRequest("/current-chat", creds: c))
        try validateOK(data: data, response: response)
        return try JSONDecoder().decode(CurrentChatSnapshot.self, from: data)
    }

    func saveCurrentChatDraft(identity: String, expectedRevision: UInt64, text: String,
                              pendingAttachmentReferences: [String]) async throws -> CurrentChatSnapshot {
        struct Body: Encodable {
            let current_chat_identity: String
            let expected_revision: UInt64
            let text: String
            let pending_attachment_references: [String]
        }
        return try await currentChatMutation(path: "/current-chat/draft", body: Body(
            current_chat_identity: identity, expected_revision: expectedRevision, text: text,
            pending_attachment_references: pendingAttachmentReferences))
    }

    func clearCurrentChat(identity: String, expectedRevision: UInt64, commandIdentity: String,
                          confirmedNonEmpty: Bool) async throws -> CurrentChatSnapshot {
        struct Body: Encodable {
            let current_chat_identity: String
            let expected_revision: UInt64
            let command_identity: String
            let confirmed_non_empty: Bool
        }
        return try await currentChatMutation(path: "/current-chat/clear", body: Body(
            current_chat_identity: identity, expected_revision: expectedRevision,
            command_identity: commandIdentity, confirmed_non_empty: confirmedNonEmpty))
    }

    private func currentChatMutation<Body: Encodable>(path: String, body: Body) async throws -> CurrentChatSnapshot {
        let c = try await loadCreds()
        var request = authedRequest(path, creds: c)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw CurrentChatMutationError.unavailable
        }
        guard (200..<300).contains(http.statusCode) else {
            let code = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["code"] as? String
            switch code {
            case "current_chat_conflict", "current_chat_invalid":
                throw CurrentChatMutationError.conflict
            case "current_chat_bound":
                throw CurrentChatMutationError.bound
            default:
                throw CurrentChatMutationError.unavailable
            }
        }
        return try JSONDecoder().decode(CurrentChatSnapshot.self, from: data)
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

    // MARK: - Authoritative Work projection

    func fetchSnapshot(consumerFence: String) async throws -> NotchWorkSnapshot {
        let c = try await loadCreds()
        var components = URLComponents(
            url: authedRequest("/work/snapshot", creds: c).url!,
            resolvingAgainstBaseURL: false
        )!
        components.queryItems = [URLQueryItem(name: "consumer_fence", value: consumerFence)]
        var request = authedRequest("/work/snapshot", creds: c)
        request.url = components.url
        let (data, response) = try await URLSession.shared.data(for: request)
        if (response as? HTTPURLResponse)?.statusCode == 409 {
            throw NotchEventTransportError.consumerFenced
        }
        try validateOK(data: data, response: response)
        return try NotchProjectionDecoder.decodeSnapshot(data)
    }

    func fetchEvents(
        after cursor: UInt64,
        daemonGeneration: String,
        consumerFence: String
    ) async throws -> NotchEventBatch {
        let c = try await loadCreds()
        var components = URLComponents(
            url: authedRequest("/work/events", creds: c).url!,
            resolvingAgainstBaseURL: false
        )!
        components.queryItems = [
            URLQueryItem(name: "after", value: String(cursor)),
            URLQueryItem(name: "daemon_generation", value: daemonGeneration),
            URLQueryItem(name: "consumer_fence", value: consumerFence),
        ]
        var request = authedRequest("/work/events", creds: c)
        request.url = components.url
        let (data, response) = try await URLSession.shared.data(for: request)
        if (response as? HTTPURLResponse)?.statusCode == 409 {
            throw NotchEventTransportError.consumerFenced
        }
        try validateOK(data: data, response: response)

        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let kind = object["kind"] as? String
        else { throw DaemonError.badResponse }
        switch kind {
        case "events":
            guard Set(object.keys) == ["kind", "events"],
                  let values = object["events"] as? [Any]
            else { throw DaemonError.badResponse }
            return .events(try values.map { value in
                try NotchProjectionDecoder.decodeEvent(JSONSerialization.data(withJSONObject: value))
            })
        case "gap":
            guard Set(object.keys) == ["kind", "snapshot"],
                  let value = object["snapshot"]
            else { throw DaemonError.badResponse }
            return .gap(try NotchProjectionDecoder.decodeSnapshot(
                JSONSerialization.data(withJSONObject: value)
            ))
        default:
            throw DaemonError.badResponse
        }
    }

    enum WorkAttentionAcknowledgement: Equatable {
        case acknowledged
        case authoritativeConflict
    }

    static func decodeWorkAttentionAcknowledgement(
        statusCode: Int,
        data: Data
    ) throws -> WorkAttentionAcknowledgement {
        if (200..<300).contains(statusCode) { return .acknowledged }
        if statusCode == 409 {
            struct ErrorResponse: Decodable { let code: String }
            let code = try JSONDecoder().decode(ErrorResponse.self, from: data).code
            if code == "stale_consumer_fence" {
                throw NotchEventTransportError.consumerFenced
            }
            if code == "work_conflict" { return .authoritativeConflict }
            throw DaemonError.badResponse
        }
        throw DaemonError.badStatus
    }

    func acknowledgeWorkAttention(
        workIdentity: String,
        expectedRevision: UInt64,
        consumerFence: String
    ) async throws -> WorkAttentionAcknowledgement {
        let credentials = try await loadCreds()
        var request = authedRequest("/work/attention/acknowledge", creds: credentials)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable {
            let commandIdentity: String
            let consumerFence: String
            let workIdentity: String
            let expectedRevision: UInt64
        }
        request.httpBody = try JSONEncoder().encode(Body(
            commandIdentity: UUID().uuidString,
            consumerFence: consumerFence,
            workIdentity: workIdentity,
            expectedRevision: expectedRevision
        ))
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let statusCode = (response as? HTTPURLResponse)?.statusCode else {
            throw DaemonError.badResponse
        }
        return try Self.decodeWorkAttentionAcknowledgement(statusCode: statusCode, data: data)
    }

    // MARK: - Automations

    func listAutomations() async throws -> [AutomationRecord] {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(for: authedRequest("/automations", creds: c))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw DaemonError.badStatus }
        struct Resp: Decodable { let automations: [AutomationRecord] }
        return try JSONDecoder().decode(Resp.self, from: data).automations
    }

    func getAutomation(id: String) async throws -> AutomationRecord {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(for: authedRequest("/automations/\(id)", creds: c))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw DaemonError.badStatus }
        return try JSONDecoder().decode(AutomationRecord.self, from: data)
    }

    struct AutomationDraft: Encodable {
        var name: String
        var prompt: String
        var timezone: String
        var schedule: AutomationSchedule
        var enabled: Bool = true
    }

    func createAutomation(_ draft: AutomationDraft) async throws -> AutomationRecord {
        try await automationRequest(path: "/automations", method: "POST", body: draft)
    }

    struct AutomationPatch: Encodable {
        var name: String?
        var prompt: String?
        var timezone: String?
        var schedule: AutomationSchedule?
        var enabled: Bool?
    }

    func patchAutomation(id: String, _ patch: AutomationPatch) async throws -> AutomationRecord {
        try await automationRequest(path: "/automations/\(id)", method: "PATCH", body: patch)
    }

    private struct APIErrorBody: Decodable { let error: String }

    private func automationRequest<B: Encodable>(
        path: String, method: String, body: B
    ) async throws -> AutomationRecord {
        let c = try await loadCreds()
        var req = authedRequest(path, creds: c)
        req.httpMethod = method
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await URLSession.shared.data(for: req)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            let message = (try? JSONDecoder().decode(APIErrorBody.self, from: data))?.error
            throw DaemonError.serverError(message ?? "HTTP \(status)")
        }
        return try JSONDecoder().decode(AutomationRecord.self, from: data)
    }

    /// POST helper for enable/disable/run-now/delete-style endpoints.
    private func automationAction(path: String, method: String = "POST") async throws {
        let c = try await loadCreds()
        var req = authedRequest(path, creds: c)
        req.httpMethod = method
        let (data, response) = try await URLSession.shared.data(for: req)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            let message = (try? JSONDecoder().decode(APIErrorBody.self, from: data))?.error
            throw DaemonError.serverError(message ?? "HTTP \(status)")
        }
    }

    func enableAutomation(id: String) async throws { try await automationAction(path: "/automations/\(id)/enable") }
    func disableAutomation(id: String) async throws { try await automationAction(path: "/automations/\(id)/disable") }
    func deleteAutomation(id: String) async throws { try await automationAction(path: "/automations/\(id)", method: "DELETE") }
    func runNowAutomation(id: String) async throws { try await automationAction(path: "/automations/\(id)/run-now") }

    func automationRuns(id: String, limit: Int = 10) async throws -> [AutomationRunRecord] {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(
            for: authedRequest("/automations/\(id)/runs?limit=\(limit)", creds: c))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else { throw DaemonError.badStatus }
        struct Resp: Decodable { let runs: [AutomationRunRecord] }
        return try JSONDecoder().decode(Resp.self, from: data).runs
    }

    func automationRun(id: String, runIdentity: String) async throws -> AutomationRunRecord {
        let credentials = try await loadCreds()
        let path = "/automations/\(id)/runs/\(runIdentity)"
        let (data, response) = try await URLSession.shared.data(
            for: authedRequest(path, creds: credentials)
        )
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw DaemonError.badStatus
        }
        struct Response: Decodable { let run: AutomationRunRecord }
        return try JSONDecoder().decode(Response.self, from: data).run
    }

    func automationSession(identity: String) async throws -> AutomationSessionRecord {
        let c = try await loadCreds()
        let path = "/automation-sessions/\(identity)"
        let (data, response) = try await URLSession.shared.data(
            for: authedRequest(path, creds: c))
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw DaemonError.badStatus
        }
        return try JSONDecoder().decode(AutomationSessionRecord.self, from: data)
    }

    func openAutomationSession(
        identity: String,
        commandIdentity: String,
        expectedRevision: UInt64
    ) async throws {
        struct Body: Encodable {
            let commandIdentity: String
            let expectedRevision: UInt64
        }
        let credentials = try await loadCreds()
        var request = authedRequest("/automation-sessions/\(identity)/open", creds: credentials)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(
            Body(commandIdentity: commandIdentity, expectedRevision: expectedRevision))
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            throw DaemonError.serverError(serverErrorMessage(from: data))
        }
    }

    func continueAutomationSession(
        identity: String,
        seed: String,
        confirmedReplacement: Bool,
        commandIdentity: String
    ) async throws -> AutomationContinuationProvenance {
        let body = ContinueAutomationSessionBody(
            seed: seed,
            confirmedReplacement: confirmedReplacement,
            commandIdentity: commandIdentity)
        let c = try await loadCreds()
        var request = authedRequest(
            "/automation-sessions/\(identity)/continue", creds: c)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await URLSession.shared.data(for: request)
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200..<300).contains(status) else {
            throw DaemonError.serverError(serverErrorMessage(from: data))
        }
        return try JSONDecoder().decode(AutomationContinuationProvenance.self, from: data)
    }

    func deleteAutomationSession(identity: String) async throws {
        try await automationAction(
            path: "/automation-sessions/\(identity)", method: "DELETE")
    }

    private struct ContinueAutomationSessionBody: Encodable {
        let seed: String
        let confirmedReplacement: Bool
        let commandIdentity: String

        enum CodingKeys: String, CodingKey {
            case seed
            case confirmedReplacement = "confirmedReplacement"
            case commandIdentity = "commandIdentity"
        }
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

    // MARK: - Odoo (Phase 6B — MCP)

    func configureTavily(apiKey: String?) async throws -> TavilyConfigurationStatus {
        let c = try await loadCreds()
        var req = authedRequest("/web/tavily/config", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable { let api_key: String? }
        req.httpBody = try JSONEncoder().encode(Body(api_key: apiKey))
        let (data, response) = try await URLSession.shared.data(for: req)
        try validateOK(data: data, response: response)
        struct Resp: Decodable { let status: TavilyConfigurationStatus }
        return try JSONDecoder().decode(Resp.self, from: data).status
    }

    struct OdooConfigResult: Decodable, Sendable {
        let ok: Bool
        let version: String?
        let uid: Int?
        let error: String?
        /// `false` means uvx/uv is not installed — show install hint.
        let mcp_available: Bool?
        let tool_count: Int?
    }

    struct OdooStatusResult: Decodable, Sendable {
        let configured: Bool
        let connected: Bool
        let version: String?
        let uid: Int?
        let error: String?
        let mcp_available: Bool?
        let tool_count: Int?
    }

    /// Authenticate via MCP and store the connector in-memory.
    /// Also used as the Settings "Testovať Odoo" action — returns version + mcp_available on success.
    func odooConfigure(
        url: String, db: String, user: String, apiKey: String,
        uvxPath: String? = nil
    ) async throws -> OdooConfigResult {
        let c = try await loadCreds()
        var req = authedRequest("/odoo/config", creds: c)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        struct Body: Encodable {
            let base_url: String
            let db: String
            let username: String
            let api_key: String
            let uvx_path: String?
        }
        req.httpBody = try JSONEncoder().encode(
            Body(base_url: url, db: db, username: user, api_key: apiKey, uvx_path: uvxPath)
        )
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
        let last_loading: WhatsappLoadingState?
        let last_state: String?
    }

    struct WhatsappLoadingState: Decodable, Sendable {
        let percent: Double?
        let message: String?
        let at: String?
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
        let (data, response) = try await URLSession.shared.data(for: authedRequest("/whatsapp/status", creds: c))
        try validateOK(data: data, response: response)
        return try JSONDecoder().decode(WhatsappStatusResult.self, from: data)
    }

    func whatsappStart() async throws {
        let c = try await loadCreds()
        var req = authedRequest("/whatsapp/start", creds: c)
        req.httpMethod = "POST"
        let (data, response) = try await URLSession.shared.data(for: req)
        try validateOK(data: data, response: response)
    }

    func whatsappStop() async throws {
        let c = try await loadCreds()
        var req = authedRequest("/whatsapp/stop", creds: c)
        req.httpMethod = "POST"
        let (data, response) = try await URLSession.shared.data(for: req)
        try validateOK(data: data, response: response)
    }

    func whatsappQr() async throws -> WhatsappQrResult {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(for: authedRequest("/whatsapp/qr", creds: c))
        try validateOK(data: data, response: response)
        return try JSONDecoder().decode(WhatsappQrResult.self, from: data)
    }

    func whatsappDebug() async throws -> String {
        let c = try await loadCreds()
        let (data, response) = try await URLSession.shared.data(for: authedRequest("/whatsapp/debug", creds: c))
        try validateOK(data: data, response: response)
        return prettyJSONString(data)
    }

    func whatsappLogout() async throws {
        let c = try await loadCreds()
        var req = authedRequest("/whatsapp/logout", creds: c)
        req.httpMethod = "POST"
        let (data, response) = try await URLSession.shared.data(for: req)
        try validateOK(data: data, response: response)
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

    func evidenceDiagnosticExport(turnId: String) async throws -> String {
        let c = try await loadCreds()
        let encoded = turnId.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? turnId
        let (data, response) = try await URLSession.shared.data(
            for: authedRequest("/diagnostics/evidence/\(encoded)/export", creds: c)
        )
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
    /// Provenance for approvals raised by unattended automations.
    var origin: ApprovalOrigin?

    enum CodingKeys: String, CodingKey {
        case id, description, origin
        case toolName  = "tool_name"
        case expiresAt = "expires_at"
        case createdAt = "created_at"
    }
}

struct ApprovalOrigin: Decodable, Sendable {
    let kind: String
    let automationName: String?

    enum CodingKeys: String, CodingKey {
        case kind
        case automationName = "automation_name"
    }
}

enum DaemonError: LocalizedError {
    case notReady
    case badStatus
    case badResponse
    case serverError(String)

    var errorDescription: String? {
        switch self {
        case .notReady:           return "Daemon sa nespustil včas"
        case .badStatus:          return "Neplatná odpoveď od daemona"
        case .badResponse:        return "Neplatný formát odpovede daemona"
        case .serverError(let m): return "Chyba servera: \(m)"
        }
    }
}

struct SSEEvent: Decodable {
    let type: String
    let content: String?
    let message: String?
    let id: String?
    let session_id: String?
    let tool: String?
    let description: String?
    let title: String?
    let detail: String?
    let status: String?
    let duration_ms: Int?
    // Evidence event fields
    let turn_id: String?
    let phase: String?
    let completed: Int?
    let total: Int?
    let activity_id: String?
    let normalized_operation: String?
    let argument_hash: String?
    let execution_status: String?
    let contribution: String?
    let evidence_count: Int?
    let source_domains: [String]?
    let attempt_count: Int?
    let retries: Int?
    let duplicates_suppressed: Int?
    let failure_reason: String?
    let state: String?
    let acquired: Int?
    let requested: Int?
    let source_count: Int?
    let provider: String?
    let provider_status: String?
    let domain: String?
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

    var outcome_kind: String? {
        guard type == "evidence_outcome" else { return nil }
        return kind
    }
}
