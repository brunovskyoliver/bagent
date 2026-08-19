import SwiftUI

struct ActivityPeekStageRailView: View {
    let presentation: NotchPresentation
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 7) {
                    ForEach(Array(StageRailStage.allCases.enumerated()), id: \.element) { index, stage in
                        stageLabel(stage)
                        if index < StageRailStage.allCases.count - 1 {
                            Capsule()
                                .fill(NotchWrapMetrics.notchTextFaint.opacity(0.35))
                                .frame(width: 14, height: 1)
                                .accessibilityHidden(true)
                        }
                    }
                }
                HStack(spacing: 8) {
                    StageRailActivityIcon(
                        stage: presentation.rail.selectedStage,
                        category: presentation.rail.activityCategory,
                        reduceMotion: presentation.motion.reduceMotion
                    )
                    .id(iconIdentity)
                    .transition(.opacity)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(presentation.rail.caption)
                            .font(.caption.weight(.medium))
                            .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                            .lineLimit(1)
                        if let secondary = presentation.rail.secondaryCaption {
                            Text(secondary)
                                .font(.caption2)
                                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                                .lineLimit(1)
                        }
                    }
                    .id(presentation.focusedWorkIdentity)
                    .transition(.opacity)
                    .animation(
                        .easeInOut(duration: presentation.motion.reduceMotion ? 0.12 : 0.16),
                        value: presentation.focusedWorkIdentity
                    )
                    Spacer(minLength: 0)
                }
                .animation(
                    .easeInOut(duration: presentation.motion.reduceMotion ? 0.12 : 0.16),
                    value: iconIdentity
                )
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!presentation.canOpenFocusedDestination)
        .keyboardShortcut(.return, modifiers: [])
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Activity")
        .accessibilityValue(presentation.rail.accessibilityValue)
    }

    private var iconIdentity: String {
        [
            presentation.focusedWorkIdentity ?? "idle",
            presentation.rail.selectedStage?.rawValue ?? "none",
            presentation.rail.activityCategory?.rawValue ?? "none",
        ].joined(separator: ":")
    }

    private func stageLabel(_ stage: StageRailStage) -> some View {
        HStack(spacing: 3) {
            Text(stage.rawValue)
            if stage == .done, presentation.rail.terminalAttentionMarker != nil {
                Circle()
                    .fill(NotchWrapMetrics.notchTextPrimary)
                    .frame(width: 4, height: 4)
                    .accessibilityHidden(true)
            }
        }
        .font(.caption2.weight(stage == presentation.rail.selectedStage ? .semibold : .regular))
        .foregroundStyle(
            stage == presentation.rail.selectedStage
                ? NotchWrapMetrics.notchTextPrimary
                : NotchWrapMetrics.notchTextSecondary
        )
        .opacity(stage == presentation.rail.selectedStage ? 1 : 0.86)
        .animation(.easeInOut(duration: 0.16), value: presentation.rail.selectedStage)
    }
}

struct InvariantNotchStatusPill: View {
    let presentation: NotchStatusPillPresentation
    let activeAutomationCount: Int
    let action: () -> Void

    var body: some View {
        Group {
            if presentation.opensAutomations(activeAutomationCount: activeAutomationCount) {
                Button(action: action) { capsule }
                    .buttonStyle(.plain)
            } else {
                capsule
            }
        }
        .frame(width: NotchPillLayout.size.width, height: NotchPillLayout.size.height)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(presentation.accessibilityLabel)
        .accessibilityValue(presentation.accessibilityValue)
    }

    private var capsule: some View {
        HStack(spacing: 5) {
            Circle()
                .fill(dotColor)
                .frame(width: 5, height: 5)
                .accessibilityHidden(true)
            Text(presentation.label ?? "")
                .id(presentation.label)
                .font(.system(size: 9, weight: .semibold, design: .rounded))
                .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                .lineLimit(1)
                .transition(.opacity)
                .animation(.easeInOut(duration: 0.16), value: presentation.label)
        }
        .frame(width: NotchPillLayout.size.width, height: NotchPillLayout.size.height)
        .background(Color.white.opacity(0.06), in: Capsule())
        .contentShape(Capsule())
    }

    private var dotColor: Color {
        switch presentation.label {
        case "APPROVE": .yellow
        case "FAILED": .red
        case "PARTIAL": .orange
        case "UNREAD": .blue
        case "LOADING": .cyan
        default: .green
        }
    }
}

private struct StageRailActivityIcon: View {
    let stage: StageRailStage?
    let category: NotchActivityCategory?
    let reduceMotion: Bool
    @State private var phase = false

    var body: some View {
        Image(systemName: symbolName)
            .font(.system(size: 13, weight: .semibold))
            .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
            .frame(width: 18, height: 18)
            .rotationEffect(rotation, anchor: category == .mail ? .top : .center)
            .offset(y: verticalOffset)
            .scaleEffect(scale)
            .opacity(opacity)
            .animation(animation, value: phase)
            .accessibilityLabel(accessibilityLabel)
            .onAppear { phase = true }
            .onDisappear { phase = false }
            .onChange(of: category) {
                phase = false
                phase = !reduceMotion
            }
            .onChange(of: reduceMotion) { _, reduced in
                if reduced {
                    phase = true
                } else {
                    phase = false
                    DispatchQueue.main.async { phase = true }
                }
            }
    }

    private var symbolName: String {
        if stage == .model { return "cpu" }
        if stage == .done { return "checkmark.circle.fill" }
        return category?.symbolName ?? (stage == .think ? "bubble.left.fill" : "wrench.and.screwdriver.fill")
    }

    private var accessibilityLabel: String {
        if stage == .model { return "Model" }
        if stage == .done { return "Done" }
        return category?.label ?? (stage == .think ? "Thinking" : "Tool")
    }

    private var rotation: Angle {
        guard !reduceMotion else { return .zero }
        switch category {
        case .mail: return .degrees(phase ? 7 : -7)
        case .genericTool: return .degrees(phase ? 12 : -12)
        case .web, .automation: return .degrees(phase ? 360 : 0)
        default: return stage == .model ? .degrees(phase ? 360 : 0) : .zero
        }
    }

    private var verticalOffset: CGFloat {
        guard !reduceMotion else { return 0 }
        return category == .filesystem || category == .codex ? (phase ? 2 : -2) : 0
    }

    private var scale: CGFloat {
        guard !reduceMotion else { return 1 }
        if stage == .done { return phase ? 1 : 0.9 }
        return category == .odoo || category == .chat ? (phase ? 1.08 : 0.90) : 1
    }

    private var opacity: CGFloat {
        stage == .done && !phase ? 0 : 1
    }

    private var animation: Animation? {
        guard !reduceMotion else { return .easeOut(duration: 0.12) }
        if stage == .done { return .easeOut(duration: 0.24) }
        switch category {
        case .mail, .genericTool:
            return .easeInOut(duration: 0.38).repeatForever(autoreverses: true)
        case .filesystem, .codex, .odoo, .chat:
            return .easeInOut(duration: 0.72).repeatForever(autoreverses: true)
        case .web, .automation:
            return .linear(duration: 1.8).repeatForever(autoreverses: false)
        default:
            return stage == .model
                ? .linear(duration: 1.8).repeatForever(autoreverses: false)
                : nil
        }
    }
}
