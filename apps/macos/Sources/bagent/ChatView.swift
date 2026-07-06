import SwiftUI

// MARK: - Notch wrap geometry constants

enum NotchWrapMetrics {
    static let idleWingWidth: CGFloat     = 32
    static let hoverWingWidth: CGFloat    = 72   // wider hover target, still proportional
    static let idleBridgeHeight: CGFloat  = 0
    static let hoverBridgeHeight: CGFloat = 22   // wraps under the notch
    static let cornerRadius: CGFloat      = 10   // bottom corners only
    static let innerCornerRadius: CGFloat = 8    // notch border
    static let syntheticNotchWidth: CGFloat = 221 // measured Mac17,2 notch width for external/non-notch displays
    static let expandedBridgeHeight: CGFloat = 520  // matches chatH
    static let expandedWingWidth: CGFloat   = 200   // chatW / 2
    static let inlineWingWidth: CGFloat   = 221
    static let inlineBridgeHeight: CGFloat = 72
    static let outputWingWidth: CGFloat   = 154
    static let outputMinWingWidth: CGFloat = 72
    static let outputBridgeHeight: CGFloat = 96
    static let outputMinBridgeHeight: CGFloat = 88
    static let outputMaxBridgeHeight: CGFloat = 280
    static let inlineContentScale: CGFloat = 0.90
    static let voiceWingWidth: CGFloat    = 100  // voice mode — wide enough for sentence
    static let voiceBridgeHeight: CGFloat = 120  // voice mode — fits wave + 2 text lines
    static let cmuxWingWidth: CGFloat     = 84   // cmux banner — fits one-line event title
    static let cmuxBridgeHeight: CGFloat  = 19   // single caption line, minimal growth
    static let maxWingWidth: CGFloat      = 260
    static let maxBridgeHeight: CGFloat   = 280
    static let surfaceDuration: Double     = 0.58
    static let notchBorderColor           = Color(white: 0.30)

    // Soft off-white for notch text — grayish, never pure white.
    static let notchTextPrimary   = Color(white: 0.80)   // primary lines
    static let notchTextSecondary = Color(white: 0.55)   // subtitles / echoes
    static let notchTextFaint     = Color(white: 0.42)   // placeholder
    static let notchTextPrimaryNS = NSColor(white: 0.80, alpha: 1)  // AppKit output
}

private enum NotchOutputLayout {
    static let lineSpacing: CGFloat = 1.5
    static let bottomSlack: CGFloat = 12
    static let chromePadding: CGFloat = 26
    static let contentVerticalInset: CGFloat = 14
    static let minTextWidth: CGFloat = 120
    static let resizeThreshold: CGFloat = 14
    /// Output scroll viewport height once the notch bridge is fully grown.
    /// Mirrors the layout chain in NotchWrapView: bridge height minus contentVerticalInset.
    /// While the viewport is below this, `bridgeHeight(text:visibleWidth:message:)`
    /// guarantees the text fits, so the panel grows instead of scrolling.
    static var maxViewportHeight: CGFloat {
        NotchWrapMetrics.outputMaxBridgeHeight - contentVerticalInset
    }
    static func font() -> NSFont { NSFont.systemFont(ofSize: 13, weight: .regular) }

    static func responseText(
        latestText: String,
        isStreaming: Bool,
        isThinking: Bool
    ) -> String {
        latestText.isEmpty
            ? (isStreaming || isThinking ? "Thinking" : "Done")
            : latestText
    }

    static func reservedWidth(for message: ChatMessage?) -> CGFloat {
        message?.debugTraceId == nil ? CGFloat(0) : CGFloat(34)
    }

    /// Width of the widest rendered line when nothing wraps — drives the
    /// content-fit panel width the same way `textHeight` drives the height.
    static func longestLineWidth(_ text: String) -> CGFloat {
        let attributed = NotchMarkdown.attributedString(text.isEmpty ? " " : text)
        let rect = attributed.boundingRect(
            with: NSSize(width: CGFloat.greatestFiniteMagnitude,
                         height: CGFloat.greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading]
        )
        return ceil(rect.width)
    }

    /// Wing width that fits the longest line, clamped to
    /// [outputMinWingWidth, outputWingWidth]. Inverts the visible-width formula
    /// used by the output surface. The +16 slack absorbs measurement rounding
    /// so text measured to exactly fit doesn't wrap at the rendered width.
    static func wingWidth(text: String, notchWidth: CGFloat, message: ChatMessage?) -> CGFloat {
        let contentWidth = longestLineWidth(text) + reservedWidth(for: message) + 16
        let visibleWidth = contentWidth / NotchWrapMetrics.inlineContentScale
        let wing = (visibleWidth - notchWidth) / 2
        return min(NotchWrapMetrics.outputWingWidth,
                   max(NotchWrapMetrics.outputMinWingWidth, ceil(wing)))
    }

    static func textHeight(_ text: String, width: CGFloat) -> CGFloat {
        let attributed = NotchMarkdown.attributedString(text.isEmpty ? " " : text)
        let rect = attributed.boundingRect(
            with: NSSize(width: width, height: CGFloat.greatestFiniteMagnitude),
            options: [.usesLineFragmentOrigin, .usesFontLeading]
        )
        return ceil(rect.height)
    }

    static func bridgeHeight(
        text: String,
        visibleWidth: CGFloat,
        message: ChatMessage?
    ) -> CGFloat {
        let textWidth = max(minTextWidth, visibleWidth - reservedWidth(for: message))
        let wanted = ceil(textHeight(text, width: textWidth) + chromePadding + bottomSlack)
        return min(
            NotchWrapMetrics.outputMaxBridgeHeight,
            max(NotchWrapMetrics.outputMinBridgeHeight, wanted)
        )
    }
}

private struct NotchWrapBorderShape: Shape {
    var wingWidth: CGFloat
    var bridgeHeight: CGFloat
    let notchOffset: CGFloat
    let notchWidth: CGFloat
    let notchHeight: CGFloat
    let cornerRadius: CGFloat

    var animatableData: AnimatablePair<CGFloat, CGFloat> {
        get { AnimatablePair(wingWidth, bridgeHeight) }
        set {
            wingWidth = newValue.first
            bridgeHeight = newValue.second
        }
    }

    func path(in rect: CGRect) -> Path {
        let r = cornerRadius
        let br = max(0, bridgeHeight)
        let x = notchOffset - wingWidth
        let w = 2 * wingWidth + notchWidth
        let h = notchHeight + br

        var path = Path()
        path.move(to: CGPoint(x: x + w, y: 0))
        path.addLine(to: CGPoint(x: x + w, y: h - r))
        path.addArc(
            center: CGPoint(x: x + w - r, y: h - r),
            radius: r,
            startAngle: .degrees(0),
            endAngle: .degrees(90),
            clockwise: false
        )
        path.addLine(to: CGPoint(x: x + r, y: h))
        path.addArc(
            center: CGPoint(x: x + r, y: h - r),
            radius: r,
            startAngle: .degrees(90),
            endAngle: .degrees(180),
            clockwise: false
        )
        path.addLine(to: CGPoint(x: x, y: 0))
        return path
    }
}

// MARK: - Status panel content (always visible, never moves)

struct StatusPillView: View {
    let notchWidth: CGFloat
    let notchHeight: CGFloat
    @ObservedObject var viewModel: ChatViewModel
    let onTap: () -> Void
    let onHoverChanged: (Bool) -> Void

    var body: some View {
        NotchWrapView(
            notchWidth: notchWidth,
            notchHeight: notchHeight,
            viewModel: viewModel,
            onTap: onTap,
            onHoverChanged: onHoverChanged
        )
    }
}

// MARK: - Notch wrap view (built-in display, notch present)

struct NotchWrapView: View {
    let notchWidth: CGFloat
    let notchHeight: CGFloat
    @ObservedObject var viewModel: ChatViewModel
    let onTap: () -> Void
    let onHoverChanged: (Bool) -> Void

    // Explicit @State so withAnimation directly tweens the shape's animatableData.
    @State private var wingWidth: CGFloat    = NotchWrapMetrics.idleWingWidth
    @State private var bridgeHeight: CGFloat = NotchWrapMetrics.idleBridgeHeight
    @State private var targetWingWidth: CGFloat = NotchWrapMetrics.idleWingWidth
    @State private var targetBridgeHeight: CGFloat = NotchWrapMetrics.idleBridgeHeight
    @State private var isHovered = false
    @State private var pulsing = false
    @State private var copyFlashed = false
    @State private var isDragTargeted = false
    @State private var isVoiceActive = false
    @State private var voiceContentOpacity: CGFloat = 0
    @State private var inlineContentOpacity: CGFloat = 0
    @State private var cmuxContentOpacity: CGFloat = 0
    /// When the pointer entered a cmux hover-reveal — gates the ≥0.4s dwell before
    /// a hover-leave counts as "seen".
    @State private var cmuxRevealStartedAt: Date?
    @State private var hoverIconRevealID = UUID()
    @State private var inlineRevealID = UUID()
    @State private var borderPulseOpacity: CGFloat = 0.35
    @State private var previousNotchInteractionMode: NotchInteractionMode = .collapsed
    @State private var returningStatusDotFromOutput = false
    /// Content-fit output wing — grow-only per assistant message so the panel
    /// never pumps narrower mid-stream; reset when a new message starts.
    @State private var outputWingRatchet: CGFloat = NotchWrapMetrics.outputMinWingWidth
    @State private var outputStatusReturnStartPos: CGPoint = .zero
    @State private var outputStatusReturnStartBridgeHeight: CGFloat = NotchWrapMetrics.outputMinBridgeHeight
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    // Panel is sized for voice mode (the widest/tallest state) so the frame never needs
    // AppKit resizing. notchOffset = voiceWingWidth = left edge of the physical notch.
    private let notchOffset = NotchWrapMetrics.maxWingWidth

    private var surfaceAnimation: Animation {
        reduceMotion
            ? .easeInOut(duration: 0.18)
            : .easeInOut(duration: NotchWrapMetrics.surfaceDuration)
    }
    private var status: AgentStatus { viewModel.agentStatus }
    private var isInlineActive: Bool {
        viewModel.notchInteractionMode == .input || viewModel.notchInteractionMode == .output
    }

    /// Transient cmux banner (5s after an agent event, latest wins).
    private var isCmuxBannerActive: Bool {
        viewModel.cmuxBanner != nil && !isInlineActive && !isVoiceActive
    }
    /// Hovering the collapsed notch re-reveals pending cmux events.
    private var isCmuxHoverReveal: Bool {
        viewModel.cmuxBanner == nil && !viewModel.cmuxPending.isEmpty
            && isHovered && !isInlineActive && !isVoiceActive
    }
    private var isCmuxSurfaceActive: Bool { isCmuxBannerActive || isCmuxHoverReveal }
    private var displayedCmuxNotification: CmuxNotification? {
        viewModel.cmuxBanner ?? viewModel.cmuxPending.first
    }

    /// SF Symbol name for the right-wing voice indicator.
    private var voiceIconName: String {
        switch viewModel.speech.state {
        case .loadingModel:        return "waveform.badge.magnifyingglass"
        case .listening:           return "waveform"
        case .finalizing:          return "waveform.badge.clock"
        case .error:               return "exclamationmark.triangle"
        default:                   return "waveform"
        }
    }
    private var maxSize: CGSize {
        CGSize(
            width: 2 * NotchWrapMetrics.maxWingWidth + notchWidth,
            height: notchHeight + NotchWrapMetrics.maxBridgeHeight
        )
    }

    private func inlineWingWidth(for mode: NotchInteractionMode) -> CGFloat {
        mode == .output ? outputWingWidth() : NotchWrapMetrics.inlineWingWidth
    }

    private func inlineBridgeHeight(for mode: NotchInteractionMode) -> CGFloat {
        mode == .output ? outputBridgeHeight() : NotchWrapMetrics.inlineBridgeHeight
    }

    private var outputResponseText: String {
        NotchOutputLayout.responseText(
            latestText: viewModel.latestAssistantText,
            isStreaming: viewModel.isLatestAssistantStreaming,
            isThinking: viewModel.isThinking
        )
    }

    private func outputWingWidth() -> CGFloat {
        let wanted = NotchOutputLayout.wingWidth(
            text: outputResponseText,
            notchWidth: notchWidth,
            message: viewModel.latestAssistantMessage
        )
        if wanted > outputWingRatchet { outputWingRatchet = wanted }
        return outputWingRatchet
    }

    private func outputBridgeHeight() -> CGFloat {
        // Height must be measured at the same width the text will render at,
        // so wrap width and measured height always agree.
        let visibleWidth = (notchWidth + 2 * outputWingWidth()) * NotchWrapMetrics.inlineContentScale
        return NotchOutputLayout.bridgeHeight(
            text: outputResponseText,
            visibleWidth: visibleWidth,
            message: viewModel.latestAssistantMessage
        )
    }

    private func estimatedNotchTextHeight(_ text: String, width: CGFloat) -> CGFloat {
        NotchOutputLayout.textHeight(text, width: width)
    }

    private func refreshOutputSurfaceIfNeeded(force: Bool = false) {
        guard viewModel.notchInteractionMode == .output, !isVoiceActive else { return }
        let nextWing = outputWingWidth()
        let nextBridge = outputBridgeHeight()
        let shouldResize = force
            || !viewModel.isLatestAssistantStreaming
            || abs(nextBridge - targetBridgeHeight) >= NotchOutputLayout.resizeThreshold
            || nextBridge == NotchWrapMetrics.outputMaxBridgeHeight
            || abs(nextWing - targetWingWidth) >= NotchOutputLayout.resizeThreshold
            || nextWing == NotchWrapMetrics.outputWingWidth
        guard shouldResize else { return }
        refreshSurface()
    }

