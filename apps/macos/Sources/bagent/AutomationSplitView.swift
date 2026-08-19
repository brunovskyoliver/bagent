import Foundation
import SwiftUI

/// The terminal outcome owned by one Automation Run and its isolated
/// Automation Session. Legacy Run Records are intentionally not represented
/// by this type.
enum AutomationRunOutcome: String, CaseIterable, Codable, Equatable, Hashable, Sendable {
    case completed
    case partial
    case failed
    case skipped
    case cancelled
    case abandoned

    var isContinuable: Bool {
        switch self {
        case .completed, .partial, .failed, .cancelled, .abandoned: true
        case .skipped: false
        }
    }

    var stateText: String {
        switch self {
        case .completed: "Dokončené"
        case .partial: "Čiastočné"
        case .failed: "Zlyhané"
        case .skipped: "Preskočené"
        case .cancelled: "Zrušené"
        case .abandoned: "Opustené"
        }
    }

    var stateGlyph: String {
        switch self {
        case .completed: "✓"
        case .partial: "◐"
        case .failed: "!"
        case .skipped: "↷"
        case .cancelled: "×"
        case .abandoned: "…"
        }
    }
}

enum AutomationSessionAttention: String, Codable, Equatable, Hashable, Sendable {
    case none
    case unread
    case viewed
}

struct AutomationContinuationConfirmation: Equatable, Sendable {
    let sessionIdentity: String
    let currentChatIdentity: String
    let seed: String
}

enum AutomationMasterRowKind: Equatable, Hashable, Sendable {
    case active
    case unreadTerminal
    case history
}

/// A privacy-safe master-column projection. It contains identifiers and
/// display-safe state only; session content is loaded after an explicit open.
struct AutomationMasterRow: Identifiable, Equatable, Sendable {
    let id: String
    let kind: AutomationMasterRowKind
    let runIdentity: String?
    let workIdentity: String?
    let sessionIdentity: String?
    let definitionIdentity: String?
    let workRevision: UInt64
    let displayName: String
    let outcome: AutomationRunOutcome?
    private(set) var attention: AutomationSessionAttention
    let detached: Bool
    let claimedOrder: UInt64?
    let finishedAt: Date?
    let schedulerReason: String?

    static func active(
        runIdentity: String,
        workIdentity: String,
        displayName: String,
        claimedOrder: UInt64
    ) -> Self {
        Self(
            id: runIdentity,
            kind: .active,
            runIdentity: runIdentity,
            workIdentity: workIdentity,
            sessionIdentity: nil,
            definitionIdentity: nil,
            workRevision: 0,
            displayName: displayName,
            outcome: nil,
            attention: .none,
            detached: false,
            claimedOrder: claimedOrder,
            finishedAt: nil,
            schedulerReason: nil)
    }

    static func terminal(
        sessionIdentity: String,
        runIdentity: String,
        displayName: String,
        outcome: AutomationRunOutcome,
        attention: AutomationSessionAttention,
        finishedAt: Date?,
        detached: Bool = false,
        schedulerReason: String? = nil,
        workIdentity: String? = nil,
        definitionIdentity: String? = nil,
        workRevision: UInt64 = 0
    ) -> Self {
        Self(
            id: sessionIdentity,
            kind: .unreadTerminal,
            runIdentity: runIdentity,
            workIdentity: workIdentity,
            sessionIdentity: sessionIdentity,
            definitionIdentity: definitionIdentity,
            workRevision: workRevision,
            displayName: displayName,
            outcome: outcome,
            attention: attention,
            detached: detached,
            claimedOrder: nil,
            finishedAt: finishedAt,
            schedulerReason: schedulerReason)
    }

    static let history = Self(
        id: "history",
        kind: .history,
        runIdentity: nil,
        workIdentity: nil,
        sessionIdentity: nil,
        definitionIdentity: nil,
        workRevision: 0,
        displayName: "História",
        outcome: nil,
        attention: .none,
        detached: false,
        claimedOrder: nil,
        finishedAt: nil,
        schedulerReason: nil)

    var isTerminal: Bool { kind == .unreadTerminal }

    var stateText: String {
        if kind == .active { return "Aktívne" }
        return outcome?.stateText ?? "História"
    }

    var stateGlyph: String {
        if kind == .active { return "▶" }
        return outcome?.stateGlyph ?? "⌁"
    }

