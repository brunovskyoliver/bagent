import SwiftUI

/// The `/automations` surface rendered inside the notch bridge — a compact
/// list of upcoming automations, a detail page, and inline delete
/// confirmation. No extra windows, popovers, or sheets.
struct AutomationsNotchContent: View {
    @ObservedObject var viewModel: ChatViewModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var contentTransition: AnyTransition {
        reduceMotion ? .opacity : .opacity.combined(with: .move(edge: .trailing))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            header
            switch viewModel.automationsSurface {
            case .list:
                listView.transition(.opacity)
            case .detail:
                if let a = viewModel.selectedAutomation {
                    detailView(a).transition(contentTransition)
                }
            case .deleteConfirmation:
                if let a = viewModel.selectedAutomation {
                    deleteConfirmView(a).transition(contentTransition)
                }
            }
            if let error = viewModel.automationsError {
                Text(error)
                    .font(.system(size: 10))
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                    .lineLimit(1)
                    .accessibilityLabel("Chyba: \(error)")
            }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: viewModel.automationsSurface)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var header: some View {
        HStack(spacing: 6) {
            Image(systemName: "clock.arrow.2.circlepath")
                .font(.system(size: 11, weight: .semibold))
            Text(headerTitle)
                .font(.system(size: 13, weight: .semibold))
            Spacer()
            if case .list = viewModel.automationsSurface {
                Button {
                    viewModel.startAutomationCreation()
                } label: {
                    Image(systemName: "plus")
                        .font(.system(size: 11, weight: .semibold))
                        .frame(width: 18, height: 18)
                        .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 5))
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Nová automatizácia")
            }
        }
        .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
    }

    private var headerTitle: String {
        switch viewModel.automationsSurface {
        case .list: return "Automatizácie"
        case .detail: return viewModel.selectedAutomation?.name ?? "Automatizácia"
        case .deleteConfirmation: return "Vymazať?"
        }
    }

    // MARK: List

    private var listView: some View {
        VStack(alignment: .leading, spacing: 3) {
            if viewModel.automations.isEmpty {
                Text("Žiadne automatizácie — pridaj cez +")
                    .font(.system(size: 11))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                    .padding(.top, 6)
            } else {
                // A one-second surface: the next few enabled automations first.
                ForEach(Array(viewModel.automations.prefix(4).enumerated()), id: \.element.id) { index, a in
                    listRow(a, selected: index == viewModel.automationsSelectionIndex)
                        .onTapGesture {
                            viewModel.automationsSelectionIndex = index
                            _ = viewModel.openSelectedAutomationDetail()
                        }
                }
            }
        }
    }

    private func listRow(_ a: AutomationRecord, selected: Bool) -> some View {
        HStack(spacing: 7) {
            Circle()
                .fill(a.enabled ? Color.white.opacity(0.75) : Color.white.opacity(0.22))
                .frame(width: 5, height: 5)
                .accessibilityHidden(true)
            Text(a.name)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                .lineLimit(1)
            Spacer(minLength: 4)
            if let glyph = AutomationTimeFormat.statusGlyph(a.lastRunStatus) {
                Image(systemName: glyph)
                    .font(.system(size: 9))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                    .accessibilityHidden(true)
            }
            Text(a.nextRunLabel ?? a.scheduleLabel)
                .font(.system(size: 11))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                .lineLimit(1)
        }
        .padding(.horizontal, 8)
        .frame(height: 24)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(Color.white.opacity(selected ? 0.12 : 0.06))
        )
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            "Automatizácia \(a.name), \(a.enabled ? "zapnutá" : "vypnutá"), ďalší beh \(a.nextRunLabel ?? "žiadny")")
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }

    // MARK: Detail

    private func detailView(_ a: AutomationRecord) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Label(a.scheduleLabel, systemImage: "calendar")
                    .font(.system(size: 11))
                Spacer()
                Text(a.timezone)
                    .font(.system(size: 9))
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                    .accessibilityLabel("Časové pásmo \(a.timezone)")
            }
            .foregroundStyle(NotchWrapMetrics.notchTextSecondary)

            if let next = a.nextRunLabel {
                Label("ďalší beh \(next)", systemImage: "arrow.forward.circle")
                    .font(.system(size: 11))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            }

            if let status = a.lastRunStatus {
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 5) {
                        if let glyph = AutomationTimeFormat.statusGlyph(status) {
                            Image(systemName: glyph).font(.system(size: 10))
                        }
                        Text(lastRunLabel(status: status, at: a.lastRunAt))
                            .font(.system(size: 11, weight: .medium))
                    }
                    .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                    if let summary = a.lastResultSummary, !summary.isEmpty {
                        Text(summary)
                            .font(.system(size: 11))
                            .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                            .lineLimit(3)
                            .accessibilityLabel("Posledný výsledok: \(summary)")
                    }
                }
                .padding(6)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))
            }

            Spacer(minLength: 0)

            HStack(spacing: 6) {
                detailButton(
                    a.enabled ? "Vypnúť" : "Zapnúť",
                    symbol: a.enabled ? "pause" : "play"
                ) {
                    viewModel.setAutomationEnabled(a, enabled: !a.enabled)
                }
                detailButton("Spustiť", symbol: "bolt") {
                    viewModel.runAutomationNow(a)
                }
                detailButton("Upraviť", symbol: "pencil") {
                    viewModel.startAutomationEdit(a)
                }
                detailButton("Vymazať", symbol: "trash") {
                    viewModel.automationsSurface = .deleteConfirmation(a.id)
                }
            }
            .disabled(viewModel.automationsBusy)
        }
    }

    private func lastRunLabel(status: String, at: String?) -> String {
        let when = AutomationTimeFormat.shortLocal(at).map { " · \($0)" } ?? ""
        let name: String
        switch status {
        case "completed": name = "dokončené"
        case "partial": name = "čiastočné"
        case "failed": name = "zlyhalo"
        case "abandoned": name = "prerušené"
        case "running": name = "beží"
        case "skipped_overlap": name = "preskočené (beh aktívny)"
        case "skipped_stale": name = "preskočené (zmeškané)"
        default: name = status
        }
        return name + when
    }

    private func detailButton(_ title: String, symbol: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 2) {
                Image(systemName: symbol).font(.system(size: 10, weight: .medium))
                Text(title).font(.system(size: 9))
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 5)
            .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
        }
        .buttonStyle(.plain)
        .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
        .accessibilityLabel(title)
    }

    // MARK: Delete confirmation

    private func deleteConfirmView(_ a: AutomationRecord) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Naozaj vymazať „\(a.name)“?")
                .font(.system(size: 12))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
            HStack(spacing: 8) {
                Button("Zrušiť") {
                    viewModel.automationsSurface = .detail(a.id)
                }
                .buttonStyle(.plain)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 5)
                .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
                .accessibilityLabel("Zrušiť vymazanie")

                Button("Vymazať") {
                    viewModel.deleteAutomation(a)
                }
                .buttonStyle(.plain)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.black)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 5)
                .background(Color.white.opacity(0.9), in: RoundedRectangle(cornerRadius: 6))
                .keyboardShortcut(.return, modifiers: [])
                .accessibilityLabel("Potvrdiť vymazanie")
            }
        }
    }
}
