import AppKit
import Combine
import SwiftUI

private func sourceModeCommandDigitIndex(for event: NSEvent) -> Int? {
    let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
    guard flags.contains(.control),
          !flags.contains(.shift),
          !flags.contains(.option),
          !flags.contains(.command)
    else { return nil }

    switch event.keyCode {
    case 18: return 0
    case 19: return 1
    case 20: return 2
    case 21: return 3
    default: return nil
    }
}

// Borderless NSPanel by default returns canBecomeKey = false, which silently
// prevents makeKeyAndOrderFront from making the panel a key window, so keyboard
// events never reach the text field. Subclass to fix.
private final class BagentPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        // Control+1–4 go through keyDown, not performKeyEquivalent — handled by localKeyMonitor.
        return super.performKeyEquivalent(with: event)
    }
}

@MainActor
final class NotchWindowController: NSObject {

    /// Always-visible pill that shows the label/status.
    /// On notch displays the window frame stays at max voice size; SwiftUI
    /// animates the visible shape inside it to avoid AppKit resize clipping.
    private var statusPanel: BagentPanel!
    /// The expandable chat sheet — appears below the pill, hidden when collapsed.
    private var chatPanel: BagentPanel!
    private let chatViewModel: ChatViewModel
    private let legacySpotlightEnabled = false
    private let legacyAutoChatEnabled = false
    private(set) var isExpanded = false
    private(set) var isInputShowing = false
    private(set) var isVoiceShowing = false
    var isNotchInteractionShowing: Bool { chatViewModel.notchInteractionMode != .collapsed }
    var isVoiceModeEnabled: Bool { chatViewModel.voiceModeEnabled }
    private var hasNotch = false
    private let usesNotchSurface = true
    private var localKeyMonitor: Any?
    private var globalMouseMonitor: Any?
    /// Monitors used for the inline voice surface (click-away + Escape).
    private var voiceMouseMonitor: Any?
    private var voiceKeyMonitor: Any?
    /// Pending auto-dismiss of the transient cmux banner (5s after presenting).
    private var cmuxBannerDismissWork: DispatchWorkItem?

    private var pillFrame: NSRect = .zero
    private var chatFrame: NSRect = .zero
    private var inputFrame: NSRect = .zero
    private var notchWidth: CGFloat = 0
    private var notchHeight: CGFloat = 0
    /// The real bottom-of-menu-bar Y coordinate (screen space).
    /// Used to anchor the chat panel independently of the oversized voice pill frame.
    private var menuBarBottomY: CGFloat = 0

    /// Y below which the chat panel should start — accounts for the hover bridge
    /// (22 pt) that hangs below the menu bar when the notch is expanded.
    private var chatAnchorY: CGFloat {
        usesNotchSurface
            ? menuBarBottomY - NotchWrapMetrics.hoverBridgeHeight
            : menuBarBottomY
    }
    private var sizeCancellable: AnyCancellable?
    private var visibilityCancellable: AnyCancellable?
    private var previousApp: NSRunningApplication?

    init(chatViewModel: ChatViewModel) {
        self.chatViewModel = chatViewModel
        super.init()
        computeGeometry()
        buildStatusPanel()
        buildChatPanel()
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
            guard self.chatViewModel.voiceModeEnabled else { return }
            // Give the user time to read/absorb the response before re-opening voice.
            // Delay is proportional to response word count (≈150 WPM reading pace),
            // clamped to a sensible range so very short or very long replies don't
            // feel awkward.
            let readDelay = self.chatViewModel.voiceTurnResumeDelay()
            self.collapse()
            DispatchQueue.main.asyncAfter(deadline: .now() + readDelay) { [weak self] in
                guard let self, self.chatViewModel.voiceModeEnabled else { return }
                self.presentVoice()
            }
        }
        chatViewModel.onVoiceModeDisabled = { [weak self] in
            guard let self else { return }
            if self.isVoiceShowing {
                self.teardownVoiceNotch(restoreApp: true)
            }
        }
        chatViewModel.onInputOnlySubmitted = { [weak self] in
            self?.collapseInputForThinking()
        }
        chatViewModel.onFirstAssistantToken = { [weak self] in
            self?.presentOutputChat()
        }