    var accessibilityValue: String {
        var value = displayName + ", " + stateText
        if attention == .unread { value += ", neprečítané" }
        if attention == .viewed { value += ", zobrazené" }
        if detached { value += ", Odpojená" }
        return value
    }

    fileprivate mutating func markViewed() {
        guard isTerminal, outcome != .skipped else { return }
        attention = .viewed
    }
}

struct AutomationSplitViewProjection: Equatable, Sendable {
    let rows: [AutomationMasterRow]

    static func make(
        active: [AutomationMasterRow],
        unreadTerminal: [AutomationMasterRow]
    ) -> Self {
        let orderedActive = active.sorted {
            ($0.claimedOrder ?? .max, $0.workIdentity ?? $0.id)
                < ($1.claimedOrder ?? .max, $1.workIdentity ?? $1.id)
        }
        let orderedUnread = unreadTerminal.sorted {
            switch ($0.finishedAt, $1.finishedAt) {
            case let (left?, right?) where left != right:
                return left > right
            case (.some, nil):
                return true
            case (nil, .some):
                return false
            default:
                return ($0.runIdentity ?? $0.id) < ($1.runIdentity ?? $1.id)
            }
        }
        return Self(rows: orderedActive + orderedUnread + [.history])
    }

    static func from(_ snapshot: NotchWorkSnapshot) -> Self {
        let works = snapshot.works.filter { $0.origin == .automation }
        let active = works.filter { !$0.state.isTerminal }.map { work in
            AutomationMasterRow.active(
                runIdentity: runIdentity(for: work),
                workIdentity: work.identity,
                displayName: work.automationDisplayName ?? "Odpojená relácia",
                claimedOrder: work.claimedOrder)
        }
        let unread = works.filter { $0.state.isTerminal && $0.terminalAttention != nil }.map { work in
            AutomationMasterRow.terminal(
                sessionIdentity: work.automationSessionIdentity ?? "automation-session:" + work.identity,
                runIdentity: runIdentity(for: work),
                displayName: work.automationDisplayName ?? "Odpojená relácia",
                outcome: outcome(for: work.state),
                attention: .unread,
                finishedAt: AutomationTimeFormat.parse(work.terminalFinishedAt),
                detached: work.automationDefinitionDetached,
                workIdentity: work.identity,
                definitionIdentity: work.automationDefinitionIdentity,
                workRevision: work.revision)
        }
        return make(active: active, unreadTerminal: unread)
    }

    private static func runIdentity(for work: NotchWork) -> String {
        guard let sessionIdentity = work.automationSessionIdentity,
              sessionIdentity.hasPrefix("automation-session:")
        else { return work.identity }
        return String(sessionIdentity.dropFirst("automation-session:".count))
    }

    private static func outcome(for state: NotchWorkState) -> AutomationRunOutcome {
        switch state {
        case .completed: return .completed
        case .partial: return .partial
        case .failed: return .failed
        case .cancelled: return .cancelled
        case .abandoned: return .abandoned
        default: return .failed
        }
    }
}

enum AutomationSplitViewDepth: Equatable, Sendable {
    case split
    case detail
    case child(String)
}

/// Navigation state for the fixed notch panel. The master viewport is a
/// bounded page, not a scroll view, so keyboard navigation remains complete.
struct AutomationSplitViewNavigator: Equatable, Sendable {
    private(set) var rows: [AutomationMasterRow]
    private(set) var selectedIndex = 0
    private(set) var viewportStart = 0
    private(set) var depth: AutomationSplitViewDepth = .split

    static let pageSize = 4

    init(projection: AutomationSplitViewProjection) {
        rows = projection.rows
    }

    var visibleRows: [AutomationMasterRow] {
        Array(rows.dropFirst(viewportStart).prefix(Self.pageSize))
    }

    var selectedRow: AutomationMasterRow? {
        guard rows.indices.contains(selectedIndex) else { return nil }
        return rows[selectedIndex]
    }

    @discardableResult
    mutating func select(rowID: String) -> Bool {
        guard let index = rows.firstIndex(where: { $0.id == rowID }) else { return false }
        selectedIndex = index
        if selectedIndex < viewportStart {
            viewportStart = selectedIndex
        } else if selectedIndex >= viewportStart + Self.pageSize {
            viewportStart = selectedIndex - Self.pageSize + 1
        }
        return true
    }

