import Combine
import ScreenCaptureKit
import SwiftUI
import UniformTypeIdentifiers

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

enum ChatSurfaceMode: Equatable {
    case collapsed
    case inputOnly
    case thinkingHidden
    case outputExpanded
}

enum NotchInteractionMode: Equatable {
    case collapsed
    case input
    case thinking
    case output
    case settings
    case automations
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

/// Pages of the notch-style settings surface (`/settings`).
enum NotchSettingsPage: Int, CaseIterable, Equatable {
    case general
    case permissions
    case model
    case connectors
    case setup

    var title: String {
        switch self {
        case .general:     return "Všeobecné"
        case .permissions: return "Povolenia"
        case .model:       return "Model"
        case .connectors:  return "Konektory"
        case .setup:       return "Nastavenie"
        }
    }

    var symbolName: String {
        switch self {
        case .general:     return "gearshape"
        case .permissions: return "lock.shield"
        case .model:       return "cpu"
        case .connectors:  return "puzzlepiece.extension"
        case .setup:       return "slider.horizontal.3"
        }
    }

    var next: NotchSettingsPage {
        NotchSettingsPage(rawValue: (rawValue + 1) % Self.allCases.count) ?? .general
    }
    var previous: NotchSettingsPage {
        NotchSettingsPage(rawValue: (rawValue + Self.allCases.count - 1) % Self.allCases.count) ?? .general
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
    @Published var messages: [ChatMessage] = []
    @Published var inputText: String = "" {
        didSet { updateSlashSuggestions() }
    }
    @Published var isThinking = false
    /// Transient tool-loop status ("🔎 Searching mail…") shown next to the thinking indicator.
    @Published var toolStatus: String? = nil
    @Published var isExpanded = false
    @Published var chatSurfaceMode: ChatSurfaceMode = .collapsed
    @Published var notchInteractionMode: NotchInteractionMode = .collapsed
    @Published var notchHoverResetID = UUID()
    @Published var selectedSourceMode: SourceMode? = nil
    @Published var hoveredSourceMode: SourceMode? = nil
    @Published var isSourcePickerForced = false
    @Published var hasNotch = false
    @Published var availableModels: [String] = [BaseRTLaunchAgent.model]
    @Published var daemonHealth: DaemonHealth?
    @Published var isSyncing = false
    @Published var lastSyncResult: String? = nil
    @Published var streamingChunk: Int = 0
    @Published var pendingApprovals: [ApprovalItem] = []
    /// Files queued to send with the next message.
    @Published var pendingAttachments: [ChatAttachment] = []
    /// True while uploading a file to the daemon.
    @Published var isUploadingAttachment = false
    @Published var streamingAssistantMessageId: UUID? = nil
    @Published var isActivityTranscriptExpanded = false

    /// Set to true by NotchWindowController before expanding so the pill
    /// animates to its hover state before the chat panel appears.
    @Published var pillHovered = false

    /// Current page of the notch settings surface.
    @Published var notchSettingsPage: NotchSettingsPage = .general

    /// Open the notch-style settings surface (`/settings` command).
    func openNotchSettings() {
        inputText = ""
        historyBrowseIndex = nil
        notchSettingsPage = .general
        notchInteractionMode = .settings
    }

    // MARK: - Automations surface (/automations)

    @Published var automationsSurface: AutomationsSurfaceState = .list
    @Published var automations: [AutomationRecord] = []
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
        notchInteractionMode = .automations
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
                if !automations.contains(where: { $0.id == id }) {
                    automationsSurface = .list
                } else if case .detail = automationsSurface {
                    loadAutomationDetailRuns(id)
                }
            case .list, .editorTask, .editorSchedule, .editorRecurrence, .editorReview,
                 .editorSaving:
                // Editor keeps its draft through concurrent SSE updates; an
                // edited record deleted elsewhere fails at save with 404.
                break
            }
        } catch {
            automationsError = "Daemon nedostupný"
        }
    }

    /// Recent runs shown on the detail page (fetched on entry + on events).
    @Published var automationDetailRuns: [AutomationRunRecord] = []

    func loadAutomationDetailRuns(_ id: String) {
        Task {
            automationDetailRuns = (try? await client.automationRuns(id: id, limit: 3)) ?? []
        }
    }

    /// Show the full latest result through the existing output presentation.
    func showAutomationResult(_ automation: AutomationRecord) {
        guard let summary = automation.lastResultSummary, !summary.isEmpty else { return }
        messages.append(ChatMessage(role: .assistant, content: "**\(automation.name)**\n\(summary)"))
        historyBrowseIndex = nil
        notchInteractionMode = .output
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
        switch automationsSurface {
        case .deleteConfirmation(let id):
            automationsSurface = .detail(id)
            return true
        case .detail:
            automationsSurface = .list
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
        guard notchInteractionMode == .automations, automationsSurface == .list,
              !automations.isEmpty else { return false }
        let count = automations.count
        automationsSelectionIndex = (automationsSelectionIndex + delta + count) % count
        return true
    }

    func openSelectedAutomationDetail() -> Bool {
        guard notchInteractionMode == .automations, automationsSurface == .list,
              let a = automations[safe: automationsSelectionIndex] else { return false }
        automationsSurface = .detail(a.id)
        loadAutomationDetailRuns(a.id)
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
        automationsSurface = .editorTask
    }

    func startAutomationEdit(_ automation: AutomationRecord) {
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
    private func updateSlashSuggestions() {
        // Called only from inputText.didSet — an Escape dismissal therefore
        // holds until the text changes again.
        let matches = notchInteractionMode == .settings
            ? []
            : SlashCommandRegistry.suggestions(for: inputText)
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

    /// Return/Tab/click on a suggestion: canonical spelling, then execute.
    func acceptSlashSuggestion(_ command: SlashCommand? = nil) -> Bool {
        guard let cmd = command ?? slashSuggestions[safe: slashSelectionIndex] else { return false }
        slashSuggestions = []
        slashSelectionIndex = 0
        inputText = cmd.command
        execute(cmd)
        return true
    }

    func execute(_ command: SlashCommand) {
        switch command.action {
        case .openSettings:
            openNotchSettings()
        case .openAutomations:
            openAutomations()
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
        if !pendingApprovals.isEmpty { return .awaitingApproval }
        if isThinking { return .thinking }
        if let h = daemonHealth, (!h.daemonUp || !h.baseRTUp) { return .error }
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
        notchInteractionMode = .output
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
        notchInteractionMode = .input
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

    @Published var selectedModel: String = BaseRTLaunchAgent.model {
        didSet { UserDefaults.standard.set(selectedModel, forKey: "bagent.model") }
    }

    @Published var selectedClassifierModel: String = BaseRTLaunchAgent.model {
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
    @Published var odooMcpAvailable: Bool = false
    @Published var odooToolCount: Int = 0

    /// Save creds to Keychain + UserDefaults and authenticate with the daemon via MCP.
    func configureOdoo() {
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

    private let client = DaemonClient()
    let permissions = PermissionsManager()

    /// Invoked after an input-only turn is submitted so AppKit can collapse the panel.
    var onInputOnlySubmitted: (() -> Void)?
    /// Invoked when output becomes displayable, or completion must recover a
    /// missed first-token transition, so AppKit can reveal the output surface.
    var onFirstAssistantToken: (() -> Void)?

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
    // Session ID persisted in UserDefaults so it survives app restarts
    private var sessionId: String? {
        get { UserDefaults.standard.string(forKey: "bagent.session_id") }
        set { UserDefaults.standard.set(newValue, forKey: "bagent.session_id") }
    }

    var currentSessionId: String? { sessionId }

    // MARK: - Init

    init(startMonitoring: Bool = true) {
        if startMonitoring {
            startHealthMonitor()
            startCmuxMonitor()
            Task { await refreshHealth() }
        }
    }

    func ensureCompletedTurnOutputPresented() {
        if notchInteractionMode != .output || chatSurfaceMode != .outputExpanded {
            onFirstAssistantToken?()
        }
        // Tests and non-window consumers may not install the AppKit callback.
        // Completion is authoritative even if click-away collapsed the thinking
        // surface before the first token reached the main actor.
        if notchInteractionMode != .output || chatSurfaceMode != .outputExpanded {
            isExpanded = true
            chatSurfaceMode = .outputExpanded
            notchInteractionMode = .output
        }
    }

    // MARK: - Actions

    func clear() {
        messages = []
        inputText = ""
        isThinking = false
        streamingAssistantMessageId = nil
        pendingAttachments = []
        pendingConnectorActions = []
        selectedSourceMode = nil
        hoveredSourceMode = nil
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
                    selectedModel = fetched.first ?? BaseRTLaunchAgent.model
                }
                if !fetched.contains(selectedClassifierModel) {
                    selectedClassifierModel = fetched.first ?? BaseRTLaunchAgent.model
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
        if !messages.isEmpty && sessionId == nil {
            await startNewSession()
        }
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
        if let sessionId {
            do {
                conversationPayload = try await client.debugConversation(id: sessionId)
            } catch {
                conversationPayload = "Conversation debug unavailable: \(error.localizedDescription)"
            }
        } else {
            conversationPayload = "No conversation id yet."
        }

        let sessionLine = sessionId ?? "(none)"
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
        let text = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !pendingAttachments.isEmpty else { return }
        // A keyboard-selected suggestion wins over raw text; only complete
        // recognized commands execute. Unknown slash text submits normally.
        if !slashSuggestions.isEmpty, acceptSlashSuggestion() {
            return
        }
        if let cmd = SlashCommandRegistry.exactMatch(text) {
            execute(cmd)
            return
        }
        guard !isThinking else { return }
        let sourceMode = selectedSourceMode
        let wasInputOnly = chatSurfaceMode == .inputOnly
        if messages.isEmpty {
            sessionId = nil
        }
        inputText = ""
        historyBrowseIndex = nil
        pendingConnectorActions = []
        let model = selectedModel
        let sid = sessionId
        let attachments = pendingAttachments
        pendingAttachments = []
        let attachmentIds = attachments.map { $0.id }
        // Sliding-window history so the model can resolve follow-ups.
        // Daemon clamps again (10 turns / 8k chars) — these caps must match.
        let history: [DaemonClient.HistoryTurn] = messages
            .filter { !$0.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .suffix(10)
            .map { .init(role: $0.role == .user ? "user" : "assistant",
                         content: String($0.content.prefix(1500))) }
        var userMsg = ChatMessage(role: .user, content: text)
        userMsg.attachments = attachments
        messages.append(userMsg)
        isThinking = true
        if let sourceMode {
            recordSourceModeUse(sourceMode)
        }
        if wasInputOnly {
            onInputOnlySubmitted?()
        }

        Task {
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

                let stream = client.chatStream(text: text, sessionId: sid, model: model, attachmentIds: attachmentIds, screenContext: screenCtx, sourceMode: sourceMode, history: history)
                var first = true
                var didAutoOpen = false
                let presenter = AdaptiveStreamPresenter { [weak self] edit in
                    guard let self, messages.indices.contains(idx) else { return }
                    messages[idx].displayedContent += edit
                    streamingChunk += 1
                }
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
                        messages[idx].content += t
                        presenter.enqueue(t)
                        if first {
                            isThinking = false
                            first = false
                            ensureCompletedTurnOutputPresented()
                        }
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
                        toolStatus = event.title
                    case .activityCompleted(let event):
                        if let activityIndex = messages[idx].activities.firstIndex(where: { $0.id == event.id }) {
                            messages[idx].activities[activityIndex].status = event.status ?? "completed"
                            messages[idx].activities[activityIndex].durationMs = event.durationMs
                        }
                    case .evidencePhase, .logicalActivityStarted, .logicalActivityCompleted,
                         .evidenceOutcome, .evidencePolish:
                        if let status = EvidencePresentation.apply(event, to: &messages[idx]) {
                            toolStatus = status
                        }
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
                        isThinking = true
                        toolStatus = Self.toolStatusLabel(for: tool)
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
                        isThinking = false
                        streamingAssistantMessageId = nil
                        messages[idx].content = message
                        ensureCompletedTurnOutputPresented()
                    case .taskRating(let level, let score, let reasons, let privacyRisk):
                        messages[idx].taskRating = (level: level, score: score, reasons: reasons, privacyRisk: privacyRisk)
                    case .done(let returnedSessionId):
                        await presenter.finish()
                        for activityIndex in messages[idx].activities.indices
                            where messages[idx].activities[activityIndex].status == "running" {
                            messages[idx].activities[activityIndex].status = "completed"
                        }
                        if let sid = returnedSessionId { sessionId = sid }
                        toolStatus = nil
                        isThinking = false
                        streamingAssistantMessageId = nil
                        ensureCompletedTurnOutputPresented()
                        Task { await loadDebugTrace(for: messages[idx].id) }
                    }
                }
                await presenter.finish()
                isThinking = false
                streamingAssistantMessageId = nil
                ensureCompletedTurnOutputPresented()
            } catch {
                isThinking = false
                toolStatus = nil
                streamingAssistantMessageId = nil
                messages[idx].content = "Chyba: \(error.localizedDescription)"
                ensureCompletedTurnOutputPresented()
            }
        }
    }

    // MARK: - Daemon-wide events (automations + background approvals)

    /// Set by AppDelegate — opens the notch when a background approval arrives.
    var onApprovalArrived: (() -> Void)?
    /// Bumped on any automation lifecycle event; the automations surface
    /// refetches authoritative records when it changes.
    @Published var automationsRefreshID = UUID()
    private var eventsMonitorTask: Task<Void, Never>?

    /// Subscribe to GET /events with reconnect. Pending approvals are fetched
    /// at start and after every reconnect so durable approvals are not missed.
    func startEventsMonitor() {
        eventsMonitorTask?.cancel()
        eventsMonitorTask = Task {
            while !Task.isCancelled {
                await refreshPendingApprovals()
                do {
                    for try await event in client.globalEvents() {
                        guard !Task.isCancelled else { return }
                        await handleGlobalEvent(event)
                    }
                } catch {
                    // Daemon restarting or unreachable — retry shortly.
                }
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    func stopEventsMonitor() {
        eventsMonitorTask?.cancel()
        eventsMonitorTask = nil
    }

    private func handleGlobalEvent(_ event: DaemonClient.GlobalEvent) async {
        switch event.type {
        case "approval_requested":
            // Event payloads are notifications, not truth — refetch.
            await refreshPendingApprovals()
        case let t where t.hasPrefix("automation"):
            automationsRefreshID = UUID()
            if notchInteractionMode == .automations {
                await refreshAutomations()
            }
        default:
            break
        }
    }

    /// Refetch the authoritative pending-approval list; opens the notch when
    /// approvals exist and nothing is being shown.
    func refreshPendingApprovals() async {
        guard let items = try? await client.pendingApprovals() else { return }
        let hadNone = pendingApprovals.isEmpty
        pendingApprovals = items
        if hadNone && !items.isEmpty {
            onApprovalArrived?()
        }
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
