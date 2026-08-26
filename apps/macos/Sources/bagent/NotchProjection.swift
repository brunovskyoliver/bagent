import CoreGraphics
import Foundation

enum NotchInteractionMode: String, Codable, Equatable, Sendable {
    case collapsed
    case input
    case thinking
    case output
    case settings
    case automations
}

enum NotchWorkOrigin: String, Codable, Equatable, Sendable {
    case conversation
    case automation
}

enum NotchWorkState: String, Codable, CaseIterable, Equatable, Sendable {
    case queued
    case waitingForModel = "waiting_for_model"
    case running
    case waitingForApproval = "waiting_for_approval"
    case cancelling
    case completed
    case partial
    case failed
    case cancelled
    case abandoned

    var isTerminal: Bool {
        switch self {
        case .completed, .partial, .failed, .cancelled, .abandoned: true
        default: false
        }
    }
}

enum NotchActivityCategory: String, Codable, CaseIterable, Equatable, Sendable {
    case mail
    case web
    case filesystem
    case odoo
    case codex
    case chat
    case automation
    case genericTool = "generic_tool"

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer().decode(String.self)
        self = Self(rawValue: value) ?? .genericTool
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }

    var label: String {
        switch self {
        case .mail: "Checking Mail"
        case .web: "Checking Web"
        case .filesystem: "Checking Files"
        case .odoo: "Checking Odoo"
        case .codex: "Running Codex"
        case .chat: "Preparing answer"
        case .automation: "Running Automation"
        case .genericTool: "Using a tool"
        }
    }

    var symbolName: String {
        switch self {
        case .mail: "envelope.fill"
        case .web: "globe"
        case .filesystem: "folder.fill"
        case .odoo: "building.2.fill"
        case .codex: "chevron.left.forwardslash.chevron.right"
        case .chat: "bubble.left.fill"
        case .automation: "clock.arrow.2.circlepath"
        case .genericTool: "wrench.and.screwdriver.fill"
        }
    }
}

struct NotchActivity: Codable, Equatable, Sendable {
    let category: NotchActivityCategory
}

enum NotchTerminalAttention: String, Codable, CaseIterable, Equatable, Sendable {
    case unread
    case partial
    case failed
}

enum NotchModelPhase: String, Codable, CaseIterable, Equatable, Sendable {
    case unavailable
    case unloaded
    case loading
    case loadedNotReady = "loaded_not_ready"
    case ready
    case retiring
    case poisoned
    case restarting

    var isTransitioning: Bool {
        switch self {
        case .loading, .loadedNotReady, .retiring, .restarting: true
        default: false
        }
    }
}

struct NotchWork: Codable, Equatable, Identifiable, Sendable {
    let identity: String
    let revision: UInt64
    let origin: NotchWorkOrigin
    let state: NotchWorkState
    let activity: NotchActivity?
    let queuePosition: Int?
    let automationDisplayName: String?
    var automationDefinitionIdentity: String? = nil
    var automationDefinitionDetached: Bool = false
    var automationSessionIdentity: String? = nil
    let terminalAttention: NotchTerminalAttention?
    var terminalFinishedAt: String? = nil
    var terminalOrder: UInt64? = nil
    var claimedOrder: UInt64 = 0

    var id: String { identity }