    @discardableResult
    mutating func moveSelection(by offset: Int) -> Bool {
        guard !rows.isEmpty else { return false }
        let target = selectedIndex + offset
        guard rows.indices.contains(target), target != selectedIndex else { return false }
        selectedIndex = target
        if selectedIndex < viewportStart {
            viewportStart = selectedIndex
        } else if selectedIndex >= viewportStart + Self.pageSize {
            viewportStart = selectedIndex - Self.pageSize + 1
        }
        return true
    }

    func previewSelectedRow() -> AutomationMasterRow? { selectedRow }

    @discardableResult
    mutating func openSelectedTerminal() -> Bool {
        guard rows.indices.contains(selectedIndex), rows[selectedIndex].isTerminal else {
            return false
        }
        rows[selectedIndex].markViewed()
        depth = .detail
        return true
    }

    @discardableResult
    mutating func openSelectedActive() -> Bool {
        guard rows.indices.contains(selectedIndex), rows[selectedIndex].kind == .active else {
            return false
        }
        depth = .detail
        return true
    }

    mutating func openChild(_ child: String) {
        depth = .child(child)
    }

    mutating func resetToSplit() {
        depth = .split
    }

    @discardableResult
    mutating func goBack() -> Bool {
        switch depth {
        case .child:
            depth = .detail
        case .detail:
            depth = .split
        case .split:
            return false
        }
        return true
    }
}

/// The only Automation Run and Automation Session presentation. The master
/// column is a four-row page; full-width children replace the split inside
/// the same fixed bridge.
struct AutomationSessionSplitView: View {
    @ObservedObject var viewModel: ChatViewModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    private var navigator: AutomationSplitViewNavigator {
        get { viewModel.automationSessionNavigator }
        nonmutating set { viewModel.automationSessionNavigator = newValue }
    }