    private func refreshSurface() {
        // Captured before updateStatusDotTravelState() mutates previousNotchInteractionMode.
        let previousModeWasOutput = previousNotchInteractionMode == .output
        updateStatusDotTravelState()

        let hoverExpanded = isHovered || isDragTargeted || viewModel.pillHovered
        var targetWing: CGFloat
        let targetBridge: CGFloat
        if isVoiceActive {
            targetWing = NotchWrapMetrics.voiceWingWidth
            targetBridge = NotchWrapMetrics.voiceBridgeHeight
        } else if isInlineActive {
            targetWing = inlineWingWidth(for: viewModel.notchInteractionMode)
            targetBridge = inlineBridgeHeight(for: viewModel.notchInteractionMode)
        } else if isCmuxSurfaceActive {
            targetWing = NotchWrapMetrics.cmuxWingWidth
            targetBridge = NotchWrapMetrics.cmuxBridgeHeight
        } else if hoverExpanded {
            targetWing = NotchWrapMetrics.hoverWingWidth
            targetBridge = NotchWrapMetrics.hoverBridgeHeight
        } else {
            targetWing = NotchWrapMetrics.idleWingWidth
            targetBridge = NotchWrapMetrics.idleBridgeHeight
        }
        // The pill widens (in any non-voice state) so the pending left-wing
        // icon row always fits inside the black shape.
        if !isVoiceActive {
            targetWing = max(targetWing, requiredLeftWingWidth)
        }

        let surfaceTargetChanged = targetWingWidth != targetWing || targetBridgeHeight != targetBridge
        // Width/height growth while already in output mode must not replay the
        // inline reveal — only the initial transition into output resets it.
        let outputContentOnlyResize = viewModel.notchInteractionMode == .output
            && previousModeWasOutput
        var instantTargetUpdate = Transaction()
        instantTargetUpdate.disablesAnimations = true
        withTransaction(instantTargetUpdate) {
            targetWingWidth = targetWing
            targetBridgeHeight = targetBridge
            if surfaceTargetChanged {
                hoverIconRevealID = UUID()
                if !outputContentOnlyResize {
                    inlineRevealID = UUID()
                }
            }
        }

        withAnimation(surfaceAnimation) {
            wingWidth = targetWing
            bridgeHeight = targetBridge
        }
        updateInlineOpacity(active: isInlineActive && !isVoiceActive)
        updateCmuxOpacity(active: isCmuxSurfaceActive)
    }

    private func updateStatusDotTravelState() {
        let currentMode = viewModel.notchInteractionMode
        if previousNotchInteractionMode == .output && currentMode != .output {
            outputStatusReturnStartPos = outputStatusTargetPos
            outputStatusReturnStartBridgeHeight = max(1, targetBridgeHeight)
            returningStatusDotFromOutput = !reduceMotion
        } else if currentMode == .output {
            returningStatusDotFromOutput = false
        } else if returningStatusDotFromOutput && bridgeHeight <= NotchWrapMetrics.idleBridgeHeight + 0.5 {
            returningStatusDotFromOutput = false
        }
        previousNotchInteractionMode = currentMode
    }

    private func setVoiceState(active: Bool) {
        refreshSurface()
        if active {
            if !reduceMotion {
                withAnimation(.easeInOut(duration: 1.4).repeatForever(autoreverses: true)) {
                    borderPulseOpacity = 0.75
                }
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) {
                withAnimation(.easeIn(duration: 0.20)) { voiceContentOpacity = 1 }
            }
        } else {
            withAnimation(.easeOut(duration: 0.15)) {
                voiceContentOpacity = 0
                borderPulseOpacity = 0.35
            }
        }
    }

