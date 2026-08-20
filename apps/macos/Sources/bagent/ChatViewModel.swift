import Combine
import ScreenCaptureKit
import SwiftUI
import UniformTypeIdentifiers

// MARK: - Attachment types

enum ChatAttachmentKind: String, Sendable {
    case image, pdf, text, other
}

enum ChatAttachmentAvailability: Sendable, Equatable {
    case available
    case unavailable
}

struct ChatAttachment: Identifiable, @unchecked Sendable {
    let id: String          // server-assigned UUID
    let filename: String
    let mime: String
    let kind: ChatAttachmentKind
    /// Local URL where the original file lives (for thumbnail generation).
    let localURL: URL?
    let sizeBytes: Int
    let availability: ChatAttachmentAvailability
    /// Base-64 encoded thumbnail (JPEG, max 120×120) for image attachments.
    var thumbnail: NSImage? = nil
}

struct ChatMessage: Identifiable, @unchecked Sendable {
    let id = UUID()
    let role: Role
    var content: String
    /// Presentation prefix paced independently from BaseRT transport chunks.
    var displayedContent: String = ""
    var activities: [TurnActivity] = []
    var evidencePhase: DaemonClient.EvidencePhaseEvent? = nil
    var evidenceActivities: [EvidenceLogicalActivity] = []
    var evidenceOutcome: DaemonClient.EvidenceOutcomeEvent? = nil
    var evidencePolishStatus: DaemonClient.EvidencePolishStatus? = nil
    var sources: [DaemonClient.TranscriptSource] = []
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

struct EvidenceLogicalActivity: Identifiable, Equatable {
    let id: String
    var operation: String
    var argumentHash: String
    var executionStatus: DaemonClient.EvidenceExecutionStatus
    var contribution: DaemonClient.EvidenceContribution
    var evidenceCount: Int
    var sourceDomains: [String]
    var durationMs: Int
    var attemptCount: Int
    var retries: Int
    var duplicatesSuppressed: Int
    var failureReason: String?

    init(event: DaemonClient.LogicalActivityEvent) {
        id = event.activityId
        operation = event.normalizedOperation
        argumentHash = event.argumentHash
        executionStatus = event.executionStatus
        contribution = event.contribution
        evidenceCount = event.evidenceCount
        sourceDomains = event.sourceDomains
        durationMs = event.durationMs
        attemptCount = event.attemptCount
        retries = event.retries
        duplicatesSuppressed = event.duplicatesSuppressed
        failureReason = event.failureReason
    }

    mutating func update(from event: DaemonClient.LogicalActivityEvent) {
        operation = event.normalizedOperation
        argumentHash = event.argumentHash
        executionStatus = event.executionStatus
        contribution = event.contribution
        evidenceCount = event.evidenceCount
        sourceDomains = event.sourceDomains
        durationMs = event.durationMs
        attemptCount = event.attemptCount
        retries = event.retries
        duplicatesSuppressed = event.duplicatesSuppressed
        failureReason = event.failureReason
    }
}

enum EvidencePresentation {
    @discardableResult
    static func apply(_ event: DaemonClient.ChatEvent, to message: inout ChatMessage) -> String? {
        switch event {
        case .evidencePhase(let phase):
            message.evidencePhase = phase
            return phaseLabel(phase)
        case .logicalActivityStarted(let activity), .logicalActivityCompleted(let activity):
            if let index = message.evidenceActivities.firstIndex(where: { $0.id == activity.activityId }) {
                message.evidenceActivities[index].update(from: activity)
            } else {
                message.evidenceActivities.append(EvidenceLogicalActivity(event: activity))
            }
            return nil
        case .evidenceOutcome(let outcome):
            message.evidenceOutcome = outcome
            return outcomeLabel(outcome)
        case .evidencePolish(let polish):
            message.evidencePolishStatus = polish.status
            return nil
        default:
            return nil
        }
    }

    static func phaseLabel(_ event: DaemonClient.EvidencePhaseEvent) -> String {
        let progress = progressSuffix(completed: event.completed, total: event.total)
        switch event.phase {
        case .findingMail: return "Finding Mail\(progress)"
        case .reading: return "Reading\(progress)"
        case .searching: return "Searching\(progress)"
        case .verifying: return "Verifying\(progress)"
        case .loadingSynthesisModel: return "Loading synthesis model"
        case .preparingAnswer: return "Preparing answer"
        case .repairing: return "Repairing answer"
        case .fallingBack: return "Falling back"
        case .validating: return "Validating answer"
        case .deterministicRendering: return "Preparing verified result"
        }
    }

    static func outcomeLabel(_ outcome: DaemonClient.EvidenceOutcomeEvent) -> String {
        outcome.message
    }

    static func activityDetail(_ activity: EvidenceLogicalActivity) -> String {
        var parts = [activity.contribution.rawValue]
        if activity.evidenceCount > 0 {
            parts.append("\(activity.evidenceCount) evidence")
        }
        if !activity.sourceDomains.isEmpty {
            parts.append(activity.sourceDomains.joined(separator: ", "))
        }
        if activity.retries > 0 {
            parts.append("\(activity.retries) retries")
        }
        if activity.duplicatesSuppressed > 0 {
            parts.append("\(activity.duplicatesSuppressed) duplicates suppressed")
        }
        if let failure = activity.failureReason {
            parts.append(failure)
        }
        return parts.joined(separator: " · ")
    }

    static func accessibilityLabel(
        outcome: DaemonClient.EvidenceOutcomeEvent,
        expanded: Bool
    ) -> String {
        "\(expanded ? "Collapse" : "Expand") evidence activity. \(outcomeLabel(outcome))"
    }

    private static func progressSuffix(completed: Int?, total: Int?) -> String {
        guard let completed, let total, total > 0 else { return "" }
        return " \(completed) of \(total)"
    }
}

struct TurnActivity: Identifiable, Equatable {
    let id: String
    var kind: String
    var tool: String?
    var title: String
    var detail: String?
    var status: String
    var durationMs: Int?
}

enum AgentStatus {
    case ready, thinking, awaitingApproval

    var color: Color {
        switch self {
        case .ready:            return Color(red: 0.18, green: 0.80, blue: 0.44)
        case .thinking:         return Color(red: 0.20, green: 0.60, blue: 1.00)
        case .awaitingApproval: return Color(red: 1.00, green: 0.78, blue: 0.15)
        }
    }

    var accessibilityLabel: String {
        switch self {
        case .ready:            return "Pripravený"
        case .thinking:         return "Spracováva"
        case .awaitingApproval: return "Čaká na schválenie"
        }
    }
}

/// Step-based state of the `/automations` surface (single source of truth
/// stays NotchInteractionMode — this only selects what renders inside it).
enum AutomationsSurfaceState: Equatable {
    case list
    case detail(String)
    case deleteConfirmation(String)
    // Step-based editor (create + edit). Divided into compact steps instead
    // of scrolling — each fits the fixed notch bridge.
    case editorTask
    case editorSchedule
    case editorRecurrence
    case editorReview
    case editorSaving
}

/// Which day a run-once automation fires. Backend stays authoritative for
/// validation; this only builds the request.
enum AutomationDayChoice: Equatable {
    case today
    case tomorrow
    case custom(Date)
}

/// How the drafted automation repeats. Structured — no cron strings.
enum AutomationDraftRecurrence: Equatable {
    case once
    case everyNHours(Int)
    case daily
    case weekdays
    case selectedWeekdays(Set<String>)   // lowercase wire days: mon…sun
    case weekly(String)

    var isOnce: Bool { self == .once }
}

/// Editor draft — typed state, no scattered booleans.
struct AutomationDraft: Equatable {
    var editingID: String? = nil
    var name = ""
    var prompt = ""
    var day: AutomationDayChoice = .today
    var hour: Int
    var minute: Int
    var recurrence: AutomationDraftRecurrence = .once
    var enabled = true

    /// Default: the next full hour.
    init(now: Date = Date(), calendar: Calendar = .current) {
        let next = calendar.date(byAdding: .hour, value: 1, to: now) ?? now
        hour = calendar.component(.hour, from: next)
        minute = 0
    }
}

enum AutomationDraftBuilder {
    /// Resolve the draft's local wall-clock choice to a concrete Date.
    static func scheduledDate(
        _ draft: AutomationDraft, calendar: Calendar = .current, now: Date = Date()
    ) -> Date? {
        let base: Date
        switch draft.day {
        case .today: base = now
        case .tomorrow:
            guard let t = calendar.date(byAdding: .day, value: 1, to: now) else { return nil }
            base = t
        case .custom(let d): base = d
        }
        var comps = calendar.dateComponents([.year, .month, .day], from: base)
        comps.hour = draft.hour
        comps.minute = draft.minute
        comps.second = 0
        return calendar.date(from: comps)
    }

    static func isoUTC(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        return f.string(from: date)
    }

    /// Structured schedule for the daemon; recurrence times are local
    /// wall-clock in the automation's zone (backend calculates occurrences).
    static func schedule(
        _ draft: AutomationDraft, calendar: Calendar = .current, now: Date = Date()
    ) -> AutomationSchedule? {
        let localTime = String(format: "%02d:%02d:00", draft.hour, draft.minute)
        switch draft.recurrence {
        case .once:
            guard let at = scheduledDate(draft, calendar: calendar, now: now) else { return nil }
            return .once(at: isoUTC(at))
        case .everyNHours(let n):
            return .recurring(rule: RecurrenceRuleWire(
                type: "every_n_hours", hours: n, time: nil, day: nil, days: nil))
        case .daily:
            return .recurring(rule: RecurrenceRuleWire(
                type: "daily", hours: nil, time: localTime, day: nil, days: nil))
        case .weekdays:
            return .recurring(rule: RecurrenceRuleWire(
                type: "weekdays", hours: nil, time: localTime, day: nil, days: nil))
        case .selectedWeekdays(let days):
            guard !days.isEmpty else { return nil }
            let order = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
            return .recurring(rule: RecurrenceRuleWire(
                type: "selected_weekdays", hours: nil, time: localTime, day: nil,
                days: order.filter { days.contains($0) }))
        case .weekly(let day):
            return .recurring(rule: RecurrenceRuleWire(
                type: "weekly", hours: nil, time: localTime, day: day, days: nil))
        }
    }

    /// Wire draft for POST /automations.
    static func wireDraft(
        _ draft: AutomationDraft, timezone: String = TimeZone.current.identifier,
        calendar: Calendar = .current, now: Date = Date()
    ) -> DaemonClient.AutomationDraft? {
        guard let schedule = schedule(draft, calendar: calendar, now: now) else { return nil }
        return DaemonClient.AutomationDraft(
            name: draft.name.trimmingCharacters(in: .whitespacesAndNewlines),
            prompt: draft.prompt.trimmingCharacters(in: .whitespacesAndNewlines),
            timezone: timezone,
            schedule: schedule,
            enabled: draft.enabled
        )
    }
}

enum SourceMode: String, CaseIterable, Equatable, Identifiable {
    case mail
    case filesystem
    case whatsapp
    case odoo

    var id: String { rawValue }

    var title: String {
        switch self {
        case .mail:       return "Mail"
        case .filesystem: return "Files"
        case .whatsapp:   return "WhatsApp"
        case .odoo:       return "Odoo"
        }
    }