    enum CodingKeys: String, CodingKey {
        case identity, revision, origin, state, activity, queuePosition
        case automationDisplayName, automationDefinitionIdentity, automationDefinitionDetached
        case automationSessionIdentity, terminalAttention, terminalFinishedAt, terminalOrder, claimedOrder
    }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        identity = try values.decode(String.self, forKey: .identity)
        revision = try values.decode(UInt64.self, forKey: .revision)
        origin = try values.decode(NotchWorkOrigin.self, forKey: .origin)
        state = try values.decode(NotchWorkState.self, forKey: .state)
        activity = try values.decodeIfPresent(NotchActivity.self, forKey: .activity)
        queuePosition = try values.decodeIfPresent(Int.self, forKey: .queuePosition)
        automationDisplayName = try values.decodeIfPresent(String.self, forKey: .automationDisplayName)
        automationDefinitionIdentity = try values.decodeIfPresent(String.self, forKey: .automationDefinitionIdentity)
        automationDefinitionDetached = try values.decodeIfPresent(Bool.self, forKey: .automationDefinitionDetached) ?? false
        automationSessionIdentity = try values.decodeIfPresent(String.self, forKey: .automationSessionIdentity)
        terminalAttention = try values.decodeIfPresent(NotchTerminalAttention.self, forKey: .terminalAttention)
        terminalFinishedAt = try values.decodeIfPresent(String.self, forKey: .terminalFinishedAt)
        terminalOrder = try values.decodeIfPresent(UInt64.self, forKey: .terminalOrder)
        claimedOrder = try values.decodeIfPresent(UInt64.self, forKey: .claimedOrder) ?? 0
    }

    init(
        identity: String,
        revision: UInt64,
        origin: NotchWorkOrigin,
        state: NotchWorkState,
        activity: NotchActivity?,
        queuePosition: Int?,
        automationDisplayName: String?,
        automationDefinitionIdentity: String? = nil,
        automationDefinitionDetached: Bool = false,
        automationSessionIdentity: String? = nil,
        terminalAttention: NotchTerminalAttention?,
        terminalFinishedAt: String? = nil,
        terminalOrder: UInt64? = nil,
        claimedOrder: UInt64 = 0
    ) {
        self.identity = identity
        self.revision = revision
        self.origin = origin
        self.state = state
        self.activity = activity
        self.queuePosition = queuePosition
        self.automationDisplayName = automationDisplayName
        self.automationDefinitionIdentity = automationDefinitionIdentity
        self.automationDefinitionDetached = automationDefinitionDetached
        self.automationSessionIdentity = automationSessionIdentity
        self.terminalAttention = terminalAttention
        self.terminalFinishedAt = terminalFinishedAt
        self.terminalOrder = terminalOrder
        self.claimedOrder = claimedOrder
    }
}

struct NotchApproval: Codable, Equatable, Identifiable, Sendable {
    let identity: String
    let workIdentity: String
    var id: String { identity }
}

struct NotchWorkSnapshot: Codable, Equatable, Sendable {
    let schemaVersion: Int
    let cursor: UInt64
    let daemonGeneration: String
    var works: [NotchWork]
    var pendingApprovals: [NotchApproval]
    var model: NotchModelPhase
}

struct NotchWorkEvent: Codable, Equatable, Sendable {
    let schemaVersion: Int
    let cursor: UInt64
    let daemonGeneration: String
    let work: NotchWork
    var pendingApprovals: [NotchApproval] = []
    let model: NotchModelPhase
}

enum NotchLocalIntent: Equatable, Sendable {
    case collapse
    case openInput
    case openOutput
    case openSettings
    case openAutomations
    case selectAutomation(String)
    case cycleAutomation
    case motionPreferenceChanged
}

enum NotchProjectionInput: Equatable, Sendable {
    case snapshot(NotchWorkSnapshot)
    case event(NotchWorkEvent)
    case localIntent(NotchLocalIntent)
}

struct NotchProjectionRevision: Codable, Equatable, Sendable {
    let cursor: UInt64
    let daemonGeneration: String
}

enum StageRailStage: String, Codable, CaseIterable, Equatable, Sendable {
    case model = "Model"
    case think = "Think"
    case tool = "Tool"
    case done = "Done"
}

struct StageRailPresentation: Codable, Equatable, Sendable {
    let selectedStage: StageRailStage?
    let activityCategory: NotchActivityCategory?
    let caption: String
    let secondaryCaption: String?
    let terminalAttentionMarker: NotchTerminalAttention?
    let accessibilityValue: String
}

enum NotchFocusedDestination: Equatable, Sendable {
    case currentChat
    case activeAutomation(definitionIdentity: String)
    case terminalAutomation(
        definitionIdentity: String,
        sessionIdentity: String,
        workIdentity: String,
        expectedRevision: UInt64
    )
}

struct NotchStatusPillPresentation: Codable, Equatable, Sendable {
    let label: String?
    let accessibilityLabel: String
    let accessibilityValue: String

    func opensAutomations(activeAutomationCount: Int) -> Bool {
        activeAutomationCount > 0
            && (label == "ACTIVE" || label?.hasSuffix(" ACTIVE") == true)
    }
}

