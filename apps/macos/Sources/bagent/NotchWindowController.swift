import AppKit
import Combine
import SwiftUI

// Borderless NSPanel by default returns canBecomeKey = false, which silently
// prevents makeKeyAndOrderFront from making the panel a key window, so keyboard
// events never reach the text field. Subclass to fix.
private final class BagentPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

@MainActor
final class NotchWindowController: NSObject {

    /// Always-visible pill that shows the label/status.
    /// On notch displays the window frame stays at max hover size; SwiftUI
    /// animates the visible shape inside it to avoid AppKit resize clipping.
    private var statusPanel: BagentPanel!
    /// The expandable chat sheet — appears below the pill, hidden when collapsed.
    private var chatPanel: BagentPanel!
    /// Voice-only overlay — appears below the pill while recording, hidden otherwise.
    private var voicePanel: BagentPanel!
    private let chatViewModel: ChatViewModel
    private(set) var isExpanded = false
    private(set) var isVoiceShowing = false
    private var hasNotch = false
    private var localKeyMonitor: Any?
    private var globalMouseMonitor: Any?
    private var voiceMouseMonitor: Any?

    private var pillFrame: NSRect = .zero
    private var chatFrame: NSRect = .zero
    private var notchWidth: CGFloat = 0
    private var notchHeight: CGFloat = 0
    private var sizeCancellable: AnyCancellable?
    private var previousApp: NSRunningApplication?

    init(chatViewModel: ChatViewModel) {
        self.chatViewModel = chatViewModel
        super.init()
        computeGeometry()
        buildStatusPanel()
        buildChatPanel()
        buildVoicePanel()
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(screensChanged),
            name: NSApplication.didChangeScreenParametersNotification,
            object: nil
        )