    var placeholder: String {
        switch self {
        case .mail:       return "Mail"
        case .filesystem: return "Files"
        case .whatsapp:   return "WhatsApp"
        case .odoo:       return "Odoo"
        }
    }

    var symbolName: String {
        switch self {
        case .mail:       return "envelope"
        case .filesystem: return "folder"
        case .whatsapp:   return "message"
        case .odoo:       return "building.2"
        }
    }
}

@MainActor
final class ChatViewModel: ObservableObject {
    enum TerminalAcknowledgementDelivery {
        case authoritative(DaemonClient.WorkAttentionAcknowledgement)
        case retryableFailure
    }
    @Published var messages: [ChatMessage] = []
    @Published var inputText: String = "" {
        didSet {
            guard !isApplyingCurrentChatSnapshot else { return }
            guard inputText.utf8.count <= 16 * 1024 else {
                isApplyingCurrentChatSnapshot = true
                inputText = oldValue
                isApplyingCurrentChatSnapshot = false
                slashCommandError = String(localized: "currentChat.draft.limit", defaultValue: "Current Chat Draft is limited to 16 KiB")
                return
            }
            slashCommandError = nil
            updateSlashSuggestions()
            scheduleDraftPersistence()
        }
    }
    @Published var hasUncommittedMarkedText = false {
        didSet { updateSlashSuggestions() }
    }
    var notchPresentation: NotchPresentation { notchEventConsumer.presentation }
    var notchPresentationPublisher: AnyPublisher<NotchPresentation, Never> {
        notchEventConsumer.$presentation.eraseToAnyPublisher()
    }
    var notchInteractionMode: NotchInteractionMode { notchPresentation.interactionMode }
    var isThinking: Bool { notchPresentation.isThinking }
    var isExpanded: Bool { notchPresentation.isExpanded }
    var toolStatus: String? {
        notchPresentation.rail.selectedStage == .tool ? notchPresentation.rail.caption : nil
    }
    var authoritativePendingApproval: ApprovalItem? {
        guard let identity = notchPresentation.pendingApprovalIdentity else { return nil }
        return pendingApprovals.first(where: { $0.id == identity })
    }
    @Published var notchHoverResetID = UUID()
    @Published var selectedSourceMode: SourceMode? = nil
    @Published var hoveredSourceMode: SourceMode? = nil
    @Published var isSourcePickerForced = false
    @Published var hasNotch = false
    @Published var daemonHealth: DaemonHealth?
    @Published var isSyncing = false
    @Published var lastSyncResult: String? = nil
    @Published var streamingChunk: Int = 0
    @Published var pendingApprovals: [ApprovalItem] = []
    /// Files queued to send with the next message.
    @Published var pendingAttachments: [ChatAttachment] = []
    @Published private(set) var restoredPendingAttachmentReferences: [String] = []
    @Published private(set) var restoredSubmittedAttachments: [ChatAttachment] = []
    @Published private(set) var restoredValidatedSources: [DaemonClient.CurrentChatAvailability] = []
    @Published private(set) var restoredConnectorReferences: [DaemonClient.CurrentChatAvailability] = []
    @Published private(set) var restoredApprovalPresentations: [DaemonClient.CompletedApprovalPresentation] = []
    /// True while uploading a file to the daemon.
    @Published var isUploadingAttachment = false
    @Published var streamingAssistantMessageId: UUID? = nil
    @Published var isActivityTranscriptExpanded = false
    @Published private(set) var currentChatSnapshot: DaemonClient.CurrentChatSnapshot?
    @Published var clearCurrentChatConfirmationPresented = false
    @Published var currentChatFocusRequestID = UUID()
    private var pendingCurrentChatCaretRestoration = false
    private var clearCurrentChatCommandIdentity: String?
    private var draftPersistenceTask: Task<Void, Never>?
    private var isApplyingCurrentChatSnapshot = false

    /// Set to true by NotchWindowController before expanding so the pill
    /// animates to its hover state before the chat panel appears.
    @Published var pillHovered = false

    /// The sole writable settings-selection state for the Compass Rail.
    @Published var compassRailRoute: CompassRailRoute = .initial

    /// Open the notch-style settings surface (`/settings` command).
    func openNotchSettings() {
        inputText = ""
        historyBrowseIndex = nil
        compassRailRoute = .initial
        applyNotchIntent(.openSettings)
    }

    func selectCompassRailArea(_ area: CompassRailArea) {
        compassRailRoute = .area(area)
    }

    func openCompassRailChild(_ child: CompassRailChild) {
        compassRailRoute = .child(child)
    }

    @discardableResult
    func goBackInCompassRail() -> Bool {
        guard let parent = compassRailRoute.parent else { return false }
        compassRailRoute = parent
        return true
    }

    /// Routes settings-only keyboard commands after the native editor has had
    /// the first opportunity to keep its own left/right keys.
    @discardableResult
    func handleCompassRailKey(
        _ key: CompassRailKey,
        focusedControl: CompassRailFocusedControl?
    ) -> CompassRailKeyboardAction? {
        guard notchInteractionMode == .settings else { return nil }
        guard let action = CompassRailKeyboard.route(
            key,
            route: compassRailRoute,
            focusedControl: focusedControl
        ) else { return nil }
        switch action {
        case .select(let area): selectCompassRailArea(area)
        case .back: _ = goBackInCompassRail()
        case .collapse: break
        }
        return action
    }

    // MARK: - Automations surface (/automations)

    @Published var automationsSurface: AutomationsSurfaceState = .list {
        didSet {
            guard let pendingTerminalAcknowledgement else { return }
            guard case .detail(let identity) = automationsSurface,
                  identity == pendingTerminalAcknowledgement.definitionIdentity
            else {
                focusedAutomationSessionIdentity = nil
                self.pendingTerminalAcknowledgement = nil
                return
            }
        }
    }
    @Published var automations: [AutomationRecord] = []
    @Published var automationSessionNavigator = AutomationSplitViewNavigator(
        projection: AutomationSplitViewProjection.make(active: [], unreadTerminal: []))
    @Published var automationSessionDetail: AutomationSessionRecord?
    @Published var automationContinuationConfirmation: AutomationContinuationConfirmation?
    @Published var automationsSelectionIndex: Int = 0
    @Published var automationsError: String? = nil
    /// True while a run-now/enable/delete request is in flight.
    @Published var automationsBusy = false

    /// Open the `/automations` surface on its list.
    func openAutomations() {
        inputText = ""
        historyBrowseIndex = nil
        automationsSurface = .list
        automationsSelectionIndex = 0
        automationsError = nil
        focusedAutomationSessionIdentity = nil
        pendingTerminalAcknowledgement = nil
        automationContinuationConfirmation = nil
        automationSessionNavigator.resetToSplit()
        refreshAutomationSessionProjection()
        applyNotchIntent(.openAutomations)
        Task { await refreshAutomations() }
    }

    func openAutomationDetail(_ identity: String, focusedSessionIdentity: String? = nil) {
        inputText = ""
        historyBrowseIndex = nil
        automationsSurface = .detail(identity)
        automationsError = nil
        if focusedSessionIdentity == nil {
            pendingTerminalAcknowledgement = nil
        }
        focusedAutomationSessionIdentity = focusedSessionIdentity
        applyNotchIntent(.openAutomations)
        Task { await refreshAutomations() }
    }

    /// Refetch the authoritative list; handles records deleted elsewhere by
    /// falling back to the list when an open detail disappears.
    func refreshAutomations() async {
        do {
            automations = try await client.listAutomations()
            automationsError = nil
            if automationsSelectionIndex >= automations.count {
                automationsSelectionIndex = max(0, automations.count - 1)
            }
            switch automationsSurface {
            case .detail(let id), .deleteConfirmation(let id):
                // The split view's detail identity is an Automation Session,
                // not an Automation Definition. Keep it open while the
                // definition list refreshes.
                _ = id
            case .list, .editorTask, .editorSchedule, .editorRecurrence, .editorReview,
                 .editorSaving:
                // Editor keeps its draft through concurrent SSE updates; an
                // edited record deleted elsewhere fails at save with 404.
                break
            }
        } catch {
            automationsError = "Daemon nedostupný"
        }
        refreshAutomationSessionProjection()
    }

    func refreshAutomationSessionProjection() {
        guard automationSessionNavigator.depth == .split else { return }
        let next = AutomationSplitViewNavigator(
            projection: AutomationSplitViewProjection.from(notchPresentation.snapshot))
        let currentHasSessionRows = automationSessionNavigator.rows.contains { $0.kind != .history }
        guard currentHasSessionRows else {
            automationSessionNavigator = next
            return
        }
        let selected = automationSessionNavigator.selectedRow
        let matchingRow = next.rows.first {
            $0.id == selected?.id || (
                selected?.workIdentity != nil && $0.workIdentity == selected?.workIdentity
            )
        }
        if next.rows != automationSessionNavigator.rows {
            var updated = next
            if let matchingRow {
                _ = updated.select(rowID: matchingRow.id)
            }
            automationSessionNavigator = updated
        }
    }

    @Published private(set) var focusedAutomationSessionIdentity: String?
    var pendingTerminalAcknowledgement: (
        definitionIdentity: String,
        sessionIdentity: String,
        workIdentity: String,
        expectedRevision: UInt64
    )?

    func acknowledgeFocusedAutomationSessionIfPresented(runIdentity: String) {
        guard let pendingTerminalAcknowledgement,
              "automation-session:\(runIdentity)" == pendingTerminalAcknowledgement.sessionIdentity
        else { return }
        Task {
            do {
                let outcome = try await client.acknowledgeWorkAttention(
                    workIdentity: pendingTerminalAcknowledgement.workIdentity,
                    expectedRevision: pendingTerminalAcknowledgement.expectedRevision,
                    consumerFence: notchEventConsumer.activeConsumerFence
                )
                handlePendingTerminalAcknowledgement(
                    sessionIdentity: pendingTerminalAcknowledgement.sessionIdentity,
                    delivery: .authoritative(outcome)
                )
            } catch {
                handlePendingTerminalAcknowledgement(
                    sessionIdentity: pendingTerminalAcknowledgement.sessionIdentity,
                    delivery: .retryableFailure
                )
            }
        }
    }

    func handlePendingTerminalAcknowledgement(
        sessionIdentity: String,
        delivery: TerminalAcknowledgementDelivery
    ) {
        guard pendingTerminalAcknowledgement?.sessionIdentity == sessionIdentity else { return }
        if case .authoritative = delivery {
            pendingTerminalAcknowledgement = nil
        }
    }

    private func openAutomationSession(
        definitionIdentity: String,
        sessionIdentity: String,
        workIdentity: String,
        expectedRevision: UInt64
    ) {
        pendingTerminalAcknowledgement = (
            definitionIdentity,
            sessionIdentity,
            workIdentity,
            expectedRevision
        )
        openAutomationDetail(definitionIdentity, focusedSessionIdentity: sessionIdentity)
        acknowledgeFocusedAutomationSessionIfPresented(runIdentity: String(sessionIdentity.dropFirst("automation-session:".count)))
    }