    private func updateInlineOpacity(active: Bool) {
        if active {
            let delay = reduceMotion ? 0.0 : NotchWrapMetrics.surfaceDuration * 0.62
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                guard viewModel.notchInteractionMode != .collapsed, !isVoiceActive else { return }
                withAnimation(.easeOut(duration: reduceMotion ? 0.10 : 0.24)) {
                    inlineContentOpacity = 1
                }
            }
        } else {
            withAnimation(.easeOut(duration: 0.14)) {
                inlineContentOpacity = 0
            }
        }
    }

    private func updateCmuxOpacity(active: Bool) {
        if active {
            let delay = reduceMotion ? 0.0 : NotchWrapMetrics.surfaceDuration * 0.55
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                guard isCmuxSurfaceActive else { return }
                withAnimation(.easeOut(duration: reduceMotion ? 0.10 : 0.24)) {
                    cmuxContentOpacity = 1
                }
            }
        } else {
            withAnimation(.easeOut(duration: 0.14)) {
                cmuxContentOpacity = 0
            }
        }
    }

    // Icon y is clamped to the notch/hover area so it doesn't drift into bridge content.
    private var iconY: CGFloat {
        let clampedBridge = min(bridgeHeight, NotchWrapMetrics.hoverBridgeHeight)
        return (notchHeight + clampedBridge) / 2
    }
    private var targetLeftIconPos: CGPoint {
        let clampedBridge = min(targetBridgeHeight, NotchWrapMetrics.hoverBridgeHeight)
        let y = (notchHeight + clampedBridge) / 2
        return CGPoint(x: notchOffset - targetWingWidth / 2, y: y)
    }
    private var rightIconPos: CGPoint {
        CGPoint(x: notchOffset + notchWidth + wingWidth / 2, y: iconY)
    }
    private var collapsedStatusPos: CGPoint {
        CGPoint(
            x: notchOffset + notchWidth + NotchWrapMetrics.idleWingWidth / 2,
            y: notchHeight / 2
        )
    }
    private var outputStatusTargetPos: CGPoint {
        let visibleW = notchWidth + 2 * targetWingWidth
        let contentW = visibleW * NotchWrapMetrics.inlineContentScale
        return CGPoint(
            x: notchOffset + notchWidth / 2 + contentW / 2 - 10,
            y: notchHeight + targetBridgeHeight / 2
        )
    }
    private var outputStatusTravelProgress: CGFloat {
        guard viewModel.notchInteractionMode == .output else { return 0 }
        guard targetBridgeHeight > 0 else { return 1 }
        return min(1, max(0, bridgeHeight / targetBridgeHeight))
    }
    private var animatedInlineStatusPos: CGPoint {
        let visibleW = notchWidth + 2 * wingWidth
        let contentW = visibleW * NotchWrapMetrics.inlineContentScale
        return CGPoint(
            x: notchOffset + notchWidth / 2 + contentW / 2 - 10,
            y: notchHeight + bridgeHeight / 2
        )
    }
    private var statusDotPos: CGPoint {
        if viewModel.notchInteractionMode == .output {
            let progress = reduceMotion ? CGFloat(1) : outputStatusTravelProgress
            return CGPoint(
                x: collapsedStatusPos.x + (outputStatusTargetPos.x - collapsedStatusPos.x) * progress,
                y: collapsedStatusPos.y + (outputStatusTargetPos.y - collapsedStatusPos.y) * progress
            )
        }
        if returningStatusDotFromOutput || (previousNotchInteractionMode == .output && viewModel.notchInteractionMode != .output) {
            let startPos = returningStatusDotFromOutput ? outputStatusReturnStartPos : outputStatusTargetPos
            let startBridgeHeight = returningStatusDotFromOutput
                ? outputStatusReturnStartBridgeHeight
                : max(1, targetBridgeHeight)
            let progress = min(1, max(0, bridgeHeight / startBridgeHeight))
            return CGPoint(
                x: collapsedStatusPos.x + (startPos.x - collapsedStatusPos.x) * progress,
                y: collapsedStatusPos.y + (startPos.y - collapsedStatusPos.y) * progress
            )
        }
        if isInlineActive {
            return animatedInlineStatusPos
        }
        return rightIconPos
    }

    private var animatedNotchClipShape: NotchWrapShape {
        NotchWrapShape(
            wingWidth: wingWidth,
            bridgeHeight: bridgeHeight,
            notchOffset: notchOffset,
            notchWidth: notchWidth,
            notchHeight: notchHeight,
            cornerRadius: NotchWrapMetrics.cornerRadius
        )
    }

    @ViewBuilder
    private var inlineSurfaceLayer: some View {
        if isInlineActive && !isVoiceActive {
            let contentWing = wingWidth
            let contentBridge = bridgeHeight
            ZStack(alignment: .topLeading) {
                InlineNotchContent(viewModel: viewModel)
                    .frame(
                        width: (notchWidth + 2 * contentWing) * NotchWrapMetrics.inlineContentScale,
                        height: max(1, contentBridge - NotchOutputLayout.contentVerticalInset)
                    )
                    .position(x: notchOffset + notchWidth / 2,
                              y: notchHeight + contentBridge / 2)
                    .opacity(inlineContentOpacity)
                    .id(inlineRevealID)
                    .animation(surfaceAnimation, value: viewModel.notchInteractionMode)
            }
            .frame(width: maxSize.width, height: maxSize.height, alignment: .topLeading)
            .clipShape(animatedNotchClipShape)
        }
    }

    @ViewBuilder
    private var statusDotLayer: some View {
        if !isVoiceActive && (status != .ready || (viewModel.isExpanded && !isInlineActive)) {
            StatusDotView(status: status, pulsing: $pulsing, reduceMotion: reduceMotion, copyFlashed: copyFlashed, isDragTargeted: isDragTargeted)
                .position(statusDotPos)
                .frame(width: maxSize.width, height: maxSize.height, alignment: .topLeading)
                .clipShape(animatedNotchClipShape)
        }
    }

    /// Pending cmux events — animated dot on the right wing, own agent status wins.
    @ViewBuilder
    private var cmuxDotLayer: some View {
        if let kind = viewModel.cmuxDotKind,
           !isVoiceActive, !isInlineActive, !viewModel.isExpanded, status == .ready {
            CmuxStatusDotView(kind: kind, reduceMotion: reduceMotion)
                .position(rightIconPos)
                .frame(width: maxSize.width, height: maxSize.height, alignment: .topLeading)
                .clipShape(animatedNotchClipShape)
        }
    }

    /// Left-wing icon row: colorful connector action icons plus the cmux app
    /// icon. cmux keeps its collapsed-only visibility; connector icons stay
    /// visible while the inline input/output surface is open too — they
    /// persist (no timeout) until clicked or the next user message. The row is
    /// right-aligned against the notch's left edge and enters with the same
    /// bell-swing the cmux icon uses.
    @ViewBuilder
    private var leftWingIconLayer: some View {
        if leftWingIconCount > 0 {
            HStack(spacing: Self.leftWingIconSpacing) {
                ForEach(Array(visibleConnectorActions.enumerated()), id: \.element.id) { index, action in
                    Button {
                        viewModel.performConnectorAction(action, slotIndex: index)
                    } label: {
                        ConnectorIconView(kind: action.kind, reduceMotion: reduceMotion,
                                          size: Self.leftWingIconSize)
                    }
                    .buttonStyle(.plain)
                    .help(action.kind.accessibilityLabel)
                    .accessibilityLabel(action.kind.accessibilityLabel)
                }
                if showsCmuxLeftIcon, let notification = displayedCmuxNotification {
                    CmuxIconView(kind: notification.kind, reduceMotion: reduceMotion)
                        .opacity(isCmuxSurfaceActive ? cmuxContentOpacity : 1)
                }
            }
            .position(x: notchOffset - Self.leftWingRowInset - leftWingRowWidth / 2, y: iconY)
        }
    }

    private static let leftWingIconSize: CGFloat = 18
    private static let leftWingIconSpacing: CGFloat = 6
    /// Gap between the row's right edge and the notch's left edge.
    private static let leftWingRowInset: CGFloat = 8

    /// Connector icons hide only for voice and the expanded chat panel — unlike
    /// the cmux icon they stay through inline input/output.
    private var visibleConnectorActions: [ConnectorAction] {
        guard !isVoiceActive, !viewModel.isExpanded else { return [] }
        return viewModel.pendingConnectorActions
    }

    private var leftWingIconCount: Int {
        visibleConnectorActions.count + (showsCmuxLeftIcon ? 1 : 0)
    }

    private var leftWingRowWidth: CGFloat {
        let n = CGFloat(leftWingIconCount)
        guard n > 0 else { return 0 }
        return n * Self.leftWingIconSize + (n - 1) * Self.leftWingIconSpacing
    }

    /// Wing floor so the whole row fits inside the black pill.
    private var requiredLeftWingWidth: CGFloat {
        leftWingIconCount == 0 ? 0 : leftWingRowWidth + 2 * Self.leftWingRowInset
    }

    /// Where the connector icon sat at click time — the row still had the
    /// departing icon, so measure against a one-wider row.
    private func connectorSlotPosition(_ slotIndex: Int) -> CGPoint {
        let n = CGFloat(leftWingIconCount + 1)
        let rowWidth = n * Self.leftWingIconSize + (n - 1) * Self.leftWingIconSpacing
        let left = notchOffset - Self.leftWingRowInset - rowWidth
        let step = Self.leftWingIconSize + Self.leftWingIconSpacing
        return CGPoint(x: left + Self.leftWingIconSize / 2 + CGFloat(slotIndex) * step, y: iconY)
    }

    /// The cmux icon sits in the row's rightmost slot.
    private var cmuxSlotPosition: CGPoint {
        CGPoint(x: notchOffset - Self.leftWingRowInset - Self.leftWingIconSize / 2, y: iconY)
    }

    /// The left cmux icon shows while any cmux event is pending and the notch is in
    /// its collapsed presentation (not inline/voice/expanded).
    private var showsCmuxLeftIcon: Bool {
        guard !isInlineActive, !isVoiceActive, !viewModel.isExpanded else { return false }
        return displayedCmuxNotification != nil
    }

    /// Where an acknowledged cmux icon flies to: on a notched display, up into the
    /// physical notch gap; on an external display, off the top edge (same centered X
    /// since the synthetic notch is centered).
    private var cmuxFlyoffTarget: CGPoint {
        CGPoint(x: notchOffset + notchWidth / 2,
                y: viewModel.hasNotch ? notchHeight * 0.15 : -40)
    }

    /// Fly-off animations for cues the user just acknowledged (tab-open / hover-leave
    /// / click). Rendered unclipped so the icon is visible travelling into the notch.
    @ViewBuilder
    private var iconDepartureLayer: some View {
        ForEach(viewModel.cmuxDeparting) { departure in
            IconFlyoffView(
                start: cmuxSlotPosition,
                target: cmuxFlyoffTarget,
                hasNotch: viewModel.hasNotch,
                onFinished: { viewModel.finishCmuxDeparture(departure.id) }
            ) {
                Image(nsImage: CmuxEventMonitor.appIcon)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
            }
        }
        ForEach(viewModel.connectorDeparting) { departure in
            IconFlyoffView(
                start: connectorSlotPosition(departure.slotIndex),
                target: cmuxFlyoffTarget,
                hasNotch: viewModel.hasNotch,
                size: Self.leftWingIconSize,
                onFinished: { viewModel.finishConnectorDeparture(departure.id) }
            ) {
                ConnectorIconView(kind: departure.kind, reduceMotion: true,
                                  size: Self.leftWingIconSize)
            }
        }
    }

    /// Click-through on a cmux banner/reveal focuses the cmux workspace/tab.
    private func handleTap() {
        if isCmuxSurfaceActive, let notification = displayedCmuxNotification {
            viewModel.focusCmux(notification)
            refreshSurface()
        } else {
            onTap()
        }
    }

    /// One-line cmux event title shown in the bridge while the banner or
    /// hover-reveal surface is grown.
    @ViewBuilder
    private var cmuxBannerLayer: some View {
        if isCmuxSurfaceActive, let notification = displayedCmuxNotification {
            HStack(spacing: 5) {
                Text(notification.title)
                    .font(.system(size: 11, weight: .regular))
                    .kerning(0.2)
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                    .lineLimit(1)
                if isCmuxHoverReveal && viewModel.cmuxPending.count > 1 {
                    Text("+\(viewModel.cmuxPending.count - 1)")
                        .font(.system(size: 9, weight: .medium))
                        .foregroundStyle(NotchWrapMetrics.notchTextFaint)
                }
            }
            .frame(maxWidth: notchWidth + 2 * NotchWrapMetrics.cmuxWingWidth - 36)
            .position(x: notchOffset + notchWidth / 2,
                      y: notchHeight + max(1, bridgeHeight) / 2)
            .opacity(cmuxContentOpacity)
            .frame(width: maxSize.width, height: maxSize.height, alignment: .topLeading)
            .clipShape(animatedNotchClipShape)
        }
    }

    private var showsLeftStatusIcon: Bool {
        !isInlineActive
            && !isCmuxSurfaceActive
            && viewModel.notchInteractionMode != .thinking
            && (viewModel.isExpanded || isHovered || isVoiceActive)
    }

    private func isNearVisibleSurface(_ point: CGPoint) -> Bool {
        let margin: CGFloat = isInlineActive || isVoiceActive ? 3 : 6
        let x = notchOffset - wingWidth
        let w = 2 * wingWidth + notchWidth
        let h = notchHeight + max(0, bridgeHeight)
        return CGRect(
            x: x - margin,
            y: 0,
            width: w + margin * 2,
            height: h + margin
        ).contains(point)
    }

    private func setPointerHover(_ active: Bool) {
        guard isHovered != active else { return }
        let wasRevealingCmux = isCmuxHoverReveal
        isHovered = active
        if active {
            if isCmuxHoverReveal { cmuxRevealStartedAt = Date() }
        } else if wasRevealingCmux {
            // Cues are seen once the hover reveal ends — but only if the pointer
            // actually dwelt (≥0.4s), so a mouse merely passing through the notch
            // doesn't wipe everything.
            let dwelt = (cmuxRevealStartedAt.map { Date().timeIntervalSince($0) } ?? 0) >= 0.4
            cmuxRevealStartedAt = nil
            if dwelt { viewModel.markAllCmuxSeen() }
        }
        if !isVoiceActive { refreshSurface() }
        onHoverChanged(active || isDragTargeted)
    }

    // Split out of `body` so the modifier chain below stays type-checkable.
    private var surfaceLayers: some View {
        ZStack(alignment: .topLeading) {
            NotchWrapShape(
                wingWidth: wingWidth,
                bridgeHeight: bridgeHeight,
                notchOffset: notchOffset,
                notchWidth: notchWidth,
                notchHeight: notchHeight,
                cornerRadius: NotchWrapMetrics.cornerRadius
            )
            .fill(.black)

            NotchWrapBorderShape(
                wingWidth: wingWidth,
                bridgeHeight: bridgeHeight,
                notchOffset: notchOffset,
                notchWidth: notchWidth,
                notchHeight: notchHeight,
                cornerRadius: NotchWrapMetrics.cornerRadius
            )
            .stroke(
                NotchWrapMetrics.notchBorderColor.opacity(
                    isVoiceActive ? borderPulseOpacity : (isHovered || isInlineActive ? 0.80 : 0.35)
                ),
                lineWidth: 1
            )

            leftWingIconLayer
            iconDepartureLayer

            // Left icon — only when chat open, hovered, or voice active (idle = blank notch)
            if showsLeftStatusIcon {
                DrawInSymbol(
                    systemName: viewModel.selectedSourceMode?.symbolName ?? "sparkles",
                    trigger: hoverIconRevealID,
                    duration: NotchWrapMetrics.surfaceDuration,
                    reduceMotion: reduceMotion
                )
                .position(x: targetLeftIconPos.x, y: targetLeftIconPos.y)
            }

            // Right icon — animated waveform symbol in voice mode, status dot otherwise.
            // Dot hidden when idle and collapsed (pure notch bg); shown when chat open,
            // task running, approval pending, or error (so a down daemon always surfaces).
            if isVoiceActive {
                Image(systemName: voiceIconName)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.white)
                    // `.repeating` is the macOS 14-compatible equivalent of `.repeat(.continuous)`.
                    .symbolEffect(
                        .variableColor.iterative.dimInactiveLayers.nonReversing,
                        options: .repeating,
                        isActive: viewModel.speech.state == .listening
                    )
                    .contentTransition(.symbolEffect(.replace))
                    .position(rightIconPos)
                    .opacity(voiceContentOpacity)
            }

            // Voice content in bridge area (wave + live sentence)
            if isVoiceActive {
                VoiceNotchContent(speech: viewModel.speech, viewModel: viewModel)
                    .frame(
                        width: notchWidth + 2 * NotchWrapMetrics.voiceWingWidth - 20,
                        height: NotchWrapMetrics.voiceBridgeHeight - 12
                    )
                    .position(x: notchOffset + notchWidth / 2,
                              y: notchHeight + NotchWrapMetrics.voiceBridgeHeight / 2)
                    .opacity(voiceContentOpacity)
            }

            inlineSurfaceLayer
            cmuxBannerLayer
            statusDotLayer
            cmuxDotLayer
        }
    }

    private var interactiveSurface: some View {
        surfaceLayers
        .frame(width: maxSize.width, height: maxSize.height, alignment: .topLeading)
        .contentShape(
            NotchWrapShape(
                wingWidth: wingWidth,
                bridgeHeight: bridgeHeight,
                notchOffset: notchOffset,
                notchWidth: notchWidth,
                notchHeight: notchHeight,
                cornerRadius: NotchWrapMetrics.cornerRadius
            )
        )
        .onTapGesture { handleTap() }
        .onDrop(of: [.fileURL], isTargeted: $isDragTargeted) { providers in
            // Expand the chat panel, then queue the dropped files
            onTap()
            for provider in providers {
                provider.loadItem(forTypeIdentifier: "public.file-url", options: nil) { item, _ in
                    guard let data = item as? Data,
                          let url = URL(dataRepresentation: data, relativeTo: nil) else { return }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                        viewModel.addAttachments(urls: [url])
                    }
                }
            }
            return true
        }
        .onContinuousHover { phase in
            switch phase {
            case .active(let location):
                setPointerHover(isNearVisibleSurface(location))
            case .ended:
                setPointerHover(false)
            }
        }
        .onChange(of: viewModel.pillHovered) {
            if !isVoiceActive { refreshSurface() }
        }
        .onChange(of: isDragTargeted) { _, targeted in
            if !isVoiceActive { refreshSurface() }
            onHoverChanged(targeted || isHovered)
        }
        .onChange(of: viewModel.isVoiceNotchActive) { _, active in
            isVoiceActive = active
            setVoiceState(active: active)
        }
        .onChange(of: viewModel.notchInteractionMode) { _, mode in
            if mode != .output { outputWingRatchet = NotchWrapMetrics.outputMinWingWidth }
            if !isVoiceActive { refreshSurface() }
        }
        .onChange(of: viewModel.cmuxBanner) {
            if !isVoiceActive { refreshSurface() }
        }
        .onChange(of: viewModel.pendingConnectorActions) {
            if !isVoiceActive { refreshSurface() }
        }
    }

    var body: some View {
        interactiveSurface
        .onChange(of: viewModel.streamingChunk) {
            refreshOutputSurfaceIfNeeded()
        }
        .onChange(of: viewModel.latestAssistantMessageId) {
            outputWingRatchet = NotchWrapMetrics.outputMinWingWidth
            refreshOutputSurfaceIfNeeded(force: true)
        }
        .onChange(of: viewModel.isLatestAssistantStreaming) {
            refreshOutputSurfaceIfNeeded(force: true)
        }
        .onChange(of: viewModel.notchHoverResetID) {
            isHovered = false
            isDragTargeted = false
            if !isVoiceActive { refreshSurface() }
            onHoverChanged(false)
        }
        .onChange(of: status) {
            pulsing = (status == .thinking)
        }
        .onAppear {
            pulsing = (status == .thinking)
        }
        .onReceive(NotificationCenter.default.publisher(for: .bagentCodeCopied)) { _ in
            guard !reduceMotion else { return }
            withAnimation(.spring(response: 0.25, dampingFraction: 0.6)) { copyFlashed = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.3) {
                withAnimation(.easeInOut(duration: 0.3)) { copyFlashed = false }
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("bagent — \(status.accessibilityLabel)")
        .accessibilityHint("Otvoriť chat")
        .accessibilityAddTraits(.isButton)
    }
}

// MARK: - Inline notch input / output

private struct DrawInSymbol: View {
    let systemName: String
    let trigger: UUID
    let duration: Double
    let reduceMotion: Bool
    @State private var progress: CGFloat = 0

    var body: some View {
        Image(systemName: systemName)
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(Color.white.opacity(0.92))
            .contentTransition(.symbolEffect(.replace))
            .scaleEffect(0.82 + progress * 0.18)
            .opacity(0.20 + progress * 0.80)
            .mask(alignment: .leading) {
                GeometryReader { geo in
                    Rectangle()
                        .frame(width: geo.size.width * progress)
                }
            }
            .onAppear { runReveal() }
            .onChange(of: trigger) { runReveal() }
    }

    private func runReveal() {
        progress = reduceMotion ? 1 : 0
        let currentTrigger = trigger
        DispatchQueue.main.async {
            guard currentTrigger == trigger else { return }
            withAnimation(.easeInOut(duration: reduceMotion ? 0.01 : duration)) {
                progress = 1
            }
        }
    }
}

struct InlineNotchContent: View {
    @ObservedObject var viewModel: ChatViewModel
    let showsInputLeadingIcon: Bool
    @FocusState private var inputFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var placeholderRevealID = UUID()
    @State private var inlineFocusRetryID = UUID()

    init(viewModel: ChatViewModel, showsInputLeadingIcon: Bool = true) {
        self.viewModel = viewModel
        self.showsInputLeadingIcon = showsInputLeadingIcon
    }

    private var canSend: Bool {
        (!viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !viewModel.pendingAttachments.isEmpty)
            && !viewModel.isThinking
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            switch viewModel.notchInteractionMode {
            case .input:
                inputRow
            case .thinking:
                thinkingRow
            case .output:
                outputView
            case .collapsed:
                EmptyView()
            }
        }
        .font(.system(size: 15, weight: .regular))
        .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
        .onAppear { focusIfNeeded() }
        .onChange(of: viewModel.notchInteractionMode) {
            placeholderRevealID = UUID()
            focusIfNeeded()
        }
        .onChange(of: inputFocused) { _, focused in
            guard !focused else { return }
            keepInlineInputFocused(reason: "focus-lost")
        }
        .onChange(of: viewModel.inputText.isEmpty) { _, isEmpty in
            if isEmpty { placeholderRevealID = UUID() }
        }
    }

    private var inputRow: some View {
        HStack(spacing: 10) {
            if showsInputLeadingIcon {
                Image(systemName: viewModel.selectedSourceMode?.symbolName ?? "sparkles")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(Color.white.opacity(0.82))
                    .frame(width: 18, height: 18)
            }

            ZStack(alignment: .leading) {
                if viewModel.inputText.isEmpty {
                    LeftToRightRevealText(
                        text: "What's on your mind?",
                        trigger: placeholderRevealID,
                        delay: NotchWrapMetrics.surfaceDuration * 0.82,
                        reduceMotion: reduceMotion
                    )
                        .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                        .allowsHitTesting(false)
                        .id(placeholderRevealID)
                }

                TextField("", text: $viewModel.inputText)
                    .textFieldStyle(.plain)
                    .font(.system(size: 14, weight: .regular))
                    .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                    .focused($inputFocused)
                    .onSubmit {
                        if canSend { viewModel.send() }
                    }
                    .opacity(viewModel.inputText.isEmpty ? 0.02 : 1)
                    .animation(.easeOut(duration: 0.18), value: viewModel.inputText.isEmpty)
            }
            .frame(height: 24)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var thinkingRow: some View {
        HStack(spacing: 10) {
            ThinkingIndicator()
                .scaleEffect(0.74)
            VStack(alignment: .leading, spacing: 3) {
                Text("Thinking")
                    .font(.system(size: 14, weight: .regular))
                    .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                if !viewModel.latestUserText.isEmpty {
                    Text(viewModel.latestUserText)
                        .font(.system(size: 12, weight: .regular))
                        .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var outputView: some View {
        let text = NotchOutputLayout.responseText(
            latestText: viewModel.latestAssistantText,
            isStreaming: viewModel.isLatestAssistantStreaming,
            isThinking: viewModel.isThinking
        )
        let latestMessage = viewModel.latestAssistantMessage
        return VStack(alignment: .leading, spacing: 6) {
            ZStack(alignment: .topTrailing) {
                LatestAssistantOutputScrollView(
                    text: text,
                    messageId: viewModel.latestAssistantMessageId,
                    isStreaming: viewModel.isLatestAssistantStreaming,
                    reduceMotion: reduceMotion
                )
                .padding(.leading, latestMessage?.debugTraceId == nil ? 0 : 34)
                .frame(maxWidth: .infinity)
                .frame(maxHeight: .infinity)
                .accessibilityLabel("Latest assistant response")
                .accessibilityValue(text)

                if latestMessage?.debugTraceId != nil {
                    NotchDebugCopyButton(viewModel: viewModel)
                        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                        .transition(.opacity.combined(with: .scale(scale: 0.94, anchor: .topLeading)))
                }
            }

        }
    }

    private func focusIfNeeded() {
        guard viewModel.notchInteractionMode == .input else {
            inlineFocusRetryID = UUID()
            inputFocused = false
            return
        }
        keepInlineInputFocused(reason: "input-mode")
    }

    private func keepInlineInputFocused(reason: String) {
        _ = reason
        let retryID = UUID()
        inlineFocusRetryID = retryID
        for attempt in 0..<5 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.02 + Double(attempt) * 0.06) {
                guard inlineFocusRetryID == retryID,
                      viewModel.notchInteractionMode == .input
                else { return }
                NSApp.activate(ignoringOtherApps: true)
                inputFocused = true
            }
        }
    }
}

private struct LatestAssistantOutputScrollView: NSViewRepresentable {
    let text: String
    let messageId: UUID?
    let isStreaming: Bool
    let reduceMotion: Bool

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = UserTrackingScrollView()
        scrollView.onUserScroll = { [weak coordinator = context.coordinator] in
            coordinator?.handleUserScroll()
        }
        scrollView.drawsBackground = false
        scrollView.borderType = .noBorder
        scrollView.hasVerticalScroller = false
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.verticalScrollElasticity = .allowed
        scrollView.horizontalScrollElasticity = .none
        scrollView.contentView.postsBoundsChangedNotifications = true

        let textView = NSTextView()
        textView.drawsBackground = false
        textView.isEditable = false
        textView.isSelectable = true
        textView.isRichText = false
        textView.importsGraphics = false
        textView.textContainerInset = .zero
        textView.textContainer?.lineFragmentPadding = 0
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.heightTracksTextView = false
        textView.isHorizontallyResizable = false
        textView.isVerticallyResizable = true
        textView.autoresizingMask = [.width]
        textView.backgroundColor = .clear
        textView.insertionPointColor = .white

        scrollView.documentView = textView
        context.coordinator.scrollView = scrollView
        context.coordinator.textView = textView
        NotificationCenter.default.addObserver(
            context.coordinator,
            selector: #selector(Coordinator.boundsDidChange(_:)),
            name: NSView.boundsDidChangeNotification,
            object: scrollView.contentView
        )
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = context.coordinator.textView else { return }
        let coordinator = context.coordinator
        let messageChanged = coordinator.messageId != messageId

        if messageChanged {
            coordinator.messageId = messageId
            coordinator.userScrolledAway = false
            coordinator.lastText = nil
        }

        if coordinator.lastText != text {
            coordinator.lastText = text
            textView.textStorage?.setAttributedString(attributedText(text))
        }

        coordinator.resizeTextView()
        coordinator.applyAutoScroll()
    }

    static func dismantleNSView(_ nsView: NSScrollView, coordinator: Coordinator) {
        NotificationCenter.default.removeObserver(
            coordinator,
            name: NSView.boundsDidChangeNotification,
            object: nsView.contentView
        )
    }

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    private func attributedText(_ text: String) -> NSAttributedString {
        NotchMarkdown.attributedString(text)
    }

    @MainActor
    final class Coordinator: NSObject {
        weak var scrollView: NSScrollView?
        weak var textView: NSTextView?
        var messageId: UUID?
        var lastText: String?
        var userScrolledAway = false
        private var lastClipSize: NSSize = .zero

        /// Called only for scroll-wheel/trackpad events (including momentum),
        /// never for programmatic pins — so it is the single source of truth
        /// for user scroll intent. Scrolling back to the bottom re-engages
        /// the streaming auto-scroll.
        func handleUserScroll() {
            userScrolledAway = !isNearBottom()
        }

        @objc func boundsDidChange(_ note: Notification) {
            guard let scrollView else { return }
            let size = scrollView.contentView.bounds.size
            let clipHeightChanged = abs(size.height - lastClipSize.height) > 0.5
            let clipWidthChanged = abs(size.width - lastClipSize.width) > 0.5
            lastClipSize = size
            guard clipHeightChanged || clipWidthChanged else { return }
            resizeTextView()
            applyAutoScroll()
        }

        /// Two-phase streaming scroll policy:
        /// - Growth phase (viewport below its max height): the text stays
        ///   top-anchored and perfectly still while the notch panel grows
        ///   downward to reveal new lines. Bottom-pinning here would drag the
        ///   text down with the animating panel edge while token pins push it
        ///   up — the vertical jumping this replaces.
        /// - Scroll phase (viewport fully grown): pin to the bottom so new
        ///   lines scroll into view. Never overrides a user's manual scroll.
        func applyAutoScroll() {
            guard !userScrolledAway else { return }
            if viewportFullyGrown() {
                pinToBottom()
            } else {
                scrollTo(y: 0)
            }
        }

        func viewportFullyGrown() -> Bool {
            guard let scrollView else { return false }
            return scrollView.contentView.bounds.height >= NotchOutputLayout.maxViewportHeight - 1
        }

        func resizeTextView() {
            guard let scrollView, let textView, let textContainer = textView.textContainer else { return }
            let width = max(1, scrollView.contentSize.width)
            textContainer.containerSize = NSSize(width: width, height: CGFloat.greatestFiniteMagnitude)
            textView.frame.size.width = width
            textView.layoutManager?.ensureLayout(for: textContainer)
            let usedRect = textView.layoutManager?.usedRect(for: textContainer) ?? .zero
            let height = max(scrollView.contentSize.height, ceil(usedRect.height) + NotchOutputLayout.bottomSlack)
            textView.setFrameSize(NSSize(width: width, height: height))
        }

        func isNearBottom(threshold: CGFloat = 12) -> Bool {
            guard let scrollView, let documentView = scrollView.documentView else { return true }
            let maxY = max(0, documentView.bounds.height - scrollView.contentView.bounds.height)
            let distance = maxY - scrollView.contentView.bounds.origin.y
            return distance <= threshold
        }

        func pinToBottom() {
            guard let scrollView, let documentView = scrollView.documentView else { return }
            scrollTo(y: max(0, documentView.bounds.height - scrollView.contentView.bounds.height))
        }

        func scrollTo(y targetY: CGFloat) {
            guard let scrollView else { return }
            let clipView = scrollView.contentView
            guard abs(clipView.bounds.origin.y - targetY) > 0.5 else { return }
            clipView.setBoundsOrigin(NSPoint(x: 0, y: targetY))
            scrollView.reflectScrolledClipView(clipView)
        }
    }
}

/// NSScrollView that reports user-initiated scrolling. Programmatic
/// `setBoundsOrigin` pins do not go through `scrollWheel`, so this cleanly
/// separates user intent from streaming auto-scroll.
private final class UserTrackingScrollView: NSScrollView {
    var onUserScroll: (() -> Void)?

    override func scrollWheel(with event: NSEvent) {
        super.scrollWheel(with: event)
        onUserScroll?()
    }
}

private struct NotchDebugCopyButton: View {
    @ObservedObject var viewModel: ChatViewModel
    @State private var isHovered = false
    @State private var isLoading = false

    var body: some View {
        Button {
            guard !isLoading else { return }
            isLoading = true
            Task {
                let payload = await viewModel.latestDebugClipboardPayload()
                copyToPasteboard(payload)
                isLoading = false
            }
        } label: {
            Image(systemName: isLoading ? "arrow.triangle.2.circlepath" : "wrench.and.screwdriver.fill")
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(Color.white.opacity(isHovered ? 0.95 : 0.72))
                .frame(width: 24, height: 24)
                .background(
                    Circle()
                        .fill(Color.black.opacity(isHovered ? 0.52 : 0.38))
                        .overlay(Circle().stroke(Color.cyan.opacity(isHovered ? 0.58 : 0.32), lineWidth: 0.6))
                )
        }
        .buttonStyle(.plain)
        .help("Copy latest session and debug trace")
        .accessibilityLabel("Copy latest session and debug trace")
        .disabled(isLoading)
        .onHover { hovering in
            withAnimation(.spring(response: 0.22, dampingFraction: 0.78)) {
                isHovered = hovering
            }
            hovering ? NSCursor.pointingHand.push() : NSCursor.pop()
        }
    }
}

private struct LeftToRightRevealText: View {
    let text: String
    let trigger: UUID
    let delay: Double
    let reduceMotion: Bool
    @State private var reveal: CGFloat = 0

    var body: some View {
        Text(text)
            .font(.system(size: 14, weight: .regular))
            .lineLimit(1)
            .mask(alignment: .leading) {
                GeometryReader { geo in
                    Rectangle()
                        .frame(width: geo.size.width * reveal)
                }
            }
            .onAppear {
                runReveal()
            }
            .onChange(of: trigger) {
                runReveal()
            }
    }

    private func runReveal() {
        reveal = reduceMotion ? 1 : 0
        let currentTrigger = trigger
        DispatchQueue.main.asyncAfter(deadline: .now() + (reduceMotion ? 0 : delay)) {
            guard currentTrigger == trigger else { return }
            withAnimation(.easeOut(duration: reduceMotion ? 0.01 : 0.46)) {
                reveal = 1
            }
        }
    }
}

// MARK: - Status dot

struct LightbulbStatusDotView: View {
    let status: AgentStatus
    let reduceMotion: Bool
    var copyFlashed: Bool = false
    var isDragTargeted: Bool = false

    @State private var bulbOpacity: CGFloat = 1
    @State private var bulbScale: CGFloat = 1
    @State private var ignitionID = UUID()

    var body: some View {
        Circle()
            .fill(status.color)
            .frame(width: 8, height: 8)
            .scaleEffect((copyFlashed || isDragTargeted) ? 0.2 : bulbScale)
            .opacity((copyFlashed || isDragTargeted) ? 0 : bulbOpacity)
            .shadow(color: status.color.opacity(bulbOpacity * 0.65), radius: 3, x: 0, y: 0)
            .onAppear { runIgnition() }
            .onChange(of: status) { runIgnition() }
    }

    private func runIgnition() {
        ignitionID = UUID()
        let currentID = ignitionID
        guard !reduceMotion else {
            bulbOpacity = 1
            bulbScale = 1
            return
        }

        let frames: [(delay: Double, opacity: CGFloat, scale: CGFloat)] = [
            (0.00, 0.10, 0.84),
            (0.10, 0.92, 1.08),
            (0.21, 0.24, 0.92),
            (0.39, 1.00, 1.13),
            (0.54, 0.36, 0.96),
            (0.77, 0.88, 1.05),
            (0.98, 1.00, 1.00),
        ]

        for frame in frames {
            DispatchQueue.main.asyncAfter(deadline: .now() + frame.delay) {
                guard currentID == ignitionID else { return }
                withAnimation(.easeInOut(duration: 0.08)) {
                    bulbOpacity = frame.opacity
                    bulbScale = frame.scale
                }
            }
        }
    }
}

struct StatusDotView: View {
    let status: AgentStatus
    @Binding var pulsing: Bool
    let reduceMotion: Bool
    var copyFlashed: Bool = false
    var isDragTargeted: Bool = false

    @State private var dotBlink = false
    @State private var dropFlashScale: CGFloat = 1.0
    @State private var showDropPlus = false

    private let flashGreen = Color(red: 0.18, green: 0.80, blue: 0.44)
    private let dotBlinkDuration: Double = 0.6   // half-cycle; full = 1.2 s
    private var ringDuration: Double { dotBlinkDuration * 2 }  // 1.2 s — in sync with dot

    var body: some View {
        ZStack {
            // Expanding pulse ring (thinking state) — period matches dot blink
            if status == .thinking && !reduceMotion {
                Circle()
                    .fill(status.color.opacity(0.45))
                    .frame(width: 16, height: 16)
                    .scaleEffect(pulsing ? 1.9 : 1.0)
                    .opacity(pulsing ? 0.0 : 0.65)
                    .animation(
                        pulsing
                            ? .easeOut(duration: ringDuration).repeatForever(autoreverses: false)
                            : .default,
                        value: pulsing
                    )
            }

            // Normal status dot — blinks while thinking, fades on copy flash or drag
            Circle()
                .fill(status.color)
                .frame(width: 8, height: 8)
                .scaleEffect(copyFlashed || isDragTargeted ? 0.2 : 1.0)
                .opacity(copyFlashed || isDragTargeted ? 0 : (status == .thinking && !reduceMotion ? (dotBlink ? 0.28 : 1.0) : 1.0))
                .animation(
                    status == .thinking && !reduceMotion
                        ? .easeInOut(duration: dotBlinkDuration).repeatForever(autoreverses: true)
                        : .default,
                    value: dotBlink
                )

            // Green tick — scales in on copy flash
            ZStack {
                Circle()
                    .fill(flashGreen)
                    .frame(width: 14, height: 14)
                Image(systemName: "checkmark")
                    .font(.system(size: 7, weight: .heavy))
                    .foregroundStyle(.white)
            }
            .scaleEffect(copyFlashed ? 1.0 : 0.2)
            .opacity(copyFlashed ? 1 : 0)

            // + sign — shown while dragging a file over the notch, delayed until expand settles
            ZStack {
                Circle()
                    .fill(Color.accentColor)
                    .frame(width: 14, height: 14)
                Image(systemName: "plus")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(.white)
            }
            .scaleEffect(showDropPlus ? dropFlashScale : 0.2)
            .opacity(showDropPlus ? 1 : 0)
            .animation(.spring(response: 0.22, dampingFraction: 0.6), value: showDropPlus)
            .onChange(of: isDragTargeted) { _, targeted in
                if targeted {
                    // Wait for notch expand spring (~0.28s response) to settle before showing +
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                        guard isDragTargeted else { return }
                        showDropPlus = true
                        withAnimation(.spring(response: 0.15, dampingFraction: 0.35)) { dropFlashScale = 1.5 }
                        DispatchQueue.main.asyncAfter(deadline: .now() + 0.18) {
                            withAnimation(.spring(response: 0.3, dampingFraction: 0.65)) { dropFlashScale = 1.0 }
                        }
                    }
                } else {
                    showDropPlus = false
                    dropFlashScale = 1.0
                }
            }
        }
        .onAppear { dotBlink = status == .thinking }
        .onChange(of: status) { dotBlink = status == .thinking }
    }
}

/// Right-wing dot for pending cmux events — same pulse/blink rhythm as the
/// thinking dot; amber = agent needs attention, green = agent finished.
struct CmuxStatusDotView: View {
    let kind: CmuxEventKind
    let reduceMotion: Bool

    @State private var pulsing = false
    @State private var blink = false

    private let blinkDuration: Double = 0.6
    private var ringDuration: Double { blinkDuration * 2 }
    private var color: Color {
        kind == .attention ? AgentStatus.awaitingApproval.color : AgentStatus.ready.color
    }

    var body: some View {
        ZStack {
            if !reduceMotion {
                Circle()
                    .fill(color.opacity(0.45))
                    .frame(width: 16, height: 16)
                    .scaleEffect(pulsing ? 1.9 : 1.0)
                    .opacity(pulsing ? 0.0 : 0.65)
                    .animation(
                        pulsing
                            ? .easeOut(duration: ringDuration).repeatForever(autoreverses: false)
                            : .default,
                        value: pulsing
                    )
            }
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
                .opacity(reduceMotion ? 1.0 : (blink ? 0.28 : 1.0))
                .animation(
                    reduceMotion
                        ? .default
                        : .easeInOut(duration: blinkDuration).repeatForever(autoreverses: true),
                    value: blink
                )
        }
        .onAppear {
            pulsing = true
            blink = true
        }
    }
}

/// cmux app icon shown in the left wing of the notch. For attention events it
/// swings from its top edge like a rung bell — a periodic nudge that manual
/// action is waiting in cmux. Finished events keep the icon still.
struct CmuxIconView: View {
    let kind: CmuxEventKind
    let reduceMotion: Bool
    var size: CGFloat = 17

    var body: some View {
        let icon = Image(nsImage: CmuxEventMonitor.appIcon)
            .resizable()
            .aspectRatio(contentMode: .fit)
            .frame(width: size, height: size)
        if kind == .attention && !reduceMotion {
            icon.phaseAnimator(NotchIconSwing.phases) { view, angle in
                view.rotationEffect(.degrees(angle), anchor: .top)
            } animation: { angle in
                NotchIconSwing.animation(for: angle)
            }
        } else {
            icon
        }
    }
}

/// An acknowledged notch icon flying off. Four concurrent keyframe tracks
/// (travel, scale, opacity, tilt) — an anticipation pop then a quick
/// shrink-and-fade as it settles into the notch (built-in) or shoots off the
/// top edge (external). Reports back when the flight is over so the token can
/// be dropped. Only spawned when Reduce Motion is off. Shared by the cmux icon
/// and the connector icons.
struct IconFlyoffFrame {
    var x: CGFloat
    var y: CGFloat
    var scale: CGFloat
    var opacity: CGFloat
    var rotation: CGFloat
}

struct IconFlyoffView<Icon: View>: View {
    let start: CGPoint
    let target: CGPoint
    let hasNotch: Bool
    var size: CGFloat = 17
    let onFinished: () -> Void
    @ViewBuilder let icon: () -> Icon

    var body: some View {
        // Type-erased so the keyframe animator below isn't generic (avoids a
        // non-Sendable metatype capture warning in its closures).
        IconFlyoffCore(
            start: start, target: target, hasNotch: hasNotch,
            size: size, onFinished: onFinished, icon: AnyView(icon())
        )
    }
}

private struct IconFlyoffCore: View {
    let start: CGPoint
    let target: CGPoint
    let hasNotch: Bool
    let size: CGFloat
    let onFinished: () -> Void
    let icon: AnyView

    @State private var play = false

    /// Built-in settles a touch slower than the external "lose it over the top".
    private var duration: Double { hasNotch ? 0.5 : 0.42 }

    var body: some View {
        let endScale: CGFloat = hasNotch ? 0.26 : 0.55
        icon
            .frame(width: size, height: size)
            .keyframeAnimator(
                initialValue: IconFlyoffFrame(x: start.x, y: start.y, scale: 1, opacity: 1, rotation: 0),
                trigger: play
            ) { view, f in
                view
                    .rotationEffect(.degrees(f.rotation))
                    .scaleEffect(f.scale)
                    .opacity(f.opacity)
                    .position(x: f.x, y: f.y)
            } keyframes: { _ in
                KeyframeTrack(\.x) {
                    CubicKeyframe(target.x, duration: duration)
                }
                KeyframeTrack(\.y) {
                    // brief anticipation dip, then travel to the target
                    SpringKeyframe(start.y + 5, duration: duration * 0.24, spring: .snappy)
                    CubicKeyframe(target.y, duration: duration * 0.76)
                }
                KeyframeTrack(\.scale) {
                    CubicKeyframe(1.10, duration: duration * 0.24)
                    CubicKeyframe(endScale, duration: duration * 0.76)
                }
                KeyframeTrack(\.opacity) {
                    // Built-in: hold opaque so it visibly reaches the notch, then fade.
                    // External: fade earlier so it dissolves before the panel's top edge
                    // clips it (target is above the window bounds).
                    LinearKeyframe(1.0, duration: duration * (hasNotch ? 0.55 : 0.28))
                    CubicKeyframe(0.0, duration: duration * (hasNotch ? 0.45 : 0.72))
                }
                KeyframeTrack(\.rotation) {
                    CubicKeyframe(-6, duration: duration * 0.3)
                    CubicKeyframe(0, duration: duration * 0.7)
                }
            }
            .onAppear {
                play.toggle()
                // Hoisted so the async closure doesn't capture the generic self.
                let finish = onFinished
                DispatchQueue.main.asyncAfter(deadline: .now() + duration + 0.05) {
                    finish()
                }
            }
    }
}

/// "Listening" label with three animated dots that fade in sequentially and
/// drift left↔right as a group. Falls back to a static "Listening…" in
/// Reduce Motion mode.
struct ListeningDotsView: View {
    let reduceMotion: Bool
    @State private var dotOffset: CGFloat = 0

    var body: some View {
        if reduceMotion {
            Text("Listening…")
        } else {
            HStack(spacing: 1) {
                Text("Listening")
                TimelineView(.animation) { timeline in
                    HStack(spacing: 2) {
                        ForEach(0..<3, id: \.self) { i in
                            let t = timeline.date.timeIntervalSinceReferenceDate
                            let phase = t * 2.2 + Double(i) * 0.55
                            let opacity = (sin(phase) + 1) / 2   // 0…1
                            Text("•")
                                .opacity(0.30 + opacity * 0.70)
                        }
                    }
                    .offset(x: {
                        let t = timeline.date.timeIntervalSinceReferenceDate
                        return CGFloat(sin(t * 1.1)) * 2.5
                    }())
                }
            }
        }
    }
}

// MARK: - Chat panel content (shown below the pill when expanded)

struct ChatPanelContent: View {
    @ObservedObject var viewModel: ChatViewModel
    let onCollapse: () -> Void

    var body: some View {
        ZStack {
            if viewModel.chatSurfaceMode == .inputOnly {
                SpotlightInputPanel(viewModel: viewModel, onCollapse: onCollapse)
                    .transition(
                        .scale(scale: 0.82, anchor: UnitPoint(x: 0.5, y: 0))
                        .combined(with: .opacity)
                    )
            } else if viewModel.isExpanded {
                ExpandedChatView(viewModel: viewModel, onCollapse: onCollapse)
                    .transition(
                        .scale(scale: 0.82, anchor: UnitPoint(x: 0.5, y: 0))
                        .combined(with: .opacity)
                    )
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .animation(.spring(response: 0.30, dampingFraction: 0.62), value: viewModel.isExpanded)
        .animation(.spring(response: 0.30, dampingFraction: 0.68), value: viewModel.chatSurfaceMode)
    }
}

// MARK: - Spotlight-style input panel

private struct LiquidGlassFallbackSurface: ViewModifier {
    let cornerRadius: CGFloat

    func body(content: Content) -> some View {
        content
            .background(.ultraThinMaterial)
            .overlay(alignment: .topLeading) {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .fill(
                        LinearGradient(
                            colors: [
                                .white.opacity(0.18),
                                .white.opacity(0.04),
                                .clear,
                            ],
                            startPoint: .topLeading,
                            endPoint: .bottomTrailing
                        )
                    )
                    .blendMode(.screen)
                    .allowsHitTesting(false)
            }
            .overlay {
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .stroke(.white.opacity(0.16), lineWidth: 1)
            }
            .clipShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .shadow(color: .black.opacity(0.34), radius: 22, x: 0, y: 18)
    }
}

private extension View {
    func liquidGlassFallback(cornerRadius: CGFloat) -> some View {
        modifier(LiquidGlassFallbackSurface(cornerRadius: cornerRadius))
    }

    /// Capsule glass surface. On macOS 26+ uses real Liquid Glass; falls back to
    /// the hand-rolled material surface on earlier systems.
    @ViewBuilder
    func liquidGlassInputSurface() -> some View {
        if #available(macOS 26, *) {
            self.glassEffect(.regular, in: .capsule)
        } else {
            self.liquidGlassFallback(cornerRadius: 18)
        }
    }

    /// Circle glass surface for source-mode bubbles. Matches the capsule's `.regular`
    /// glass on macOS 26+; falls back to the hand-rolled material on earlier systems.
    @ViewBuilder
    func liquidGlassBubbleSurface(selected: Bool) -> some View {
        if #available(macOS 26, *) {
            self.glassEffect(
                selected ? .regular.tint(Color.accentColor) : .regular,
                in: .circle
            )
        } else {
            self
                .background(selected ? Color.accentColor.opacity(0.86) : Color.white.opacity(0.05))
                .liquidGlassFallback(cornerRadius: 22)
        }
    }
}

struct SpotlightInputPanel: View {
    @ObservedObject var viewModel: ChatViewModel
    let onCollapse: () -> Void

    @FocusState private var inputFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var fieldWidth: CGFloat = 220
    @State private var verticalOffset: CGFloat = -34
    @State private var fieldOpacity: CGFloat = 0
    @State private var localPickerVisible = false

    private let fullFieldWidth: CGFloat = 540
    private let compactFieldWidth: CGFloat = 356
    private let inputHeight: CGFloat = 56

    private var pickerVisible: Bool {
        localPickerVisible || viewModel.isSourcePickerForced
    }

    private var currentFieldWidth: CGFloat {
        pickerVisible ? compactFieldWidth : fieldWidth
    }

    var body: some View {
        glassPillLayout
            .padding(.horizontal, 18)
            .padding(.vertical, 12)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .offset(y: verticalOffset)
            .opacity(fieldOpacity)
            .onHover { hovering in
                localPickerVisible = hovering
            }
            .onAppear {
                viewModel.hoveredSourceMode = nil
                inputFocused = true
                if reduceMotion {
                    fieldOpacity = 1
                    verticalOffset = 0
                    fieldWidth = fullFieldWidth
                } else {
                    withAnimation(.spring(response: 0.34, dampingFraction: 0.66)) {
                        fieldOpacity = 1
                        verticalOffset = 0
                    }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.18) {
                        withAnimation(.easeOut(duration: 0.22)) {
                            fieldWidth = fullFieldWidth
                        }
                    }
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) {
                    inputFocused = true
                }
            }
            .onDisappear {
                viewModel.hoveredSourceMode = nil
                viewModel.isSourcePickerForced = false
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel("bagent input")
    }

    /// The pill layout container.
    ///
    /// On macOS 26+ wraps the search bar and icon row inside a `GlassEffectContainer`
    /// so both elements share a sampling region and the icon row can morph in/out
    /// as a proper glass transition. The picker stays mounted so repeated Cmd
    /// presses do not recreate each bubble mid-animation.
    ///
    /// On earlier systems keeps the original opacity/scale/frame trick unchanged.
    @ViewBuilder
    private var glassPillLayout: some View {
        // Picker is conditionally rendered (not just hidden) so GlassEffectContainer
        // never registers the glass circles when the picker is not visible — otherwise
        // the dark glass shapes show through even at opacity(0).
        let pickerTransition: AnyTransition = reduceMotion
            ? .opacity
            : .opacity.combined(with: .scale(scale: 0.88, anchor: .leading))

        if #available(macOS 26, *) {
            GlassEffectContainer(spacing: 12) {
                HStack(spacing: 12) {
                    inputField
                        .frame(width: currentFieldWidth, height: inputHeight)
                        .animation(
                            reduceMotion ? .easeOut(duration: 0.12) : .spring(response: 0.30, dampingFraction: 0.72),
                            value: pickerVisible
                        )
                        .animation(.easeOut(duration: 0.20), value: fieldWidth)

                    if pickerVisible {
                        SourceModePicker(viewModel: viewModel, visible: true)
                            .frame(width: 208, height: inputHeight)
                            .transition(pickerTransition)
                    }
                }
                .animation(
                    reduceMotion ? .easeOut(duration: 0.12) : .spring(response: 0.30, dampingFraction: 0.68),
                    value: pickerVisible
                )
            }
        } else {
            HStack(spacing: 12) {
                inputField
                    .frame(width: currentFieldWidth, height: inputHeight)
                    .animation(
                        reduceMotion ? .easeOut(duration: 0.12) : .spring(response: 0.30, dampingFraction: 0.72),
                        value: pickerVisible
                    )
                    .animation(.easeOut(duration: 0.20), value: fieldWidth)

                if pickerVisible {
                    SourceModePicker(viewModel: viewModel, visible: true)
                        .frame(width: 208, height: inputHeight)
                        .transition(pickerTransition)
                }
            }
            .animation(
                reduceMotion ? .easeOut(duration: 0.12) : .spring(response: 0.30, dampingFraction: 0.68),
                value: pickerVisible
            )
        }
    }

    private var inputField: some View {
        HStack(spacing: 10) {
            Image(systemName: viewModel.selectedSourceMode?.symbolName ?? "magnifyingglass")
                .font(.system(size: 20, weight: .medium))
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 24)

            TextField(viewModel.activeSourcePlaceholder, text: $viewModel.inputText)
                .textFieldStyle(.plain)
                .font(.system(size: 22, weight: .regular))
                .focused($inputFocused)
                .onSubmit {
                    viewModel.send()
                }

            if viewModel.selectedSourceMode != nil {
                Button { viewModel.clearSourceMode() } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(.tertiary)
                }
                .buttonStyle(.plain)
                .help("Clear source")
                .accessibilityLabel("Clear source")
            }
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
        .liquidGlassInputSurface()
    }
}