struct NotchGeometry: Codable, Equatable, Sendable {
    let wingWidth: CGFloat
    let bridgeHeight: CGFloat
}

struct NotchMotionPresentation: Codable, Equatable, Sendable {
    let reduceMotion: Bool
    let surfaceDuration: TimeInterval
    let contentRevealDelay: TimeInterval
    let contentRevealDuration: TimeInterval
    let pillCrossfadeDuration: TimeInterval
    let iconMotionEnabled: Bool
    let completionRevealDuration: TimeInterval

    static func accepted(reduceMotion: Bool) -> Self {
        if reduceMotion {
            return .init(
                reduceMotion: true,
                surfaceDuration: 0,
                contentRevealDelay: 0,
                contentRevealDuration: 0.12,
                pillCrossfadeDuration: 0.16,
                iconMotionEnabled: false,
                completionRevealDuration: 0.12
            )
        }
        return .init(
            reduceMotion: false,
            surfaceDuration: 0.58,
            contentRevealDelay: 0.36,
            contentRevealDuration: 0.22,
            pillCrossfadeDuration: 0.16,
            iconMotionEnabled: true,
            completionRevealDuration: 0.24
        )
    }
}

struct NotchPresentation: Equatable, Sendable, CustomDebugStringConvertible, CustomReflectable {
    var revision: NotchProjectionRevision
    var interactionMode: NotchInteractionMode
    var rail: StageRailPresentation
    var statusPill: NotchStatusPillPresentation
    var geometry: NotchGeometry
    var motion: NotchMotionPresentation
    var focusedWorkIdentity: String?
    var activeAutomationCount: Int
    var runPosition: Int?
    var focusedDestination: NotchFocusedDestination?
    var pendingApprovalIdentity: String?
    var hasActiveForegroundWork: Bool
    var snapshot: NotchWorkSnapshot
    fileprivate var selectedAutomationIdentity: String?

    static let idle: Self = {
        let snapshot = NotchWorkSnapshot(
            schemaVersion: 1,
            cursor: 0,
            daemonGeneration: "",
            works: [],
            pendingApprovals: [],
            model: .unloaded
        )
        return NotchProjection.render(
            snapshot: snapshot,
            mode: .collapsed,
            selectedAutomationIdentity: nil,
            reduceMotion: false
        )
    }()

    var canOpenFocusedDestination: Bool { focusedDestination != nil }

    var debugDescription: String {
        "NotchPresentation(cursor: \(revision.cursor), mode: \(interactionMode.rawValue), "
            + "stage: \(rail.selectedStage?.rawValue ?? "idle"), active: \(activeAutomationCount))"
    }

    var customMirror: Mirror {
        Mirror(self, children: [
            "cursor": revision.cursor,
            "mode": interactionMode.rawValue,
            "stage": rail.selectedStage?.rawValue ?? "idle",
            "activeAutomationCount": activeAutomationCount,
        ])
    }

    var privacySafeCaptureMetadata: [String: String] {
        [
            "mode": interactionMode.rawValue,
            "stage": rail.selectedStage?.rawValue ?? "idle",
            "status": statusPill.label ?? "hidden",
            "activeCount": String(activeAutomationCount),
        ]
    }
}

enum NotchProjectionError: Error, Equatable {
    case unsupportedSchema(Int)
    case missingSnapshot
    case cursorGap(expected: UInt64, actual: UInt64)
    case daemonGenerationChanged
    case revisionMismatch(workIdentity: String, expected: UInt64, actual: UInt64)
}

enum NotchProjection {
    static let supportedSchemaVersion = 1

