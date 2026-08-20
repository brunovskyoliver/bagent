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
    static let cmuxWingWidth: CGFloat     = 84   // cmux banner — fits one-line event title
    static let cmuxBridgeHeight: CGFloat  = 19   // single caption line, minimal growth
    static let settingsWingWidth: CGFloat   = 205  // /settings surface
    static let settingsBridgeHeight: CGFloat = 252
    /// Automation Session split view: 190 pt master + divider/gutters inside
    /// the 252 pt bridge. The accepted status pill remains at its Stage 5
    /// origin because the outer panel geometry is unchanged.
    static let automationsWingWidth: CGFloat = 248
    static let automationsBridgeHeight: CGFloat = 252
    static let slashSuggestionRowHeight: CGFloat = 24  // one command suggestion row
    static let wheelWingWidth: CGFloat    = 196  // paste wheel — 5 chips along the arc
    static let wheelBridgeHeight: CGFloat = 36   // thin strip; the dome carries the height
    static let wheelBulgeDepth: CGFloat   = 96   // circular-arc dome below the strip
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

enum NotchStatusDotGeometry {
    static func outputTopRight(
        notchOffset: CGFloat,
        notchWidth: CGFloat,
        targetWingWidth: CGFloat,
        notchHeight: CGFloat
    ) -> CGPoint {
        let visibleWidth = notchWidth + 2 * targetWingWidth
        let contentWidth = visibleWidth * NotchWrapMetrics.inlineContentScale
        return CGPoint(
            x: notchOffset + notchWidth / 2 + contentWidth / 2 - 10,
            y: notchHeight + 12
        )
    }
}

enum NotchActivityLayout {
    static let headerHeight: CGFloat = 18
    static let rowHeight: CGFloat = 22
    static let maxRowsHeight: CGFloat = 84
    static let maxVisibleRows = Int(maxRowsHeight / rowHeight)

    static func extraHeight(activityCount: Int, expanded: Bool) -> CGFloat {
        guard activityCount > 0 else { return 0 }
        let rows = expanded
            ? min(maxRowsHeight, CGFloat(activityCount) * rowHeight)
            : 0
        return headerHeight + rows
    }
}