struct SourceModePicker: View {
    @ObservedObject var viewModel: ChatViewModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let visible: Bool

    var body: some View {
        HStack(spacing: 8) {
            ForEach(Array(viewModel.topSourceModes.prefix(4).enumerated()), id: \.element.id) { idx, mode in
                SourceModeBubble(
                    mode: mode,
                    index: idx,
                    selected: viewModel.selectedSourceMode == mode,
                    visible: visible,
                    reduceMotion: reduceMotion,
                    onSelect: { viewModel.selectSourceMode(mode) },
                    onHover: { hovering in
                        viewModel.hoveredSourceMode = hovering ? mode : nil
                    }
                )
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Source modes")
    }
}

struct SourceModeBubble: View {
    let mode: SourceMode
    let index: Int
    let selected: Bool
    let visible: Bool
    let reduceMotion: Bool
    let onSelect: () -> Void
    let onHover: (Bool) -> Void

    var body: some View {
        Button(action: onSelect) {
            // Use a neutral Color.clear base so the glass circle is sized from a
            // true 44×44 square rather than from the SF Symbol's font layout box
            // (which includes baseline/side-bearing space and shifts the circle).
            // The icon is then independently centered via .overlay.
            Image(systemName: mode.symbolName)
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(selected ? .white : .primary)
                .frame(width: 44, height: 44)
                .liquidGlassBubbleSurface(selected: selected)
        }
        .buttonStyle(.plain)
        .help("\(mode.title) (⌃\(index + 1))")
        .accessibilityLabel(mode.title)
        .scaleEffect(reduceMotion || visible ? 1 : 0.82)
        .opacity(visible ? 1 : 0)
        .offset(x: reduceMotion || visible ? 0 : -4)
        .animation(
            reduceMotion
                ? .easeOut(duration: 0.10)
                : .spring(response: 0.22, dampingFraction: 0.82).delay(visible ? Double(index) * 0.024 : 0),
            value: visible
        )
        .allowsHitTesting(visible)
        .onHover(perform: onHover)
    }
}

// MARK: - Expanded chat panel

/// PreferenceKey that tracks the minY of the LazyVStack content in the ScrollView's
/// coordinate space — used to detect whether the user has scrolled away from the bottom.
private struct ScrollOffsetKey: PreferenceKey {
    nonisolated(unsafe) static var defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

struct ExpandedChatView: View {
    @ObservedObject var viewModel: ChatViewModel
    let onCollapse: () -> Void
    @FocusState private var inputFocused: Bool
    @State private var dragBaseW: CGFloat? = nil
    @State private var dragBaseH: CGFloat? = nil
    @State private var isResizing = false
    @State private var isDropTargeted = false
    /// True when the user scrolled away from the bottom during streaming.
    /// Gates the auto-scroll-to-bottom behavior.
    @State private var userScrolledUp = false
    /// Height of the ScrollView viewport — measured via GeometryReader.
    @State private var scrollViewHeight: CGFloat = 0
    /// Current content minY offset in the ScrollView coordinate space.
    @State private var contentOffsetY: CGFloat = 0

    var body: some View {
        ZStack {
            VStack(spacing: 0) {
                header
                Divider()
                if viewModel.showWhatsappPairing {
                    WhatsAppPairingView(viewModel: viewModel)
                        .transition(
                            .asymmetric(
                                insertion: .move(edge: .trailing).combined(with: .opacity),
                                removal: .move(edge: .leading).combined(with: .opacity)
                            )
                        )
                } else {
                    messageList
                    Divider()
                    inputBar
                }
            }
            // Swap .regularMaterial for a solid background while resizing to
            // prevent the vibrancy layer re-layout from shaking text content.
            .background(isResizing
                ? AnyShapeStyle(Color(nsColor: .windowBackgroundColor).opacity(0.96))
                : AnyShapeStyle(.regularMaterial)
            )
            .clipShape(RoundedRectangle(cornerRadius: 16))
            .overlay {
                if let approval = viewModel.pendingApprovals.first {
                    ApprovalModalOverlay(approval: approval, viewModel: viewModel)
                }
            }
            .overlay {
                // Drag-drop highlight border
                if isDropTargeted {
                    RoundedRectangle(cornerRadius: 16)
                        .stroke(Color.accentColor, lineWidth: 2)
                }
            }

            resizeHandles
        }
        // Accept file drops onto the conversation area
        .onDrop(of: [.fileURL], isTargeted: $isDropTargeted) { providers in
            handleFileDrop(providers)
        }
        .alert("Nainštalovať model pre obrázky?", isPresented: $viewModel.showVisionModelAlert) {
            Button("Zavrieť") {}
        } message: {
            Text("Na analýzu obrázkov je potrebný model qwen2.5vl:7b.\nSpusti v termináli: ollama pull qwen2.5vl:7b")
        }
        .onAppear {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { inputFocused = true }
            viewModel.startApprovalPolling()
            // Restore scroll viewport is handled inside messageList via ScrollViewProxy.
        }
        .onDisappear {
            viewModel.stopApprovalPolling()
            // Save whether we were at the bottom or had scrolled up.
            viewModel.savedScrollWasAtBottom = !userScrolledUp
            if userScrolledUp {
                // Save the topmost-visible message id as the anchor.
                // We approximate this as the first message whose index corresponds to
                // the current scroll offset — use the last message as a safe fallback.
                viewModel.savedScrollAnchorId = viewModel.messages.last?.id
            } else {
                viewModel.savedScrollAnchorId = nil
            }
        }
        // Escape handled by NSEvent local monitor in NotchWindowController
    }

    private func handleFileDrop(_ providers: [NSItemProvider]) -> Bool {
        var handled = false
        for provider in providers {
            if provider.hasItemConformingToTypeIdentifier("public.file-url") {
                provider.loadItem(forTypeIdentifier: "public.file-url", options: nil) { item, _ in
                    guard let data = item as? Data,
                          let url = URL(dataRepresentation: data, relativeTo: nil) else { return }
                    DispatchQueue.main.async { self.viewModel.addAttachments(urls: [url]) }
                }
                handled = true
            }
        }
        return handled
    }

    // MARK: - Resize handles

    private var resizeHandles: some View {
        ZStack {
            // Right edge
            HStack(spacing: 0) {
                Spacer()
                Color.clear.frame(width: 6)
                    .contentShape(Rectangle())
                    .onHover { h in h ? NSCursor.resizeLeftRight.push() : NSCursor.pop() }
                    .gesture(rightEdgeDrag)
            }
            // Left edge
            HStack(spacing: 0) {
                Color.clear.frame(width: 6)
                    .contentShape(Rectangle())
                    .onHover { h in h ? NSCursor.resizeLeftRight.push() : NSCursor.pop() }
                    .gesture(leftEdgeDrag)
                Spacer()
            }
            // Bottom edge
            VStack(spacing: 0) {
                Spacer()
                Color.clear.frame(height: 6)
                    .contentShape(Rectangle())
                    .onHover { h in h ? NSCursor.resizeUpDown.push() : NSCursor.pop() }
                    .gesture(bottomEdgeDrag)
            }
        }
    }

    private var rightEdgeDrag: some Gesture {
        DragGesture(minimumDistance: 1)
            .onChanged { v in
                if dragBaseW == nil { dragBaseW = viewModel.chatWindowW; isResizing = true }
                viewModel.chatWindowW = max(360, min(dragBaseW! + 2 * v.translation.width, 900))
            }
            .onEnded { _ in dragBaseW = nil; isResizing = false }
    }

    private var leftEdgeDrag: some Gesture {
        DragGesture(minimumDistance: 1)
            .onChanged { v in
                if dragBaseW == nil { dragBaseW = viewModel.chatWindowW; isResizing = true }
                viewModel.chatWindowW = max(360, min(dragBaseW! - 2 * v.translation.width, 900))
            }
            .onEnded { _ in dragBaseW = nil; isResizing = false }
    }

    private var bottomEdgeDrag: some Gesture {
        DragGesture(minimumDistance: 1)
            .onChanged { v in
                if dragBaseH == nil { dragBaseH = viewModel.chatWindowH; isResizing = true }
                viewModel.chatWindowH = max(320, min(dragBaseH! + v.translation.height, 900))
            }
            .onEnded { _ in dragBaseH = nil; isResizing = false }
    }

    // MARK: Header

    private var header: some View {
        HStack(spacing: 8) {
            Image(systemName: "sparkles")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(Color.accentColor)
            Text("bagent")
                .font(.system(size: 13, weight: .semibold))
            if !viewModel.pendingApprovals.isEmpty {
                HStack(spacing: 3) {
                    Image(systemName: "shield.lefthalf.filled")
                        .font(.system(size: 9))
                    Text("\(viewModel.pendingApprovals.count)")
                        .font(.system(size: 10, weight: .bold))
                }
                .foregroundStyle(.white)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Color.orange)
                .clipShape(Capsule())
                .transition(.opacity.combined(with: .scale(scale: 0.8)))
            }
            Spacer()
            Button { viewModel.clear() } label: {
                Image(systemName: "trash")
                    .font(.system(size: 14))
                    .foregroundStyle(.tertiary)
            }
            .buttonStyle(.plain)
            Button { onCollapse() } label: {
                Image(systemName: "xmark.circle.fill")
                    .font(.system(size: 16))
                    .foregroundStyle(.tertiary)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    // MARK: Messages

    private var messageList: some View {
        ScrollViewReader { proxy in
            GeometryReader { geo in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        if viewModel.messages.isEmpty {
                            SuggestionChips(viewModel: viewModel)
                                .padding(.top, 12)
                        }
                        ForEach(viewModel.messages) { msg in
                            let streaming = viewModel.streamingAssistantMessageId == msg.id
                            MessageBubble(message: msg, isStreaming: streaming, viewModel: viewModel)
                                .id(msg.id)
                        }
                        if viewModel.isThinking {
                            HStack(spacing: 8) {
                                ThinkingIndicator()
                                if let status = viewModel.toolStatus {
                                    Text(status)
                                        .font(.caption)
                                        .foregroundColor(.secondary)
                                }
                            }
                            .padding(.leading, 4)
                            .id("thinking")
                        }
                        // Bottom sentinel — used to detect when the user scrolled away.
                        Color.clear.frame(height: 1).id("_bottom_sentinel")
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    // Report the content's minY in the scroll view coordinate space.
                    .background(
                        GeometryReader { contentGeo in
                            Color.clear.preference(
                                key: ScrollOffsetKey.self,
                                value: contentGeo.frame(in: .named("scrollArea")).minY
                            )
                        }
                    )
                }
                .coordinateSpace(name: "scrollArea")
                .onPreferenceChange(ScrollOffsetKey.self) { minY in
                    let viewHeight = geo.size.height
                    // Estimate content height from minY + scrolled amount.
                    // If minY < -(40) the user has scrolled up at least ~40 pt.
                    let scrolledUp = minY < -40
                    if scrolledUp != userScrolledUp {
                        userScrolledUp = scrolledUp
                    }
                    contentOffsetY = minY
                    scrollViewHeight = viewHeight
                }
                // Restore scroll position on first appear (panel re-opened).
                .onAppear {
                    DispatchQueue.main.async {
                        if viewModel.savedScrollWasAtBottom {
                            scrollToLatest(proxy)
                        } else if let anchorId = viewModel.savedScrollAnchorId {
                            proxy.scrollTo(anchorId, anchor: .top)
                        }
                    }
                }
                .onChange(of: viewModel.messages.count) {
                    // New user message sent → always snap to bottom and reset flag.
                    if let last = viewModel.messages.last, last.role == .user {
                        userScrolledUp = false
                    }
                    if !userScrolledUp {
                        withAnimation(.easeOut(duration: 0.2)) {
                            scrollToLatest(proxy)
                        }
                    }
                }
                // Fires on every streaming token — keep pinned only when not scrolled up.
                .onChange(of: viewModel.streamingChunk) {
                    if !userScrolledUp {
                        scrollToLatest(proxy)
                    }
                }
            }
        }
    }

    private func scrollToLatest(_ proxy: ScrollViewProxy) {
        if viewModel.isThinking {
            proxy.scrollTo("thinking", anchor: .bottom)
        } else if let last = viewModel.messages.last {
            proxy.scrollTo(last.id, anchor: .bottom)
        }
    }

    // MARK: Input

    private var inputBar: some View {
        VStack(alignment: .leading, spacing: 6) {
            // Pending attachments chip row
            if !viewModel.pendingAttachments.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 6) {
                        ForEach(viewModel.pendingAttachments) { att in
                            AttachmentChip(attachment: att) {
                                viewModel.removeAttachment(id: att.id)
                            }
                        }
                    }
                    .padding(.horizontal, 12)
                    .padding(.top, 6)
                }
            }

            HStack(alignment: .center, spacing: 8) {
                // Attachments (+) with a mic button that reveals on hover / while recording.
                VoiceAttachControl(
                    viewModel: viewModel,
                    isUploading: viewModel.isUploadingAttachment,
                    attachDisabled: viewModel.isThinking || viewModel.pendingAttachments.count >= 5,
                    onPlus: { openFilePicker() }
                )

                TextField("Napíš správu…", text: $viewModel.inputText, axis: .vertical)
                    .lineLimit(1...4)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13))
                    .focused($inputFocused)
                    .onSubmit { viewModel.send() }
                    .padding(.vertical, 6)

                let canSend = (!viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    || !viewModel.pendingAttachments.isEmpty)
                    && !viewModel.isThinking
                Button { viewModel.send() } label: {
                    Image(systemName: canSend ? "arrow.up.circle.fill" : "arrow.up.circle")
                        .font(.system(size: 24))
                        .foregroundStyle(canSend ? Color.accentColor : Color.secondary)
                }
                .buttonStyle(.plain)
                .disabled(!canSend)
                .keyboardShortcut(.return, modifiers: .command)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }

    private func openFilePicker() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        panel.canChooseFiles = true
        panel.allowedContentTypes = [.image, .pdf, .plainText, .text, .sourceCode]
        panel.message = "Vyber súbory na priloženie"
        // Appear above status-bar-level chat window
        panel.level = NSWindow.Level(rawValue: NSWindow.Level.statusBar.rawValue + 1)
        panel.begin { response in
            guard response == .OK else { return }
            viewModel.addAttachments(urls: panel.urls)
        }
    }
}