    static func reduce(
        previous: NotchPresentation,
        input: NotchProjectionInput,
        reduceMotion: Bool = false
    ) throws -> NotchPresentation {
        switch input {
        case .snapshot(let snapshot):
            guard snapshot.schemaVersion == supportedSchemaVersion else {
                throw NotchProjectionError.unsupportedSchema(snapshot.schemaVersion)
            }
            return render(
                snapshot: snapshot,
                mode: previous.interactionMode,
                selectedAutomationIdentity: previous.selectedAutomationIdentity,
                reduceMotion: reduceMotion,
                revealForegroundCompletion: shouldRevealForegroundCompletion(
                    previous: previous.snapshot,
                    next: snapshot
                )
            )

        case .event(let event):
            guard previous.revision.daemonGeneration.isEmpty == false else {
                throw NotchProjectionError.missingSnapshot
            }
            guard event.schemaVersion == supportedSchemaVersion else {
                throw NotchProjectionError.unsupportedSchema(event.schemaVersion)
            }
            guard event.daemonGeneration == previous.revision.daemonGeneration else {
                throw NotchProjectionError.daemonGenerationChanged
            }
            if event.cursor <= previous.revision.cursor { return previous }
            let expectedCursor = previous.revision.cursor + 1
            guard event.cursor == expectedCursor else {
                throw NotchProjectionError.cursorGap(expected: expectedCursor, actual: event.cursor)
            }

            var snapshot = previous.snapshot
            if let index = snapshot.works.firstIndex(where: { $0.identity == event.work.identity }) {
                let current = snapshot.works[index]
                if event.work.revision <= current.revision { return previous }
                let expectedRevision = current.revision + 1
                guard event.work.revision == expectedRevision else {
                    throw NotchProjectionError.revisionMismatch(
                        workIdentity: event.work.identity,
                        expected: expectedRevision,
                        actual: event.work.revision
                    )
                }
                snapshot.works[index] = event.work
            } else {
                guard event.work.revision == 1 else {
                    throw NotchProjectionError.revisionMismatch(
                        workIdentity: event.work.identity,
                        expected: 1,
                        actual: event.work.revision
                    )
                }
                snapshot.works.append(event.work)
            }
            snapshot = .init(
                schemaVersion: snapshot.schemaVersion,
                cursor: event.cursor,
                daemonGeneration: snapshot.daemonGeneration,
                works: snapshot.works,
                pendingApprovals: event.pendingApprovals,
                model: event.model
            )
            return render(
                snapshot: snapshot,
                mode: previous.interactionMode,
                selectedAutomationIdentity: previous.selectedAutomationIdentity,
                reduceMotion: reduceMotion,
                revealForegroundCompletion: event.work.origin == .conversation
                    && [.completed, .partial, .failed].contains(event.work.state)
            )

        case .localIntent(let intent):
            var mode = previous.interactionMode
            var selection = previous.selectedAutomationIdentity
            switch intent {
            case .collapse: mode = .collapsed
            case .openInput: mode = .input
            case .openOutput: mode = .output
            case .openSettings: mode = .settings
            case .openAutomations: mode = .automations
            case .selectAutomation(let identity): selection = identity
            case .cycleAutomation:
                let active = orderedActiveAutomations(previous.snapshot)
                if let current = selection,
                   let index = active.firstIndex(where: { $0.identity == current }),
                   !active.isEmpty {
                    selection = active[(index + 1) % active.count].identity
                } else {
                    selection = active.first?.identity
                }
            case .motionPreferenceChanged:
                break
            }
            return render(
                snapshot: previous.snapshot,
                mode: mode,
                selectedAutomationIdentity: selection,
                reduceMotion: reduceMotion,
                revealForegroundCompletion: false
            )
        }
    }