    /// Explicit opening from the Automation Session split view. Preview rows
    /// never call this path, so selection and passive projection updates do not
    /// acknowledge Completion Attention.
    func openTerminalAutomationSession(_ row: AutomationMasterRow) {
        guard let sessionIdentity = row.sessionIdentity,
              let workIdentity = row.workIdentity,
              let runIdentity = row.runIdentity
        else { return }
        pendingTerminalAcknowledgement = (
            definitionIdentity: row.definitionIdentity ?? "detached",
            sessionIdentity: sessionIdentity,
            workIdentity: workIdentity,
            expectedRevision: row.workRevision
        )
        openAutomationDetail(
            row.definitionIdentity ?? "detached",
            focusedSessionIdentity: sessionIdentity
        )
        acknowledgeFocusedAutomationSessionIfPresented(runIdentity: runIdentity)
        Task {
            try? await client.openAutomationSession(
                identity: sessionIdentity,
                commandIdentity: "open-\(sessionIdentity)-\(row.workRevision)",
                expectedRevision: row.workRevision)
            automationSessionDetail = try? await client.automationSession(identity: sessionIdentity)
        }
    }

    func continueAutomationSessionFromDetail() {
        guard let detail = automationSessionDetail else { return }
        let sessionIdentity = detail.taskSnapshot.automationSessionIdentity
        let seed = detail.finalOutput ?? detail.resultSummary ?? detail.taskSnapshot.taskText
        if currentChatHasContent {
            automationContinuationConfirmation = AutomationContinuationConfirmation(
                sessionIdentity: sessionIdentity,
                seed: seed)
            return
        }
        performAutomationContinuation(
            sessionIdentity: sessionIdentity,
            seed: seed,
            confirmedReplacement: false)
    }

    func confirmAutomationContinuation() {
        guard let confirmation = automationContinuationConfirmation,
              automationSessionDetail != nil else { return }
        automationContinuationConfirmation = nil
        performAutomationContinuation(
            sessionIdentity: confirmation.sessionIdentity,
            seed: confirmation.seed,
            confirmedReplacement: true)
    }

    func cancelAutomationContinuation() {
        automationContinuationConfirmation = nil
        _ = automationSessionNavigator.goBack()
    }

    private func performAutomationContinuation(
        sessionIdentity: String,
        seed: String,
        confirmedReplacement: Bool
    ) {
        Task {
            do {
                _ = try await client.continueAutomationSession(
                    identity: sessionIdentity,
                    seed: seed,
                    confirmedReplacement: confirmedReplacement,
                    commandIdentity: "continue-" + UUID().uuidString)
                await MainActor.run {
                    historyBrowseIndex = nil
                    automationSessionDetail = nil
                    automationSessionNavigator.resetToSplit()
                    applyNotchIntent(.openInput)
                }
                await restoreCurrentChat()
            } catch {
                await MainActor.run { automationsError = "Pokračovanie zlyhalo" }
            }
        }
    }

    func deleteAutomationSessionFromDetail() {
        guard let detail = automationSessionDetail else { return }
        Task {
            if (try? await client.deleteAutomationSession(
                identity: detail.taskSnapshot.automationSessionIdentity)) != nil {
                automationSessionDetail = nil
                refreshAutomationSessionProjection()
            }
        }
    }

    var selectedAutomation: AutomationRecord? {
        switch automationsSurface {
        case .detail(let id), .deleteConfirmation(let id):
            return automations.first { $0.id == id }
        default:
            return automations[safe: automationsSelectionIndex]
        }
    }

    /// Escape steps back one level; returns false at the list (caller collapses).
    func automationsGoBack() -> Bool {
        if automationSessionNavigator.depth != .split {
            if automationContinuationConfirmation != nil {
                automationContinuationConfirmation = nil
            }
            let didGoBack = automationSessionNavigator.goBack()
            if didGoBack, automationSessionNavigator.depth == .split {
                refreshAutomationSessionProjection()
            }
            return didGoBack
        }
        switch automationsSurface {
        case .deleteConfirmation(let id):
            automationsSurface = .detail(id)
            return true
        case .detail:
            automationsSurface = .list
            focusedAutomationSessionIdentity = nil
            pendingTerminalAcknowledgement = nil
            return true
        case .editorTask:
            // Leaving the editor discards the draft.
            automationsSurface = automationDraft.editingID.map { .detail($0) } ?? .list
            return true
        case .editorSchedule:
            automationsSurface = .editorTask
            return true
        case .editorRecurrence:
            automationsSurface = .editorSchedule
            return true
        case .editorReview:
            automationsSurface = .editorRecurrence
            return true
        case .editorSaving:
            return true // saving is brief; swallow Escape
        case .list:
            return false
        }
    }

    func moveAutomationsSelection(by delta: Int) -> Bool {
        guard notchInteractionMode == .automations, automationsSurface == .list else {
            return false
        }
        if automationSessionNavigator.rows.contains(where: { $0.kind != .history }) {
            return automationSessionNavigator.moveSelection(by: delta)
        }
        guard !automations.isEmpty else { return false }
        let count = automations.count
        automationsSelectionIndex = (automationsSelectionIndex + delta + count) % count
        return true
    }

    func openSelectedAutomationDetail() -> Bool {
        guard notchInteractionMode == .automations, automationsSurface == .list else {
            return false
        }
        let hasSessionRows = automationSessionNavigator.rows.contains { $0.kind != .history }
        if hasSessionRows,
           let row = automationSessionNavigator.selectedRow,
           row.kind != .history {
            if row.kind == .unreadTerminal {
                guard automationSessionNavigator.openSelectedTerminal() else { return false }
                openTerminalAutomationSession(row)
            } else {
                guard automationSessionNavigator.openSelectedActive() else { return false }
                automationsSurface = .detail("automation-session-split")
            }
            return true
        }
        if hasSessionRows, automationSessionNavigator.selectedRow?.kind == .history {
            automationSessionNavigator.openChild("history")
            return true
        }
        guard let a = automations[safe: automationsSelectionIndex] else { return false }
        automationsSurface = .detail(a.id)
        return true
    }

    func setAutomationEnabled(_ automation: AutomationRecord, enabled: Bool) {
        automationsBusy = true
        Task {
            defer { automationsBusy = false }
            do {
                if enabled {
                    try await client.enableAutomation(id: automation.id)
                } else {
                    try await client.disableAutomation(id: automation.id)
                }
                await refreshAutomations()
            } catch {
                automationsError = "Zmena zlyhala"
            }
        }
    }

    func runAutomationNow(_ automation: AutomationRecord) {
        automationsBusy = true
        Task {
            defer { automationsBusy = false }
            do {
                try await client.runNowAutomation(id: automation.id)
                await refreshAutomations()
            } catch {
                automationsError = (error as? DaemonError).flatMap { e in
                    if case .serverError(let m) = e { return m } else { return nil }
                } ?? "Spustenie zlyhalo"
            }
        }
    }

    func deleteAutomation(_ automation: AutomationRecord) {
        automationsBusy = true
        Task {
            defer { automationsBusy = false }
            do {
                try await client.deleteAutomation(id: automation.id)
                automationsSurface = .list
                await refreshAutomations()
            } catch {
                automationsError = "Vymazanie zlyhalo (beží?)"
            }
        }
    }

    // MARK: Automation editor (create + edit)

    @Published var automationDraft = AutomationDraft()

    func startAutomationCreation() {
        automationDraft = AutomationDraft()
        automationsError = nil
        automationSessionNavigator.resetToSplit()
        automationsSurface = .editorTask
    }

    func startAutomationEdit(_ automation: AutomationRecord) {
        automationSessionNavigator.resetToSplit()
        var draft = AutomationDraft()
        draft.editingID = automation.id
        draft.name = automation.name
        draft.prompt = automation.prompt
        draft.enabled = automation.enabled
        switch automation.schedule {
        case .once:
            if let date = AutomationTimeFormat.parse(automation.nextRunAt) {
                let cal = Calendar.current
                if cal.isDateInToday(date) {
                    draft.day = .today
                } else if cal.isDateInTomorrow(date) {
                    draft.day = .tomorrow
                } else {
                    draft.day = .custom(date)
                }
                draft.hour = cal.component(.hour, from: date)
                draft.minute = cal.component(.minute, from: date)
            }
        case .recurring(let rule):
            switch rule.type {
            case "every_n_hours":
                draft.recurrence = .everyNHours(max(1, rule.hours ?? 1))
            case "daily":
                draft.recurrence = .daily
            case "weekdays":
                draft.recurrence = .weekdays
            case "selected_weekdays":
                draft.recurrence = .selectedWeekdays(Set(rule.days ?? []))
            case "weekly":
                draft.recurrence = .weekly(rule.day ?? "mon")
            default:
                break
            }
            if let time = rule.time, time.count >= 5,
               let h = Int(time.prefix(2)), let m = Int(time.dropFirst(3).prefix(2)) {
                draft.hour = h
                draft.minute = m
            }
        }
        automationDraft = draft
        automationsError = nil
        automationsSurface = .editorTask
    }