// MARK: - Message bubble

private func copyToPasteboard(_ text: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
    NotificationCenter.default.post(name: .bagentCodeCopied, object: nil)
}

struct WhatsAppPairingView: View {
    @ObservedObject var viewModel: ChatViewModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var showDiagnostics = false

    private var status: DaemonClient.WhatsappStatusResult? {
        viewModel.whatsappStatus
    }

    private var statusText: String {
        switch status?.status ?? "starting" {
        case "starting": return "Starting local bridge"
        case "qr": return "Waiting for scan"
        case "authenticated": return "Scan accepted, loading WhatsApp Web"
        case "authenticated_waiting_for_ready": return "Authenticated, still waiting for WhatsApp Web"
        case "ready": return "Connected"
        case "disconnected": return "Disconnected"
        case "error": return "Connection error"
        case "missing_node": return "Node.js not found"
        case "bridge_not_installed": return "Bridge dependencies missing"
        default: return status?.status ?? "Starting"
        }
    }

    private var detailText: String {
        if let loading = status?.last_loading {
            let percent = loading.percent.map { "\(Int($0))%" } ?? "loading"
            if let message = loading.message, !message.isEmpty {
                return "\(percent) · \(message)"
            }
            return percent
        }
        if let state = status?.last_state, !state.isEmpty {
            return "WhatsApp state: \(state)"
        }
        if let error = status?.error, !error.isEmpty {
            return error
        }
        return "Open WhatsApp on your phone, choose Linked devices, then scan this code."
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            ScrollView {
                VStack(alignment: .center, spacing: 18) {
                    VStack(spacing: 6) {
                        Text("Scan QR code in WhatsApp")
                            .font(.system(size: 20, weight: .semibold))
                        Text("Linked devices → Link a device")
                            .font(.system(size: 12))
                            .foregroundStyle(.secondary)
                    }
                    .padding(.top, 18)
                    .accessibilityElement(children: .combine)

                    qrSurface

                    VStack(spacing: 6) {
                        HStack(spacing: 8) {
                            statusDot
                            Text(statusText)
                                .font(.system(size: 12, weight: .medium))
                        }
                        Text(detailText)
                            .font(.system(size: 11))
                            .foregroundStyle(.secondary)
                            .multilineTextAlignment(.center)
                            .lineLimit(3)
                            .frame(maxWidth: 300)
                    }
                    .accessibilityElement(children: .combine)

                    actions
                    diagnostics
                }
                .frame(maxWidth: .infinity)
                .padding(18)
            }
        }
        .background(.ultraThinMaterial)
        .task {
            viewModel.refreshWhatsappStatus()
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Button {
                withAnimation(reduceMotion ? .easeOut(duration: 0.12) : .spring(response: 0.24, dampingFraction: 0.8)) {
                    viewModel.showWhatsappPairing = false
                }
            } label: {
                Image(systemName: "chevron.left")
                    .font(.system(size: 12, weight: .semibold))
            }
            .buttonStyle(.plain)
            .help("Späť")

            Label("WhatsApp pairing", systemImage: "qrcode.viewfinder")
                .font(.system(size: 13, weight: .semibold))
            Spacer()
            Button {
                viewModel.disconnectWhatsapp()
            } label: {
                Image(systemName: "xmark.circle")
                    .font(.system(size: 13))
            }
            .buttonStyle(.plain)
            .help("Zastaviť párovanie")
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
    }