    fileprivate static func render(
        snapshot: NotchWorkSnapshot,
        mode: NotchInteractionMode,
        selectedAutomationIdentity: String?,
        reduceMotion: Bool,
        revealForegroundCompletion: Bool = false
    ) -> NotchPresentation {
        let active = snapshot.works.filter { !$0.state.isTerminal }
        let activeAutomations = orderedActiveAutomations(snapshot)
        let foreground = active.first(where: { $0.origin == .conversation })
        let focusedAutomation = selectedAutomationIdentity
            .flatMap { selected in activeAutomations.first(where: { $0.identity == selected }) }
            ?? activeAutomations.first
        let terminal = terminalAttentionWork(snapshot)
        let terminalMarker = highestPriorityTerminalAttention(snapshot)
        let approval = snapshot.pendingApprovals.first

        let focused: NotchWork? = approval.flatMap { pending in
            snapshot.works.first(where: { $0.identity == pending.workIdentity })
        } ?? foreground ?? focusedAutomation ?? terminal

        let focusedDestination: NotchFocusedDestination? = {
            guard approval == nil, let focused else { return nil }
            if focused.origin == .conversation { return .currentChat }
            guard let definitionIdentity = focused.automationDefinitionIdentity else { return nil }
            if focused.terminalAttention != nil,
               let sessionIdentity = focused.automationSessionIdentity {
                return .terminalAutomation(
                    definitionIdentity: definitionIdentity,
                    sessionIdentity: sessionIdentity,
                    workIdentity: focused.identity,
                    expectedRevision: focused.revision
                )
            }
            return .activeAutomation(definitionIdentity: definitionIdentity)
        }()

        let focusedActivityCategory: NotchActivityCategory? = {
            if let category = focused?.activity?.category { return category }
            guard let focused, focused.state == .running else { return nil }
            return focused.origin == .conversation ? .chat : .automation
        }()

        let stage: StageRailStage? = {
            if approval != nil { return focused?.state == .waitingForModel ? .model : .tool }
            guard let focused else {
                return snapshot.model.isTransitioning ? .model : nil
            }
            if focused.terminalAttention != nil { return .done }
            switch focused.state {
            case .waitingForModel: return .model
            case .running:
                return focused.activity == nil
                    || focused.activity?.category == .chat
                    || focused.activity?.category == .automation
                    ? .think
                    : .tool
            case .queued, .cancelling, .waitingForApproval: return .think
            case .completed, .partial, .failed, .cancelled, .abandoned: return .done
            }
        }()

        let pillLabel: String? = {
            if approval != nil { return "APPROVE" }
            if !active.isEmpty {
                if !activeAutomations.isEmpty {
                    return activeAutomations.count == 1 ? "ACTIVE" : "\(activeAutomations.count) ACTIVE"
                }
                return "ACTIVE"
            }
            if snapshot.model.isTransitioning { return "LOADING" }
            if snapshot.works.contains(where: { $0.terminalAttention == .failed }) { return "FAILED" }
            if snapshot.works.contains(where: { $0.terminalAttention == .partial }) { return "PARTIAL" }
            if snapshot.works.contains(where: { $0.terminalAttention == .unread }) { return "UNREAD" }
            if snapshot.model == .ready { return "RESIDENT" }
            return nil
        }()

        let bridgeHeight: CGFloat = {
            if approval != nil { return 176 }
            if activeAutomations.count >= 2 { return 150 }
            let foregroundOutput = (mode == .output || revealForegroundCompletion)
                && snapshot.works.contains { $0.origin == .conversation && $0.state.isTerminal }
            if (foreground != nil || foregroundOutput), !activeAutomations.isEmpty { return 126 }
            if terminal != nil, active.isEmpty { return 98 }
            if !active.isEmpty || snapshot.model.isTransitioning { return 78 }
            return mode == .collapsed ? 0 : 78
        }()

        let position = focusedAutomation.flatMap { selected in
            activeAutomations.firstIndex(where: { $0.identity == selected.identity }).map { $0 + 1 }
        }
        let originLabel: String = focused?.origin == .conversation ? "foreground" : "background"
        let caption: String = {
            if approval != nil { return "Approval required" }
            if let name = focused?.automationDisplayName, !name.isEmpty {
                return String(name.prefix(80))
            }
            if let category = focusedActivityCategory { return category.label }
            switch stage {
            case .model: return "Loading model"
            case .think: return focused?.origin == .automation ? "Running Automation" : "Preparing answer"
            case .tool: return "Using a tool"
            case .done: return terminalLabel(focused?.terminalAttention)
            case nil: return "Idle"
            }
        }()
        let runText = position.map { "run \($0) of \(activeAutomations.count)" }
        let activeText = activeAutomations.isEmpty ? nil : "\(activeAutomations.count) active"
        let queueText = focused?.queuePosition.map { "queue position \($0)" }
        let accessibilityParts = [stage?.rawValue, focusedActivityCategory?.label, caption,
                                  focused == nil ? nil : originLabel, runText, activeText, queueText,
                                  focused?.terminalAttention.map(terminalLabel),
                                  terminalMarker.map { "Done marker: \(terminalLabel($0))" }]
            .compactMap { $0 }

        return .init(
            revision: .init(cursor: snapshot.cursor, daemonGeneration: snapshot.daemonGeneration),
            interactionMode: approval != nil
                ? .output
                : resolvedMode(
                    mode,
                    snapshot: snapshot,
                    revealForegroundCompletion: revealForegroundCompletion
                ),
            rail: .init(
                selectedStage: stage,
                activityCategory: focusedActivityCategory,
                caption: caption,
                secondaryCaption: foreground != nil && !activeAutomations.isEmpty
                    ? "Background work continues"
                    : runText,
                terminalAttentionMarker: terminalMarker,
                accessibilityValue: accessibilityParts.joined(separator: ", ")
            ),
            statusPill: .init(
                label: pillLabel,
                accessibilityLabel: "Status",
                accessibilityValue: statusAccessibilityValue(pillLabel, automationCount: activeAutomations.count)
            ),
            // Status lives on the compact right-wing dot; no text pill ever
            // widens the collapsed notch.
            geometry: .init(wingWidth: 32, bridgeHeight: bridgeHeight),
            motion: .accepted(reduceMotion: reduceMotion),
            focusedWorkIdentity: focused?.identity,
            activeAutomationCount: activeAutomations.count,
            runPosition: position,
            focusedDestination: focusedDestination,
            pendingApprovalIdentity: approval?.identity,
            hasActiveForegroundWork: foreground != nil,
            snapshot: snapshot,
            selectedAutomationIdentity: focusedAutomation?.identity
        )
    }