private enum NotchOutputLayout {
    static let lineSpacing: CGFloat = 1.5
    static let bottomSlack: CGFloat = 12
    static let chromePadding: CGFloat = 26
    static let contentVerticalInset: CGFloat = 14
    static let minTextWidth: CGFloat = 120
    static let resizeThreshold: CGFloat = 14
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
    static func wingWidth(text: String, notchWidth: CGFloat) -> CGFloat {
        let contentWidth = longestLineWidth(text) + 16
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
        visibleWidth: CGFloat
    ) -> CGFloat {
        let textWidth = max(minTextWidth, visibleWidth)
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
    var bulgeDepth: CGFloat = 0
    var bulgeSweep: CGFloat = 0

    var animatableData: AnimatablePair<CGFloat, AnimatablePair<CGFloat, AnimatablePair<CGFloat, CGFloat>>> {
        get {
            AnimatablePair(wingWidth,
                           AnimatablePair(bridgeHeight, AnimatablePair(bulgeDepth, bulgeSweep)))
        }
        set {
            wingWidth = newValue.first
            bridgeHeight = newValue.second.first
            bulgeDepth = newValue.second.second.first
            bulgeSweep = newValue.second.second.second
        }
    }

    func path(in rect: CGRect) -> Path {
        let r = cornerRadius
        let br = max(0, bridgeHeight)
        let x = notchOffset - wingWidth
        let w = 2 * wingWidth + notchWidth
        let h = notchHeight + br
        let depth = max(0, bulgeDepth)
        let sweep = min(1, max(0, bulgeSweep))

        var path = Path()
        path.move(to: CGPoint(x: x + w, y: 0))
        if depth > 0.5, sweep > 0.01 {
            path.addLine(to: CGPoint(x: x + w, y: h))
            let frontier = NotchWrapShape.domePoint(t: sweep, x: x, w: w, h: h, depth: depth)
            path.addLine(to: CGPoint(x: frontier.x, y: h))
            path.addLine(to: frontier)
            let steps = max(2, Int(48 * sweep))
            for i in stride(from: steps - 1, through: 0, by: -1) {
                let t = sweep * CGFloat(i) / CGFloat(steps)
                path.addLine(to: NotchWrapShape.domePoint(t: t, x: x, w: w, h: h, depth: depth))
            }
            path.addLine(to: CGPoint(x: x, y: 0))
            return path
        }
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
    var acceptanceReduceMotionOverride: Bool? = nil
    var acceptanceSettingsState: CompassRailAcceptanceState? = nil

    // Explicit @State so withAnimation directly tweens the shape's animatableData.
    @State private var wingWidth: CGFloat    = NotchWrapMetrics.idleWingWidth
    @State private var bridgeHeight: CGFloat = NotchWrapMetrics.idleBridgeHeight
    @State private var bulgeDepth: CGFloat   = 0
    @State private var bulgeSweep: CGFloat   = 0
    @State private var wheelContentOpacity: CGFloat = 0
    @State private var targetWingWidth: CGFloat = NotchWrapMetrics.idleWingWidth
    @State private var targetBridgeHeight: CGFloat = NotchWrapMetrics.idleBridgeHeight
    @State private var isHovered = false
    @State private var pulsing = false
    @State private var copyFlashed = false
    @State private var isDragTargeted = false
    @State private var inlineContentOpacity: CGFloat = 0
    @State private var cmuxContentOpacity: CGFloat = 0
    /// When the pointer entered a cmux hover-reveal — gates the ≥0.4s dwell before
    /// a hover-leave counts as "seen".
    @State private var cmuxRevealStartedAt: Date?
    @State private var hoverIconRevealID = UUID()
    @State private var inlineRevealID = UUID()
    @State private var borderPulseOpacity: CGFloat = 0.35
    @State private var returningStatusDotFromOutput = false
    /// Content-fit output wing — grow-only per assistant message so the panel
    /// never pumps narrower mid-stream; reset when a new message starts.
    @State private var outputWingRatchet: CGFloat = NotchWrapMetrics.outputMinWingWidth
    @State private var outputStatusReturnStartPos: CGPoint = .zero
    @State private var outputStatusReturnStartBridgeHeight: CGFloat = NotchWrapMetrics.outputMinBridgeHeight
    /// Multiple left-wing icons rest as an overlapped deck; hover fans them out.
    @State private var leftWingFanned = false
    @State private var leftWingRestackWork: DispatchWorkItem?
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion

    private var reduceMotion: Bool {
        acceptanceReduceMotionOverride ?? systemReduceMotion
    }

    // Panel is sized for voice mode (the widest/tallest state) so the frame never needs
    // AppKit resizing. notchOffset = voiceWingWidth = left edge of the physical notch.
    private let notchOffset = NotchWrapMetrics.maxWingWidth

    private var surfaceAnimation: Animation {
        reduceMotion
            ? .linear(duration: 0)
            : .easeInOut(duration: NotchWrapMetrics.surfaceDuration)
    }
    private var status: AgentStatus { viewModel.agentStatus }
    private var isInlineActive: Bool {
        viewModel.notchInteractionMode == .input || viewModel.notchInteractionMode == .output
            || viewModel.notchInteractionMode == .settings
            || viewModel.notchInteractionMode == .automations
            || isProjectionSurfaceActive
    }
    private var isProjectionSurfaceActive: Bool {
        (viewModel.notchPresentation.geometry.bridgeHeight > 0
            || viewModel.notchPresentation.statusPill.label != nil)
            && (viewModel.notchInteractionMode == .collapsed
                || viewModel.notchInteractionMode == .thinking)
    }
    private var isSettingsActive: Bool { viewModel.notchInteractionMode == .settings }

    /// Transient cmux banner (5s after an agent event, latest wins).
    private var isCmuxBannerActive: Bool {
        viewModel.cmuxBanner != nil && !isInlineActive
    }
    /// Hovering the collapsed notch re-reveals pending cmux events.
    private var isCmuxHoverReveal: Bool {
        viewModel.cmuxBanner == nil && !viewModel.cmuxPending.isEmpty
            && isHovered && !isInlineActive
    }
    private var isCmuxSurfaceActive: Bool { isCmuxBannerActive || isCmuxHoverReveal }
    private var displayedCmuxNotification: CmuxNotification? {
        viewModel.cmuxBanner ?? viewModel.cmuxPending.first
    }

    private var maxSize: CGSize {
        CGSize(
            width: 2 * NotchWrapMetrics.maxWingWidth + notchWidth,
            height: notchHeight + NotchWrapMetrics.maxBridgeHeight
        )
    }

    private func inlineWingWidth(for mode: NotchInteractionMode) -> CGFloat {
        if isProjectionSurfaceActive { return viewModel.notchPresentation.geometry.wingWidth }
        let base: CGFloat
        switch mode {
        case .output:      base = outputWingWidth()
        case .settings:    base = NotchWrapMetrics.settingsWingWidth
        case .automations: base = NotchWrapMetrics.automationsWingWidth
        default:           base = NotchWrapMetrics.inlineWingWidth
        }
        if mode == .settings { return base }
        return viewModel.notchPresentation.statusPill.label == nil
            ? base
            : max(base, viewModel.notchPresentation.geometry.wingWidth)
    }

    private func inlineBridgeHeight(for mode: NotchInteractionMode) -> CGFloat {
        if isProjectionSurfaceActive { return viewModel.notchPresentation.geometry.bridgeHeight }
        if mode == .output, viewModel.notchPresentation.focusedWorkIdentity != nil {
            return viewModel.notchPresentation.geometry.bridgeHeight
        }
        switch mode {
        case .output:   return outputBridgeHeight()
        case .settings: return NotchWrapMetrics.settingsBridgeHeight
        case .automations:
            return NotchWrapMetrics.automationsBridgeHeight
        default:
            // Slash-command suggestion rows sit under the input field.
            let suggestionsExtra = CGFloat(viewModel.slashSuggestions.count)
                * NotchWrapMetrics.slashSuggestionRowHeight
            let commandErrorExtra = viewModel.slashCommandError == nil
                ? 0 : NotchWrapMetrics.slashSuggestionRowHeight
            return min(
                NotchWrapMetrics.maxBridgeHeight,
                NotchWrapMetrics.inlineBridgeHeight + suggestionsExtra + commandErrorExtra
            )
        }
    }

    private var outputResponseText: String {
        NotchOutputLayout.responseText(
            latestText: viewModel.latestAssistantText,
            isStreaming: viewModel.isLatestAssistantStreaming,
            isThinking: viewModel.isThinking
        )
    }

    private func measuredOutputWingWidth() -> CGFloat {
        NotchOutputLayout.wingWidth(
            text: outputResponseText,
            notchWidth: notchWidth
        )
    }

    private func outputWingWidth() -> CGFloat {
        // Layout evaluation must be pure. Advancing the grow-only ratchet from
        // this getter publishes @State while SwiftUI is rendering, recursively
        // invalidating NSHostingView and eventually poisoning NSEventThread.
        max(outputWingRatchet, measuredOutputWingWidth())
    }

    private func advanceOutputWingRatchet() {
        let measured = measuredOutputWingWidth()
        guard measured > outputWingRatchet else { return }
        outputWingRatchet = measured
    }

    private func outputBridgeHeight() -> CGFloat {
        // Streaming text only grows, so a bridge already pinned at max stays
        // there — skip the full-text height measure. Post-stream calls still
        // measure exactly (final fit may shrink).
        if viewModel.isLatestAssistantStreaming,
           targetBridgeHeight >= NotchWrapMetrics.outputMaxBridgeHeight {
            return NotchWrapMetrics.outputMaxBridgeHeight
        }
        // Height must be measured at the same width the text will render at,
        // so wrap width and measured height always agree.
        let visibleWidth = (notchWidth + 2 * outputWingWidth()) * NotchWrapMetrics.inlineContentScale
        let base = NotchOutputLayout.bridgeHeight(
            text: outputResponseText,
            visibleWidth: visibleWidth
        )
        // Browse header row (‹n/N› + prompt) sits above the response text.
        let browseExtra: CGFloat = viewModel.historyBrowseIndex == nil ? 0 : 20
        // Tool-status chip also sits above the response text during streaming.
        let toolChipExtra: CGFloat = viewModel.toolStatus == nil ? 0 : 20
        let activityExtra = NotchActivityLayout.extraHeight(
            activityCount: viewModel.latestTranscriptActivityCount,
            expanded: viewModel.isActivityTranscriptExpanded
        )
        return min(
            NotchWrapMetrics.outputMaxBridgeHeight,
            base + browseExtra + toolChipExtra + activityExtra
        )
    }

    private func estimatedNotchTextHeight(_ text: String, width: CGFloat) -> CGFloat {
        NotchOutputLayout.textHeight(text, width: width)
    }

    private func refreshOutputSurfaceIfNeeded(force: Bool = false) {
        guard viewModel.notchInteractionMode == .output else { return }
        // Callers cross the main-queue boundary before reaching this method, so
        // ratchet state changes happen outside SwiftUI's active update pass.
        advanceOutputWingRatchet()
        // Streaming text only grows — once the panel is pinned at max in both
        // dimensions nothing a new token adds can change the surface, so skip
        // the (full-text parse + layout) measurement entirely. Without this,
        // every token past ~max-height re-measured and re-refreshed the panel.
        if !force, viewModel.isLatestAssistantStreaming,
           targetWingWidth >= NotchWrapMetrics.outputWingWidth,
           targetBridgeHeight >= NotchWrapMetrics.outputMaxBridgeHeight {
            return
        }
        let nextWing = outputWingWidth()
        let nextBridge = outputBridgeHeight()
        // The "== max" arms keep the panel re-fitting on every flush once one
        // dimension is pinned, so the growing bridge always leads the text —
        // suppressing them made the text overflow between threshold steps and
        // visibly bounce between bottom-pin and top-anchor.
        let shouldResize = force
            || !viewModel.isLatestAssistantStreaming
            || abs(nextBridge - targetBridgeHeight) >= NotchOutputLayout.resizeThreshold
            || nextBridge == NotchWrapMetrics.outputMaxBridgeHeight
            || abs(nextWing - targetWingWidth) >= NotchOutputLayout.resizeThreshold
            || nextWing == NotchWrapMetrics.outputWingWidth
        guard shouldResize else { return }
        refreshSurface()
    }

    private func deferOutputSurfaceRefresh(force: Bool = false) {
        DispatchQueue.main.async {
            refreshOutputSurfaceIfNeeded(force: force)
        }
    }

    private func refreshSurface() {
        let hoverExpanded = isHovered || isDragTargeted || viewModel.pillHovered
        var targetWing: CGFloat
        let targetBridge: CGFloat
        if viewModel.pasteWheelActive {
            targetWing = NotchWrapMetrics.wheelWingWidth
            targetBridge = NotchWrapMetrics.wheelBridgeHeight
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
        // The pill widens so the pending left-wing icon row always fits inside
        // the black shape.
        targetWing = max(targetWing, requiredLeftWingWidth)

        let surfaceTargetChanged = targetWingWidth != targetWing || targetBridgeHeight != targetBridge
        // Width/height growth while already in output mode must not replay the
        // inline reveal — only the initial transition into output resets it.
        let outputContentOnlyResize = viewModel.notchInteractionMode == .output
            && inlineContentOpacity > 0
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

        // Wheel morphs use a snappier spring than the ambient surface ease —
        // the hold gesture already cost 0.5s, the reveal must feel instant.
        let targetBulge: CGFloat = viewModel.pasteWheelActive
            ? NotchWrapMetrics.wheelBulgeDepth : 0
        let wheelInvolved = targetBulge > 0 || bulgeDepth > 0
        let animation: Animation = wheelInvolved && !reduceMotion
            ? .spring(response: 0.34, dampingFraction: 0.82)
            : surfaceAnimation
        withAnimation(animation) {
            wingWidth = targetWing
            bridgeHeight = targetBridge
        }
        // The dome doesn't grow downward — it draws itself across, left→right.
        // Depth is set instantly (invisible at sweep 0); the sweep does the reveal.
        if targetBulge > 0 {
            var instant = Transaction()
            instant.disablesAnimations = true
            withTransaction(instant) { bulgeDepth = targetBulge }
            if bulgeSweep < 1 {
                withAnimation(reduceMotion
                    ? .easeOut(duration: 0.12)
                    : .easeInOut(duration: 0.50)) {
                    bulgeSweep = 1
                }
            }
        } else if bulgeSweep > 0 || bulgeDepth > 0 {
            // Retract right→left, then flatten what's left.
            withAnimation(reduceMotion ? .easeOut(duration: 0.10) : .easeInOut(duration: 0.30)) {
                bulgeSweep = 0
            }
            withAnimation(.easeOut(duration: 0.16).delay(reduceMotion ? 0 : 0.24)) {
                bulgeDepth = 0
            }
        }
        updateInlineOpacity(active: isInlineActive)
        updateCmuxOpacity(active: isCmuxSurfaceActive)
    }

    private func updateWheelOpacity(active: Bool) {
        if active {
            let delay = reduceMotion ? 0.0 : 0.14
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                guard viewModel.pasteWheelActive else { return }
                withAnimation(.easeOut(duration: reduceMotion ? 0.10 : 0.20)) {
                    wheelContentOpacity = 1
                }
            }
        } else {
            withAnimation(.easeOut(duration: 0.12)) {
                wheelContentOpacity = 0
            }
        }
    }

    private func updateStatusDotTravelState(
        previousMode: NotchInteractionMode,
        currentMode: NotchInteractionMode
    ) {
        if previousMode == .output && currentMode != .output {
            outputStatusReturnStartPos = outputStatusTargetPos
            outputStatusReturnStartBridgeHeight = max(1, targetBridgeHeight)
            returningStatusDotFromOutput = !reduceMotion
        } else if currentMode == .output {
            returningStatusDotFromOutput = false
        } else if returningStatusDotFromOutput && bridgeHeight <= NotchWrapMetrics.idleBridgeHeight + 0.5 {
            returningStatusDotFromOutput = false
        }
    }

    private func updateInlineOpacity(active: Bool) {
        if active {
            let delay = reduceMotion ? 0.0 : NotchWrapMetrics.surfaceDuration * 0.62
            DispatchQueue.main.asyncAfter(deadline: .now() + delay) {
                guard isInlineActive else { return }
                withAnimation(.easeOut(duration: reduceMotion ? 0.12 : 0.24)) {
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
        NotchStatusDotGeometry.outputTopRight(
            notchOffset: notchOffset,
            notchWidth: notchWidth,
            targetWingWidth: targetWingWidth,
            notchHeight: notchHeight
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
        if returningStatusDotFromOutput {
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
            cornerRadius: NotchWrapMetrics.cornerRadius,
            bulgeDepth: bulgeDepth,
            bulgeSweep: bulgeSweep
        )
    }

    @ViewBuilder
    private var inlineSurfaceLayer: some View {
        if isInlineActive {
            let contentWing = wingWidth
            let contentBridge = bridgeHeight
            ZStack(alignment: .topLeading) {
                InlineNotchContent(
                    viewModel: viewModel,
                    outputGrowthPhase: viewModel.isLatestAssistantStreaming
                        && targetBridgeHeight < NotchWrapMetrics.outputMaxBridgeHeight,
                    acceptanceSettingsState: acceptanceSettingsState,
                    acceptanceReduceMotionOverride: acceptanceReduceMotionOverride
                )
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
        if viewModel.notchPresentation.statusPill.label == nil,
           status != .ready || (viewModel.isExpanded && !isInlineActive) {
            StatusDotView(status: status, pulsing: $pulsing, reduceMotion: reduceMotion, copyFlashed: copyFlashed, isDragTargeted: isDragTargeted)
                .position(statusDotPos)
                .frame(width: maxSize.width, height: maxSize.height, alignment: .topLeading)
                .clipShape(animatedNotchClipShape)
        }
    }

    @ViewBuilder
    private var invariantStatusPillLayer: some View {
        if viewModel.notchPresentation.statusPill.label != nil {
            InvariantNotchStatusPill(
                presentation: viewModel.notchPresentation.statusPill,
                activeAutomationCount: viewModel.notchPresentation.activeAutomationCount,
                action: viewModel.openActiveAutomations
            )
            .position(
                x: (isSettingsActive
                    ? NotchPillLayout.settingsOrigin(maxPanelWidth: maxSize.width)
                    : NotchPillLayout.origin(maxPanelWidth: maxSize.width)).x
                    + NotchPillLayout.size.width / 2,
                y: (isSettingsActive
                    ? NotchPillLayout.settingsOrigin(maxPanelWidth: maxSize.width)
                    : NotchPillLayout.origin(maxPanelWidth: maxSize.width)).y
                    + NotchPillLayout.size.height / 2
            )
            .frame(width: maxSize.width, height: maxSize.height, alignment: .topLeading)
            .opacity(inlineContentOpacity)
            .clipShape(animatedNotchClipShape)
        }
    }

    /// Pending cmux events — animated dot on the right wing, own agent status wins.
    @ViewBuilder
    private var cmuxDotLayer: some View {
        if let kind = viewModel.cmuxDotKind,
           !isInlineActive, !viewModel.isExpanded, status == .ready {
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
            HStack(spacing: leftWingStacked
                ? -(Self.leftWingIconSize - Self.leftWingStackedStep)
                : Self.leftWingIconSpacing) {
                ForEach(Array(visibleConnectorActions.enumerated()), id: \.element.id) { index, action in
                    Button {
                        viewModel.performConnectorAction(action, slotIndex: index)
                    } label: {
                        ConnectorIconView(kind: action.kind, reduceMotion: reduceMotion,
                                          size: Self.leftWingIconSize)
                    }
                    .buttonStyle(.plain)
                    .rotationEffect(
                        .degrees(leftWingStacked
                            ? Double(visibleConnectorActions.count - index) * -4
                            : 0),
                        anchor: .bottom
                    )
                    .help(action.kind.accessibilityLabel)
                    .accessibilityLabel(action.kind.accessibilityLabel)
                }
                if showsCmuxLeftIcon, let notification = displayedCmuxNotification {
                    CmuxIconView(kind: notification.kind, reduceMotion: reduceMotion)
                        .opacity(isCmuxSurfaceActive ? cmuxContentOpacity : 1)
                }
                // Transient "trace copied" confirmation (option+click).
                if viewModel.traceCopiedFlash {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: Self.leftWingIconSize * 0.8, weight: .semibold))
                        .symbolRenderingMode(.palette)
                        .foregroundStyle(.white, .green)
                        .frame(width: Self.leftWingIconSize, height: Self.leftWingIconSize)
                        .transition(.scale(scale: 0.4).combined(with: .opacity))
                        .accessibilityLabel("Trace skopírovaný do schránky")
                }
            }
            .animation(reduceMotion ? nil : .spring(response: 0.28, dampingFraction: 0.68),
                       value: leftWingStacked)
            .padding(6)
            .contentShape(Rectangle())
            .onHover { hovering in setLeftWingFanned(hovering) }
            .position(x: notchOffset - Self.leftWingRowInset - leftWingRowWidth / 2, y: iconY)
        }
    }

    /// Fan on hover-enter immediately; restack after a short debounce so the
    /// wing resize can't oscillate under the cursor.
    private func setLeftWingFanned(_ fanned: Bool) {
        leftWingRestackWork?.cancel()
        leftWingRestackWork = nil
        if fanned {
            guard !leftWingFanned else { return }
            leftWingFanned = true
            refreshSurface()
        } else {
            guard leftWingFanned else { return }
            let work = DispatchWorkItem {
                leftWingFanned = false
                refreshSurface()
            }
            leftWingRestackWork = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.15, execute: work)
        }
    }

    private static let leftWingIconSize: CGFloat = 18
    private static let leftWingIconSpacing: CGFloat = 6
    /// Visible sliver of each icon beneath the next one in the stacked deck.
    private static let leftWingStackedStep: CGFloat = 5
    /// Gap between the row's right edge and the notch's left edge.
    private static let leftWingRowInset: CGFloat = 8

    /// Deck rests stacked whenever more than one icon is present and the
    /// pointer isn't over the row. A single icon renders exactly as before.
    private var leftWingStacked: Bool {
        !leftWingFanned && leftWingIconCount > 1
    }

    /// Connector icons stay through inline
    /// input/output and the expanded chat panel so results are actionable
    /// the moment they stream in.
    private var visibleConnectorActions: [ConnectorAction] {
        return viewModel.pendingConnectorActions
    }

    private var leftWingIconCount: Int {
        visibleConnectorActions.count
            + (showsCmuxLeftIcon ? 1 : 0)
            + (viewModel.traceCopiedFlash ? 1 : 0)
    }

    private var leftWingRowWidth: CGFloat {
        let n = CGFloat(leftWingIconCount)
        guard n > 0 else { return 0 }
        if leftWingStacked {
            return Self.leftWingIconSize + (n - 1) * Self.leftWingStackedStep
        }
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
    /// its collapsed presentation (not inline/expanded).
    private var showsCmuxLeftIcon: Bool {
        guard !isInlineActive, !viewModel.isExpanded else { return false }
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
    /// Option+click in any state copies the session/debug trace instead.
    private func handleTap() {
        if NSApp.currentEvent?.modifierFlags.contains(.option) == true {
            viewModel.copyDebugTrace()
            return
        }
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
            && (viewModel.isExpanded || isHovered)
    }

    private func isNearVisibleSurface(_ point: CGPoint) -> Bool {
        let margin: CGFloat = isInlineActive ? 3 : 6
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
        // SwiftUI may be rebuilding the expanded transcript while AppKit is
        // still dispatching this hover event. Resizing the notch in that same
        // transaction can invalidate the event's old tracking hierarchy.
        // Cross the main-queue boundary before changing any surface geometry.
        DispatchQueue.main.async {
            refreshSurface()
            onHoverChanged(active || isDragTargeted)
        }
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
                cornerRadius: NotchWrapMetrics.cornerRadius,
                bulgeDepth: bulgeDepth,
                bulgeSweep: bulgeSweep
            )
            .fill(.black)

            NotchWrapBorderShape(
                wingWidth: wingWidth,
                bridgeHeight: bridgeHeight,
                notchOffset: notchOffset,
                notchWidth: notchWidth,
                notchHeight: notchHeight,
                cornerRadius: NotchWrapMetrics.cornerRadius,
                bulgeDepth: bulgeDepth,
                bulgeSweep: bulgeSweep
            )
            .stroke(
                NotchWrapMetrics.notchBorderColor.opacity(
                    isHovered || isInlineActive ? 0.80 : 0.35
                ),
                lineWidth: 1
            )

            leftWingIconLayer
            iconDepartureLayer

            // Left icon — only when chat open, hovered, or voice active (idle = blank notch).
            // In settings mode it shows the selected top-level area's icon;
            // child routes keep the same parent icon.
            if showsLeftStatusIcon || isSettingsActive {
                DrawInSymbol(
                    systemName: isSettingsActive
                        ? viewModel.compassRailRoute.area.symbolName
                        : (viewModel.selectedSourceMode?.symbolName ?? "sparkles"),
                    trigger: hoverIconRevealID,
                    duration: NotchWrapMetrics.surfaceDuration,
                    reduceMotion: reduceMotion
                )
                .position(x: targetLeftIconPos.x, y: targetLeftIconPos.y)
            }

            // Right icon — status dot. Hidden when idle and collapsed (pure notch
            // bg); shown when chat open, task running, approval pending, or error
            // (so a down daemon always surfaces).
            inlineSurfaceLayer
            invariantStatusPillLayer
            cmuxBannerLayer
            statusDotLayer
            cmuxDotLayer
            pasteWheelLayer
        }
    }

    /// Clipboard wheel chips laid along the bulged bottom edge.
    @ViewBuilder
    private var pasteWheelLayer: some View {
        if viewModel.pasteWheelActive || wheelContentOpacity > 0 {
            PasteWheelView(
                viewModel: viewModel,
                notchOffset: notchOffset,
                notchWidth: notchWidth,
                notchHeight: notchHeight
            )
            .opacity(wheelContentOpacity)
            .frame(width: maxSize.width, height: maxSize.height, alignment: .topLeading)
            .clipShape(animatedNotchClipShape)
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
                cornerRadius: NotchWrapMetrics.cornerRadius,
                bulgeDepth: bulgeDepth,
                bulgeSweep: bulgeSweep
            )
        )
        // The surface tap toggles the pill / focuses cmux. While an inline
        // surface (input/output/settings/automations) is open, the mask hands
        // clicks to the content instead — otherwise this ancestor gesture
        // races SwiftUI Buttons inside the bridge (e.g. the automations
        // editor's "Ďalej") and a button click collapses the notch.
        // Esc and click-away still dismiss inline surfaces.
        .gesture(
            TapGesture().onEnded { handleTap() },
            including: isInlineActive ? .subviews : .all
        )
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
            refreshSurface()
        }
        .onChange(of: isDragTargeted) { _, targeted in
            refreshSurface()
            onHoverChanged(targeted || isHovered)
        }
        .onChange(of: viewModel.pasteWheelActive) { _, active in
            refreshSurface()
            updateWheelOpacity(active: active)
        }
        .onChange(of: viewModel.notchInteractionMode) { previousMode, mode in
            updateStatusDotTravelState(previousMode: previousMode, currentMode: mode)
            if mode != .output { outputWingRatchet = NotchWrapMetrics.outputMinWingWidth }
            refreshSurface()
        }
        .onChange(of: viewModel.notchPresentation.revision) {
            refreshSurface()
        }
        .onChange(of: viewModel.compassRailRoute) {
            refreshSurface()
        }
        // Slash-command suggestion rows grow the input bridge.
        .onChange(of: viewModel.slashSuggestions) {
            refreshSurface()
        }
        .onChange(of: viewModel.slashCommandError) {
            refreshSurface()
        }
        .onChange(of: viewModel.cmuxBanner) {
            refreshSurface()
        }
        .onChange(of: viewModel.pendingConnectorActions) {
            refreshSurface()
        }
        .onChange(of: viewModel.traceCopiedFlash) {
            refreshSurface()
        }
        .onChange(of: viewModel.isActivityTranscriptExpanded) {
            deferOutputSurfaceRefresh(force: true)
        }
    }

    var body: some View {
        interactiveSurface
        .onChange(of: viewModel.streamingChunk) {
            deferOutputSurfaceRefresh()
        }
        .onChange(of: viewModel.latestAssistantMessageId) {
            outputWingRatchet = NotchWrapMetrics.outputMinWingWidth
            deferOutputSurfaceRefresh(force: true)
        }
        .onChange(of: viewModel.isLatestAssistantStreaming) {
            deferOutputSurfaceRefresh(force: true)
        }
        .onChange(of: viewModel.notchHoverResetID) {
            isHovered = false
            isDragTargeted = false
            refreshSurface()
            onHoverChanged(false)
        }
        .onChange(of: status) {
            pulsing = (status == .thinking)
        }
        .onAppear {
            pulsing = (status == .thinking)
            viewModel.setNotchReduceMotion(reduceMotion)
            refreshSurface()
        }
        .onChange(of: reduceMotion) { _, enabled in
            viewModel.setNotchReduceMotion(enabled)
            refreshSurface()
        }
        .onReceive(NotificationCenter.default.publisher(for: .bagentCodeCopied)) { _ in
            guard !reduceMotion else { return }
            withAnimation(.spring(response: 0.25, dampingFraction: 0.6)) { copyFlashed = true }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.3) {
                withAnimation(.easeInOut(duration: 0.3)) { copyFlashed = false }
            }
        }
        .modifier(NotchProjectionAccessibilityModifier(
            enabled: isProjectionSurfaceActive,
            presentation: viewModel.notchPresentation,
            approval: viewModel.authoritativePendingApproval,
            activateActivity: viewModel.activateFocusedNotchActivity,
            openAutomations: viewModel.openActiveAutomations,
            allowApproval: { item in viewModel.decideApproval(item, allow: true) },
            denyApproval: { item in viewModel.decideApproval(item, allow: false) }
        ))
    }
}

private struct NotchProjectionAccessibilityModifier: ViewModifier {
    let enabled: Bool
    let presentation: NotchPresentation
    let approval: ApprovalItem?
    let activateActivity: () -> Void
    let openAutomations: () -> Void
    let allowApproval: (ApprovalItem) -> Void
    let denyApproval: (ApprovalItem) -> Void

    @ViewBuilder
    func body(content: Content) -> some View {
        if enabled {
            content.accessibilityRepresentation {
                VStack {
                    if let approval {
                        Text("Approval required")
                            .accessibilityValue(approval.description ?? "Pending action")
                        Button("Allow") { allowApproval(approval) }
                        Button("Deny") { denyApproval(approval) }
                    } else if presentation.geometry.bridgeHeight > 0 {
                        Button("Activity", action: activateActivity)
                            .disabled(!presentation.canOpenFocusedDestination)
                            .keyboardShortcut(.return, modifiers: [])
                            .accessibilityValue(presentation.rail.accessibilityValue)
                    }
                    if presentation.statusPill.label != nil {
                        if presentation.statusPill.opensAutomations(
                            activeAutomationCount: presentation.activeAutomationCount
                        ) {
                            Button("Status", action: openAutomations)
                                .accessibilityValue(presentation.statusPill.accessibilityValue)
                        } else {
                            Text("Status")
                                .accessibilityValue(presentation.statusPill.accessibilityValue)
                        }
                    }
                }
            }
        } else {
            content
        }
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
    let outputGrowthPhase: Bool
    let acceptanceSettingsState: CompassRailAcceptanceState?
    let acceptanceReduceMotionOverride: Bool?
    @FocusState private var inputFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var systemReduceMotion
    @State private var placeholderRevealID = UUID()
    @State private var inlineFocusRetryID = UUID()

    init(viewModel: ChatViewModel, showsInputLeadingIcon: Bool = true, outputGrowthPhase: Bool = false, acceptanceSettingsState: CompassRailAcceptanceState? = nil, acceptanceReduceMotionOverride: Bool? = nil) {
        self.viewModel = viewModel
        self.showsInputLeadingIcon = showsInputLeadingIcon
        self.outputGrowthPhase = outputGrowthPhase
        self.acceptanceSettingsState = acceptanceSettingsState
        self.acceptanceReduceMotionOverride = acceptanceReduceMotionOverride
    }

    private var reduceMotion: Bool { acceptanceReduceMotionOverride ?? systemReduceMotion }

    private var canSend: Bool {
        (!viewModel.inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || !viewModel.pendingAttachments.isEmpty)
            && !viewModel.isThinking
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            // A pending approval preempts every other mode — it is the only place
            // a gated write can be allowed, and it auto-denies after 60 s.
            if let approval = viewModel.authoritativePendingApproval {
                ApprovalModalOverlay(approval: approval, viewModel: viewModel)
            } else if viewModel.notchPresentation.statusPill.label == "APPROVE" {
                ActivityPeekStageRailView(
                    presentation: viewModel.notchPresentation,
                    action: viewModel.activateFocusedNotchActivity
                )
            } else if viewModel.showWhatsappPairing {
                WhatsAppPairingView(viewModel: viewModel)
            } else if viewModel.clearCurrentChatConfirmationPresented {
                CurrentChatClearConfirmation(viewModel: viewModel)
            } else {
                switch viewModel.notchInteractionMode {
                case .input:
                    inputWithSuggestions
                case .thinking:
                    ActivityPeekStageRailView(
                        presentation: viewModel.notchPresentation,
                        action: viewModel.activateFocusedNotchActivity
                    )
                case .output:
                    outputView
                case .settings:
                    NotchSettingsContent(viewModel: viewModel, acceptanceState: acceptanceSettingsState, reduceMotionOverride: acceptanceReduceMotionOverride)
                case .automations:
                    AutomationsNotchContent(viewModel: viewModel)
                case .collapsed:
                    if viewModel.notchPresentation.geometry.bridgeHeight > 0 {
                        ActivityPeekStageRailView(
                            presentation: viewModel.notchPresentation,
                            action: viewModel.activateFocusedNotchActivity
                        )
                    }
                }
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
        .onChange(of: viewModel.currentChatFocusRequestID) {
            keepInlineInputFocused(reason: "current-chat-cleared")
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

    private var inputWithSuggestions: some View {
        VStack(alignment: .leading, spacing: 4) {
            inputRow
            pendingAttachmentRows
            restoredCurrentChatRecords
            if !viewModel.slashSuggestions.isEmpty {
                slashSuggestionRows
                    .transition(.opacity)
            }
            if let error = viewModel.slashCommandError {
                Text(error)
                    .font(.system(size: 11))
                    .foregroundStyle(.red.opacity(0.9))
                    .lineLimit(1)
                    .accessibilityAddTraits(.isStaticText)
            }
        }
        .animation(
            reduceMotion ? nil : .easeOut(duration: 0.18),
            value: viewModel.slashSuggestions
        )
    }

    private var slashSuggestionRows: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(viewModel.slashSuggestions.enumerated()), id: \.element.id) { index, cmd in
                let selected = index == viewModel.slashSelectionIndex
                Button {
                    _ = viewModel.completeSlashSuggestion(cmd)
                    inputFocused = true
                } label: {
                    HStack(spacing: 8) {
                        if let symbol = cmd.symbol {
                            Image(systemName: symbol)
                                .font(.system(size: 10, weight: .medium))
                                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                                .frame(width: 14)
                        }
                        Text(cmd.command)
                            .font(.system(size: 12, weight: .medium))
                            .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                        Text(cmd.subtitle)
                            .font(.system(size: 11))
                            .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                            .lineLimit(1)
                        Spacer(minLength: 0)
                    }
                }
                .buttonStyle(.plain)
                .padding(.horizontal, 8)
                .frame(height: NotchWrapMetrics.slashSuggestionRowHeight - 2)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .fill(Color.white.opacity(selected ? 0.12 : 0.06))
                )
                .contentShape(Rectangle())
                .accessibilityElement(children: .combine)
                .accessibilityLabel(String(
                    format: String(
                        localized: "slashCommand.suggestion.accessibilityLabel",
                        defaultValue: "Command %@, %@"),
                    cmd.command,
                    cmd.subtitle))
                .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
            }
        }
    }

    private var thinkingRow: some View {
        HStack(spacing: 10) {
            ThinkingIndicator()
                .scaleEffect(0.74)
            VStack(alignment: .leading, spacing: 3) {
                Text(viewModel.latestEvidenceStatus ?? viewModel.toolStatus ?? "Thinking")
                    .font(.system(size: 14, weight: .regular))
                    .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                    .lineLimit(1)
                    .animation(.easeOut(duration: 0.18), value: viewModel.toolStatus)
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
        return VStack(alignment: .leading, spacing: 6) {
            // History-browse header (↑/↓ on empty input): position + the prompt
            // that produced the browsed answer.
            if let pos = viewModel.historyBrowsePosition {
                HStack(spacing: 6) {
                    Text("‹ \(pos.0)/\(pos.1) ›")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                    if let prompt = viewModel.historyBrowsePrompt {
                        Text(prompt)
                            .font(.system(size: 11))
                            .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
                .transition(.opacity)
            }
            if let message = viewModel.latestAssistantMessage,
               !message.activities.isEmpty || message.evidencePhase != nil || message.evidenceOutcome != nil {
                activityTranscript(message, streaming: viewModel.isLatestAssistantStreaming)
            }
            LatestAssistantOutputScrollView(
                text: text,
                messageId: viewModel.latestAssistantMessageId,
                isStreaming: viewModel.isLatestAssistantStreaming,
                growthPhase: outputGrowthPhase,
                reduceMotion: reduceMotion,
                sources: viewModel.latestAssistantMessage?.sources ?? []
            )
            .frame(maxWidth: .infinity)
            .frame(maxHeight: .infinity)
            .accessibilityLabel("Latest assistant response")
            .accessibilityValue(text)
            if let sources = viewModel.latestAssistantMessage?.sources, !sources.isEmpty {
                sourceLinks(sources)
            }
            restoredCurrentChatRecords
        }
    }

    @ViewBuilder
    private var pendingAttachmentRows: some View {
        if !viewModel.pendingAttachments.isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                ForEach(viewModel.pendingAttachments) { attachment in
                    HStack(spacing: 6) {
                        Image(systemName: attachment.availability == .available
                            ? "paperclip" : "exclamationmark.triangle")
                        Text(attachment.filename).lineLimit(1)
                        if attachment.availability == .unavailable {
                            Text(String(
                                localized: "currentChat.attachment.unavailable",
                                defaultValue: "Attachment unavailable"))
                                .foregroundStyle(.orange)
                        }
                        Spacer(minLength: 2)
                        Button {
                            viewModel.removeAttachment(id: attachment.id)
                        } label: {
                            Image(systemName: "xmark")
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(String(
                            localized: "currentChat.attachment.remove",
                            defaultValue: "Remove attachment"))
                    }
                }
            }
            .font(.system(size: 10.5))
            .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
        }
    }

    @ViewBuilder
    private var restoredCurrentChatRecords: some View {
        let attachments = viewModel.restoredSubmittedAttachments
        let sources = viewModel.restoredValidatedSources
        let connectors = viewModel.restoredConnectorReferences
        let approvals = viewModel.restoredApprovalPresentations
        if !attachments.isEmpty || !sources.isEmpty || !connectors.isEmpty || !approvals.isEmpty {
            VStack(alignment: .leading, spacing: 3) {
                ForEach(attachments) { attachment in
                    retainedRecord(
                        icon: "paperclip",
                        label: attachment.filename,
                        available: attachment.availability == .available)
                }
                ForEach(sources, id: \.identity) { source in
                    retainedRecord(icon: "link", label: source.label, available: source.availability == "available")
                }
                ForEach(connectors, id: \.identity) { reference in
                    retainedRecord(icon: "point.3.connected.trianglepath.dotted", label: reference.label,
                                   available: reference.availability == "available")
                }
                ForEach(approvals, id: \.identity) { approval in
                    Label("\(approval.category): \(approval.outcome)", systemImage: "checkmark.shield")
                }
            }
            .font(.system(size: 10.5))
            .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            .accessibilityElement(children: .contain)
            .accessibilityLabel(String(
                localized: "currentChat.restoredRecords.accessibilityLabel",
                defaultValue: "Restored Current Chat records"))
        }
    }

    private func retainedRecord(icon: String, label: String, available: Bool) -> some View {
        HStack(spacing: 5) {
            Image(systemName: available ? icon : "exclamationmark.triangle")
            Text(label).lineLimit(1)
            if !available {
                Text(String(
                    localized: "currentChat.attachment.unavailable",
                    defaultValue: "Unavailable"))
                    .foregroundStyle(.orange)
            }
        }
    }

    @ViewBuilder
    private func activityTranscript(_ message: ChatMessage, streaming: Bool) -> some View {
        if message.evidencePhase != nil || message.evidenceOutcome != nil {
            evidenceActivityTranscript(message, streaming: streaming)
        } else {
            legacyActivityTranscript(message.activities, streaming: streaming)
        }
    }

    @ViewBuilder
    private func legacyActivityTranscript(_ activities: [TurnActivity], streaming: Bool) -> some View {
        let current = activities.last(where: { $0.status == "running" }) ?? activities.last!
        let failureCount = activities.filter { $0.status == "failed" }.count
        let completedSummary = failureCount == 0
            ? "Legacy activity complete · \(activities.count) actions"
            : "Legacy activity · \(activities.count) actions · \(failureCount) failed"
        Button {
            viewModel.isActivityTranscriptExpanded.toggle()
        } label: {
            HStack(spacing: 5) {
                Image(systemName: viewModel.isActivityTranscriptExpanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 8, weight: .semibold))
                Text(streaming && current.status == "running" ? current.title : completedSummary)
                    .lineLimit(1)
                Spacer(minLength: 2)
                if streaming && current.status == "running" { ProgressView().controlSize(.mini) }
            }
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(viewModel.isActivityTranscriptExpanded ? "Collapse assistant activity" : "Expand assistant activity")

        if viewModel.isActivityTranscriptExpanded {
            VStack(alignment: .leading, spacing: 3) {
                ForEach(Array(activities.suffix(NotchActivityLayout.maxVisibleRows))) { activity in
                    HStack(alignment: .firstTextBaseline, spacing: 5) {
                        Image(systemName: activity.status == "failed"
                            ? "exclamationmark.circle"
                            : (activity.status == "running" ? "circle.dotted" : "checkmark.circle"))
                            .foregroundStyle(activity.status == "failed" ? Color.orange : NotchWrapMetrics.notchTextSecondary)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(activity.title)
                            if let detail = activity.detail, !detail.isEmpty {
                                Text(detail).foregroundStyle(NotchWrapMetrics.notchTextSecondary).lineLimit(1)
                            }
                        }
                        Spacer(minLength: 2)
                        if let ms = activity.durationMs {
                            Text(String(format: "%.1fs", Double(ms) / 1000))
                                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                        }
                    }
                }
            }
            .font(.system(size: 9.5))
            .padding(.leading, 2)
            .frame(maxHeight: 84)
            .clipped()
        }
    }

    @ViewBuilder
    private func evidenceActivityTranscript(_ message: ChatMessage, streaming: Bool) -> some View {
        let summary = message.evidenceOutcome.map(EvidencePresentation.outcomeLabel)
            ?? message.evidencePhase.map(EvidencePresentation.phaseLabel)
            ?? "Working with evidence"
        Button {
            viewModel.isActivityTranscriptExpanded.toggle()
        } label: {
            HStack(spacing: 5) {
                Image(systemName: viewModel.isActivityTranscriptExpanded ? "chevron.down" : "chevron.right")
                    .font(.system(size: 8, weight: .semibold))
                Text(summary).lineLimit(1)
                Spacer(minLength: 2)
                if streaming && message.evidenceOutcome == nil {
                    ProgressView().controlSize(.mini)
                }
            }
            .font(.system(size: 10.5, weight: .medium))
            .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(
            message.evidenceOutcome.map {
                EvidencePresentation.accessibilityLabel(
                    outcome: $0,
                    expanded: viewModel.isActivityTranscriptExpanded
                )
            } ?? "\(viewModel.isActivityTranscriptExpanded ? "Collapse" : "Expand") evidence activity. \(summary)"
        )

        if viewModel.isActivityTranscriptExpanded {
            VStack(alignment: .leading, spacing: 3) {
                ForEach(Array(message.evidenceActivities.suffix(NotchActivityLayout.maxVisibleRows))) { activity in
                    HStack(alignment: .firstTextBaseline, spacing: 5) {
                        Image(systemName: evidenceActivitySymbol(activity))
                            .foregroundStyle(
                                activity.executionStatus == .failed
                                    || activity.executionStatus == .denied
                                    || activity.executionStatus == .timedOut
                                    ? Color.orange
                                    : NotchWrapMetrics.notchTextSecondary
                            )
                        VStack(alignment: .leading, spacing: 1) {
                            Text(activity.operation)
                            Text(EvidencePresentation.activityDetail(activity))
                                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                                .lineLimit(2)
                        }
                        Spacer(minLength: 2)
                        if activity.durationMs > 0 {
                            Text(String(format: "%.1fs", Double(activity.durationMs) / 1000))
                                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(
                        "\(activity.operation), \(activity.executionStatus.rawValue), \(EvidencePresentation.activityDetail(activity))"
                    )
                }
            }
            .font(.system(size: 9.5))
            .padding(.leading, 2)
            .frame(maxHeight: 84)
            .clipped()
        }
    }

    private func evidenceActivitySymbol(_ activity: EvidenceLogicalActivity) -> String {
        switch activity.executionStatus {
        case .inProgress: return "circle.dotted"
        case .failed, .denied, .timedOut: return "exclamationmark.circle"
        case .succeeded:
            return activity.contribution == .satisfied || activity.contribution == .partial
                ? "checkmark.circle"
                : "minus.circle"
        }
    }

    private func sourceLinks(_ sources: [DaemonClient.TranscriptSource]) -> some View {
        HStack(spacing: 5) {
            Text("Sources")
                .font(.system(size: 9.5, weight: .medium))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
            ForEach(Array(sources.prefix(6).enumerated()), id: \.element.id) { index, source in
                Button("[\(index + 1)]") {
                    NSWorkspace.shared.open(source.url)
                }
                .buttonStyle(.plain)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(Color.accentColor)
                .help("\(source.title) — \(source.domain)")
                .accessibilityLabel("Open source \(index + 1), \(source.title)")
            }
        }
    }

    private func focusIfNeeded() {
        guard !viewModel.clearCurrentChatConfirmationPresented else {
            inputFocused = false
            return
        }
        guard viewModel.notchInteractionMode == .input else {
            inlineFocusRetryID = UUID()
            inputFocused = false
            return
        }
        keepInlineInputFocused(reason: "input-mode")
    }

    private func keepInlineInputFocused(reason: String) {
        _ = reason
        let restoreCaretAtEnd = viewModel.consumeCurrentChatCaretRestoration()
        let retryID = UUID()
        inlineFocusRetryID = retryID
        for attempt in 0..<12 {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.02 + Double(attempt) * 0.08) {
                guard inlineFocusRetryID == retryID,
                      viewModel.notchInteractionMode == .input,
                      !viewModel.clearCurrentChatConfirmationPresented
                else { return }
                NSApp.activate(ignoringOtherApps: true)
                inputFocused = true
                DispatchQueue.main.async {
                    guard restoreCaretAtEnd,
                          let editor = NSApp.keyWindow?.firstResponder as? NSTextView
                    else { return }
                    viewModel.restoreCurrentChatCaret(in: editor)
                }
            }
        }
    }
}

private struct CurrentChatClearConfirmation: View {
    @ObservedObject var viewModel: ChatViewModel
    @FocusState private var cancelFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(String(localized: "currentChat.clear.title", defaultValue: "Clear Current Chat?"))
                .font(.system(size: 14, weight: .semibold))
            Text(String(
                localized: "currentChat.clear.explanation",
                defaultValue: "Clears completed and interrupted turns, the draft, attachments, sources, Connector References, approvals, and any Continuation Seed and Provenance. No hidden archive is kept. Automation Sessions, Runs and Definitions, Saved Long-Term Memory, external side effects, and source-session viewed state remain; no Automation Work is cancelled."))
                .font(.system(size: 12))
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(String(localized: "currentChat.clear.cancel", defaultValue: "Cancel")) {
                    viewModel.cancelCurrentChatClear()
                }
                .focused($cancelFocused)
                .keyboardShortcut(.cancelAction)
                Button(
                    String(localized: "currentChat.clear.confirm", defaultValue: "Clear Current Chat"),
                    role: .destructive
                ) {
                    viewModel.confirmCurrentChatClear()
                }
            }
        }
        .onAppear { cancelFocused = true }
        .accessibilityElement(children: .contain)
    }
}

private struct LatestAssistantOutputScrollView: NSViewRepresentable {
    let text: String
    let messageId: UUID?
    let isStreaming: Bool
    let growthPhase: Bool
    let reduceMotion: Bool
    let sources: [DaemonClient.TranscriptSource]

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
        textView.delegate = context.coordinator

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
        guard context.coordinator.textView != nil else { return }
        let coordinator = context.coordinator
        let messageChanged = coordinator.messageId != messageId

        if messageChanged {
            coordinator.messageId = messageId
            coordinator.userScrolledAway = false
            coordinator.lastText = nil
        }

        coordinator.growthPhase = growthPhase
        let sourceIDs = sources.map(\.id)
        if coordinator.sourceIDs != sourceIDs {
            coordinator.sourceIDs = sourceIDs
            coordinator.sources = sources
            coordinator.lastText = nil
        }
        if coordinator.lastText != text {
            coordinator.lastText = text
            coordinator.applyText(text)
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

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        weak var scrollView: NSScrollView?
        weak var textView: NSTextView?
        var messageId: UUID?
        var lastText: String?
        var userScrolledAway = false
        var growthPhase = false
        var sources: [DaemonClient.TranscriptSource] = []
        var sourceIDs: [String] = []
        private var lastClipSize: NSSize = .zero

        /// Render the complete canonical value, then append only when the
        /// complete attributed prefix is unchanged. The former source-line
        /// bookkeeping inferred a TextKit range from independently normalized
        /// markdown fragments. A formatter normalization at finalization could
        /// therefore leave `finalizedRendered` beyond the live storage and
        /// crash `NSMutableRLEArray` from `replaceCharacters`.
        func applyText(_ text: String) {
            guard let storage = textView?.textStorage else { return }
            let rendered = NSMutableAttributedString(
                attributedString: NotchMarkdown.attributedString(text)
            )
            decorateCitations(rendered)
            NotchTextStorageUpdater.apply(rendered, to: storage)
        }

        private func decorateCitations(_ output: NSMutableAttributedString) {
            guard !sources.isEmpty else { return }
            let nsText = output.string as NSString
            let regex = try? NSRegularExpression(pattern: #"\[(\d+)\]"#)
            regex?.enumerateMatches(
                in: output.string,
                range: NSRange(location: 0, length: nsText.length)
            ) { match, _, _ in
                guard let match, match.numberOfRanges == 2 else { return }
                let number = Int(nsText.substring(with: match.range(at: 1))) ?? 0
                guard sources.indices.contains(number - 1) else { return }
                output.addAttributes(
                    [
                        .link: sources[number - 1].url,
                        .foregroundColor: NSColor.controlAccentColor,
                        .underlineStyle: NSUnderlineStyle.single.rawValue,
                    ],
                    range: match.range
                )
            }
        }

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
        /// - Scroll phase (panel target at max height): pin to the bottom so
        ///   new lines scroll into view. Never overrides a user's manual
        ///   scroll. The phase signal is `growthPhase` (from the panel's
        ///   *target* height, set by SwiftUI) — not momentary overflow against
        ///   the animating clip: mid-animation the clip lags the text by a
        ///   line, so overflow flip-flops per line (bottom-pin jump up, then
        ///   slide back to top anchor — the jitter this replaces). Overflow is
        ///   still checked once pinning, because siblings above the scroll
        ///   view (tool-status chip, browse header) shrink the clip below the
        ///   theoretical max.
        func applyAutoScroll() {
            guard !userScrolledAway else { return }
            if !growthPhase && contentOverflows() {
                pinToBottom()
            } else {
                scrollTo(y: 0)
            }
        }

        func contentOverflows() -> Bool {
            guard let scrollView, let documentView = scrollView.documentView else { return false }
            return documentView.bounds.height > scrollView.contentView.bounds.height + 1
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

        func textView(
            _ textView: NSTextView,
            clickedOnLink link: Any,
            at charIndex: Int
        ) -> Bool {
            let url: URL?
            if let value = link as? URL {
                url = value
            } else if let value = link as? String {
                url = URL(string: value)
            } else {
                url = nil
            }
            guard let url,
                  ["http", "https"].contains(url.scheme?.lowercased() ?? "")
            else { return true }
            NSWorkspace.shared.open(url)
            return true
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
extension Notification.Name {
    static let bagentCodeCopied = Notification.Name("bagentCodeCopied")
}

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

/// Approval prompt for gated write actions, rendered inside the notch.
/// The only approval surface — the daemon auto-denies after 60 s, so this must
/// stay reachable whenever `pendingApprovals` is non-empty.
struct ApprovalModalOverlay: View {
    let approval: ApprovalItem
    @ObservedObject var viewModel: ChatViewModel
    @State private var secondsLeft: Int = 60

    private let timer = Timer.publish(every: 1, on: .main, in: .common).autoconnect()

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 7) {
                Image(systemName: "shield.lefthalf.filled")
                    .font(.system(size: 13, weight: .semibold))
                Text("Schválenie akcie")
                    .font(.system(size: 14, weight: .semibold))
                Spacer()
                Text("\(secondsLeft) s")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(NotchWrapMetrics.notchTextFaint)
            }
            .foregroundStyle(NotchWrapMetrics.notchTextPrimary)

            // Identify the originating automation for unattended approvals.
            if let origin = approval.origin, origin.kind == "automation",
               let name = origin.automationName {
                HStack(spacing: 5) {
                    Image(systemName: "clock.arrow.2.circlepath")
                        .font(.system(size: 10, weight: .medium))
                    Text("Automatizácia · \(name)")
                        .font(.system(size: 11, weight: .medium))
                        .lineLimit(1)
                }
                .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                .accessibilityLabel("Požiadavka z automatizácie \(name)")
            }

            VStack(alignment: .leading, spacing: 4) {
                Text(approval.toolName)
                    .font(.system(size: 12, weight: .medium, design: .monospaced))
                    .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                if let desc = approval.description {
                    Text(desc)
                        .font(.system(size: 12))
                        .foregroundStyle(NotchWrapMetrics.notchTextPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        .lineLimit(4)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(8)
            .background(Color.white.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))

            HStack(spacing: 8) {
                // Styling lives inside the label + contentShape: with a
                // `.plain` button, outside styling leaves only the text
                // glyphs clickable and the pill is effectively dead.
                Button { viewModel.decideApproval(approval, allow: false) } label: {
                    Text("Zamietnuť")
                        .font(.system(size: 12, weight: .medium))
                        .foregroundStyle(NotchWrapMetrics.notchTextSecondary)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 5)
                        .background(Color.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))
                        .contentShape(RoundedRectangle(cornerRadius: 6))
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.escape, modifiers: [])
                Button { viewModel.decideApproval(approval, allow: true) } label: {
                    Text("Schváliť")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(.black)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 5)
                        .background(Color.white.opacity(0.9), in: RoundedRectangle(cornerRadius: 6))
                        .contentShape(RoundedRectangle(cornerRadius: 6))
                }
                .buttonStyle(.plain)
                .keyboardShortcut(.return, modifiers: [])
            }
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