    @ViewBuilder
    private var qrSurface: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 10)
                .fill(Color(nsColor: .windowBackgroundColor).opacity(0.85))
                .overlay(
                    RoundedRectangle(cornerRadius: 10)
                        .stroke(Color.secondary.opacity(0.16), lineWidth: 1)
                )
                .frame(width: 236, height: 236)

            if let qrStr = viewModel.whatsappQrString, let img = QRImage.generate(from: qrStr) {
                Image(nsImage: img)
                    .resizable()
                    .interpolation(.none)
                    .frame(width: 204, height: 204)
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                    .transition(.opacity.combined(with: .scale(scale: 0.96)))
                    .accessibilityLabel("WhatsApp QR code")
            } else if status?.status == "authenticated" || status?.status == "authenticated_waiting_for_ready" {
                VStack(spacing: 10) {
                    ProgressView()
                        .scaleEffect(0.8)
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 26))
                        .foregroundStyle(Color.green)
                    Text("Scan accepted")
                        .font(.system(size: 12, weight: .medium))
                }
                .transition(.opacity)
            } else {
                VStack(spacing: 10) {
                    ProgressView()
                        .scaleEffect(0.8)
                    Text("Generating QR code")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                }
                .transition(.opacity)
            }
        }
        .animation(reduceMotion ? .easeOut(duration: 0.12) : .spring(response: 0.28, dampingFraction: 0.82), value: viewModel.whatsappQrString)
        .animation(reduceMotion ? .easeOut(duration: 0.12) : .easeInOut(duration: 0.18), value: status?.status)
    }

    private var actions: some View {
        HStack(spacing: 10) {
            Button {
                viewModel.refreshWhatsappQr()
            } label: {
                Label("Refresh QR", systemImage: "arrow.clockwise")
            }
            .buttonStyle(.bordered)
            .disabled(status?.needs_qr != true)

            Button {
                viewModel.disconnectWhatsapp()
            } label: {
                Label("Stop", systemImage: "stop.circle")
            }
            .buttonStyle(.bordered)
        }
        .font(.system(size: 12))
    }

    private var diagnostics: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                withAnimation(.easeInOut(duration: 0.16)) {
                    showDiagnostics.toggle()
                }
                if showDiagnostics {
                    Task { await viewModel.loadWhatsappDebug() }
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: showDiagnostics ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .semibold))
                        .frame(width: 12)
                    Text("Diagnostics")
                        .font(.system(size: 11, weight: .medium))
                    Spacer()
                    if viewModel.isLoadingWhatsappDebug {
                        ProgressView()
                            .scaleEffect(0.55)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if showDiagnostics {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        Button {
                            Task { await viewModel.loadWhatsappDebug() }
                        } label: {
                            Label("Reload", systemImage: "arrow.clockwise")
                        }
                        .buttonStyle(.plain)

                        Button {
                            copyToPasteboard(viewModel.whatsappDebugPayload ?? "")
                        } label: {
                            Label("Copy JSON", systemImage: "doc.on.doc")
                        }
                        .buttonStyle(.plain)
                        .disabled((viewModel.whatsappDebugPayload ?? "").isEmpty)
                    }
                    .font(.system(size: 10))
                    .foregroundStyle(Color.accentColor)

                    ScrollView {
                        Text(viewModel.whatsappDebugPayload ?? "No diagnostics loaded.")
                            .font(.system(size: 10, design: .monospaced))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(8)
                    }
                    .frame(maxHeight: 130)
                    .background(Color.black.opacity(0.08))
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .frame(maxWidth: 340)
        .padding(10)
        .background(Color.secondary.opacity(0.08))
        .clipShape(RoundedRectangle(cornerRadius: 8))
    }

    private var statusDot: some View {
        Circle()
            .fill(statusColor)
            .frame(width: 8, height: 8)
            .accessibilityHidden(true)
    }

    private var statusColor: Color {
        switch status?.status {
        case "ready": return .green
        case "error", "disconnected", "missing_node", "bridge_not_installed": return .red
        case "qr", "starting", "authenticated", "authenticated_waiting_for_ready": return .yellow
        default: return .gray
        }
    }
}

