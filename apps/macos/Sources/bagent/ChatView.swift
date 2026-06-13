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
            MenuBarPillView()
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    // Panel is always (2*hoverWingWidth + notchWidth) wide.
    // notchOffset = fixed x of the notch left edge in panel coords.
    private let notchOffset = NotchWrapMetrics.hoverWingWidth

    private var spring: Animation {
        reduceMotion ? .easeInOut(duration: 0.18) : .spring(response: 0.28, dampingFraction: 0.78)
    }
    private var status: AgentStatus { viewModel.agentStatus }
    private var maxSize: CGSize {
        CGSize(
            width: 2 * NotchWrapMetrics.hoverWingWidth + notchWidth,
            height: notchHeight + NotchWrapMetrics.hoverBridgeHeight
        )
    }

    private func setExpansion(expanded: Bool) {
        withAnimation(spring) {
            wingWidth    = expanded ? NotchWrapMetrics.hoverWingWidth    : NotchWrapMetrics.idleWingWidth
            bridgeHeight = expanded ? NotchWrapMetrics.hoverBridgeHeight : NotchWrapMetrics.idleBridgeHeight
        }
    }

    // Icon positions track the center of the current wing area (x) and shape height (y).
    private var leftIconPos: CGPoint {
        CGPoint(x: notchOffset - wingWidth / 2, y: (notchHeight + bridgeHeight) / 2)
    }
    private var rightIconPos: CGPoint {
        CGPoint(x: notchOffset + notchWidth + wingWidth / 2, y: (notchHeight + bridgeHeight) / 2)
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
                           with: .color(NotchWrapMetrics.notchBorderColor.opacity(isHovered ? 0.80 : 0.35)),
                           lineWidth: 1)
            }

            // Left icon — tracks center of wing as it expands
            Image(systemName: "sparkles")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(Color.white.opacity(isHovered ? 1.0 : 0.75))
                .position(leftIconPos)

            // Right status dot — tracks center of wing as it expands
            StatusDotView(status: status, pulsing: $pulsing, reduceMotion: reduceMotion, copyFlashed: copyFlashed, isDragTargeted: isDragTargeted)
                .position(rightIconPos)
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
            setExpansion(expanded: hovering || isDragTargeted || viewModel.pillHovered)
            onHoverChanged(hovering || isDragTargeted)
        }
        .onChange(of: viewModel.pillHovered) {
            setExpansion(expanded: viewModel.pillHovered || isHovered || isDragTargeted)
        }
        .onChange(of: isDragTargeted) { _, targeted in
            setExpansion(expanded: targeted || isHovered || viewModel.pillHovered)
            onHoverChanged(targeted || isHovered)
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
    var body: some View {
        Color.clear
            .overlay {
                HStack(spacing: 5) {
                    Image(systemName: "sparkles")
                        .font(.system(size: 11, weight: .semibold))
                    Text("bagent")
                        .font(.system(size: 12, weight: .medium))
                }
                .foregroundStyle(.primary)
            }
    }
}

// MARK: - Chat panel content (shown below the pill when expanded)

struct ChatPanelContent: View {
    @ObservedObject var viewModel: ChatViewModel
    let onCollapse: () -> Void

    var body: some View {
        ZStack {
            if viewModel.isExpanded {
                ExpandedChatView(viewModel: viewModel, onCollapse: onCollapse)
                    .transition(
                        .scale(scale: 0.82, anchor: UnitPoint(x: 0.5, y: 0))
                        .combined(with: .opacity)
                    )
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .animation(.spring(response: 0.30, dampingFraction: 0.62), value: viewModel.isExpanded)
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
                if viewModel.showSettings {
                    SettingsView(viewModel: viewModel)
                } else if viewModel.showMemory {
                    MemoryPanelView(viewModel: viewModel)
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
        return parts.joined(separator: " · ")
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
                        // Extra top padding when the button is present so text doesn't overlap it.
                        .padding(.top, (message.mailRef != nil && !isStreaming) ? 38 : 7)
                        .padding(.bottom, 7)
                        .background(Color(nsColor: .controlBackgroundColor))
                        .clipShape(RoundedRectangle(cornerRadius: 12))
                        .overlay(alignment: .topTrailing) {
                            // Show only after streaming ends so no layout jump mid-response.
                            if let ref = message.mailRef, !isStreaming {
                                MailOpenButton(ref: ref) { viewModel.openMail(ref) }
                                    .padding(.top, 6)
                                    .padding(.trailing, 8)
                                    .transition(.opacity)
                            }
                        }
                    // Mail attachments shown below the assistant response (Phase 5C)
                    if !message.attachments.isEmpty {
                        AttachmentStrip(attachments: message.attachments)
                    }
                }
            }

            if message.role == .assistant { Spacer(minLength: 40) }
        }
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
                    .foregroundStyle(viewModel.isVoiceRecording ? Color.accentColor : Color.secondary)
                    // `.repeating` is the macOS 14 equivalent of `.repeat(.continuous)`.
                    .symbolEffect(.pulse.byLayer, options: .repeating,
                                  isActive: viewModel.isVoiceRecording)
            }
            .buttonStyle(.plain)
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