        // Hands-free voice loop: after a voice-initiated reply finishes, collapse
        // the chat and re-open the voice overlay for the next utterance.
        chatViewModel.onVoiceTurnComplete = { [weak self] in
            guard let self else { return }
            self.collapse()
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.45) { [weak self] in
                self?.presentVoice()
            }
        }

        sizeCancellable = Publishers.CombineLatest(
            chatViewModel.$chatWindowW,
            chatViewModel.$chatWindowH
        )
        .dropFirst()
        .receive(on: DispatchQueue.main)
        .sink { [weak self] (w, h) in
            // Call synchronously — no async hop so AppKit frame update is
            // in the same runloop pass as the SwiftUI layout change.
            self?.updateChatSize(w: w, h: h)
        }
    }

    // MARK: - Geometry

    private func computeGeometry() {
        guard let screen = NSScreen.main else { return }

        let notchCenterX: CGFloat
        let menuBarBottomY: CGFloat
        let menuBarH: CGFloat

        if let tl = screen.auxiliaryTopLeftArea, let tr = screen.auxiliaryTopRightArea {
            hasNotch = true
            notchCenterX = (tl.maxX + tr.minX) / 2
            menuBarH = tl.height
            menuBarBottomY = tl.minY
            notchWidth  = tr.minX - tl.maxX
            notchHeight = menuBarH
        } else {
            hasNotch = false
            notchCenterX = screen.frame.midX
            menuBarH = NSStatusBar.system.thickness
            menuBarBottomY = screen.frame.maxY - menuBarH
            notchWidth  = 0
            notchHeight = 0
        }

        chatViewModel.hasNotch = hasNotch

        if hasNotch {
            // pillFrame spans both wings + the notch gap + bridge clearance.
            // Width = 2*hoverWingWidth + notchWidth (wide enough for hover state).
            // Height = menuBarH + hoverBridgeHeight (small room for the bridge).
            let totalW = 2 * NotchWrapMetrics.hoverWingWidth + notchWidth
            let totalH = menuBarH + NotchWrapMetrics.hoverBridgeHeight
            pillFrame = NSRect(
                x: notchCenterX - totalW / 2,
                y: menuBarBottomY - NotchWrapMetrics.hoverBridgeHeight,
                width: totalW,
                height: totalH
            )
        } else {
            let pillW = min(500, max(260, screen.frame.width * 0.40))
            pillFrame = NSRect(
                x: notchCenterX - pillW / 2,
                y: menuBarBottomY,
                width: pillW,
                height: menuBarH
            )
        }

        // Chat panel sits below the pill with a small visual gap.
        let chatW = chatViewModel.chatWindowW
        let chatH = chatViewModel.chatWindowH
        let chatGap: CGFloat = 8
        chatFrame = NSRect(
            x: notchCenterX - chatW / 2,
            y: pillFrame.minY - chatH - chatGap,
            width: chatW,
            height: chatH
        )
    }

    private func updateChatSize(w: CGFloat, h: CGFloat) {
        let notchCenterX = pillFrame.midX
        chatFrame = NSRect(
            x: notchCenterX - w / 2,
            y: pillFrame.minY - h - 8,
            width: w,
            height: h
        )
        if isExpanded {
            chatPanel.setFrame(chatFrame, display: true, animate: false)
        }
    }

    // MARK: - Panels

    private func makeBasePanel(frame: NSRect, styleMask: NSWindow.StyleMask) -> BagentPanel {
        let panel = BagentPanel(
            contentRect: frame,
            styleMask: styleMask,
            backing: .buffered,
            defer: false
        )
        panel.level = .statusBar
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = false
        panel.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        panel.isMovableByWindowBackground = false
        return panel
    }

    private func buildStatusPanel() {
        let panel = makeBasePanel(frame: pillFrame, styleMask: [.borderless, .nonactivatingPanel])
        let content = StatusPillView(
            isOnNotch: hasNotch,
            notchWidth: notchWidth,
            notchHeight: notchHeight,
            viewModel: chatViewModel,
            onTap: { [weak self] in self?.toggle() },
            onHoverChanged: { [weak self] hovering in self?.hoverChanged(isHovered: hovering) }
        )
        panel.contentView = NSHostingView(rootView: content)
        panel.orderFront(nil)
        self.statusPanel = panel
    }

    private func hoverChanged(isHovered: Bool) {
        guard hasNotch else { return }
        // Keep the AppKit window stable. Resizing this panel while SwiftUI also
        // animates the notch path can clip the bottom arcs into sharp corners.
        statusPanel.setFrame(pillFrame, display: true, animate: false)
    }

    private func buildChatPanel() {
        let panel = makeBasePanel(frame: chatFrame, styleMask: [.borderless, .nonactivatingPanel])
        let content = ChatPanelContent(
            viewModel: chatViewModel,
            onCollapse: { [weak self] in self?.collapse() }
        )
        let hostingView = NSHostingView(rootView: content)
        // Prevent mid-resize re-rasterization which causes text shake during drag.
        hostingView.wantsLayer = true
        hostingView.layerContentsRedrawPolicy = .onSetNeedsDisplay
        panel.contentView = hostingView
        // Stays hidden until expand() is called.
        self.chatPanel = panel
    }

    private func buildVoicePanel() {
        let panel = makeBasePanel(frame: voiceFrame, styleMask: [.borderless, .nonactivatingPanel])
        let content = VoiceOverlayView(
            speech: chatViewModel.speech,
            onCancel: { [weak self] in self?.dismissVoice() }
        )
        panel.contentView = NSHostingView(rootView: content)
        // Stays hidden until presentVoice() is called.
        self.voicePanel = panel
    }

    /// Voice overlay frame — centered on the notch, below the pill.
    private var voiceFrame: NSRect {
        let w: CGFloat = 360, h: CGFloat = 240
        let cx = pillFrame.midX
        return NSRect(x: cx - w / 2, y: pillFrame.minY - h - 8, width: w, height: h)
    }

    // MARK: - Voice overlay

    /// Open the voice-only overlay instantly (single ⌥Space when collapsed).
    func presentVoice() {
        guard !isExpanded, !isVoiceShowing else { return }
        isVoiceShowing = true
        previousApp = NSWorkspace.shared.frontmostApplication

        // On finalize (silence auto-stop), morph straight into the chat window.
        chatViewModel.speech.onFinalTranscript = { [weak self] text in
            self?.voiceToChatHandoff(text: text)
        }

        // Charge the notch, then pop the overlay (same timing as the chat panel).
        chatViewModel.pillHovered = true
        if hasNotch { hoverChanged(isHovered: true) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            guard let self, self.isVoiceShowing else { return }
            self.showVoicePanel()
        }

        Task { await chatViewModel.speech.startSession(mode: .overlay) }
    }

    private func showVoicePanel() {
        voicePanel.styleMask = [.borderless]
        voicePanel.hasShadow = true
        voicePanel.setFrame(voiceFrame, display: false)
        voicePanel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        voiceMouseMonitor = NSEvent.addGlobalMonitorForEvents(
            matching: [.leftMouseDown, .rightMouseDown]
        ) { [weak self] _ in
            guard let self else { return }
            let loc = NSEvent.mouseLocation
            Task { @MainActor [weak self] in
                guard let self else { return }
                if !self.voicePanel.frame.contains(loc) && !self.statusPanel.frame.contains(loc) {
                    self.dismissVoice()
                }
            }
        }
    }

    /// Cancel voice capture and hide the overlay (Escape / click-away).
    func dismissVoice() {
        guard isVoiceShowing else { return }
        chatViewModel.speech.cancel()
        teardownVoice(restoreApp: true)
    }

    /// Double ⌥Space: drop voice and open the chat window instead.
    func openChatFromVoice() {
        guard isVoiceShowing else { return }
        chatViewModel.speech.cancel()
        let original = previousApp
        teardownVoice(restoreApp: false)
        expand()
        previousApp = original   // expand() overwrote it; restore pre-voice target
    }

    private func voiceToChatHandoff(text: String) {
        guard isVoiceShowing else { return }
        let original = previousApp
        teardownVoice(restoreApp: false)
        expand()
        previousApp = original
        chatViewModel.voiceTurnActive = true   // re-arm voice once the reply finishes
        chatViewModel.submitTranscript(text)
    }

    private func teardownVoice(restoreApp: Bool) {
        isVoiceShowing = false
        if let m = voiceMouseMonitor { NSEvent.removeMonitor(m); voiceMouseMonitor = nil }
        voicePanel.styleMask = [.borderless, .nonactivatingPanel]
        voicePanel.hasShadow = false
        let appToRestore = restoreApp ? previousApp : nil
        if restoreApp { previousApp = nil }
        voicePanel.orderOut(nil)
        voicePanel.resignKey()
        if let appToRestore { appToRestore.activate(options: []) }
        let loc = NSEvent.mouseLocation
        if !statusPanel.frame.contains(loc) {
            chatViewModel.pillHovered = false
            if hasNotch { hoverChanged(isHovered: false) }
        }
    }

    // MARK: - Toggle

    func toggle() {
        isExpanded ? collapse() : expand()
    }

    func expand() {
        guard !isExpanded else { return }
        isExpanded = true

        // Save the app that was active before bagent takes focus
        previousApp = NSWorkspace.shared.frontmostApplication

        // Step 1 — animate notch to hover state so it "charges up" before the panel appears.
        chatViewModel.pillHovered = true
        if hasNotch { hoverChanged(isHovered: true) }

        // Step 2 — after hover spring mostly settles, pop the chat panel from the notch.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            guard let self, self.isExpanded else { return }
            self.showChatPanel()
        }
    }

    private func showChatPanel() {
        chatPanel.styleMask = [.borderless]
        chatPanel.hasShadow = true
        chatPanel.setFrame(chatFrame, display: false)
        chatPanel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        chatViewModel.isExpanded = true

        globalMouseMonitor = NSEvent.addGlobalMonitorForEvents(
            matching: [.leftMouseDown, .rightMouseDown]
        ) { [weak self] _ in
            guard let self else { return }
            let loc = NSEvent.mouseLocation
            Task { @MainActor [weak self] in
                guard let self else { return }
                if !self.chatPanel.frame.contains(loc) && !self.statusPanel.frame.contains(loc) {
                    self.collapse()
                }
            }
        }

        localKeyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            if event.keyCode == 53 { self?.collapse(); return nil }
            if event.modifierFlags.contains(.command) {
                let consumed: Bool
                switch event.keyCode {
                case 9:  consumed = NSApp.sendAction(#selector(NSText.paste(_:)),     to: nil, from: nil)
                case 8:  consumed = NSApp.sendAction(#selector(NSText.copy(_:)),      to: nil, from: nil)
                case 7:  consumed = NSApp.sendAction(#selector(NSText.cut(_:)),       to: nil, from: nil)
                case 0:  consumed = NSApp.sendAction(#selector(NSText.selectAll(_:)), to: nil, from: nil)
                case 6:
                    NSApp.keyWindow?.firstResponder?.undoManager?.undo()
                    consumed = true
                default: consumed = false
                }
                if consumed { return nil }
            }
            return event
        }
    }

    func collapse() {
        guard isExpanded else { return }
        isExpanded = false
        chatViewModel.isExpanded = false  // triggers SwiftUI spring-out animation

        if let m = localKeyMonitor    { NSEvent.removeMonitor(m); localKeyMonitor    = nil }
        if let m = globalMouseMonitor { NSEvent.removeMonitor(m); globalMouseMonitor = nil }
        chatPanel.styleMask = [.borderless, .nonactivatingPanel]

        // Hide chat panel after spring settles (~0.35 s), then contract notch back to idle.
        let appToRestore = previousApp
        previousApp = nil
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { [weak self] in
            guard let self else { return }
            self.chatPanel.hasShadow = false
            self.chatPanel.orderOut(nil)
            self.chatPanel.resignKey()
            // Restore focus to the app that was active before bagent opened
            appToRestore?.activate(options: [])
            // Contract notch back to idle only if mouse is no longer over the pill.
            let loc = NSEvent.mouseLocation
            if !self.statusPanel.frame.contains(loc) {
                self.chatViewModel.pillHovered = false
                if self.hasNotch { self.hoverChanged(isHovered: false) }
            }
        }
    }

    // MARK: - Screen changes

    @objc private func screensChanged() {
        computeGeometry()
        // Rebuild status panel so SwiftUI picks up new notchWidth/notchHeight.
        statusPanel.orderOut(nil)
        buildStatusPanel()
        statusPanel.setFrame(pillFrame, display: true)
        if isExpanded {
            chatPanel.setFrame(chatFrame, display: true)
        }
        if isVoiceShowing {
            voicePanel.setFrame(voiceFrame, display: true)
        }
    }
}