    private let masterWidth: CGFloat = 190
    private let bridgeInset: CGFloat = 16

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            masterColumn
                .frame(width: masterWidth)
            Rectangle()
                .fill(NotchWrapMetrics.notchTextFaint.opacity(0.45))
                .frame(width: 1)
                .padding(.vertical, 8)
                .padding(.horizontal, 8)
            detailColumn
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .padding(.horizontal, bridgeInset)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .onAppear { viewModel.refreshAutomationSessionProjection() }
        .onChange(of: viewModel.automationsRefreshID) { _, _ in
            viewModel.refreshAutomationSessionProjection()
        }
        .accessibilityElement(children: .contain)
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.16), value: navigator.depth)
    }

    private var masterColumn: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 5) {
                Text("Automation Runs")
                    .font(.system(size: 11, weight: .semibold))
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 0)
                Button {
                    viewModel.startAutomationCreation()
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 9, weight: .semibold))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Nová automatizácia")
            }
            ForEach(navigator.visibleRows) { row in
                Button {
                    _ = navigator.select(rowID: row.id)
                } label: {
                    masterRow(row, selected: row.id == navigator.selectedRow?.id)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(row.accessibilityValue)
                .accessibilityAddTraits(row.id == navigator.selectedRow?.id ? [.isSelected] : [])
            }
            Spacer(minLength: 0)
        }
        .frame(maxHeight: .infinity, alignment: .topLeading)
    }

    private func masterRow(_ row: AutomationMasterRow, selected: Bool) -> some View {
        HStack(spacing: 6) {
            Text(row.stateGlyph)
                .font(.system(size: 11, weight: .semibold, design: .monospaced))
                .frame(width: 13)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text(row.displayName)
                    .font(.system(size: 11, weight: .medium))
                    .lineLimit(1)
                Text(row.stateText + (row.detached ? " · Odpojená" : ""))
                    .font(.system(size: 9))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 6)
        .frame(height: 29)
        .background(
            RoundedRectangle(cornerRadius: 5)
                .fill(Color.white.opacity(selected ? 0.12 : 0.04))
        )
    }

    @ViewBuilder
    private var detailColumn: some View {
        switch navigator.depth {
        case .child(let child):
            childView(child)
        case .split:
            if let row = navigator.selectedRow {
                switch row.kind {
                case .active: activeDetail(row)
                case .unreadTerminal: terminalPreview(row)
                case .history: historyDetail
                }
            }
        case .detail:
            if let row = navigator.selectedRow {
                switch row.kind {
                case .active: activeDetail(row)
                case .unreadTerminal: terminalDetail(row)
                case .history: historyDetail
                }
            }
        }
    }

    private func activeDetail(_ row: AutomationMasterRow) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            Text(row.displayName)
                .font(.system(size: 13, weight: .semibold))
                .accessibilityAddTraits(.isHeader)
            ActivityPeekStageRailView(presentation: viewModel.notchPresentation) {
                navigator.openChild("activity")
            }
            Text(viewModel.notchPresentation.rail.caption)
                .font(.system(size: 10))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                .lineLimit(1)
            Text("Aktívne: " + String(viewModel.notchPresentation.activeAutomationCount))
                .font(.system(size: 10))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            Button("Otvoriť priebeh") { navigator.openChild("activity") }
                .buttonStyle(.plain)
                .accessibilityLabel("Otvoriť priebeh")
            Button("Príkazy") { navigator.openChild("commands") }
                .buttonStyle(.plain)
                .accessibilityLabel("Príkazy")
            Spacer(minLength: 0)
        }
    }

    private func terminalPreview(_ row: AutomationMasterRow) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Dvojpanelový prehľad")
                .font(.system(size: 13, weight: .semibold))
                .accessibilityAddTraits(.isHeader)
            Text(row.stateGlyph + "  " + row.stateText)
                .font(.system(size: 11, weight: .medium))
            Text(row.displayName + (row.detached ? " · Odpojená" : ""))
                .font(.system(size: 10))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            Text(row.attention == .unread ? "Neprečítané" : "Zobrazené")
                .font(.system(size: 10))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            Text("Dokončené: " + (row.finishedAt.map { $0.formatted(date: .abbreviated, time: .shortened) } ?? "—"))
                .font(.system(size: 9))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            if let outcome = row.outcome,
               [.failed, .cancelled, .abandoned].contains(outcome) {
                Text("Finálny výstup nevznikol")
                    .font(.system(size: 10, weight: .medium))
            } else {
                Text("Zhrnutie výsledku")
                    .font(.system(size: 10, weight: .medium))
                Text("Výsledok sa zobrazí po otvorení relácie.")
                    .font(.system(size: 9))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            }
            Spacer(minLength: 0)
            Button("Otvoriť reláciu") {
                guard let selected = navigator.selectedRow,
                      navigator.openSelectedTerminal()
                else { return }
                viewModel.openTerminalAutomationSession(selected)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Otvoriť reláciu")
        }
    }

    private func terminalDetail(_ row: AutomationMasterRow) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Automation Session")
                .font(.system(size: 13, weight: .semibold))
                .accessibilityAddTraits(.isHeader)
            Text(row.stateGlyph + "  " + row.stateText + " · " + (row.detached ? "Odpojená" : "historická"))
                .font(.system(size: 10, weight: .medium))
            if let detail = viewModel.automationSessionDetail {
                Text(detail.resultSummary ?? "Bez zhrnutia výsledku.")
                    .font(.system(size: 10))
                    .lineLimit(3)
                if detail.finalOutputAvailable {
                    Button("Finálny výstup") { navigator.openChild("output") }
                        .buttonStyle(.plain)
                } else {
                    Text("Finálny výstup nevznikol")
                        .font(.system(size: 10))
                }
                Text("Aktivity: " + String(detail.activityTimeline.count))
                    .font(.system(size: 9))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                Text("Proveniencia: \(detail.taskSnapshot.displayName), revízia \(detail.taskSnapshot.definitionRevision)")
                    .font(.system(size: 9))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                Text("Zdroje: \(detail.validatedSources.count) · referencie: \(detail.connectorReferences.count)")
                    .font(.system(size: 9))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                ForEach(Array(detail.validatedSources.prefix(3)), id: \.sourceIdentity) { source in
                    Text("Zdroj · \(source.label)")
                        .font(.system(size: 9))
                        .lineLimit(1)
                }
                ForEach(Array(detail.connectorReferences.prefix(3)), id: \.connectorKind) { reference in
                    Text("Referencia · \(reference.connectorKind): \(reference.availability)")
                        .font(.system(size: 9))
                        .lineLimit(1)
                }
                Text("Historické schválenia: \(detail.historicalApprovals.count) · skrátenia: \(detail.truncationDisclosures.count)")
                    .font(.system(size: 9))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                if !detail.activityTimeline.isEmpty {
                    VStack(alignment: .leading, spacing: 2) {
                        ForEach(Array(detail.activityTimeline.prefix(4).enumerated()), id: \.offset) { _, activity in
                            Text("• \(activity.caption)")
                                .font(.system(size: 9))
                                .lineLimit(1)
                        }
                    }
                }
            } else {
                ProgressView().controlSize(.small)
            }
            Spacer(minLength: 0)
            HStack(spacing: 5) {
                if row.outcome?.isContinuable == true {
                    Button("Pokračovať") { navigator.openChild("continue") }
                        .buttonStyle(.plain)
                }
                Button("Relácia Export") { navigator.openChild("sessionExport") }
                    .buttonStyle(.plain)
                Button("Diagnostika Export") { navigator.openChild("diagnosticExport") }
                    .buttonStyle(.plain)
                Button("Príkazy") { navigator.openChild("commands") }
                    .buttonStyle(.plain)
                Button("Vymazať Automation Session") { navigator.openChild("delete") }
                    .buttonStyle(.plain)
            }
        }
    }

    private var historyDetail: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("História")
                .font(.system(size: 13, weight: .semibold))
                .accessibilityAddTraits(.isHeader)
            Text("Vyberte Automation Session v hlavnom zozname.")
                .font(.system(size: 10))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            Spacer(minLength: 0)
        }
    }

    @ViewBuilder
    private func childView(_ child: String) -> some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack {
                Button("Späť") { _ = navigator.goBack() }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Späť")
                Spacer(minLength: 0)
                Text(childTitle(child))
                    .font(.system(size: 12, weight: .semibold))
            }
            .accessibilityElement(children: .contain)
            if child == "output" {
                ScrollView {
                    Text(viewModel.automationSessionDetail?.finalOutput ?? "Finálny výstup nevznikol.")
                        .font(.system(size: 11))
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            } else if child == "continue" {
                Text("Existujúci Current Chat bude vyčistený. Skrytý archív sa nevytvorí. Automation Session zostane nezmenená. Nový chat dostane obmedzenú viditeľnú provenienciu a seed. Neskoršie zápisy vyžadujú Fresh Approval.")
                    .font(.system(size: 10))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                if viewModel.automationContinuationConfirmation != nil {
                    Button("Potvrdiť vyčistenie a pokračovať") {
                        viewModel.confirmAutomationContinuation()
                    }
                    .buttonStyle(.plain)
                    Button("Zrušiť") {
                        viewModel.cancelAutomationContinuation()
                    }
                    .buttonStyle(.plain)
                } else {
                    Button("Pokračovať") {
                        viewModel.continueAutomationSessionFromDetail()
                    }
                    .buttonStyle(.plain)
                }
            } else if child == "delete" {
                Text("Odstránia sa retained product data. Current Chat vytvorený z tejto relácie zostane oddelený.")
                    .font(.system(size: 10))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                Button("Vymazať Automation Session") {
                    viewModel.deleteAutomationSessionFromDetail()
                }
                .buttonStyle(.plain)
            } else if child == "sessionExport" || child == "diagnosticExport" {
                let disclosures = viewModel.automationSessionDetail?.truncationDisclosures ?? []
                Text(child == "sessionExport"
                     ? "Session Export obsahuje iba immutable Task Snapshot, výsledok, bezpečnú časovú os, zdroje, dostupnosť referencií, redigované schválenia a všetky applicable disclosures."
                     : "Diagnostic Export obsahuje iba privacy-safe diagnostiku; skryté uvažovanie, prompty, argumenty a výsledky nástrojov sa neexportujú.")
                    .font(.system(size: 10))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                if !disclosures.isEmpty {
                    Text("Truncation Disclosures: \(disclosures.count)")
                        .font(.system(size: 9))
                }
            } else if child == "commands" {
                Text("Príkazy sú dostupné iba pre platnú akciu. Každý neskorší zápis vyžaduje Fresh Approval.")
                    .font(.system(size: 10))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            } else {
                Text(child == "activity"
                     ? "Bezpečná časová os aktivít; transcript sa v tomto paneli nezobrazuje."
                     : "Táto akcia vyžaduje explicitné otvorenie Automation Session.")
                    .font(.system(size: 10))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            }
            Spacer(minLength: 0)
        }
    }

    private func childTitle(_ child: String) -> String {
        switch child {
        case "activity": return "Priebeh"
        case "output": return "Finálny výstup"
        case "sessionExport": return "Session Export"
        case "diagnosticExport": return "Diagnostic Export"
        case "commands": return "Príkazy"
        default: return "Automation Session"
        }
    }

}