struct DebugPanelView: View {
    @ObservedObject var viewModel: ChatViewModel

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Label("Debug", systemImage: "ladybug")
                    .font(.system(size: 13, weight: .semibold))
                Spacer()
                if let id = viewModel.currentSessionId {
                    Button {
                        copyToPasteboard(id)
                    } label: {
                        Image(systemName: "doc.on.doc")
                            .font(.system(size: 12))
                    }
                    .buttonStyle(.plain)
                    .help("Kopírovať ID konverzácie")
                }
                Button {
                    if let payload = viewModel.debugConversationPayload {
                        copyToPasteboard(payload)
                    }
                } label: {
                    Image(systemName: "square.and.arrow.up")
                        .font(.system(size: 12))
                }
                .buttonStyle(.plain)
                .help("Kopírovať debug payload")
            }

            if let id = viewModel.currentSessionId {
                HStack(spacing: 6) {
                    Text("Conversation ID")
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(.secondary)
                    Text(id)
                        .font(.system(size: 10, design: .monospaced))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .textSelection(.enabled)
                }
            }

            if viewModel.isLoadingDebug {
                ProgressView()
                    .scaleEffect(0.8)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                ScrollView {
                    Text(viewModel.debugConversationPayload ?? "Žiadne debug dáta.")
                        .font(.system(size: 11, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .background(Color.black.opacity(0.08))
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                }
            }
        }
        .padding(12)
        .task { await viewModel.loadDebugConversation() }
    }
}

struct PromptTraceDisclosure: View {
    let message: ChatMessage
    @ObservedObject var viewModel: ChatViewModel
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Button {
                    expanded.toggle()
                    if expanded {
                        Task { await viewModel.loadDebugTrace(for: message.id) }
                    }
                } label: {
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                    Text(previewText)
                        .font(.system(size: 10))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                .buttonStyle(.plain)
                Spacer(minLength: 8)
                if let id = message.debugTraceId {
                    Button {
                        copyToPasteboard(id)
                    } label: {
                        Image(systemName: "number")
                            .font(.system(size: 10))
                    }
                    .buttonStyle(.plain)
                    .help("Kopírovať trace ID")
                }
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 5)
            .background(Color.gray.opacity(0.13))
            .clipShape(RoundedRectangle(cornerRadius: 7))

            if expanded {
                VStack(alignment: .leading, spacing: 6) {
                    HStack {
                        Text(message.debugTraceId ?? "")
                            .font(.system(size: 9, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer()
                        Button {
                            copyToPasteboard(message.debugPayload ?? "")
                        } label: {
                            Image(systemName: "doc.on.doc")
                                .font(.system(size: 11))
                        }
                        .buttonStyle(.plain)
                        .help("Kopírovať prompt/debug trace")
                    }
                    contextPlanChips
                    if let ids = message.debugSelectedMemoryIds, !ids.isEmpty {
                        Text("Pamäť: \(ids.count) záznamov")
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                    }
                    if message.debugConversationRecallInjected == true {
                        Label("Recall histórie konverzácie", systemImage: "clock.arrow.circlepath")
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                    }
                    ScrollView {
                        Text(message.debugPayload ?? "Načítavam trace…")
                            .font(.system(size: 10, design: .monospaced))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(8)
                    }
                    .frame(maxHeight: 220)
                    .background(Color.black.opacity(0.08))
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                }
                .padding(8)
                .background(Color.gray.opacity(0.10))
                .clipShape(RoundedRectangle(cornerRadius: 8))
            }
        }
    }

    private var previewText: String {
        let base = message.debugPreview?.isEmpty == false ? message.debugPreview! : "Prompt trace"
        var parts = [base]
        if let count = message.debugMessageCount { parts.append("\(count) msgs") }
        if let tokens = message.debugTokenEstimate { parts.append("~\(tokens) tok") }
        if let skills = message.debugSelectedSkills, !skills.isEmpty { parts.append("\(skills.count) skills") }
        if message.debugConversationRecallInjected == true { parts.append("recall") }
        return parts.joined(separator: " · ")
    }

    @ViewBuilder
    private var contextPlanChips: some View {
        if let skills = message.debugSelectedSkills, !skills.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 4) {
                    ForEach(skills, id: \.self) { name in
                        Text(name)
                            .font(.system(size: 9, weight: .medium))
                            .padding(.horizontal, 5)
                            .padding(.vertical, 2)
                            .background(Color.teal.opacity(0.15))
                            .foregroundStyle(Color.teal)
                            .clipShape(Capsule())
                    }
                }
                .padding(.horizontal, 2)
            }
        }
    }
}

struct MessageBubble: View {
    let message: ChatMessage
    let isStreaming: Bool
    @ObservedObject var viewModel: ChatViewModel

    var body: some View {
        HStack(alignment: .top) {
            if message.role == .user { Spacer(minLength: 40) }

            if message.role == .user {
                VStack(alignment: .trailing, spacing: 4) {
                    if !message.content.isEmpty {
                        Text(message.content)
                            .font(.system(size: 13))
                            .foregroundStyle(Color.white)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 7)
                            .background(Color.accentColor)
                            .clipShape(RoundedRectangle(cornerRadius: 12))
                    }
                    // Attachments shown below the text bubble, fixed-size so they stay right-aligned
                    if !message.attachments.isEmpty {
                        AttachmentStrip(attachments: message.attachments, trailingAligned: true)
                    }
                }
            } else {
                VStack(alignment: .leading, spacing: 4) {
                    if message.debugTraceId != nil {
                        PromptTraceDisclosure(message: message, viewModel: viewModel)
                    }
                    MessageContentView(text: message.content, isStreaming: isStreaming)
                        .padding(.horizontal, 10)
                        // Extra top padding when a button is present so text doesn't overlap it.
                        .padding(.top, ((message.mailRef != nil || message.odooRef != nil || message.whatsappRef != nil) && !isStreaming) ? 38 : 7)
                        .padding(.bottom, 7)
                        .background(Color(nsColor: .controlBackgroundColor))
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                        .overlay(alignment: .topTrailing) {
                            // Show only after streaming ends so no layout jump mid-response.
                            if !isStreaming {
                                HStack(spacing: 6) {
                                    if let ref = message.whatsappRef {
                                        WhatsAppOpenButton(ref: ref)
                                    }
                                    if let ref = message.odooRef {
                                        OdooOpenButton(ref: ref) { viewModel.openOdoo(ref) }
                                    }
                                    if let ref = message.mailRef {
                                        MailOpenButton(ref: ref) { viewModel.openMail(ref) }
                                    }
                                }
                                .padding(.top, 6)
                                .padding(.trailing, 8)
                                .transition(.opacity)
                            }
                        }
                    // Mail attachments shown below the assistant response (Phase 5C)
                    if !message.attachments.isEmpty {
                        AttachmentStrip(attachments: message.attachments)
                    }
                    // Codex task rating badge (Phase 8) — shown after streaming ends
                    if !isStreaming, let rating = message.taskRating {
                        CodexRatingBadge(rating: rating)
                    }
                }
            }

            if message.role == .assistant { Spacer(minLength: 40) }
        }
    }
}

// MARK: - Codex Task Rating Badge (Phase 8)

/// Small inline chip shown below an assistant message when the daemon emitted a `task_rating` SSE event.
/// Only shown for CodexCandidate+ levels; provides transparency about complexity classification.
struct CodexRatingBadge: View {
    let rating: (level: String, score: Int, reasons: [String], privacyRisk: String)

