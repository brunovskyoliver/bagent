import SwiftUI

// MARK: - Notch wrap geometry constants

enum NotchWrapMetrics {
    static let idleWingWidth: CGFloat    = 32
    static let hoverWingWidth: CGFloat   = 96
    static let idleBridgeHeight: CGFloat = 0
    static let hoverBridgeHeight: CGFloat = 8
    static let outerCornerRadius: CGFloat = 10
    static let innerCornerRadius: CGFloat = 8
    static let expandedBridgeHeight: CGFloat = 520  // matches chatH
    static let expandedWingWidth: CGFloat   = 200   // chatW / 2
}

// MARK: - Status panel content (always visible, never moves)

/// Permanent pill shown in the status panel. Tap to open/close the chat window.
struct StatusPillView: View {
    let isOnNotch: Bool
    let notchWidth: CGFloat
    let notchHeight: CGFloat
    let onTap: () -> Void
    let onHoverChanged: (Bool) -> Void

    var body: some View {
        if isOnNotch {
            NotchWrapView(
                notchWidth: notchWidth,
                notchHeight: notchHeight,
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
    let onTap: () -> Void
    let onHoverChanged: (Bool) -> Void

    @State private var isHovered = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var wingWidth: CGFloat {
        isHovered ? NotchWrapMetrics.hoverWingWidth : NotchWrapMetrics.idleWingWidth
    }
    private var bridgeHeight: CGFloat {
        isHovered ? NotchWrapMetrics.hoverBridgeHeight : NotchWrapMetrics.idleBridgeHeight
    }
    private var iconOpacity: Double { isHovered ? 1.0 : 0.7 }
    private var spring: Animation {
        reduceMotion
            ? .easeInOut(duration: 0.18)
            : .spring(response: 0.28, dampingFraction: 0.78)
    }

    var body: some View {
        let shape = NotchWrapShape(
            wingWidth: wingWidth,
            bridgeHeight: bridgeHeight,
            notchWidth: notchWidth,
            notchHeight: notchHeight,
            outerCornerRadius: NotchWrapMetrics.outerCornerRadius,
            innerCornerRadius: NotchWrapMetrics.innerCornerRadius
        )

        ZStack {
            shape.fill(Color.black)
            if isHovered {
                shape.stroke(Color.white.opacity(0.06), lineWidth: 1)
            }

            // Icon row: left wing | notch gap | right wing
            HStack(spacing: 0) {
                Image(systemName: "sparkles")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Color.white.opacity(iconOpacity))
                    .frame(width: wingWidth)
                Spacer().frame(width: notchWidth)
                Image(systemName: "chevron.down")
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(Color.white.opacity(iconOpacity))
                    .frame(width: wingWidth)
            }
            .animation(spring, value: wingWidth)
        }
        .contentShape(shape)
        .onTapGesture { onTap() }
        .onHover { hovering in
            withAnimation(spring) { isHovered = hovering }
            onHoverChanged(hovering)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("bagent — aplikácia")
        .accessibilityHint("Otvoriť chat")
        .accessibilityAddTraits(.isButton)
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
                        .scale(scale: 0.96, anchor: UnitPoint(x: 0.5, y: 0))
                        .combined(with: .opacity)
                    )
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .animation(.spring(response: 0.32, dampingFraction: 0.72), value: viewModel.isExpanded)
    }
}

// MARK: - Expanded chat panel

struct ExpandedChatView: View {
    @ObservedObject var viewModel: ChatViewModel
    let onCollapse: () -> Void
    @FocusState private var inputFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            if viewModel.showSettings {
                SettingsView(viewModel: viewModel)
            } else {
                messageList
                Divider()
                inputBar
            }
        }
        .background(.regularMaterial)
        .clipShape(RoundedRectangle(cornerRadius: 16))
        .onAppear {
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { inputFocused = true }
            viewModel.startApprovalPolling()
        }
        .onDisappear { viewModel.stopApprovalPolling() }
        .overlay {
            if let approval = viewModel.pendingApprovals.first {
                ApprovalModalOverlay(approval: approval, viewModel: viewModel)
            }
        }
        // Escape handled by NSEvent local monitor in NotchWindowController
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
            Button { viewModel.showSettings.toggle() } label: {
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
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 8) {
                    if viewModel.messages.isEmpty {
                        SuggestionChips(viewModel: viewModel)
                            .padding(.top, 12)
                    }
                    ForEach(viewModel.messages) { msg in
                        let streaming = viewModel.isThinking && msg.id == viewModel.messages.last?.id
                        MessageBubble(message: msg, isStreaming: streaming)
                            .id(msg.id)
                    }
                    if viewModel.isThinking {
                        ThinkingIndicator()
                            .padding(.leading, 4)
                            .id("thinking")
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
            }
            .onChange(of: viewModel.messages.count) {
                withAnimation(.easeOut(duration: 0.2)) {
                    scrollToLatest(proxy)
                }
            }
            // Fires on every streaming token — keeps the view pinned to the bottom.
            .onChange(of: viewModel.streamingChunk) {
                scrollToLatest(proxy)
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
        HStack(alignment: .bottom, spacing: 8) {
            TextField("Napíš správu…", text: $viewModel.inputText, axis: .vertical)
                .lineLimit(1...4)
                .textFieldStyle(.plain)
                .font(.system(size: 13))
                .focused($inputFocused)
                .onSubmit { viewModel.send() }
                .padding(.vertical, 6)

            Button { viewModel.send() } label: {
                Image(systemName: viewModel.inputText.isEmpty ? "arrow.up.circle" : "arrow.up.circle.fill")
                    .font(.system(size: 24))
                    .foregroundStyle(viewModel.inputText.isEmpty ? Color.secondary : Color.accentColor)
            }
            .buttonStyle(.plain)
            .disabled(viewModel.inputText.isEmpty || viewModel.isThinking)
            .keyboardShortcut(.return, modifiers: .command)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
    }
}

// MARK: - Message bubble

struct MessageBubble: View {
    let message: ChatMessage
    let isStreaming: Bool

    var body: some View {
        HStack(alignment: .top) {
            if message.role == .user { Spacer(minLength: 40) }

            if message.role == .user {
                Text(message.content)
                    .font(.system(size: 13))
                    .foregroundStyle(Color.white)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 7)
                    .background(Color.accentColor)
                    .clipShape(RoundedRectangle(cornerRadius: 12))
            } else {
                MessageContentView(text: message.content, isStreaming: isStreaming)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 7)
                    .background(Color(nsColor: .controlBackgroundColor))
                    .clipShape(RoundedRectangle(cornerRadius: 12))
            }

            if message.role == .assistant { Spacer(minLength: 40) }
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