    var automationDraftTaskValid: Bool {
        !automationDraft.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !automationDraft.prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Advance the editor one step; validation errors keep the current step.
    func automationEditorNext() {
        switch automationsSurface {
        case .editorTask:
            guard automationDraftTaskValid else {
                automationsError = "Zadaj názov aj úlohu"
                return
            }
            automationsError = nil
            automationsSurface = .editorSchedule
        case .editorSchedule:
            // The first-run instant only constrains one-shot schedules;
            // recurrence computes its own next occurrence server-side.
            if automationDraft.recurrence.isOnce {
                guard let at = AutomationDraftBuilder.scheduledDate(automationDraft), at > Date() else {
                    automationsError = "Čas už uplynul"
                    return
                }
            }
            automationsError = nil
            automationsSurface = .editorRecurrence
        case .editorRecurrence:
            if case .selectedWeekdays(let days) = automationDraft.recurrence, days.isEmpty {
                automationsError = "Vyber aspoň jeden deň"
                return
            }
            automationsError = nil
            automationsSurface = .editorReview
        case .editorReview:
            saveAutomationDraft()
        default:
            break
        }
    }

    /// Concise review line: task — schedule — zone.
    var automationDraftSummary: String {
        let when: String
        switch AutomationDraftBuilder.schedule(automationDraft) {
        case .once?:
            let at = AutomationDraftBuilder.scheduledDate(automationDraft)
                .flatMap { AutomationTimeFormat.shortLocal(AutomationDraftBuilder.isoUTC($0)) } ?? "—"
            when = "jednorazovo \(at)"
        case .recurring(let rule)?:
            when = rule.displayLabel
        case nil:
            when = "—"
        }
        return "\(automationDraft.name) — \(when) (\(TimeZone.current.identifier))"
    }

    /// Persist via the daemon; success is claimed only after it confirms.
    func saveAutomationDraft() {
        guard let wire = AutomationDraftBuilder.wireDraft(automationDraft) else {
            automationsError = "Neplatný dátum"
            return
        }
        automationsSurface = .editorSaving
        automationsError = nil
        let editingID = automationDraft.editingID
        Task {
            do {
                if let id = editingID {
                    _ = try await client.patchAutomation(id: id, DaemonClient.AutomationPatch(
                        name: wire.name, prompt: wire.prompt, timezone: wire.timezone,
                        schedule: wire.schedule, enabled: wire.enabled))
                } else {
                    _ = try await client.createAutomation(wire)
                }
                await refreshAutomations()
                automationsSurface = .list
            } catch {
                if case DaemonError.serverError(let message) = error {
                    automationsError = message
                } else {
                    automationsError = "Daemon nedostupný — skús znova"
                }
                automationsSurface = .editorReview
            }
        }
    }

    // MARK: - Slash-command suggestions

    /// Matching commands for the current input (max 3, prefix, case-insensitive).
    @Published var slashSuggestions: [SlashCommand] = []
    /// Keyboard-selected suggestion row.
    @Published var slashSelectionIndex: Int = 0
    @Published var slashCommandError: String?
    private var modifiedReturnEventID: UUID?
    var onSlashSuggestionCompletion: ((String) -> Bool)?

    func preserveModifiedReturnForNativeEditing() {
        let eventID = UUID()
        modifiedReturnEventID = eventID
        DispatchQueue.main.async { [weak self] in
            guard self?.modifiedReturnEventID == eventID else { return }
            self?.modifiedReturnEventID = nil
        }
    }
    private func updateSlashSuggestions() {
        // Called only from inputText.didSet — an Escape dismissal therefore
        // holds until the text changes again.
        let matches = notchInteractionMode == .settings
            ? []
            : SlashCommandRegistry.suggestions(
                for: inputText,
                hasMarkedText: hasUncommittedMarkedText
            )
        if matches != slashSuggestions { slashSuggestions = matches }
        if slashSelectionIndex >= matches.count { slashSelectionIndex = 0 }
    }

    /// Escape while suggestions are showing: hide them, keep the text editable.
    func dismissSlashSuggestions() {
        slashSuggestions = []
        slashSelectionIndex = 0
    }

    func moveSlashSelection(by delta: Int) -> Bool {
        guard !slashSuggestions.isEmpty else { return false }
        let count = slashSuggestions.count
        slashSelectionIndex = (slashSelectionIndex + delta + count) % count
        return true
    }

    /// Tab or click completes one candidate. Completion never executes it.
    func completeSlashSuggestion(_ command: SlashCommand? = nil) -> Bool {
        guard let cmd = command ?? slashSuggestions[safe: slashSelectionIndex] else { return false }
        if onSlashSuggestionCompletion?(cmd.command) != true {
            inputText = cmd.command
        }
        slashSuggestions = []
        slashSelectionIndex = 0
        return true
    }

    func execute(_ command: SlashCommand) {
        switch (command.destination, command.confirmationPolicy) {
        case (.settings, .none):
            openNotchSettings()
        case (.automations, .none):
            openAutomations()
        case (.currentChat, .whenCurrentChatIsNonEmpty):
            requestCurrentChatClear()
        default:
            assertionFailure("Invalid Slash Command destination and confirmation policy")
        }
    }

    // MARK: - Clipboard paste wheel (hold right ⌘)
    @Published var pasteWheelActive = false
    /// Snapshot of clipboard history taken when the wheel opens (newest first).
    @Published var pasteWheelItems: [ClipboardItem] = []
    /// Slot flashed briefly on selection before the raindrop departs.
    @Published var pasteWheelFlashSlot: Int? = nil
    /// Wired by NotchWindowController — chip hover pins the wheel,
    /// click pastes a slot, drag-out dismisses without pasting.
    var onPasteWheelPinned: (() -> Void)?
    var onPasteWheelChipClicked: ((Int) -> Void)?
    var onPasteWheelDragStarted: (() -> Void)?

    // MARK: - Scroll viewport persistence (Phase 1B)
    /// The id of the message that was topmost-visible when the panel last collapsed.
    /// `nil` means "no saved position" → scroll to bottom on open.
    var savedScrollAnchorId: UUID? = nil
    /// True when the chat was scrolled to (or near) the bottom when last collapsed.
    var savedScrollWasAtBottom: Bool = true

    var agentStatus: AgentStatus {
        if notchPresentation.pendingApprovalIdentity != nil { return .awaitingApproval }
        if isThinking { return .thinking }
        return .ready
    }

    var latestAssistantText: String {
        guard let message = latestAssistantMessage else { return "" }
        return isLatestAssistantStreaming ? message.displayedContent : message.content
    }

    /// The message the notch output surface shows: the browsed past response
    /// while ↑/↓ history browsing is active, otherwise the newest assistant turn.
    var latestAssistantMessage: ChatMessage? {
        if let idx = historyBrowseIndex, assistantResponses.indices.contains(idx) {
            return assistantResponses[idx]
        }
        return messages.last(where: { $0.role == .assistant })
    }

    var latestAssistantMessageId: UUID? {
        latestAssistantMessage?.id
    }

    // MARK: - Response history browsing (↑/↓ on empty notch input)

    /// Index into `assistantResponses` while browsing; nil = not browsing.
    @Published var historyBrowseIndex: Int? = nil

    var assistantResponses: [ChatMessage] {
        messages.filter {
            $0.role == .assistant
                && !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
    }

    /// "(current, total)" for the n/N position hint; nil when not browsing.
    var historyBrowsePosition: (Int, Int)? {
        guard let idx = historyBrowseIndex else { return nil }
        return (idx + 1, assistantResponses.count)
    }

    /// User prompt that produced the browsed answer; nil when not browsing.
    var historyBrowsePrompt: String? {
        guard let idx = historyBrowseIndex, assistantResponses.indices.contains(idx) else {
            return nil
        }
        let browsedId = assistantResponses[idx].id
        guard let mi = messages.firstIndex(where: { $0.id == browsedId }) else { return nil }
        return messages[..<mi].last(where: { $0.role == .user })?.content
    }

    /// ↑ on empty input: step to an older response. Returns true if consumed.
    func browseOlderResponse() -> Bool {
        guard inputText.isEmpty, !isThinking else { return false }
        guard notchInteractionMode == .input || historyBrowseIndex != nil else { return false }
        let list = assistantResponses
        guard !list.isEmpty else { return false }
        let next = (historyBrowseIndex ?? list.count) - 1
        guard next >= 0 else { return true } // already at oldest — swallow the key
        historyBrowseIndex = next
        applyNotchIntent(.openOutput)
        return true
    }

    /// ↓ while browsing: step newer; past the newest exits back to input.
    func browseNewerResponse() -> Bool {
        guard let idx = historyBrowseIndex else { return false }
        if idx + 1 < assistantResponses.count {
            historyBrowseIndex = idx + 1
        } else {
            exitHistoryBrowse()
        }
        return true
    }

    func exitHistoryBrowse() {
        guard historyBrowseIndex != nil else { return }
        historyBrowseIndex = nil
        applyNotchIntent(.openInput)
    }

    var isLatestAssistantStreaming: Bool {
        guard let latestAssistantMessageId else { return false }
        return streamingAssistantMessageId == latestAssistantMessageId
    }

    var latestEvidenceStatus: String? {
        guard let message = latestAssistantMessage else { return nil }
        if let outcome = message.evidenceOutcome {
            return EvidencePresentation.outcomeLabel(outcome)
        }
        return message.evidencePhase.map(EvidencePresentation.phaseLabel)
    }

    var latestTranscriptActivityCount: Int {
        guard let message = latestAssistantMessage else { return 0 }
        return message.evidenceOutcome == nil && message.evidencePhase == nil
            ? message.activities.count
            : max(1, message.evidenceActivities.count)
    }

    var latestUserText: String {
        messages.last(where: { $0.role == .user })?.content ?? ""
    }

    private var approvalPollTask: Task<Void, Never>?
    private var healthMonitorTask: Task<Void, Never>?
    private let tavilyConfigurationSynchronizer = TavilyConfigurationSynchronizer()

    // MARK: - Codex (Phase 8)

    /// User-configured path to the `codex` binary. Empty = auto-discover from $PATH.
    @Published var codexBinaryPath: String = UserDefaults.standard.string(forKey: "bagent.codex_path") ?? "" {
        didSet {
            if !isSettingsFixture { UserDefaults.standard.set(codexBinaryPath, forKey: "bagent.codex_path") }
        }
    }
    /// Last result from "Testovať Codex" — nil while not tested, true/false after.
    @Published var codexTestResult: String? = nil
    @Published var isTestingCodex: Bool = false
    @Published var codexServiceAvailable: Bool? = nil

    func testCodex() {
        isTestingCodex = true
        codexTestResult = nil
        Task {
            do {
                let status = try await client.codexStatus()
                await MainActor.run {
                    if status.available {
                        self.codexTestResult = "✓ \(status.version ?? "dostupný")"
                        self.codexServiceAvailable = status.available
                    } else {
                        self.codexTestResult = "✗ \(status.error ?? "nenájdený")"
                        self.codexServiceAvailable = false
                    }
                    self.isTestingCodex = false
                }
            } catch {
                await MainActor.run {
                    self.codexTestResult = "✗ \(error.localizedDescription)"
                    self.codexServiceAvailable = false
                    self.isTestingCodex = false
                }
            }
        }
    }

    // MARK: - Odoo (Phase 6B — MCP)

    /// Odoo connection settings — URL, DB, user stored in UserDefaults (not secrets).
    @Published var odooURL:  String = UserDefaults.standard.string(forKey: "bagent.odoo.url")  ?? ""
    @Published var odooDB:   String = UserDefaults.standard.string(forKey: "bagent.odoo.db")   ?? ""
    @Published var odooUser: String = UserDefaults.standard.string(forKey: "bagent.odoo.user") ?? ""
    /// API key is loaded from Keychain; the `@Published` field holds the live session value only.
    @Published var odooAPIKey: String = ""
    /// Optional override path to the `uvx` binary (for non-standard installs).
    @Published var odooUvxPath: String = UserDefaults.standard.string(forKey: "bagent.odoo.uvx_path") ?? ""

    @Published var odooTestResult: String? = nil
    @Published var isTestingOdoo: Bool = false
    @Published var odooMcpAvailable: Bool? = nil
    @Published var odooToolCount: Int = 0
    @Published private(set) var rulesPolicySummary: String = "Not reported"

    var canTestOdoo: Bool {
        !odooURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !odooDB.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !odooUser.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !odooAPIKey.isEmpty
    }

    func refreshRulesPolicySummary() {
        Task {
            do {
                let yaml = try await client.rulesYaml()
                let summary = yaml.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    ? "Unavailable"
                    : "Configured by daemon"
                await MainActor.run { self.rulesPolicySummary = summary }
            } catch {
                await MainActor.run { self.rulesPolicySummary = "Unavailable" }
            }
        }
    }

    /// Save creds to Keychain + UserDefaults and authenticate with the daemon via MCP.
    func configureOdoo() {
        guard canTestOdoo else { return }
        // Persist non-secret fields in UserDefaults (URL, DB, user, uvx path).
        UserDefaults.standard.set(odooURL,     forKey: "bagent.odoo.url")
        UserDefaults.standard.set(odooDB,      forKey: "bagent.odoo.db")
        UserDefaults.standard.set(odooUser,    forKey: "bagent.odoo.user")
        UserDefaults.standard.set(odooUvxPath, forKey: "bagent.odoo.uvx_path")
        // API key goes to Keychain only.
        KeychainStore.saveOdoo(url: odooURL, db: odooDB, user: odooUser, apiKey: odooAPIKey)

        isTestingOdoo = true
        odooTestResult = nil
        let uvxOverride = odooUvxPath.trimmingCharacters(in: .whitespaces)
        Task {
            do {
                let result = try await client.odooConfigure(
                    url: odooURL, db: odooDB, user: odooUser, apiKey: odooAPIKey,
                    uvxPath: uvxOverride.isEmpty ? nil : uvxOverride
                )
                await MainActor.run {
                    self.odooMcpAvailable = result.mcp_available ?? true
                    self.odooToolCount = result.tool_count ?? 0
                    if result.ok {
                        let tools = result.tool_count.map { " (\($0) tools)" } ?? ""
                        self.odooTestResult = "✓ Odoo MCP \(result.version ?? "")  uid=\(result.uid ?? 0)\(tools)"
                    } else if result.mcp_available == false {
                        self.odooTestResult = "✗ MCP nedostupný — nainštalujte uv/uvx: \(result.error ?? "")"
                    } else {
                        self.odooTestResult = "✗ \(result.error ?? "chyba autentifikácie")"
                    }
                    self.isTestingOdoo = false
                }
            } catch {
                await MainActor.run {
                    self.odooTestResult = "✗ \(error.localizedDescription)"
                    self.odooMcpAvailable = false
                    self.isTestingOdoo = false
                }
            }
        }
    }

    func loadSavedOdooAPIKey() {
        if let key = KeychainStore.loadOdooAPIKey(), !key.isEmpty {
            odooAPIKey = key
            odooTestResult = "✓ API kľúč načítaný z Keychain"
        } else {
            odooTestResult = "✗ Uložený API kľúč sa nenašiel"
        }
    }

    /// Load saved Odoo credentials and connect the daemon-side MCP connector.
    /// This intentionally runs lazily, only for Odoo turns or explicit Settings tests.
    @discardableResult
    func restoreOdooFromKeychain() async -> Bool {
        guard let creds = KeychainStore.loadOdoo() else { return false }
        // Update live fields so Settings shows the saved values.
        odooURL  = creds.url
        odooDB   = creds.db
        odooUser = creds.user
        odooAPIKey = creds.apiKey
        let uvxOverride = odooUvxPath.trimmingCharacters(in: .whitespaces)
        do {
            let result = try await client.odooConfigure(
                url: creds.url, db: creds.db, user: creds.user, apiKey: creds.apiKey,
                uvxPath: uvxOverride.isEmpty ? nil : uvxOverride
            )
            odooMcpAvailable = result.mcp_available ?? true
            odooToolCount = result.tool_count ?? 0
            return result.ok
        } catch {
            odooTestResult = "✗ \(error.localizedDescription)"
            return false
        }
    }

    private func ensureOdooConfiguredIfNeeded(for text: String, sourceMode: SourceMode?) async {
        guard sourceMode == .odoo || Self.looksLikeOdooTurn(text) else { return }
        if let status = try? await client.odooStatus(), status.connected { return }
        _ = await restoreOdooFromKeychain()
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
    @Published var showWhatsappPairing: Bool = false
    @Published var whatsappDebugPayload: String? = nil
    @Published var isLoadingWhatsappDebug: Bool = false
    private var whatsappPollingTask: Task<Void, Never>? = nil

    func connectWhatsapp() {
        whatsappPollingTask?.cancel()
        isConnectingWhatsapp = true
        whatsappStatusMessage = nil
        whatsappQrString = nil
        whatsappDebugPayload = nil
        withAnimation(.spring(response: 0.28, dampingFraction: 0.78)) {
            showWhatsappPairing = true
        }
        Task {
            do {
                try await client.whatsappStart()
                await MainActor.run {
                    self.startWhatsappPairingPoll()
                }
            } catch {
                await MainActor.run {
                    self.whatsappStatusMessage = "✗ \(error.localizedDescription)"
                    self.isConnectingWhatsapp = false
                    withAnimation(.easeOut(duration: 0.18)) {
                        self.showWhatsappPairing = false
                    }
                }
            }
        }
    }

    func disconnectWhatsapp() {
        whatsappPollingTask?.cancel()
        Task {
            try? await client.whatsappStop()
            await pollWhatsappStatus()
            await MainActor.run {
                withAnimation(.easeOut(duration: 0.18)) {
                    self.showWhatsappPairing = false
                }
            }
        }
    }

    func logoutWhatsapp() {
        whatsappPollingTask?.cancel()
        Task {
            try? await client.whatsappLogout()
            await pollWhatsappStatus()
            await MainActor.run {
                self.whatsappQrString = nil
                self.whatsappDebugPayload = nil
                withAnimation(.easeOut(duration: 0.18)) {
                    self.showWhatsappPairing = false
                }
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

    @MainActor
    private func startWhatsappPairingPoll() {
        whatsappPollingTask?.cancel()
        whatsappPollingTask = Task { [weak self] in
            guard let self else { return }
            for _ in 0..<720 {
                if Task.isCancelled { break }
                do {
                    let status = try await client.whatsappStatus()
                    self.whatsappStatus = status
                    self.isConnectingWhatsapp = status.status == "starting"

                    if status.needs_qr {
                        if self.whatsappQrString == nil {
                            self.whatsappQrString = (try? await client.whatsappQr())?.qr
                        }
                    } else if status.status == "authenticated" || status.status == "authenticated_waiting_for_ready" {
                        self.whatsappQrString = nil
                    }

                    if status.status == "ready" {
                        self.isConnectingWhatsapp = false
                        self.whatsappQrString = nil
                        self.whatsappStatusMessage = "✓ WhatsApp je pripojený"
                        try? await Task.sleep(for: .milliseconds(650))
                        if !Task.isCancelled {
                            withAnimation(.spring(response: 0.28, dampingFraction: 0.78)) {
                                self.showWhatsappPairing = false
                            }
                        }
                        break
                    }

                    if status.status == "error" || status.status == "disconnected" || status.status == "missing_node" || status.status == "bridge_not_installed" {
                        self.isConnectingWhatsapp = false
                        break
                    }
                } catch {
                    self.whatsappStatusMessage = "✗ \(error.localizedDescription)"
                    self.isConnectingWhatsapp = false
                    break
                }
                try? await Task.sleep(for: .seconds(1))
            }
            self.isConnectingWhatsapp = false
        }
    }

    func refreshWhatsappStatus() {
        Task {
            await refreshWhatsappStatusNow()
        }
    }

    private func refreshWhatsappStatusNow() async {
        if let s = try? await client.whatsappStatus() {
            whatsappStatus = s
            if s.needs_qr { refreshWhatsappQr() }
        }
    }

    func loadWhatsappDebug() async {
        isLoadingWhatsappDebug = true
        do {
            whatsappDebugPayload = try await client.whatsappDebug()
        } catch {
            whatsappDebugPayload = "Chyba: \(error.localizedDescription)"
        }
        isLoadingWhatsappDebug = false
    }


    var topSourceModes: [SourceMode] {
        SourceMode.allCases.sorted { lhs, rhs in
            let lc = sourceModeUseCount(lhs)
            let rc = sourceModeUseCount(rhs)
            if lc == rc {
                return SourceMode.allCases.firstIndex(of: lhs)! < SourceMode.allCases.firstIndex(of: rhs)!
            }
            return lc > rc
        }
    }

    var activeSourcePlaceholder: String {
        if let hoveredSourceMode { return hoveredSourceMode.placeholder }
        if let selectedSourceMode { return selectedSourceMode.placeholder }
        return "Ask bagent"
    }

    func selectSourceMode(_ mode: SourceMode) {
        selectedSourceMode = mode
        hoveredSourceMode = nil
        isSourcePickerForced = false
    }

    func clearSourceMode() {
        selectedSourceMode = nil
        hoveredSourceMode = nil
    }

    private func sourceModeUseCount(_ mode: SourceMode) -> Int {
        UserDefaults.standard.integer(forKey: "bagent.sourceMode.\(mode.rawValue).useCount")
    }

    private func recordSourceModeUse(_ mode: SourceMode) {
        let key = "bagent.sourceMode.\(mode.rawValue).useCount"
        UserDefaults.standard.set(UserDefaults.standard.integer(forKey: key) + 1, forKey: key)
    }

    private let client: DaemonClient
    private let isSettingsFixture: Bool
    private let notchEventConsumer: NotchEventConsumer
    private var projectionCancellable: AnyCancellable?
    let permissions = PermissionsManager()

    /// Invoked after an input-only turn is submitted so AppKit can collapse the panel.
    var onInputOnlySubmitted: (() -> Void)?
    /// Invoked when output becomes displayable, or completion must recover a
    /// missed first-token transition, so AppKit can reveal the output surface.

    // MARK: - cmux notifications

    /// Ambient notifications from cmux agents (needs attention / finished run).
    @Published var cmuxNotificationsEnabled: Bool = UserDefaults.standard.object(forKey: "bagent.cmux.enabled") as? Bool ?? true {
        didSet {
            UserDefaults.standard.set(cmuxNotificationsEnabled, forKey: "bagent.cmux.enabled")
            if cmuxNotificationsEnabled {
                startCmuxMonitor()
            } else {
                cmuxMonitor.stop()
                cmuxPending = []
                cmuxBanner = nil
            }
        }
    }
    /// Unread cmux events (newest first). Drives the persistent corner dot.
    @Published var cmuxPending: [CmuxNotification] = []
    /// Notification currently shown in the transient notch banner.
    @Published var cmuxBanner: CmuxNotification? = nil
    /// Cues that have been acknowledged (seen) and are mid fly-off. Rendered by
    /// the notch as a transient departing icon after the pending entry is gone;
    /// removed once the animation reports completion.
    @Published var cmuxDeparting: [CmuxDeparture] = []
    /// Attention (amber) outranks finished (green).
    var cmuxDotKind: CmuxEventKind? {
        if cmuxPending.contains(where: { $0.kind == .attention }) { return .attention }
        return cmuxPending.isEmpty ? nil : .finished
    }
    let cmuxMonitor = CmuxEventMonitor()
    /// Invoked for each fresh cmux event so AppKit can present the transient banner.
    var onCmuxNotification: ((CmuxNotification) -> Void)?

    func startCmuxMonitor() {
        guard cmuxNotificationsEnabled, CmuxEventMonitor.isAvailable else { return }
        cmuxMonitor.onEvent = { [weak self] notification in
            self?.handleCmuxEvent(notification)
        }
        cmuxMonitor.start()
    }

    private func handleCmuxEvent(_ raw: CmuxNotification) {
        // Suppress only when the user is already looking at that exact
        // workspace (cmux frontmost + workspace selected). Events from
        // background workspaces still cue even while cmux is focused.
        if CmuxEventMonitor.isCmuxFrontmost() {
            Task { [weak self] in
                let visible = await CmuxEventMonitor.visibleWorkspaceIds()
                guard !visible.contains(raw.workspaceId) else { return }
                self?.deliverCmuxEvent(raw)
            }
        } else {
            deliverCmuxEvent(raw)
        }
    }

    private func deliverCmuxEvent(_ raw: CmuxNotification) {
        var notification = raw
        if notification.workspaceName == nil {
            notification.workspaceName = cmuxMonitor.workspaceName(for: notification.workspaceId)
        }
        // Latest event per agent session wins; separate sessions (even in one
        // workspace) keep their own pending slot.
        cmuxPending.removeAll { $0.dedupeKey == notification.dedupeKey }
        cmuxPending.insert(notification, at: 0)
        if cmuxPending.count > 10 { cmuxPending.removeLast(cmuxPending.count - 10) }
        onCmuxNotification?(notification)
    }

    /// All cues clear once the user has acknowledged them (hover reveal ended).
    /// Both amber (attention) and green (finished) fly off — seeing = seen.
    func markAllCmuxSeen() {
        guard !cmuxPending.isEmpty else { return }
        beginCmuxDeparture(kind: cmuxDotKind ?? .finished)
        cmuxPending.removeAll()
        cmuxBanner = nil
    }

    /// Auto-detect: the user opened the notified workspace in cmux. Clear every
    /// cue for that workspace with a fly-off.
    func markCmuxSeen(workspaceId: String) {
        let matched = cmuxPending.filter { $0.workspaceId == workspaceId }
        guard !matched.isEmpty else { return }
        let kind: CmuxEventKind = matched.contains { $0.kind == .attention } ? .attention : .finished
        beginCmuxDeparture(kind: kind)
        cmuxPending.removeAll { $0.workspaceId == workspaceId }
        if let banner = cmuxBanner, banner.workspaceId == workspaceId { cmuxBanner = nil }
    }

    /// Click-through: focus cmux on the workspace/tab and clear the cue with a fly-off.
    func focusCmux(_ notification: CmuxNotification) {
        // Spawn the fly-off before activating cmux — once bagent resigns active its
        // panel animations can be throttled, so start the flight while still frontmost.
        let kind: CmuxEventKind = cmuxPending
            .filter { $0.workspaceId == notification.workspaceId }
            .contains { $0.kind == .attention } ? .attention : notification.kind
        beginCmuxDeparture(kind: kind)
        cmuxPending.removeAll { $0.workspaceId == notification.workspaceId }
        cmuxBanner = nil
        cmuxMonitor.focus(notification)
    }

    /// Spawn a fly-off token unless Reduce Motion is on (then the cue just clears).
    private func beginCmuxDeparture(kind: CmuxEventKind) {
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }
        cmuxDeparting.append(CmuxDeparture(kind: kind))
    }

    /// The notch reports the fly-off animation finished; drop the token.
    func finishCmuxDeparture(_ id: UUID) {
        cmuxDeparting.removeAll { $0.id == id }
    }

    // MARK: - Tool loop status

    static func toolStatusLabel(for tool: String) -> String {
        switch tool {
        case let t where t.hasPrefix("mail"): return "🔎 Hľadám v pošte…"
        case let t where t.hasPrefix("filesystem"): return "🔎 Hľadám súbory…"
        case let t where t.hasPrefix("notes"): return "🔎 Hľadám v poznámkach…"
        case let t where t.hasPrefix("whatsapp"): return "🔎 WhatsApp…"
        case let t where t.hasPrefix("odoo"): return "🔎 Odoo…"
        case "web_search": return "🌐 Hľadám na webe…"
        case "web_fetch": return "🌐 Čítam stránku…"
        case let t where t.hasPrefix("macos"): return "⚙️ macOS…"
        default: return "⚙️ \(tool)…"
        }
    }

    // MARK: - Connector actions (left-wing icons)

    /// Clickable connector results found during the latest response. Rendered
    /// as colorful icons in the notch left wing (next to the cmux icon);
    /// cleared on the next user message or session clear — no timeout.
    @Published var pendingConnectorActions: [ConnectorAction] = []
    /// Clicked connector icons mid fly-off (same lifecycle as `cmuxDeparting`).
    @Published var connectorDeparting: [ConnectorDeparture] = []

    /// Latest result per connector wins — a fresh mail_found replaces the
    /// previous mail slot instead of stacking duplicates.
    private func upsertConnectorAction(_ action: ConnectorAction) {
        pendingConnectorActions.removeAll { $0.kind == action.kind }
        pendingConnectorActions.append(action)
    }

    /// Click-through on a left-wing connector icon: perform the open action
    /// and send the icon flying into the notch.
    func performConnectorAction(_ action: ConnectorAction, slotIndex: Int) {
        switch action.payload {
        case .mail(let ref):
            openMail(ref)
        case .odoo(let ref):
            openOdoo(ref)
        case .whatsapp:
            // No daemon open endpoint yet — best-effort hand-off to the app.
            if let url = URL(string: "whatsapp://") {
                NSWorkspace.shared.open(url)
            }
        case .file(let ref):
            Task { try? await client.revealInFinder(path: ref.path) }
        }
        if !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion {
            connectorDeparting.append(ConnectorDeparture(kind: action.kind, slotIndex: slotIndex))
        }
        pendingConnectorActions.removeAll { $0.id == action.id }
    }

    /// The notch reports the connector fly-off finished; drop the token.
    func finishConnectorDeparture(_ id: UUID) {
        connectorDeparting.removeAll { $0.id == id }
    }

    // MARK: - Debug trace copy (option+click on the notch)

    /// Transient "copied" checkmark in the left wing after option+click.
    @Published var traceCopiedFlash = false
    private var traceFlashTask: Task<Void, Never>?

    /// Option+click anywhere on the notch/panel: copy the latest session +
    /// debug trace to the clipboard and flash the left-wing checkmark.
    func copyDebugTrace() {
        traceFlashTask?.cancel()
        traceFlashTask = Task {
            let payload = await latestDebugClipboardPayload()
            let pb = NSPasteboard.general
            pb.clearContents()
            pb.setString(payload, forType: .string)
            withAnimation(.spring(response: 0.28, dampingFraction: 0.68)) {
                traceCopiedFlash = true
            }
            try? await Task.sleep(nanoseconds: 1_200_000_000)
            guard !Task.isCancelled else { return }
            withAnimation(.easeOut(duration: 0.3)) {
                traceCopiedFlash = false
            }
        }
    }
    // MARK: - Init

    init(startMonitoring: Bool = true, client: DaemonClient = DaemonClient(), settingsFixture: Bool = false) {
        self.client = client
        self.isSettingsFixture = settingsFixture
        if settingsFixture {
            codexBinaryPath = ""
            odooURL = ""
            odooDB = ""
            odooUser = ""
            odooUvxPath = ""
        }
        notchEventConsumer = NotchEventConsumer(transport: client)
        projectionCancellable = notchEventConsumer.objectWillChange.sink { [weak self] _ in
            self?.objectWillChange.send()
        }
        if startMonitoring {
            startHealthMonitor()
            startCmuxMonitor()
            Task { await refreshHealth() }
        }
        if startMonitoring {
            Task { await restoreCurrentChat() }
        }
    }

    func applyNotchIntent(_ intent: NotchLocalIntent) {
        try? notchEventConsumer.applyLocalIntent(intent)
    }

    func setNotchReduceMotion(_ enabled: Bool) {
        try? notchEventConsumer.setReduceMotion(enabled)
    }

    func applyAuthoritativeSnapshot(_ snapshot: NotchWorkSnapshot) throws {
        try notchEventConsumer.replace(with: snapshot)
    }

    func activateFocusedNotchActivity() {
        switch notchPresentation.focusedDestination {
        case .currentChat:
            applyNotchIntent(.openOutput)
        case .activeAutomation(let definitionIdentity):
            if notchPresentation.activeAutomationCount > 1 {
                applyNotchIntent(.cycleAutomation)
            } else {
                openAutomationDetail(definitionIdentity)
            }
        case .terminalAutomation(
            let definitionIdentity,
            let sessionIdentity,
            let workIdentity,
            let expectedRevision
        ):
            openAutomationSession(
                definitionIdentity: definitionIdentity,
                sessionIdentity: sessionIdentity,
                workIdentity: workIdentity,
                expectedRevision: expectedRevision
            )
        case nil:
            break
        }
    }

    func openActiveAutomations() {
        guard notchPresentation.activeAutomationCount > 0 else { return }
        openAutomations()
    }

    // MARK: - Actions

    private var currentChatHasContent: Bool {
        guard let snapshot = currentChatSnapshot else { return false }
        let draftCommand = snapshot.draft.flatMap {
            SlashCommandRegistry.exactMatch($0.text)
        }
        let draftIsOnlyClearCommand = draftCommand?.destination == .currentChat
            && draftCommand?.confirmationPolicy == .whenCurrentChatIsNonEmpty
            && snapshot.draft?.pendingAttachmentReferences.isEmpty == true
        return !snapshot.turns.isEmpty
            || (snapshot.draft != nil && !draftIsOnlyClearCommand)
            || snapshot.continuation != nil
            || !snapshot.submittedAttachments.isEmpty
            || !snapshot.validatedSources.isEmpty
            || !snapshot.connectorReferences.isEmpty
            || !snapshot.completedApprovalPresentations.isEmpty
    }

    func restoreCurrentChat(preservingActivePresentation: Bool = false) async {
        do {
            let snapshot = try await client.currentChat()
            var restoredPending: [ChatAttachment] = []
            for identity in snapshot.draft?.pendingAttachmentReferences ?? [] {
                if let attachment = try? await client.attachment(id: identity) {
                    restoredPending.append(attachment)
                } else {
                    restoredPending.append(Self.unavailablePendingAttachment(identity: identity))
                }
            }
            applyCurrentChatSnapshot(
                snapshot,
                rebuildMessages: !preservingActivePresentation,
                restoreCaretAtEnd: !preservingActivePresentation)
            pendingAttachments = restoredPending
            restoredPendingAttachmentReferences = restoredPending.map(\.id)
        } catch {
            if currentChatSnapshot == nil {
                slashCommandError = String(localized: "currentChat.restore.failure", defaultValue: "Current Chat could not be restored")
            }
        }
    }

    func applyCurrentChatSnapshot(
        _ snapshot: DaemonClient.CurrentChatSnapshot,
        rebuildMessages: Bool,
        restoreCaretAtEnd: Bool = false
    ) {
        currentChatSnapshot = snapshot
        restoredSubmittedAttachments = snapshot.submittedAttachments.map(Self.restoredChatAttachment)
        restoredValidatedSources = snapshot.validatedSources
        restoredConnectorReferences = snapshot.connectorReferences
        restoredApprovalPresentations = snapshot.completedApprovalPresentations
        isApplyingCurrentChatSnapshot = true
        inputText = snapshot.draft?.text ?? ""
        restoredPendingAttachmentReferences = snapshot.draft?.pendingAttachmentReferences ?? []
        isApplyingCurrentChatSnapshot = false
        updateSlashSuggestions()
        if restoreCaretAtEnd {
            pendingCurrentChatCaretRestoration = true
            currentChatFocusRequestID = UUID()
        }
        guard rebuildMessages else { return }

        var restored: [ChatMessage] = []
        if let continuation = snapshot.continuation {
            restored.append(ChatMessage(role: .assistant, content: continuation.seed))
        }
        for turn in snapshot.turns {
            var userMessage = ChatMessage(role: .user, content: turn.userMessage)
            userMessage.attachments = snapshot.submittedAttachments
                .filter { $0.conversationTurnIdentity == turn.identity }
                .map(Self.restoredChatAttachment)
            restored.append(userMessage)
            if let output = turn.assistantOutput {
                restored.append(ChatMessage(role: .assistant, content: output))
            } else if turn.state == "interrupted" {
                restored.append(ChatMessage(
                    role: .assistant,
                    content: String(localized: "currentChat.turn.interrupted", defaultValue: "This response was interrupted. Submit a new message to continue.")))
            }
        }
        messages = restored
        historyBrowseIndex = nil
    }

    private static func restoredChatAttachment(
        _ attachment: DaemonClient.SubmittedAttachment
    ) -> ChatAttachment {
        let kind: ChatAttachmentKind
        if attachment.mime.hasPrefix("image/") {
            kind = .image
        } else if attachment.mime == "application/pdf" {
            kind = .pdf
        } else if attachment.mime.hasPrefix("text/") {
            kind = .text
        } else {
            kind = .other
        }
        return ChatAttachment(
            id: attachment.identity,
            filename: attachment.filename,
            mime: attachment.mime,
            kind: kind,
            localURL: nil,
            sizeBytes: Int(attachment.sizeBytes),
            availability: attachment.available ? .available : .unavailable)
    }

    static func unavailablePendingAttachment(identity: String) -> ChatAttachment {
        ChatAttachment(
            id: identity,
            filename: String(
                localized: "currentChat.attachment.unavailable",
                defaultValue: "Attachment unavailable"),
            mime: "application/octet-stream",
            kind: .other,
            localURL: nil,
            sizeBytes: 0,
            availability: .unavailable)
    }

    func applyRejectedSubmissionDraft(
        text: String,
        attachments: [ChatAttachment],
        availableAttachmentIdentities: Set<String>
    ) {
        isApplyingCurrentChatSnapshot = true
        inputText = text
        pendingAttachments = attachments.map { attachment in
            guard !availableAttachmentIdentities.contains(attachment.id) else { return attachment }
            return ChatAttachment(
                id: attachment.id,
                filename: attachment.filename,
                mime: attachment.mime,
                kind: attachment.kind,
                localURL: attachment.localURL,
                sizeBytes: attachment.sizeBytes,
                availability: .unavailable,
                thumbnail: attachment.thumbnail)
        }
        restoredPendingAttachmentReferences = attachments.map(\.id)
        isApplyingCurrentChatSnapshot = false
        updateSlashSuggestions()
        scheduleDraftPersistence()
    }

    static func submissionWasAdmitted(
        before: DaemonClient.CurrentChatSnapshot,
        after: DaemonClient.CurrentChatSnapshot?,
        exactText: String
    ) -> Bool {
        guard let after, after.identity == before.identity, let newest = after.turns.last else {
            return false
        }
        let previousTurnIdentities = Set(before.turns.map(\.identity))
        return !previousTurnIdentities.contains(newest.identity) && newest.userMessage == exactText
    }

    func consumeCurrentChatCaretRestoration() -> Bool {
        defer { pendingCurrentChatCaretRestoration = false }
        return pendingCurrentChatCaretRestoration
    }

    func restoreCurrentChatCaret(in editor: NSTextView) {
        CurrentChatTextRestoration.placeCaretAtEnd(in: editor)
    }

    private func scheduleDraftPersistence() {
        draftPersistenceTask?.cancel()
        guard let snapshot = currentChatSnapshot else { return }
        let text = inputText
        let references = Array(Set(
            pendingAttachments.map(\.id) + restoredPendingAttachmentReferences)).sorted()
        draftPersistenceTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(180))
            guard !Task.isCancelled, let self else { return }
            do {
                let updated = try await client.saveCurrentChatDraft(
                    identity: snapshot.identity,
                    expectedRevision: snapshot.revision,
                    text: text,
                    pendingAttachmentReferences: references)
                guard inputText == text else { return }
                currentChatSnapshot = updated
            } catch {
                guard inputText == text else { return }
                if let authoritative = try? await client.currentChat(),
                   authoritative.identity == snapshot.identity,
                   let retried = try? await client.saveCurrentChatDraft(
                       identity: authoritative.identity,
                       expectedRevision: authoritative.revision,
                       text: text,
                       pendingAttachmentReferences: references),
                   inputText == text {
                    currentChatSnapshot = retried
                } else {
                    slashCommandError = String(localized: "currentChat.draft.saveFailure", defaultValue: "Current Chat Draft could not be saved")
                }
            }
        }
    }

    func requestCurrentChatClear() {
        guard currentChatSnapshot != nil else {
            slashCommandError = String(localized: "currentChat.loading", defaultValue: "Current Chat is still loading")
            return
        }
        guard !isThinking, authoritativePendingApproval == nil else {
            slashCommandError = String(localized: "currentChat.clear.active", defaultValue: "Finish the active turn or approval before clearing Current Chat")
            return
        }
        clearCurrentChatCommandIdentity = clearCurrentChatCommandIdentity ?? UUID().uuidString
        if currentChatHasContent {
            clearCurrentChatConfirmationPresented = true
        } else {
            performCurrentChatClear(confirmedNonEmpty: false)
        }
    }

    func cancelCurrentChatClear() {
        clearCurrentChatConfirmationPresented = false
        clearCurrentChatCommandIdentity = nil
    }

    func confirmCurrentChatClear() {
        clearCurrentChatConfirmationPresented = false
        performCurrentChatClear(confirmedNonEmpty: true)
    }

    private func performCurrentChatClear(confirmedNonEmpty: Bool) {
        guard let snapshot = currentChatSnapshot,
              let commandIdentity = clearCurrentChatCommandIdentity
        else { return }
        draftPersistenceTask?.cancel()
        Task {
            if let replacement = await resolveCurrentChatClear(
                original: snapshot,
                commandIdentity: commandIdentity,
                confirmedNonEmpty: confirmedNonEmpty
            ) {
                completeCurrentChatClear(with: replacement)
            }
        }
    }

    private func resolveCurrentChatClear(
        original: DaemonClient.CurrentChatSnapshot,
        commandIdentity: String,
        confirmedNonEmpty: Bool
    ) async -> DaemonClient.CurrentChatSnapshot? {
        var requestSnapshot = original
        var lastAuthoritative: DaemonClient.CurrentChatSnapshot?
        for _ in 0..<3 {
            do {
                return try await client.clearCurrentChat(
                    identity: requestSnapshot.identity,
                    expectedRevision: requestSnapshot.revision,
                    commandIdentity: commandIdentity,
                    confirmedNonEmpty: confirmedNonEmpty)
            } catch {
                guard let authoritative = try? await client.currentChat() else { continue }
                lastAuthoritative = authoritative
                // Same identity means a draft/revision race: retry the same
                // command key at the refetched revision. A changed identity
                // may be this command's lost response, so replay its original
                // arguments and let the daemon's idempotency record decide.
                requestSnapshot = authoritative.identity == original.identity
                    ? authoritative : original
            }
        }
        if let authoritative = lastAuthoritative {
            applyCurrentChatSnapshot(
                authoritative,
                rebuildMessages: true,
                restoreCaretAtEnd: true)
            slashCommandError = authoritative.identity == original.identity
                ? String(localized: "currentChat.clear.failure", defaultValue: "Current Chat could not be cleared. Try again.")
                : String(localized: "currentChat.clear.changed", defaultValue: "Current Chat changed in another window. Review it before clearing.")
        } else {
            slashCommandError = String(localized: "currentChat.clear.failure", defaultValue: "Current Chat could not be cleared. Try again.")
        }
        return nil
    }

    private func completeCurrentChatClear(with snapshot: DaemonClient.CurrentChatSnapshot) {
        applyCurrentChatSnapshot(snapshot, rebuildMessages: true, restoreCaretAtEnd: true)
        pendingAttachments = []
        restoredPendingAttachmentReferences = []
        pendingConnectorActions = []
        selectedSourceMode = nil
        hoveredSourceMode = nil
        savedScrollAnchorId = nil
        savedScrollWasAtBottom = true
        slashSuggestions = []
        slashCommandError = nil
        clearCurrentChatCommandIdentity = nil
        let announcement = String(localized: "currentChat.clear.announcement", defaultValue: "Current Chat cleared")
        NSAccessibility.post(
            element: NSApplication.shared,
            notification: .announcementRequested,
            userInfo: [.announcement: announcement, .priority: NSAccessibilityPriorityLevel.high.rawValue])
        applyNotchIntent(.openInput)
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
        for attempt in 0..<24 {
            let health = await client.healthStatus()
            daemonHealth = health
            if health.daemonUp && health.baseRTUp { break }
            if attempt < 23 {
                try? await Task.sleep(for: .milliseconds(500))
            }
        }
        // Odoo connects lazily on the first Odoo turn, avoiding MCP startup and
        // Keychain prompts during app launch.
        await refreshWhatsappStatusNow()
        permissions.refresh()
    }

    func startHealthMonitor() {
        healthMonitorTask?.cancel()
        healthMonitorTask = Task { [weak self] in
            guard let self else { return }
            while !Task.isCancelled {
                let health = await client.healthStatus()
                _ = await tavilyConfigurationSynchronizer.synchronize(
                    health: health,
                    loadCredential: KeychainStore.loadTavilyAPIKey,
                    configure: client.configureTavily
                )
                await MainActor.run {
                    self.daemonHealth = health
                }
                try? await Task.sleep(for: .seconds(3))
            }
        }
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

    func latestDebugClipboardPayload() async -> String {
        guard let latest = latestAssistantMessage,
              let traceId = latest.debugTraceId
        else {
            return "No debug trace is available for the latest assistant message."
        }

        await loadDebugTrace(for: latest.id)
        let tracePayload = messages
            .first(where: { $0.id == latest.id })?
            .debugPayload ?? "Trace payload unavailable."

        let conversationPayload: String
        if let identity = currentChatSnapshot?.identity {
            do {
                conversationPayload = try await client.debugConversation(id: identity)
            } catch {
                conversationPayload = "Conversation debug unavailable: \(error.localizedDescription)"
            }
        } else {
            conversationPayload = "No conversation id yet."
        }

        let sessionLine = currentChatSnapshot?.identity ?? "(none)"
        let assistantText = messages
            .first(where: { $0.id == latest.id })?
            .content ?? latest.content

        return """
        bagent debug payload
        generated_at: \(Date().ISO8601Format())
        session_id: \(sessionLine)
        prompt_trace_id: \(traceId)

        latest_assistant_response:
        \(assistantText)

        debug_trace:
        \(tracePayload)

        conversation_debug:
        \(conversationPayload)
        """

    }

    func send() {
        let text = inputText
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                || !pendingAttachments.isEmpty
        else { return }
        if modifiedReturnEventID != nil {
            modifiedReturnEventID = nil
            return
        }
        if let cmd = SlashCommandRegistry.exactMatch(
            text,
            hasMarkedText: hasUncommittedMarkedText
        ) {
            execute(cmd)
            return
        }
        guard !isThinking else { return }
        let sourceMode = selectedSourceMode
        let wasInputOnly = notchInteractionMode == .input
        guard let currentChatSnapshot else {
            slashCommandError = String(localized: "currentChat.loading", defaultValue: "Current Chat is still loading")
            return
        }
        let submissionSnapshot = currentChatSnapshot
        draftPersistenceTask?.cancel()
        isApplyingCurrentChatSnapshot = true
        inputText = ""
        isApplyingCurrentChatSnapshot = false
        updateSlashSuggestions()
        historyBrowseIndex = nil
        pendingConnectorActions = []
        guard let model = daemonHealth?.model,
              !model.isEmpty,
              model != "—"
        else {
            slashCommandError = "Model Runtime is unavailable."
            return
        }
        let attachments = pendingAttachments
        pendingAttachments = []
        restoredPendingAttachmentReferences = []
        let attachmentIds = attachments.map { $0.id }
        var userMsg = ChatMessage(role: .user, content: text)
        userMsg.attachments = attachments
        messages.append(userMsg)
        if let sourceMode {
            recordSourceModeUse(sourceMode)
        }
        if wasInputOnly {
            onInputOnlySubmitted?()
        }

        Task {
            var admissionWasObserved = false
            let assistantMsg = ChatMessage(role: .assistant, content: "")
            messages.append(assistantMsg)
            isActivityTranscriptExpanded = false
            streamingAssistantMessageId = assistantMsg.id
            let idx = messages.count - 1

            do {
                await ensureOdooConfiguredIfNeeded(for: text, sourceMode: sourceMode)

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
                            wantsSelection: intent.wants_selection
                        )
                        screenCtx = ScreenContextFields(
                            ocrText: raw.ocrText,
                            activeApp: raw.activeApp,
                            selectedText: raw.selectedText
                        )
                    }
                }

                let stream = client.chatStream(
                    text: text,
                    currentChatIdentity: currentChatSnapshot.identity,
                    expectedRevision: currentChatSnapshot.revision,
                    model: model,
                    attachmentIds: attachmentIds,
                    screenContext: screenCtx,
                    sourceMode: sourceMode)
                var didAutoOpen = false
                let presenter = AdaptiveStreamPresenter { [weak self] edit in
                    guard let self, messages.indices.contains(idx) else { return }
                    messages[idx].displayedContent += edit
                    streamingChunk += 1
                }
                for try await event in stream {
                    admissionWasObserved = true
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
                    case .token(let t):
                        messages[idx].content += t
                        presenter.enqueue(t)
                        // toolStatus intentionally NOT cleared here — the action chip
                        // stays visible while the answer streams; cleared on .done.
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
                    case .memorySaved:
                        break
                    case .activityStarted(let event):
                        let activity = TurnActivity(
                            id: event.id,
                            kind: event.kind,
                            tool: event.tool,
                            title: event.title,
                            detail: event.detail,
                            status: "running",
                            durationMs: nil
                        )
                        messages[idx].activities.removeAll { $0.id == event.id }
                        messages[idx].activities.append(activity)
                    case .activityCompleted(let event):
                        if let activityIndex = messages[idx].activities.firstIndex(where: { $0.id == event.id }) {
                            messages[idx].activities[activityIndex].status = event.status ?? "completed"
                            messages[idx].activities[activityIndex].durationMs = event.durationMs
                        }
                    case .evidencePhase, .logicalActivityStarted, .logicalActivityCompleted,
                         .evidenceOutcome, .evidencePolish:
                        _ = EvidencePresentation.apply(event, to: &messages[idx])
                    case .evidenceAcquisitionDiagnostic:
                        break
                    case .sourceDiscovered(let source):
                        if !messages[idx].sources.contains(where: { $0.id == source.id }) {
                            messages[idx].sources.append(source)
                        }
                    case .approvalRequested(let id, let tool, let desc):
                        let item = ApprovalItem(
                            id: id, toolName: tool, description: desc,
                            expiresAt: "", createdAt: "", origin: nil
                        )
                        pendingApprovals.append(item)
                        messages[idx].activities.append(TurnActivity(
                            id: "approval:\(id)",
                            kind: "approval",
                            tool: tool,
                            title: "Waiting for approval",
                            detail: desc,
                            status: "running",
                            durationMs: nil
                        ))
                    case .toolBlocked(let tool):
                        messages[idx].activities.append(TurnActivity(
                            id: "blocked:\(tool):\(messages[idx].activities.count)",
                            kind: "blocked",
                            tool: tool,
                            title: "Action blocked",
                            detail: tool,
                            status: "failed",
                            durationMs: nil
                        ))
                    case .toolCall(let tool):
                        _ = tool
                    case .mailAttachments(let refs):
                        let chips = refs.map { ref in
                            ChatAttachment(
                                id: UUID().uuidString,
                                filename: ref.filename,
                                mime: "application/pdf",
                                kind: .pdf,
                                localURL: URL(fileURLWithPath: ref.path),
                                sizeBytes: ref.size,
                                availability: .available
                            )
                        }
                        messages[idx].attachments.append(contentsOf: chips)
                    case .mailFound(let ref):
                        messages[idx].mailRef = ref
                        upsertConnectorAction(ConnectorAction(kind: .mail, payload: .mail(ref)))
                    case .fileFound(let ref):
                        messages[idx].fileRef = ref
                        upsertConnectorAction(ConnectorAction(kind: .file, payload: .file(ref)))
                    case .odooFound(let ref):
                        messages[idx].odooRef = ref
                        upsertConnectorAction(ConnectorAction(kind: .odoo, payload: .odoo(ref)))
                    case .whatsappFound(let ref):
                        messages[idx].whatsappRef = ref
                        upsertConnectorAction(ConnectorAction(kind: .whatsapp, payload: .whatsapp(ref)))
                    case .fileOpened:
                        break // no UI action for now; daemon already opened the file
                    case .actionTaken(let message):
                        streamingAssistantMessageId = nil
                        messages[idx].content = message
                    case .taskRating(let level, let score, let reasons, let privacyRisk):
                        messages[idx].taskRating = (level: level, score: score, reasons: reasons, privacyRisk: privacyRisk)
                    case .done:
                        await presenter.finish()
                        for activityIndex in messages[idx].activities.indices
                            where messages[idx].activities[activityIndex].status == "running" {
                            messages[idx].activities[activityIndex].status = "completed"
                        }
                        streamingAssistantMessageId = nil
                        Task { await loadDebugTrace(for: messages[idx].id) }
                    }
                }
                await presenter.finish()
                streamingAssistantMessageId = nil
                await restoreCurrentChat(preservingActivePresentation: true)
            } catch {
                streamingAssistantMessageId = nil
                messages[idx].content = "Chyba: \(error.localizedDescription)"
                await restoreCurrentChat(preservingActivePresentation: true)
                let authoritativeAdmission = Self.submissionWasAdmitted(
                    before: submissionSnapshot,
                    after: self.currentChatSnapshot,
                    exactText: text)
                if !admissionWasObserved && !authoritativeAdmission {
                    var availableIdentities = Set<String>()
                    for attachment in attachments {
                        if (try? await client.attachment(id: attachment.id)) != nil {
                            availableIdentities.insert(attachment.id)
                        }
                    }
                    applyRejectedSubmissionDraft(
                        text: text,
                        attachments: attachments,
                        availableAttachmentIdentities: availableIdentities)
                }
            }
        }
    }