    private static func resolvedMode(
        _ requested: NotchInteractionMode,
        snapshot: NotchWorkSnapshot,
        revealForegroundCompletion: Bool
    ) -> NotchInteractionMode {
        if !snapshot.pendingApprovals.isEmpty { return .output }
        if snapshot.works.contains(where: { !$0.state.isTerminal && $0.origin == .conversation }) {
            return requested == .output ? .output : .thinking
        }
        if revealForegroundCompletion {
            return .output
        }
        if requested == .thinking { return .collapsed }
        return requested
    }

    private static func shouldRevealForegroundCompletion(
        previous: NotchWorkSnapshot,
        next: NotchWorkSnapshot
    ) -> Bool {
        next.works.contains { nextWork in
            guard nextWork.origin == .conversation,
                  [.completed, .partial, .failed].contains(nextWork.state)
            else { return false }
            return previous.works.first(where: { $0.identity == nextWork.identity }).map {
                $0.revision != nextWork.revision || ![.completed, .partial, .failed].contains($0.state)
            } ?? false
        }
    }

    private static func orderedActiveAutomations(_ snapshot: NotchWorkSnapshot) -> [NotchWork] {
        snapshot.works
            .filter { $0.origin == .automation && !$0.state.isTerminal }
            .sorted {
                if $0.claimedOrder != $1.claimedOrder { return $0.claimedOrder < $1.claimedOrder }
                return $0.identity < $1.identity
            }
    }

    private static func terminalAttentionWork(_ snapshot: NotchWorkSnapshot) -> NotchWork? {
        return snapshot.works
            .filter { $0.terminalAttention != nil }
            .sorted {
                let lhsOrder = $0.terminalOrder ?? 0
                let rhsOrder = $1.terminalOrder ?? 0
                if lhsOrder != rhsOrder { return lhsOrder > rhsOrder }
                return $0.identity < $1.identity
            }
            .first
    }

    private static func highestPriorityTerminalAttention(
        _ snapshot: NotchWorkSnapshot
    ) -> NotchTerminalAttention? {
        if snapshot.works.contains(where: { $0.terminalAttention == .failed }) { return .failed }
        if snapshot.works.contains(where: { $0.terminalAttention == .partial }) { return .partial }
        if snapshot.works.contains(where: { $0.terminalAttention == .unread }) { return .unread }
        return nil
    }

    private static func terminalLabel(_ attention: NotchTerminalAttention?) -> String {
        switch attention {
        case .failed: "Failed"
        case .partial: "Partial"
        case .unread: "Unread completion"
        case nil: "Done"
        }
    }