    @State private var expanded: Bool = false

    private var levelColor: Color {
        switch rating.level {
        case "LocalOnly", "LocalPreferred": return .secondary
        case "CodexCandidate": return .orange
        case "CodexRecommended": return Color.accentColor
        case "CodexRequired": return .red
        default: return .secondary
        }
    }

    private var levelLabel: String {
        switch rating.level {
        case "LocalOnly": return "Lokálna úloha"
        case "LocalPreferred": return "Lokálna (preferovaná)"
        case "CodexCandidate": return "Kandidát pre Codex"
        case "CodexRecommended": return "Odporúčaný Codex"
        case "CodexRequired": return "Vyžaduje Codex"
        default: return rating.level
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(.easeInOut(duration: 0.15)) { expanded.toggle() }
            } label: {
                HStack(spacing: 5) {
                    Image(systemName: "cpu")
                        .font(.system(size: 10))
                        .foregroundStyle(levelColor)
                    Text(levelLabel)
                        .font(.system(size: 10, weight: .medium))
                        .foregroundStyle(levelColor)
                    Text("·")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                    Text("skóre \(rating.score)")
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                    if !rating.privacyRisk.isEmpty && rating.privacyRisk != "Low" {
                        Text("·")
                            .font(.system(size: 10))
                            .foregroundStyle(.tertiary)
                        Image(systemName: "lock.fill")
                            .font(.system(size: 9))
                            .foregroundStyle(.orange)
                        Text(rating.privacyRisk)
                            .font(.system(size: 10))
                            .foregroundStyle(.orange)
                    }
                    Image(systemName: expanded ? "chevron.up" : "chevron.down")
                        .font(.system(size: 9))
                        .foregroundStyle(.tertiary)
                }
            }
            .buttonStyle(.plain)

            if expanded && !rating.reasons.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(rating.reasons, id: \.self) { reason in
                        HStack(alignment: .top, spacing: 4) {
                            Text("·")
                                .font(.system(size: 10))
                                .foregroundStyle(.tertiary)
                            Text(reason)
                                .font(.system(size: 10))
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .padding(.leading, 4)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(
            RoundedRectangle(cornerRadius: 6)
                .fill(levelColor.opacity(0.06))
                .overlay(
                    RoundedRectangle(cornerRadius: 6)
                        .stroke(levelColor.opacity(0.2), lineWidth: 0.5)
                )
        )
    }
}

// MARK: - Attachment strip (thumbnails + chips)

struct AttachmentStrip: View {
    let attachments: [ChatAttachment]
    var trailingAligned: Bool = false

    var body: some View {
        if trailingAligned {
            // Fixed-size so it stays right-aligned inside a trailing VStack
            chipRow.fixedSize()
        } else {
            ScrollView(.horizontal, showsIndicators: false) { chipRow }
        }
    }

    private var chipRow: some View {
        HStack(spacing: 6) {
            ForEach(attachments) { att in
                if att.kind == .image, let thumb = att.thumbnail {
                    Image(nsImage: thumb)
                        .resizable()
                        .scaledToFill()
                        .frame(width: 72, height: 72)
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                        .onTapGesture { NSWorkspace.shared.open(att.localURL) }
                } else {
                    Button {
                        NSWorkspace.shared.open(att.localURL)
                    } label: {
                        HStack(spacing: 4) {
                            Image(systemName: iconName(for: att.kind))
                                .font(.system(size: 11))
                                .foregroundStyle(.secondary)
                            Text(att.filename)
                                .font(.system(size: 11))
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Text(formatSize(att.sizeBytes))
                                .font(.system(size: 10))
                                .foregroundStyle(.tertiary)
                        }
                        .padding(.horizontal, 8)
                        .padding(.vertical, 5)
                        .background(Color(nsColor: .controlBackgroundColor))
                        .clipShape(Capsule())
                        .overlay(Capsule().stroke(Color.secondary.opacity(0.2), lineWidth: 0.5))
                    }
                    .buttonStyle(.plain)
                    .onHover { h in h ? NSCursor.pointingHand.push() : NSCursor.pop() }
                }
            }
        }
    }

    private func iconName(for kind: ChatAttachmentKind) -> String {
        switch kind {
        case .pdf:   return "doc.fill"
        case .text:  return "doc.text"
        case .image: return "photo"
        default:     return "paperclip"
        }
    }

    private func formatSize(_ bytes: Int) -> String {
        if bytes < 1024 { return "\(bytes) B" }
        if bytes < 1024 * 1024 { return "\(bytes / 1024) KB" }
        return String(format: "%.1f MB", Double(bytes) / (1024 * 1024))
    }
}

// MARK: - "Otvoriť mail" animated button (Phase 5E)

/// Attachments `+` button and a microphone button, side by side. `+` opens the
/// file picker; the mic toggles inline voice transcription into the text field.
struct VoiceAttachControl: View {
    @ObservedObject var viewModel: ChatViewModel
    var isUploading: Bool
    var attachDisabled: Bool
    var onPlus: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Button { onPlus() } label: {
                if isUploading {
                    ProgressView().scaleEffect(0.7).frame(width: 20, height: 20)
                } else {
                    Image(systemName: "plus.circle")
                        .font(.system(size: 20))
                        .foregroundStyle(Color.secondary)
                }
            }
            .buttonStyle(.plain)
            .disabled(attachDisabled)

            Button { viewModel.toggleInlineVoice() } label: {
                Image(systemName: viewModel.isVoiceRecording ? "waveform" : "mic")
                    .font(.system(size: 18))
                    .foregroundStyle(
                        !viewModel.voiceModeEnabled
                            ? Color.secondary.opacity(0.45)
                            : (viewModel.isVoiceRecording ? Color.accentColor : Color.secondary)
                    )
                    // `.repeating` is the macOS 14 equivalent of `.repeat(.continuous)`.
                    .symbolEffect(.pulse.byLayer, options: .repeating,
                                  isActive: viewModel.isVoiceRecording)
            }
            .buttonStyle(.plain)
            .disabled(!viewModel.voiceModeEnabled)
            .accessibilityLabel(viewModel.voiceModeEnabled ? "Hlasový vstup" : "Hlasový vstup je vypnutý")
        }
    }
}

/// Circle-to-pill hover-morph button that opens the found email in Mail.app.
/// Collapsed: 28 pt envelope-filled circle.
/// Hovered:   expands to a ~150 pt rounded rect; icon slides left; text fades in.
struct MailOpenButton: View {
    let ref: DaemonClient.MailRef
    let onOpen: () -> Void

    @State private var isHovered = false

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: 6) {
                if isHovered {
                    Text("Otvoriť mail")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                        .fixedSize()
                        .transition(
                            .asymmetric(
                                insertion: .opacity.combined(with: .move(edge: .trailing)),
                                removal: .opacity.combined(with: .move(edge: .trailing))
                            )
                        )
                }
                Image(systemName: "envelope.fill")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(.white)
            }
            .padding(.horizontal, 9)
            .frame(height: 28)
            .frame(minWidth: 28)
            .background(Capsule().fill(Color.accentColor))
        }
        .buttonStyle(.plain)
        .animation(.spring(response: 0.28, dampingFraction: 0.68), value: isHovered)
        .onHover { h in
            withAnimation(.spring(response: 0.28, dampingFraction: 0.68)) {
                isHovered = h
            }
            h ? NSCursor.pointingHand.push() : NSCursor.pop()
        }
    }
}

// MARK: - Odoo open button (Phase 6)

/// Capsule button displayed above assistant messages that found an Odoo record.
/// Clicking opens the record in Safari via the daemon's `/odoo/open` route.
struct OdooOpenButton: View {
    let ref: DaemonClient.OdooRef
    let onOpen: () -> Void

    @State private var isHovered = false

    var body: some View {
        Button(action: onOpen) {
            HStack(spacing: 6) {
                if isHovered {
                    Text("Otvoriť v Safari")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.white)
                        .lineLimit(1)
                        .fixedSize()
                        .transition(
                            .asymmetric(
                                insertion: .opacity.combined(with: .move(edge: .trailing)),
                                removal: .opacity.combined(with: .move(edge: .trailing))
                            )
                        )
                }
                Image(systemName: "globe")
                    .font(.system(size: 13, weight: .medium))
                    .foregroundStyle(.white)
            }
            .padding(.horizontal, 9)
            .frame(height: 28)
            .frame(minWidth: 28)
            .background(Capsule().fill(Color.orange))
        }
        .buttonStyle(.plain)
        .animation(.spring(response: 0.28, dampingFraction: 0.68), value: isHovered)
        .onHover { h in
            withAnimation(.spring(response: 0.28, dampingFraction: 0.68)) {
                isHovered = h
            }
            h ? NSCursor.pointingHand.push() : NSCursor.pop()
        }
    }
}

// MARK: - WhatsApp chat chip (Phase 11)

/// Minimal chip shown above assistant messages that found a WhatsApp chat.
struct WhatsAppOpenButton: View {
    let ref: DaemonClient.WhatsappRef

    @State private var isHovered = false

    var body: some View {
        HStack(spacing: 5) {
            if isHovered {
                Text(ref.contact_name ?? "WhatsApp")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.white)
                    .lineLimit(1)
                    .fixedSize()
                    .transition(
                        .asymmetric(
                            insertion: .opacity.combined(with: .move(edge: .trailing)),
                            removal: .opacity.combined(with: .move(edge: .trailing))
                        )
                    )
            }
            Image(systemName: "bubble.left.and.bubble.right.fill")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(.white)
        }
        .padding(.horizontal, 9)
        .frame(height: 28)
        .frame(minWidth: 28)
        .background(Capsule().fill(Color.green))
        .animation(.spring(response: 0.28, dampingFraction: 0.68), value: isHovered)
        .onHover { h in
            withAnimation(.spring(response: 0.28, dampingFraction: 0.68)) {
                isHovered = h
            }
            h ? NSCursor.pointingHand.push() : NSCursor.pop()
        }
    }
}

// MARK: - Pending attachment chip (input bar)

struct AttachmentChip: View {
    let attachment: ChatAttachment
    let onRemove: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            if attachment.kind == .image, let thumb = attachment.thumbnail {
                Image(nsImage: thumb)
                    .resizable()
                    .scaledToFill()
                    .frame(width: 18, height: 18)
                    .clipShape(RoundedRectangle(cornerRadius: 3))
            } else {
                Image(systemName: chipIcon)
                    .font(.system(size: 10))
                    .foregroundStyle(.secondary)
            }
            Text(attachment.filename)
                .font(.system(size: 11))
                .lineLimit(1)
                .truncationMode(.middle)
                .frame(maxWidth: 100)
            Button {
                onRemove()
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 7)
        .padding(.vertical, 4)
        .background(Color(nsColor: .controlBackgroundColor))
        .clipShape(Capsule())
        .overlay(Capsule().stroke(Color.secondary.opacity(0.25), lineWidth: 0.5))
    }

    private var chipIcon: String {
        switch attachment.kind {
        case .pdf:   return "doc.fill"
        case .text:  return "doc.text"
        case .image: return "photo"
        default:     return "paperclip"
        }
    }
}

// MARK: - Thinking indicator

struct ThinkingIndicator: View {
    @State private var animating = false

    var body: some View {
        HStack(spacing: 4) {
            ForEach(0..<3, id: \.self) { i in
                Circle()
                    .fill(Color.secondary)
                    .frame(width: 7, height: 7)
                    .scaleEffect(animating ? 1.0 : 0.5)
                    .opacity(animating ? 1.0 : 0.4)
                    .animation(
                        .easeInOut(duration: 0.55)
                        .repeatForever(autoreverses: true)
                        .delay(Double(i) * 0.18),
                        value: animating
                    )
            }
        }
        .onAppear { animating = true }
    }
}

// MARK: - Suggestion chips

struct SuggestionChips: View {
    @ObservedObject var viewModel: ChatViewModel

    private let suggestions: [(String, String)] = [
        ("envelope.badge", "Zhrň neprečítané správy"),
        ("square.and.pencil", "Navrhni odpoveď po slovensky"),
        ("info.circle",  "Čo vieš urobiť?"),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(suggestions, id: \.1) { icon, text in
                Button {
                    viewModel.inputText = text
                    viewModel.send()
                } label: {
                    Label(text, systemImage: icon)
                        .font(.system(size: 12))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(Color(nsColor: .controlBackgroundColor))
                        .clipShape(Capsule())
                }
                .buttonStyle(.plain)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

// MARK: - Approval modal overlay

struct ApprovalModalOverlay: View {
    let approval: ApprovalItem
    @ObservedObject var viewModel: ChatViewModel
    @State private var secondsLeft: Int = 60

    private let timer = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        ZStack {
            Color.black.opacity(0.45)
                .ignoresSafeArea()
            VStack(spacing: 14) {
                HStack(spacing: 8) {
                    Image(systemName: "shield.lefthalf.filled")
                        .font(.system(size: 18, weight: .semibold))
                        .foregroundStyle(Color.orange)
                    Text("Schválenie akcie")
                        .font(.system(size: 14, weight: .semibold))
                }
                VStack(alignment: .leading, spacing: 6) {
                    Label(approval.toolName, systemImage: "wrench.and.screwdriver")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(.secondary)
                    if let desc = approval.description {
                        Text(desc)
                            .font(.system(size: 13))
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(10)
                .background(Color(nsColor: .controlBackgroundColor))
                .clipShape(RoundedRectangle(cornerRadius: 8))

                Text("Automatické zamietnutie za \(secondsLeft) s")
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)

                HStack(spacing: 10) {
                    Button {
                        viewModel.decideApproval(approval, allow: false)
                    } label: {
                        Text("Zamietnuť")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.bordered)
                    .keyboardShortcut(.escape, modifiers: [])

                    Button {
                        viewModel.decideApproval(approval, allow: true)
                    } label: {
                        Text("Schváliť")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.borderedProminent)
                    .tint(Color.green)
                    .keyboardShortcut(.return, modifiers: [])
                }
            }
            .padding(20)
            .frame(width: 300)
            .background(.regularMaterial)
            .clipShape(RoundedRectangle(cornerRadius: 14))
            .shadow(radius: 16)
        }
        .onReceive(timer) { _ in
            if secondsLeft > 0 {
                secondsLeft -= 1
            } else {
                viewModel.decideApproval(approval, allow: false)
            }
        }
    }
}