    // MARK: - Authoritative Work projection

    /// Bumped after an authoritative Work revision advances; the Automations
    /// content projection then refetches its authorized records.
    @Published var automationsRefreshID = UUID()
    private var eventsMonitorTask: Task<Void, Never>?

    /// Maintain one fenced consumer. Ordered events carry Work revisions; a
    /// one-second snapshot reconciliation also observes Model Runtime phase.
    /// Transport failures never synthesize presentation state.
    func startEventsMonitor() {
        eventsMonitorTask?.cancel()
        eventsMonitorTask = Task {
            var lastCursor: UInt64?
            var pollCount = 0
            while !Task.isCancelled {
                do {
                    try await notchEventConsumer.synchronize()
                    pollCount += 1
                    if pollCount.isMultiple(of: 4) {
                        try await notchEventConsumer.reconcileSnapshot()
                    }
                    let cursor = notchPresentation.revision.cursor
                    if cursor != lastCursor {
                        lastCursor = cursor
                        refreshAutomationSessionProjection()
                        await refreshPendingApprovals()
                        automationsRefreshID = UUID()
                        if notchInteractionMode == .automations {
                            await refreshAutomations()
                        }
                    }
                } catch NotchEventTransportError.consumerFenced {
                    return
                } catch {
                    notchEventConsumer.invalidateConsumerFence()
                }
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
    }

    func stopEventsMonitor() {
        eventsMonitorTask?.cancel()
        eventsMonitorTask = nil
    }

    /// Refetch the authoritative pending-approval list; opens the notch when
    /// approvals exist and nothing is being shown.
    func refreshPendingApprovals() async {
        guard let items = try? await client.pendingApprovals() else { return }
        pendingApprovals = items
    }

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
        guard item.id == notchPresentation.pendingApprovalIdentity else { return }
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
        let candidates = Array(urls.prefix(remaining))
        let imageURLs = candidates.filter { url in
            guard let type = UTType(filenameExtension: url.pathExtension) else { return false }
            return type.conforms(to: .image)
        }
        if !imageURLs.isEmpty {
            showUnsupportedImageAlert()
        }
        let toAdd = candidates.filter { !imageURLs.contains($0) }
        guard !toAdd.isEmpty else { return }
        isUploadingAttachment = true
        Task {
            var added: [ChatAttachment] = []
            for url in toAdd {
                do {
                    let att = try await client.uploadAttachment(url: url)
                    added.append(att)
                } catch {
                    // silently skip failed uploads
                }
            }
            pendingAttachments.append(contentsOf: added)
            scheduleDraftPersistence()
            isUploadingAttachment = false
        }
    }

    func removeAttachment(id: String) {
        pendingAttachments.removeAll { $0.id == id }
        restoredPendingAttachmentReferences.removeAll { $0 == id }
        scheduleDraftPersistence()
    }

    // MARK: - Image paste (Part B — Phase 7)

    /// Tries to read an image from the general pasteboard.
    /// Returns true and inserts `[image #n]` into `inputText` when an image is found.
    @discardableResult
    func pasteImageFromClipboard() -> Bool {
        let pb = NSPasteboard.general
        guard NSImage(pasteboard: pb) != nil else { return false }
        showUnsupportedImageAlert()
        return true
    }

    private func showUnsupportedImageAlert() {
        let alert = NSAlert()
        alert.messageText = "Obrázky nie sú podporované"
        alert.informativeText =
            "Model BaseRT Qwen3-4B je textový. Otázky o obrazovke používajú lokálne OCR."
        alert.alertStyle = .informational
        alert.addButton(withTitle: "OK")
        alert.runModal()
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

    static func looksLikeOdooTurn(_ message: String) -> Bool {
        let low = message.lowercased()
        let keywords = [
            "odoo",
            "faktúr",
            "faktura",
            "invoice",
            "partner",
            "kontakt",
            "zákazník",
            "zakaznik",
            "helpdesk",
            "tiket",
            "ticket",
            "úloh",
            "uloh",
            "objednávk",
        ]
        return keywords.contains { low.contains($0) }
    }

    // MARK: - Private

}