    private static func statusAccessibilityValue(_ label: String?, automationCount: Int) -> String {
        guard let label else { return "Hidden" }
        if label.hasSuffix("ACTIVE"), automationCount > 0 {
            return automationCount == 1 ? "one active Automation Run" : "\(automationCount) active Automation Runs"
        }
        switch label {
        case "APPROVE": return "approval required"
        case "LOADING": return "model loading"
        case "FAILED": return "unacknowledged failed Automation Session"
        case "PARTIAL": return "unacknowledged partial Automation Session"
        case "UNREAD": return "unacknowledged Automation Session"
        case "RESIDENT": return "model resident"
        default: return label.lowercased()
        }
    }
}

enum NotchPillLayout {
    static let size = CGSize(width: 74, height: 18)

    static func origin(maxPanelWidth: CGFloat) -> CGPoint {
        CGPoint(x: maxPanelWidth - (260 - 248) - size.width - 12, y: 9)
    }

    /// Settings has 205 pt wings. Keep the pill's 74 × 18 pt frame 12 pt from
    /// that visible right edge while the outer panel remains fixed at max size.
    static func settingsOrigin(maxPanelWidth: CGFloat) -> CGPoint {
        CGPoint(
            x: maxPanelWidth - (NotchWrapMetrics.maxWingWidth - NotchWrapMetrics.settingsWingWidth)
                - size.width - 12,
            y: 9
        )
    }
}

enum NotchProjectionDecodingError: Error, Equatable {
    case malformedObject
    case unknownField(String)
}

enum NotchProjectionDecoder {
    static func decodeSnapshot(_ data: Data) throws -> NotchWorkSnapshot {
        let object = try JSONSerialization.jsonObject(with: data)
        guard let dictionary = object as? [String: Any] else {
            throw NotchProjectionDecodingError.malformedObject
        }
        try requireOnly(
            dictionary,
            allowed: ["schemaVersion", "cursor", "daemonGeneration", "works", "pendingApprovals", "model"],
            path: "snapshot"
        )
        guard let works = dictionary["works"] as? [[String: Any]],
              let approvals = dictionary["pendingApprovals"] as? [[String: Any]]
        else { throw NotchProjectionDecodingError.malformedObject }
        for (index, work) in works.enumerated() {
            try validate(work: work, path: "works[\(index)]")
        }
        for (index, approval) in approvals.enumerated() {
            try requireOnly(
                approval,
                allowed: ["identity", "workIdentity"],
                path: "pendingApprovals[\(index)]"
            )
        }
        return try JSONDecoder().decode(NotchWorkSnapshot.self, from: data)
    }

    static func decodeEvent(_ data: Data) throws -> NotchWorkEvent {
        let object = try JSONSerialization.jsonObject(with: data)
        guard let dictionary = object as? [String: Any] else {
            throw NotchProjectionDecodingError.malformedObject
        }
        try requireOnly(
            dictionary,
            allowed: [
                "schemaVersion", "cursor", "daemonGeneration", "work", "pendingApprovals", "model",
            ],
            path: "event"
        )
        guard let work = dictionary["work"] as? [String: Any],
              let approvals = dictionary["pendingApprovals"] as? [[String: Any]]
        else {
            throw NotchProjectionDecodingError.malformedObject
        }
        try validate(work: work, path: "event.work")
        for (index, approval) in approvals.enumerated() {
            try requireOnly(
                approval,
                allowed: ["identity", "workIdentity"],
                path: "event.pendingApprovals[\(index)]"
            )
        }
        return try JSONDecoder().decode(NotchWorkEvent.self, from: data)
    }

    private static func validate(work: [String: Any], path: String) throws {
        try requireOnly(
            work,
            allowed: [
                "identity", "revision", "origin", "state", "activity", "queuePosition",
                "automationDisplayName", "automationDefinitionIdentity", "automationSessionIdentity",
                "automationDefinitionDetached",
                "terminalAttention", "terminalFinishedAt", "terminalOrder", "claimedOrder",
            ],
            path: path
        )
        if let activity = work["activity"] as? [String: Any] {
            try requireOnly(activity, allowed: ["category"], path: "\(path).activity")
        }
    }

    private static func requireOnly(
        _ dictionary: [String: Any],
        allowed: Set<String>,
        path: String
    ) throws {
        if let unknown = Set(dictionary.keys).subtracting(allowed).sorted().first {
            throw NotchProjectionDecodingError.unknownField("\(path).\(unknown)")
        }
    }
}
