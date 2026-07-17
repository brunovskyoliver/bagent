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
            case .editorTask:
                editorTaskView.transition(contentTransition)
            case .editorSchedule:
                editorScheduleView.transition(contentTransition)
            case .editorRecurrence:
                editorRecurrenceView.transition(contentTransition)
            case .editorReview:
                editorReviewView.transition(contentTransition)
            case .editorSaving:
                editorSavingView.transition(.opacity)
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
        case .editorTask: return viewModel.automationDraft.editingID == nil ? "Nová · úloha" : "Úprava · úloha"
        case .editorSchedule: return "Kedy"
        case .editorRecurrence: return "Opakovanie"
        case .editorReview: return "Zhrnutie"
        case .editorSaving: return "Ukladám…"
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

    // MARK: Editor

    private var editorTaskView: some View {
        VStack(alignment: .leading, spacing: 7) {
            editorField("Názov", text: Binding(
                get: { viewModel.automationDraft.name },
                set: { viewModel.automationDraft.name = $0 }
            ), placeholder: "napr. Ranná pošta", label: "Názov automatizácie")
            editorField("Úloha", text: Binding(
                get: { viewModel.automationDraft.prompt },
                set: { viewModel.automationDraft.prompt = $0 }
            ), placeholder: "napr. nájdi neprečítané maily a zhrň urgentné", label: "Úloha automatizácie")
            Spacer(minLength: 0)
            editorNav(nextEnabled: viewModel.automationDraftTaskValid)
        }
    }

    private func editorField(
        _ title: String, text: Binding<String>, placeholder: String, label: String
    ) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title)
                .font(.system(size: 9, weight: .medium))
                .foregroundStyle(NotchWrapMetrics.notchTextFaint)
            TextField(placeholder, text: text)
                .textFieldStyle(.plain)
                .font(.system(size: 12))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                .padding(.horizontal, 7)
                .padding(.vertical, 5)
                .background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))
                .accessibilityLabel(label)
        }
    }

    private var editorScheduleView: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 5) {
                dayChip("Dnes", choice: .today)
                dayChip("Zajtra", choice: .tomorrow)
                dayChip("Dátum", choice: customChoice)
            }
            if case .custom = viewModel.automationDraft.day {
                stepperRow(
                    label: customDateLabel,
                    accessibility: "Vybraný dátum \(customDateLabel)",
                    minus: { shiftCustomDay(-1) },
                    plus: { shiftCustomDay(1) }
                )
            }
            stepperRow(
                label: String(format: "%02d:%02d", viewModel.automationDraft.hour, viewModel.automationDraft.minute),
                accessibility: "Čas spustenia",
                minus: { shiftTime(-15) },
                plus: { shiftTime(15) }
            )
            Text(TimeZone.current.identifier)
                .font(.system(size: 9))
                .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                .accessibilityLabel("Časové pásmo \(TimeZone.current.identifier)")
            Spacer(minLength: 0)
            editorNav(nextEnabled: true)
        }
    }

    private var customChoice: AutomationDayChoice {
        if case .custom(let d) = viewModel.automationDraft.day { return .custom(d) }
        let dayAfter = Calendar.current.date(byAdding: .day, value: 2, to: Date()) ?? Date()
        return .custom(dayAfter)
    }

    private var customDateLabel: String {
        guard case .custom(let d) = viewModel.automationDraft.day else { return "—" }
        let cal = Calendar.current
        return "\(cal.component(.day, from: d)).\(cal.component(.month, from: d))."
    }

    private func dayChip(_ title: String, choice: AutomationDayChoice) -> some View {
        let selected: Bool
        switch (viewModel.automationDraft.day, choice) {
        case (.today, .today), (.tomorrow, .tomorrow), (.custom, .custom): selected = true
        default: selected = false
        }
        return Button(title) {
            viewModel.automationDraft.day = choice
        }
        .buttonStyle(.plain)
        .font(.system(size: 11, weight: .medium))
        .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
        .padding(.horizontal, 9)
        .padding(.vertical, 4)
        .background(Color.white.opacity(selected ? 0.12 : 0.06), in: RoundedRectangle(cornerRadius: 6))
        .accessibilityLabel("Deň: \(title)")
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }

    private func stepperRow(
        label: String, accessibility: String,
        minus: @escaping () -> Void, plus: @escaping () -> Void
    ) -> some View {
        HStack(spacing: 8) {
            stepperButton("minus", label: "Menej") { minus() }
            Text(label)
                .font(.system(size: 13, weight: .medium, design: .monospaced))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                .frame(minWidth: 52)
                .accessibilityLabel(accessibility + " " + label)
            stepperButton("plus", label: "Viac") { plus() }
        }
    }

    private func stepperButton(_ symbol: String, label: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: 10, weight: .semibold))
                .frame(width: 20, height: 20)
                .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 5))
        }
        .buttonStyle(.plain)
        .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
        .accessibilityLabel(label)
    }

    private func shiftTime(_ minutes: Int) {
        var total = viewModel.automationDraft.hour * 60 + viewModel.automationDraft.minute + minutes
        total = ((total % 1440) + 1440) % 1440
        viewModel.automationDraft.hour = total / 60
        viewModel.automationDraft.minute = total % 60
    }

    private func shiftCustomDay(_ delta: Int) {
        guard case .custom(let d) = viewModel.automationDraft.day,
              let shifted = Calendar.current.date(byAdding: .day, value: delta, to: d),
              shifted > Calendar.current.startOfDay(for: Date())
        else { return }
        viewModel.automationDraft.day = .custom(shifted)
    }

    private var editorRecurrenceView: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 5) {
                recurrenceChip("Raz", matches: { $0.isOnce }) { .once }
                recurrenceChip("Každé N h", matches: {
                    if case .everyNHours = $0 { return true } else { return false }
                }) { .everyNHours(2) }
                recurrenceChip("Denne", matches: { $0 == .daily }) { .daily }
            }
            HStack(spacing: 5) {
                recurrenceChip("Po–Pia", matches: { $0 == .weekdays }) { .weekdays }
                recurrenceChip("Vybrané dni", matches: {
                    if case .selectedWeekdays = $0 { return true } else { return false }
                }) { .selectedWeekdays(["mon"]) }
                recurrenceChip("Týždenne", matches: {
                    if case .weekly = $0 { return true } else { return false }
                }) { .weekly("mon") }
            }
            recurrenceDetailControls
            Spacer(minLength: 0)
            editorNav(nextEnabled: true)
        }
    }

    @ViewBuilder
    private var recurrenceDetailControls: some View {
        switch viewModel.automationDraft.recurrence {
        case .everyNHours(let n):
            stepperRow(
                label: "\(n) h",
                accessibility: "Interval v hodinách",
                minus: { viewModel.automationDraft.recurrence = .everyNHours(max(1, n - 1)) },
                plus: { viewModel.automationDraft.recurrence = .everyNHours(min(168, n + 1)) }
            )
        case .selectedWeekdays(let days):
            weekdayPicker(selected: days) { day in
                var next = days
                if next.contains(day) { next.remove(day) } else { next.insert(day) }
                viewModel.automationDraft.recurrence = .selectedWeekdays(next)
            }
        case .weekly(let day):
            weekdayPicker(selected: [day]) { picked in
                viewModel.automationDraft.recurrence = .weekly(picked)
            }
        case .once, .daily, .weekdays:
            EmptyView()
        }
    }

    private func weekdayPicker(selected: Set<String>, toggle: @escaping (String) -> Void) -> some View {
        HStack(spacing: 3) {
            ForEach(["mon", "tue", "wed", "thu", "fri", "sat", "sun"], id: \.self) { day in
                let isOn = selected.contains(day)
                Button(RecurrenceRuleWire.shortDay(day)) { toggle(day) }
                    .buttonStyle(.plain)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(
                        isOn ? Color.black : NotchWrapMetrics.notchTextSecondary)
                    .frame(width: 24, height: 20)
                    .background(
                        Color.white.opacity(isOn ? 0.85 : 0.06),
                        in: RoundedRectangle(cornerRadius: 5))
                    .accessibilityLabel("Deň \(RecurrenceRuleWire.shortDay(day))")
                    .accessibilityAddTraits(isOn ? [.isButton, .isSelected] : .isButton)
            }
        }
    }

    private func recurrenceChip(
        _ title: String,
        matches: (AutomationDraftRecurrence) -> Bool,
        make: @escaping () -> AutomationDraftRecurrence
    ) -> some View {
        let selected = matches(viewModel.automationDraft.recurrence)
        return Button(title) {
            viewModel.automationDraft.recurrence = make()
        }
        .buttonStyle(.plain)
        .font(.system(size: 10, weight: .medium))
        .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
        .padding(.horizontal, 7)
        .padding(.vertical, 4)
        .background(Color.white.opacity(selected ? 0.12 : 0.06), in: RoundedRectangle(cornerRadius: 6))
        .accessibilityLabel("Opakovanie: \(title)")
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
    }

    private var editorReviewView: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(viewModel.automationDraftSummary)
                .font(.system(size: 12))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                .lineLimit(3)
                .padding(7)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))
                .accessibilityLabel("Zhrnutie: \(viewModel.automationDraftSummary)")
            Toggle(isOn: Binding(
                get: { viewModel.automationDraft.enabled },
                set: { viewModel.automationDraft.enabled = $0 }
            )) {
                Text("Zapnutá")
                    .font(.system(size: 11))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            }
            .toggleStyle(.switch)
            .controlSize(.mini)
            .accessibilityLabel("Automatizácia zapnutá")
            Spacer(minLength: 0)
            editorNav(
                nextEnabled: true,
                nextTitle: viewModel.automationDraft.editingID == nil ? "Vytvoriť" : "Uložiť"
            )
        }
    }

    private var editorSavingView: some View {
        HStack(spacing: 8) {
            ProgressView().controlSize(.small)
            Text("Ukladám…")
                .font(.system(size: 12))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityLabel("Ukladám automatizáciu")
    }

    private func editorNav(nextEnabled: Bool, nextTitle: String = "Ďalej") -> some View {
        HStack(spacing: 8) {
            Button("Späť") { _ = viewModel.automationsGoBack() }
                .buttonStyle(.plain)
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 5)
                .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
                .accessibilityLabel("Späť")
            Button(nextTitle) { viewModel.automationEditorNext() }
                .buttonStyle(.plain)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(nextEnabled ? .black : NotchWrapMetrics.notchTextFaint)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 5)
                .background(
                    Color.white.opacity(nextEnabled ? 0.9 : 0.08),
                    in: RoundedRectangle(cornerRadius: 6))
                .disabled(!nextEnabled)
                .keyboardShortcut(.return, modifiers: [.command])
                .accessibilityLabel(nextTitle)
        }
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