        // Silent background action: show confirmation in notch for 2.5s then collapse.
        chatViewModel.onVoiceActionTaken = { [weak self] _ in
            guard let self else { return }
            if !self.isVoiceShowing {
                self.presentInlineOutput()
                return
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 2.5) { [weak self] in
                guard let self else { return }
                self.chatViewModel.voiceActionMessage = nil
                self.teardownVoiceNotch(restoreApp: true)
            }
        }

        // cmux agent event: transient banner in the collapsed notch for 5s, then
        // collapse back to the persistent left icon (still jiggling) + right corner
        // dot, which linger until the event clears. Latest event wins the banner.
        chatViewModel.onCmuxNotification = { [weak self] notification in
            guard let self else { return }
            guard !self.isExpanded, !self.isVoiceShowing,
                  self.chatViewModel.notchInteractionMode == .collapsed else { return }
            self.chatViewModel.cmuxBanner = notification
            self.cmuxBannerDismissWork?.cancel()
            let work = DispatchWorkItem { [weak self] in
                self?.chatViewModel.cmuxBanner = nil
            }
            self.cmuxBannerDismissWork = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 5.0, execute: work)
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

        visibilityCancellable = chatViewModel.$notchInteractionMode
            .dropFirst()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in
                self?.reconcileStatusPanelVisibility()
            }

        setupFullscreenMonitoring()
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
            let visibleMenuBarH = max(0, screen.frame.maxY - screen.visibleFrame.maxY)
            menuBarH = max(NSStatusBar.system.thickness, visibleMenuBarH)
            menuBarBottomY = screen.frame.maxY - menuBarH
            notchWidth  = NotchWrapMetrics.syntheticNotchWidth
            notchHeight = menuBarH
        }
        self.menuBarBottomY = menuBarBottomY

        chatViewModel.hasNotch = hasNotch

        // pillFrame sized for voice mode (widest/tallest state) so AppKit frame
        // never needs resizing — SwiftUI animates the visible shape within it.
        // On external/non-notch displays this creates a centered fake notch wrap
        // whose top edge is flush with the menu-bar/screen top and expands down.
        let totalW = 2 * NotchWrapMetrics.maxWingWidth + notchWidth
        let totalH = menuBarH + NotchWrapMetrics.maxBridgeHeight
        pillFrame = NSRect(
            x: notchCenterX - totalW / 2,
            y: menuBarBottomY - NotchWrapMetrics.maxBridgeHeight,
            width: totalW,
            height: totalH
        )

        // Chat panel drops from below the hover-expanded notch bridge.
        // chatAnchorY = menuBarBottomY - hoverBridgeHeight for notch-style surfaces.
        let chatW = chatViewModel.chatWindowW
        let chatH = chatViewModel.chatWindowH
        let chatGap: CGFloat = 8
        chatFrame = NSRect(
            x: notchCenterX - chatW / 2,
            y: chatAnchorY - chatH - chatGap,
            width: chatW,
            height: chatH
        )

        let inputW = min(820, max(640, screen.frame.width * 0.42))
        // Extra height gives the glass shadow room to render without clipping at the
        // transparent window edge (especially relevant for the wider, softer Liquid Glass
        // shadow on macOS 26).
        let inputH: CGFloat = 96
        inputFrame = NSRect(
            x: notchCenterX - inputW / 2,
            y: chatAnchorY - inputH - 12,
            width: inputW,
            height: inputH
        )
    }

    private func updateChatSize(w: CGFloat, h: CGFloat) {
        let notchCenterX = pillFrame.midX
        chatFrame = NSRect(
            x: notchCenterX - w / 2,
            y: chatAnchorY - h - 8,
            width: w,
            height: h
        )
        if isExpanded {
            chatPanel.setFrame(chatFrame, display: true, animate: false)
        } else if isInputShowing {
            chatPanel.setFrame(inputFrame, display: true, animate: false)
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
            notchWidth: notchWidth,
            notchHeight: notchHeight,
            viewModel: chatViewModel,
            onTap: { [weak self] in self?.toggle() },
            onHoverChanged: { [weak self] hovering in self?.hoverChanged(isHovered: hovering) }
        )
        panel.contentView = NSHostingView(rootView: content)
        self.statusPanel = panel
        reconcileStatusPanelVisibility()
    }

    private func hoverChanged(isHovered: Bool) {
        guard usesNotchSurface else { return }
        // Keep the AppKit window stable. Resizing this panel while SwiftUI also
        // animates the notch path can clip the bottom arcs into sharp corners.
        statusPanel.setFrame(pillFrame, display: true, animate: false)
    }

    private func resetNotchHoverState() {
        chatViewModel.pillHovered = false
        chatViewModel.notchHoverResetID = UUID()
        if usesNotchSurface { hoverChanged(isHovered: false) }
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

    // MARK: - Voice mode

    /// Open voice mode (single ⌥Space when collapsed).
    ///
    /// Expands the notch/fake-notch bridge shape and renders `VoiceNotchContent` inside it.
    func presentVoice() {
        guard chatViewModel.voiceModeEnabled else { return }
        guard !isExpanded, !isVoiceShowing else { return }
        isVoiceShowing = true
        previousApp = NSWorkspace.shared.frontmostApplication

        chatViewModel.speech.onFinalTranscript = { [weak self] text in
            self?.voiceToChatHandoff(text: text)
        }

        // Signal SwiftUI so the pill icon/label react in both display types.
        chatViewModel.isVoiceNotchActive = true

        chatViewModel.pillHovered = true
        hoverChanged(isHovered: true)
        voiceMouseMonitor = NSEvent.addGlobalMonitorForEvents(
            matching: [.leftMouseDown, .rightMouseDown]
        ) { [weak self] _ in
            guard let self else { return }
            let loc = NSEvent.mouseLocation
            Task { @MainActor [weak self] in
                guard let self else { return }
                if !self.statusPanel.frame.contains(loc) { self.dismissVoice() }
            }
        }
        voiceKeyMonitor = NSEvent.addGlobalMonitorForEvents(matching: .keyDown) { [weak self] event in
            if event.keyCode == 53 {
                Task { @MainActor [weak self] in self?.dismissVoice() }
            }
        }

        Task { await chatViewModel.speech.startSession(mode: .overlay) }
    }

    /// Cancel voice capture and collapse the notch back to idle.
    func dismissVoice() {
        guard isVoiceShowing else { return }
        chatViewModel.speech.cancel()
        teardownVoiceNotch(restoreApp: true)
    }

    /// Double ⌥Space: drop voice and open the chat window instead.
    func openChatFromVoice() {
        guard isVoiceShowing else { return }
        chatViewModel.speech.cancel()
        let original = previousApp
        teardownVoiceNotch(restoreApp: false)
        presentInputOnly()
        previousApp = original
    }

    private func voiceToChatHandoff(text: String) {
        guard isVoiceShowing else { return }
        guard chatViewModel.voiceModeEnabled else {
            teardownVoiceNotch(restoreApp: true)
            return
        }
        if isStopIntent(text) {
            teardownVoiceNotch(restoreApp: true)
            return
        }
        let savedApp = previousApp
        teardownVoiceNotch(restoreApp: false)
        // Brief pause so voice content fades before chat begins expanding.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { [weak self] in
            guard let self else { return }
            self.previousApp = savedApp
            self.chatViewModel.voiceTurnActive = true
            self.chatViewModel.submitTranscript(text)
        }
    }

    private func teardownVoiceNotch(restoreApp: Bool) {
        guard isVoiceShowing else { return }
        isVoiceShowing = false
        chatViewModel.isVoiceNotchActive = false   // triggers pill icon/label reset in SwiftUI

        let app = restoreApp ? previousApp : nil
        if restoreApp { previousApp = nil }

        // Remove click-away / Escape monitors added in presentVoice.
        if let m = voiceMouseMonitor { NSEvent.removeMonitor(m); voiceMouseMonitor = nil }
        if let m = voiceKeyMonitor   { NSEvent.removeMonitor(m); voiceKeyMonitor   = nil }
        // After content fades (150 ms), contract notch and restore focus.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.20) { [weak self] in
            guard let self else { return }
            if !self.isExpanded {
                self.chatViewModel.pillHovered = false
                self.hoverChanged(isHovered: false)
            }
            app?.activate(options: [])
        }
    }

    // MARK: - Stop intent detection

    private static let stopPatterns: [String] = [
        "thank you", "thanks", "that's it", "that's all", "that is it",
        "that is all", "stop", "goodbye", "bye", "we're done", "you can stop",
        "ok thanks", "okay thanks", "all good", "got it thanks",
        "ďakujem", "to je všetko", "stačí", "skončime",
        "dobre ďakujem", "dosť", "koniec", "dobré ďakujem",
    ]

    private func isStopIntent(_ text: String) -> Bool {
        let lower = text.lowercased().trimmingCharacters(in: .whitespacesAndNewlines)
        let wordCount = lower.split(separator: " ").count
        guard wordCount < 8 else { return false }
        return Self.stopPatterns.contains { lower.contains($0) }
    }

    // MARK: - Toggle

    func toggle() {
        // Opening chat retires the transient cmux banner (dot state persists).
        cmuxBannerDismissWork?.cancel()
        chatViewModel.cmuxBanner = nil
        (isExpanded || isInputShowing || isNotchInteractionShowing) ? collapse() : presentInputOnly()
    }

    func expand() {
        presentOutputChat()
    }

    func presentInputOnly() {
        if !legacySpotlightEnabled {
            presentInlineInput()
            return
        }
        guard !isExpanded, !isInputShowing else { return }
        if chatViewModel.isThinking {
            presentOutputChat()
            return
        }
        isInputShowing = true

        previousApp = NSWorkspace.shared.frontmostApplication

        chatViewModel.pillHovered = true
        if usesNotchSurface { hoverChanged(isHovered: true) }

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            guard let self, self.isInputShowing else { return }
            self.showInputPanel()
        }
    }

    func presentOutputChat() {
        if !legacyAutoChatEnabled {
            presentInlineOutput()
            return
        }
        guard !isExpanded else { return }
        isInputShowing = false
        isExpanded = true

        // Save the app that was active before bagent takes focus
        if previousApp == nil {
            previousApp = NSWorkspace.shared.frontmostApplication
        }

        // Step 1 — animate notch to hover state so it "charges up" before the panel appears.
        chatViewModel.pillHovered = true
        if usesNotchSurface { hoverChanged(isHovered: true) }

        // Step 2 — after hover spring mostly settles, pop the chat panel from the notch.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.15) { [weak self] in
            guard let self, self.isExpanded else { return }
            self.showChatPanel()
        }
    }

    private func presentInlineInput() {
        guard !isExpanded, !isInputShowing, chatViewModel.notchInteractionMode == .collapsed else { return }
        if chatViewModel.isThinking {
            presentInlineOutput()
            return
        }
        isInputShowing = true
        previousApp = NSWorkspace.shared.frontmostApplication

        chatViewModel.isExpanded = true
        chatViewModel.chatSurfaceMode = .inputOnly
        chatViewModel.notchInteractionMode = .input
        if usesNotchSurface { hoverChanged(isHovered: true) }

        statusPanel.styleMask = [.borderless]
        showStatusPanel(makeKey: true)
        NSApp.activate(ignoringOtherApps: true)
        installPanelMonitors()
    }

    private func presentInlineOutput() {
        guard chatViewModel.notchInteractionMode != .output else { return }
        isInputShowing = false
        isExpanded = true
        if previousApp == nil {
            previousApp = NSWorkspace.shared.frontmostApplication
        }

        chatViewModel.isExpanded = true
        chatViewModel.chatSurfaceMode = .outputExpanded
        chatViewModel.notchInteractionMode = .output
        if usesNotchSurface { hoverChanged(isHovered: true) }

        statusPanel.styleMask = [.borderless]
        showStatusPanel()
        installPanelMonitors()
    }

    private func showInputPanel() {
        chatPanel.styleMask = [.borderless]
        // On macOS 26+ the Liquid Glass surface renders its own shadow; disabling the
        // AppKit window shadow avoids a muddy double-shadow beneath the spotlight bar.
        // The expanded chat panel (showChatPanel) keeps hasShadow = true unchanged.
        if #available(macOS 26, *) {
            chatPanel.hasShadow = false
        } else {
            chatPanel.hasShadow = true
        }
        chatPanel.setFrame(inputFrame, display: false)
        chatViewModel.isExpanded = false
        chatViewModel.chatSurfaceMode = .inputOnly
        chatPanel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        installPanelMonitors()
    }

    private func showChatPanel() {
        chatPanel.styleMask = [.borderless]
        chatPanel.hasShadow = true
        chatPanel.setFrame(chatFrame, display: false)
        chatPanel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        chatViewModel.isExpanded = true
        chatViewModel.chatSurfaceMode = .outputExpanded
        installPanelMonitors()
    }

    private func installPanelMonitors() {
        if localKeyMonitor != nil || globalMouseMonitor != nil { return }
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

        localKeyMonitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) { [weak self] event in
            guard let self else { return event }
            if event.type == .flagsChanged {
                let forced = event.modifierFlags.intersection(.deviceIndependentFlagsMask).contains(.control)
                if self.isInputShowing {
                    self.chatViewModel.isSourcePickerForced = forced
                }
                return event
            }
            if event.keyCode == 53 { self.collapse(); return nil }
            if let index = sourceModeCommandDigitIndex(for: event),
               self.selectVisibleSourceMode(at: index) {
                return nil
            }
            if event.modifierFlags.contains(.command) {
                let consumed: Bool
                switch event.keyCode {
                case 9:
                    // Intercept ⌘V: if the pasteboard has an image, paste it as an
                    // attachment and insert [image #n] token rather than raw-pasting bytes.
                    if self.chatViewModel.pasteImageFromClipboard() == true {
                        consumed = true
                    } else {
                        consumed = NSApp.sendAction(#selector(NSText.paste(_:)), to: nil, from: nil)
                    }
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

    private func selectVisibleSourceMode(at index: Int) -> Bool {
        guard isInputShowing || chatViewModel.notchInteractionMode == .input else { return false }
        let modes = Array(chatViewModel.topSourceModes.prefix(4))
        guard modes.indices.contains(index) else { return false }
        let mode = modes[index]
        chatViewModel.selectedSourceMode = chatViewModel.selectedSourceMode == mode ? nil : mode
        chatViewModel.hoveredSourceMode = nil
        return true
    }

    private func collapseInputForThinking() {
        if !legacySpotlightEnabled {
            guard isInputShowing || chatViewModel.notchInteractionMode == .input else { return }
            isInputShowing = false
            chatViewModel.chatSurfaceMode = .thinkingHidden
            chatViewModel.notchInteractionMode = .thinking
            reconcileStatusPanelVisibility()
            return
        }
        guard isInputShowing else { return }
        isInputShowing = false
        chatViewModel.chatSurfaceMode = .thinkingHidden
        if let m = localKeyMonitor { NSEvent.removeMonitor(m); localKeyMonitor = nil }
        if let m = globalMouseMonitor { NSEvent.removeMonitor(m); globalMouseMonitor = nil }
        chatPanel.styleMask = [.borderless, .nonactivatingPanel]

        DispatchQueue.main.asyncAfter(deadline: .now() + 0.24) { [weak self] in
            guard let self, !self.isInputShowing, !self.isExpanded else { return }
            self.chatPanel.hasShadow = false
            self.chatPanel.orderOut(nil)
            self.chatPanel.resignKey()
            self.previousApp?.activate(options: [])
        }
    }

    func collapse() {
        guard isExpanded || isInputShowing || isNotchInteractionShowing else { return }
        let wasInputOnly = isInputShowing
        isExpanded = false
        isInputShowing = false
        chatViewModel.isExpanded = false  // triggers SwiftUI spring-out animation
        chatViewModel.chatSurfaceMode = .collapsed
        chatViewModel.notchInteractionMode = .collapsed
        chatViewModel.isSourcePickerForced = false
        resetNotchHoverState()

        if let m = localKeyMonitor    { NSEvent.removeMonitor(m); localKeyMonitor    = nil }
        if let m = globalMouseMonitor { NSEvent.removeMonitor(m); globalMouseMonitor = nil }
        chatPanel.styleMask = [.borderless, .nonactivatingPanel]
        statusPanel.styleMask = [.borderless, .nonactivatingPanel]
        reconcileStatusPanelVisibility()

        // Hide chat panel after spring settles (~0.35 s), then contract notch back to idle.
        let appToRestore = previousApp
        previousApp = nil
        DispatchQueue.main.asyncAfter(deadline: .now() + (wasInputOnly ? 0.22 : 0.35)) { [weak self] in
            guard let self else { return }
            self.chatPanel.hasShadow = false
            self.chatPanel.orderOut(nil)
            self.chatPanel.resignKey()
            // Restore focus to the app that was active before bagent opened
            appToRestore?.activate(options: [])
        }
    }

    // MARK: - Fullscreen detection (hide notch over fullscreen video)

    private var fullscreenPollTimer: Timer?
    /// Tracks last known hide state to avoid redundant show/hide calls.
    private var notchHiddenForFullscreen = false

    private var statusPanelAllowedOverFullscreen: Bool {
        switch chatViewModel.notchInteractionMode {
        case .input, .output:
            return true
        case .collapsed, .thinking:
            return false
        }
    }

    private var shouldShowStatusPanel: Bool {
        !notchHiddenForFullscreen || statusPanelAllowedOverFullscreen
    }

    private func showStatusPanel(makeKey: Bool = false) {
        guard shouldShowStatusPanel else {
            if statusPanel.isVisible { statusPanel.orderOut(nil) }
            return
        }
        if makeKey {
            statusPanel.makeKeyAndOrderFront(nil)
        } else if !statusPanel.isVisible {
            statusPanel.orderFront(nil)
        }
    }

    private func reconcileStatusPanelVisibility() {
        showStatusPanel()
    }

    private func setupFullscreenMonitoring() {
        let wsnc = NSWorkspace.shared.notificationCenter
        // Space switch: entering/exiting fullscreen that creates a new Space.
        wsnc.addObserver(self, selector: #selector(fullscreenEvent),
                         name: NSWorkspace.activeSpaceDidChangeNotification, object: nil)
        // App activation: catching cases where the same app re-activates.
        wsnc.addObserver(self, selector: #selector(fullscreenEvent),
                         name: NSWorkspace.didActivateApplicationNotification, object: nil)

        // Polling at 0.8 s catches inline fullscreen (Safari F-key, Netflix, etc.)
        // where no Space change or app switch notification fires. CGWindowListCopyWindowInfo
        // is fast enough (~0.1 ms) that 0.8 s polling adds negligible CPU.
        fullscreenPollTimer = Timer.scheduledTimer(withTimeInterval: 0.8, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in self?.updateNotchVisibility() }
        }
        fullscreenPollTimer?.tolerance = 0.2   // allow coalescing
    }

    @objc private func fullscreenEvent() {
        // Small delay so the window list settles after the Space transition animation.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25) { [weak self] in
            self?.updateNotchVisibility()
        }
    }

    private func updateNotchVisibility() {
        let shouldHide = isExternalFullscreenActive()
        guard shouldHide != notchHiddenForFullscreen else {
            reconcileStatusPanelVisibility()
            return
        }
        notchHiddenForFullscreen = shouldHide

        if shouldHide {
            if isVoiceShowing { dismissVoice() }
            if chatViewModel.notchInteractionMode != .output,
               isExpanded || isInputShowing || isNotchInteractionShowing {
                collapse()
            }
        }
        reconcileStatusPanelVisibility()
    }

    /// Returns true when a fullscreen video/app is covering this screen.
    ///
    /// **Detection strategy:** scan CGWindowList for a window from another process
    /// at layer 0 whose bounds cover the full screen (`Y ≈ 0`, meaning it reaches
    /// above the menu bar). This is the only signal that works for *all* fullscreen
    /// forms:
    ///
    /// - **Native fullscreen** (green button): creates a new Space and hides the menu
    ///   bar (`visibleFrame.maxY == frame.maxY`), AND the covering window reaches Y=0.
    /// - **HTML5 / F-key fullscreen** (Netflix, YouTube): draws a screen-covering
    ///   window over the *current* desktop Space — `visibleFrame` is unchanged (the
    ///   menu-bar space is still reported as reserved) so a `visibleFrame`-based check
    ///   never fires. The covering window still reaches Y=0.
    ///
    /// A tiling WM window (AeroSpace / Amethyst) that fills `frame` but leaves the
    /// menu bar visible has `Y ≈ menuBarHeight` (not 0) and fails the predicate, so
    /// there are no false positives for maximised-but-not-fullscreen windows.
    private func isExternalFullscreenActive() -> Bool {
        guard let screen = NSScreen.main else { return false }

        // Build the CG coordinate rect for this screen.
        // (AppKit uses bottom-left origin; CG uses top-left of the primary screen.)
        let primaryH = NSScreen.screens.first?.frame.height ?? screen.frame.height
        let cgScreen = CGRect(
            x: screen.frame.minX,
            y: primaryH - screen.frame.maxY,   // AppKit bottom-left → CG top-left
            width:  screen.frame.width,
            height: screen.frame.height
        )

        let ourPID = Int32(ProcessInfo.processInfo.processIdentifier)
        guard let list = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID)
                as? [[String: Any]] else { return false }

        var matched = false
        for info in list {
            // kCGWindowOwnerPID is absent when Screen Recording permission is not granted.
            // Bounds and layer are always available. Our own panels sit at layer 25
            // (NSWindow.Level.statusBar), so skipping the PID check is safe — they
            // won't satisfy layer == 0.
            let pid   = info[kCGWindowOwnerPID as String] as? Int32
            if pid == ourPID { continue }   // skip our own windows if PID is available

            guard let layer = info[kCGWindowLayer  as String] as? Int,  layer == 0,
                  let bd    = info[kCGWindowBounds as String] as? [String: Any],
                  let wx    = bd["X"] as? CGFloat, let wy = bd["Y"] as? CGFloat,
                  let ww    = bd["Width"] as? CGFloat, let wh = bd["Height"] as? CGFloat
            else { continue }

            // Safari (and other browsers) in fullscreen leave the notch safe-area
            // (~38-39 px) uncovered at the top, so the window starts at y≈38 with
            // height≈1131 instead of spanning the full 1169px.
            // AeroSpace-tiled windows have a small gap at the BOTTOM (y+h≈1163, not 1169).
            // Detecting by bottom-edge reach (y+h ≈ screenH) cleanly separates the two.
            let windowBottom = wy + wh
            if ww >= cgScreen.width - 2 &&
               windowBottom >= cgScreen.maxY - 2 &&   // reaches screen bottom
               wx <= cgScreen.minX + 3 &&             // starts at left edge
               wy <= cgScreen.minY + 50 {             // starts near top (notch safe area ≤ ~39px)
                matched = true
                break
            }
        }
        return matched
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
        } else if isInputShowing {
            chatPanel.setFrame(inputFrame, display: true)
        }
    }
}
