import SwiftUI

// MARK: - Notch wrap geometry constants

enum NotchWrapMetrics {
    static let idleWingWidth: CGFloat     = 32
    static let hoverWingWidth: CGFloat    = 52   // proportional — not too wide
    static let idleBridgeHeight: CGFloat  = 0
    static let hoverBridgeHeight: CGFloat = 22   // wraps under the notch
    static let cornerRadius: CGFloat      = 10   // bottom corners only
    static let innerCornerRadius: CGFloat = 8    // notch border
    static let expandedBridgeHeight: CGFloat = 520  // matches chatH
    static let expandedWingWidth: CGFloat   = 200   // chatW / 2
    static let voiceWingWidth: CGFloat    = 100  // voice mode — wide enough for sentence
    static let voiceBridgeHeight: CGFloat = 120  // voice mode — fits wave + 2 text lines
    static let notchBorderColor           = Color(white: 0.30)
}

// MARK: - Status panel content (always visible, never moves)

struct StatusPillView: View {
    let isOnNotch: Bool
    let notchWidth: CGFloat
    let notchHeight: CGFloat
    @ObservedObject var viewModel: ChatViewModel
    let onTap: () -> Void
    let onHoverChanged: (Bool) -> Void

    var body: some View {
        if isOnNotch {
            NotchWrapView(
                notchWidth: notchWidth,
                notchHeight: notchHeight,
                viewModel: viewModel,
                onTap: onTap,
                onHoverChanged: onHoverChanged
            )
        } else {
            MenuBarPillView(viewModel: viewModel)
                .contentShape(Rectangle())
                .onTapGesture { onTap() }
        }
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
    @State private var isHovered = false
    @State private var pulsing = false
    @State private var copyFlashed = false
    @State private var isDragTargeted = false
    @State private var isVoiceActive = false
    @State private var voiceContentOpacity: CGFloat = 0
    @State private var borderPulseOpacity: CGFloat = 0.35
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    // Panel is sized for voice mode (the widest/tallest state) so the frame never needs
    // AppKit resizing. notchOffset = voiceWingWidth = left edge of the physical notch.
    private let notchOffset = NotchWrapMetrics.voiceWingWidth

    private var spring: Animation {
        reduceMotion ? .easeInOut(duration: 0.18) : .spring(response: 0.28, dampingFraction: 0.78)
    }
    private var status: AgentStatus { viewModel.agentStatus }

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
            width: 2 * NotchWrapMetrics.voiceWingWidth + notchWidth,
            height: notchHeight + NotchWrapMetrics.voiceBridgeHeight
        )
    }

    private func setExpansion(expanded: Bool) {
        withAnimation(spring) {
            wingWidth    = expanded ? NotchWrapMetrics.hoverWingWidth    : NotchWrapMetrics.idleWingWidth
            bridgeHeight = expanded ? NotchWrapMetrics.hoverBridgeHeight : NotchWrapMetrics.idleBridgeHeight
        }
    }

    private func setVoiceState(active: Bool) {
        withAnimation(spring) {
            wingWidth    = active ? NotchWrapMetrics.voiceWingWidth    : (viewModel.pillHovered ? NotchWrapMetrics.hoverWingWidth    : NotchWrapMetrics.idleWingWidth)
            bridgeHeight = active ? NotchWrapMetrics.voiceBridgeHeight : (viewModel.pillHovered ? NotchWrapMetrics.hoverBridgeHeight : NotchWrapMetrics.idleBridgeHeight)
        }
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

    // Icon x tracks wing center. Icon y is clamped to the notch/hover area so it
    // doesn't drift into the bridge content during voice mode (bridge = 120 pt).
    private var iconY: CGFloat {
        let clampedBridge = min(bridgeHeight, NotchWrapMetrics.hoverBridgeHeight)
        return (notchHeight + clampedBridge) / 2
    }
    private var leftIconPos: CGPoint {
        CGPoint(x: notchOffset - wingWidth / 2, y: iconY)
    }
    private var rightIconPos: CGPoint {
        CGPoint(x: notchOffset + notchWidth + wingWidth / 2, y: iconY)
    }

    var body: some View {
        ZStack(alignment: .topLeading) {
            // Single Canvas — fill + border computed in one closure call from the same
            // @State values, so fill and border are always pixel-identical per frame.
            Canvas { ctx, size in
                let r  = NotchWrapMetrics.cornerRadius
                let br = max(0, bridgeHeight)
                let x  = notchOffset - wingWidth
                let w  = 2 * wingWidth + notchWidth
                let h  = notchHeight + br

                // Closed fill path — top corners sharp, bottom corners rounded r=10
                var fill = Path()
                fill.move(to:    CGPoint(x: x,         y: 0))
                fill.addLine(to: CGPoint(x: x + w,     y: 0))
                fill.addLine(to: CGPoint(x: x + w,     y: h - r))
                fill.addArc(center: CGPoint(x: x+w-r, y: h-r), radius: r,
                            startAngle: .degrees(0),   endAngle: .degrees(90),  clockwise: false)
                fill.addLine(to: CGPoint(x: x + r,     y: h))
                fill.addArc(center: CGPoint(x: x+r,   y: h-r), radius: r,
                            startAngle: .degrees(90),  endAngle: .degrees(180), clockwise: false)
                fill.closeSubpath()

                // Open border path — same arcs, no top edge
                var border = Path()
                border.move(to:    CGPoint(x: x + w,   y: 0))
                border.addLine(to: CGPoint(x: x + w,   y: h - r))
                border.addArc(center: CGPoint(x: x+w-r, y: h-r), radius: r,
                              startAngle: .degrees(0),   endAngle: .degrees(90),  clockwise: false)
                border.addLine(to: CGPoint(x: x + r,   y: h))
                border.addArc(center: CGPoint(x: x+r,  y: h-r), radius: r,
                              startAngle: .degrees(90),  endAngle: .degrees(180), clockwise: false)
                border.addLine(to: CGPoint(x: x,       y: 0))

                ctx.fill(fill, with: .color(.black))
                ctx.stroke(border,
                           with: .color(NotchWrapMetrics.notchBorderColor.opacity(
                               isVoiceActive ? borderPulseOpacity : (isHovered ? 0.80 : 0.35)
                           )),
                           lineWidth: 1)
            }

            // Left icon — only when chat open, hovered, or voice active (idle = blank notch)
            if viewModel.isExpanded || isHovered || isVoiceActive {
                Image(systemName: "sparkles")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Color.white.opacity((isHovered || isVoiceActive) ? 1.0 : 0.75))
                    .contentTransition(.symbolEffect(.replace))
                    .position(leftIconPos)
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
            } else if viewModel.isExpanded || status != .ready {
                // Show dot: chat open (green) OR non-idle status (thinking/approval/error)
                StatusDotView(status: status, pulsing: $pulsing, reduceMotion: reduceMotion, copyFlashed: copyFlashed, isDragTargeted: isDragTargeted)
                    .position(rightIconPos)
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
        }
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
        .onTapGesture { onTap() }
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
        .onHover { hovering in
            isHovered = hovering
            // Don't let hover-leave shrink the bridge during voice mode — the voice
            // dimensions are owned by setVoiceState; only hover controls expansion otherwise.
            if !isVoiceActive {
                setExpansion(expanded: hovering || isDragTargeted || viewModel.pillHovered)
            }
            onHoverChanged(hovering || isDragTargeted)
        }
        .onChange(of: viewModel.pillHovered) {
            if !isVoiceActive {
                setExpansion(expanded: viewModel.pillHovered || isHovered || isDragTargeted)
            }
        }
        .onChange(of: isDragTargeted) { _, targeted in
            if !isVoiceActive {
                setExpansion(expanded: targeted || isHovered || viewModel.pillHovered)
            }
            onHoverChanged(targeted || isHovered)
        }
        .onChange(of: viewModel.isVoiceNotchActive) { _, active in
            isVoiceActive = active
            setVoiceState(active: active)
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

// MARK: - Menu-bar inline pill (external / non-notch display)

struct MenuBarPillView: View {
    @ObservedObject var viewModel: ChatViewModel
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var isVoice: Bool { viewModel.isVoiceNotchActive }

    var body: some View {
        Color.clear
            .overlay {
                HStack(spacing: 5) {
                    // Non-notch pill always stays "bagent" + sparkles.
                    // (Voice feedback is shown in the dropdown VoiceOverlayView panel below.)
                    Image(systemName: "sparkles")
                        .font(.system(size: 11, weight: .semibold))

                    Text("bagent")
                        .font(.system(size: 12, weight: .medium))
                }
                .foregroundStyle(.primary)
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
    /// True once the open animation has fully settled — gates overflow detection
    /// so short text typed immediately after opening never triggers a promotion.
    @State private var openSettled = false
    /// Set true immediately before triggering promotion so that the macOS TextField
    /// focus-loss path (which fires onSubmit) doesn't accidentally call send().
    @State private var isPromoting = false

    private let fullFieldWidth: CGFloat = 540
    private let compactFieldWidth: CGFloat = 356
    private let inputHeight: CGFloat = 56
    /// Text wider than this (measured with NSFont at size 22) overflows one line.
    /// Computed from fullFieldWidth (540) minus horizontal padding (2×20), icon
    /// (24), HStack spacing (10), and a safety margin — ≈420 pt.
    private let textOverflowThreshold: CGFloat = 420

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
                // Gate overflow detection until the open animation fully settles
                // (0.18 s delay + 0.22 s animation + small buffer = 0.45 s).
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.45) {
                    openSettled = true
                }
            }
            .onDisappear {
                viewModel.hoveredSourceMode = nil
                viewModel.isSourcePickerForced = false
                openSettled = false
            }
            .onChange(of: viewModel.inputText) { _, text in
                guard openSettled, !isPromoting else { return }
                // Paste can insert a newline directly.
                if text.contains("\n") {
                    promoteToChat(preserving: text)
                    return
                }
                // Measure the proportional text width. Promote if it no longer fits
                // one line in the input capsule (threshold ≈ fullFieldWidth minus
                // padding, icon, and spacing). Always measure against the full-width
                // threshold so showing the source picker never triggers promotion.
                guard !text.isEmpty else { return }
                let font = NSFont.systemFont(ofSize: 22, weight: .regular)
                let measured = (text as NSString).size(withAttributes: [.font: font]).width
                if measured > textOverflowThreshold {
                    promoteToChat(preserving: text)
                }
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
                    if !isPromoting && !viewModel.isPromotingSpotlightDraft {
                        viewModel.send()
                    }
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

    private func promoteToChat(preserving text: String) {
        print("[promote] SpotlightInputPanel.promoteToChat: isPromoting=\(isPromoting), text='\(text)'")
        guard !isPromoting else { return }
        isPromoting = true
        viewModel.promoteSpotlightDraft(text)
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
        .help("\(mode.title) (⌘\(index + 1))")
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
                } else if viewModel.showSettings {
                    SettingsView(viewModel: viewModel)
                        .transition(.opacity.combined(with: .scale(scale: 0.98)))
                } else if viewModel.showMemory {
                    MemoryPanelView(viewModel: viewModel)
                } else if viewModel.showSkills {
                    SkillsPanelView(viewModel: viewModel)
                } else if viewModel.showDebug {
                    DebugPanelView(viewModel: viewModel)
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
            if viewModel.lastMemorySavedId != nil {
                HStack(spacing: 3) {
                    Image(systemName: "brain")
                        .font(.system(size: 9))
                    Text("uložené")
                        .font(.system(size: 10))
                }
                .foregroundStyle(.white)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Color.purple)
                .clipShape(Capsule())
                .transition(.opacity.combined(with: .scale(scale: 0.8)))
            }
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
            Button { viewModel.toggleDebugPanel() } label: {
                Image(systemName: viewModel.showDebug ? "ladybug.fill" : "ladybug")
                    .font(.system(size: 14))
                    .foregroundStyle(viewModel.showDebug ? AnyShapeStyle(Color.orange) : AnyShapeStyle(.tertiary))
            }
            .buttonStyle(.plain)
            .help("Debug")
            Button { viewModel.toggleSkillsPanel() } label: {
                Image(systemName: viewModel.showSkills ? "wand.and.stars" : "wand.and.stars.inverse")
                    .font(.system(size: 14))
                    .foregroundStyle(viewModel.showSkills ? AnyShapeStyle(Color.teal) : AnyShapeStyle(.tertiary))
            }
            .buttonStyle(.plain)
            .help("Schopnosti")
            Button { viewModel.toggleMemoryPanel() } label: {
                Image(systemName: viewModel.showMemory ? "brain.fill" : "brain")
                    .font(.system(size: 14))
                    .foregroundStyle(viewModel.showMemory ? AnyShapeStyle(Color.purple) : AnyShapeStyle(.tertiary))
            }
            .buttonStyle(.plain)
            .help("Pamäť")
            Button { viewModel.toggleSettingsPanel() } label: {
                Image(systemName: viewModel.showSettings ? "gear.circle.fill" : "gear")
                    .font(.system(size: 15))
                    .foregroundStyle(viewModel.showSettings ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(.tertiary))
            }
            .buttonStyle(.plain)
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
                            let streaming = viewModel.isThinking && msg.id == viewModel.messages.last?.id
                            MessageBubble(message: msg, isStreaming: streaming, viewModel: viewModel)
                                .id(msg.id)
                        }
                        if viewModel.isThinking {
                            ThinkingIndicator()
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
                    viewModel.showSettings = true
                }
            } label: {
                Image(systemName: "chevron.left")
                    .font(.system(size: 12, weight: .semibold))
            }
            .buttonStyle(.plain)
            .help("Späť do nastavení")

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
