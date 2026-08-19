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
final class BagentPanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        // Control+1–4 go through keyDown, not performKeyEquivalent — handled by localKeyMonitor.
        return super.performKeyEquivalent(with: event)
    }
}

@MainActor
final class NotchWindowController: NSObject {

    /// Always-visible pill that shows the label/status. The notch surface is the
    /// only UI: SwiftUI animates the visible shape inside a fixed AppKit frame to
    /// avoid resize clipping.
    private var statusPanel: BagentPanel!
    private let chatViewModel: ChatViewModel
    var isNotchInteractionShowing: Bool { chatViewModel.notchInteractionMode != .collapsed }
    private var hasNotch = false
    private var localKeyMonitor: Any?
    private var localMouseMonitor: Any?
    private var globalMouseMonitor: Any?
    /// Clipboard paste wheel (hold right ⌘).
    private let clipboardHistory = ClipboardHistory()
    private let pasteTap = PasteEventTap()
    /// Click-away monitor active while the wheel is pinned (mouse mode).
    private var wheelMouseMonitor: Any?
    /// Pending auto-dismiss of the transient cmux banner (5s after presenting).
    private var cmuxBannerDismissWork: DispatchWorkItem?

    private var pillFrame: NSRect = .zero
    private var notchWidth: CGFloat = 0
    private var notchHeight: CGFloat = 0
    /// The real bottom-of-menu-bar Y coordinate (screen space).
    private var menuBarBottomY: CGFloat = 0

    private var visibilityCancellable: AnyCancellable?
    private var previousApp: NSRunningApplication?

#if DEBUG
    var statusPanelForTesting: NSPanel { statusPanel }
#endif

    init(chatViewModel: ChatViewModel) {
        self.chatViewModel = chatViewModel
        super.init()
        computeGeometry()
        buildStatusPanel()
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(screensChanged),
            name: NSApplication.didChangeScreenParametersNotification,
            object: nil
        )

        chatViewModel.onInputOnlySubmitted = { [weak self] in
            self?.collapseInputForThinking()
        }
        // cmux agent event: transient banner in the collapsed notch for 5s, then
        // collapse back to the persistent left icon (still jiggling) + right corner
        // dot, which linger until the event clears. Latest event wins the banner.
        chatViewModel.onCmuxNotification = { [weak self] notification in
            guard let self else { return }
            guard self.chatViewModel.notchInteractionMode == .collapsed else { return }
            self.chatViewModel.cmuxBanner = notification
            self.cmuxBannerDismissWork?.cancel()
            let work = DispatchWorkItem { [weak self] in
                self?.chatViewModel.cmuxBanner = nil
            }
            self.cmuxBannerDismissWork = work
            DispatchQueue.main.asyncAfter(deadline: .now() + 5.0, execute: work)
        }

        visibilityCancellable = chatViewModel.notchPresentationPublisher
            .dropFirst()
            .receive(on: DispatchQueue.main)
            .sink { [weak self] _ in
                self?.reconcileStatusPanelVisibility()
            }

        setupFullscreenMonitoring()
        setupPasteWheel()
    }

    // MARK: - Clipboard paste wheel (hold right ⌘)

    static let pasteWheelEnabledKey = "pasteWheelEnabled"
    /// Default-on; the notch settings "general" page writes the same key.
    private var pasteWheelEnabled: Bool {
        UserDefaults.standard.object(forKey: Self.pasteWheelEnabledKey) == nil
            || UserDefaults.standard.bool(forKey: Self.pasteWheelEnabledKey)
    }

    private func setupPasteWheel() {
        clipboardHistory.start()

        pasteTap.canOpenWheel = { [weak self] in
            guard let self else { return false }
            // Never intercept pastes into bagent's own windows (settings, chat).
            let selfFrontmost = NSWorkspace.shared.frontmostApplication?.processIdentifier
                == ProcessInfo.processInfo.processIdentifier
            return self.pasteWheelEnabled
                && !selfFrontmost
                && self.chatViewModel.notchInteractionMode == .collapsed
                && !self.notchHiddenForFullscreen
                && !self.clipboardHistory.items.isEmpty
        }
        pasteTap.onWheelOpen = { [weak self] in self?.presentPasteWheel() }
        pasteTap.onDigit = { [weak self] slot in self?.commitPasteWheel(slot: slot) }
        pasteTap.onCommit = { [weak self] in self?.commitPasteWheel(slot: 0) }
        pasteTap.onCancel = { [weak self] in self?.dismissPasteWheel() }

        chatViewModel.onPasteWheelPinned = { [weak self] in self?.pinPasteWheel() }
        chatViewModel.onPasteWheelChipClicked = { [weak self] slot in
            self?.commitPasteWheel(slot: slot)
        }
        // A drag-out took the content with it — the OS drop is the delivery,
        // no paste.
        chatViewModel.onPasteWheelDragStarted = { [weak self] in self?.dismissPasteWheel() }

        pasteTap.start()
    }

    func presentPasteWheel() {
        guard !chatViewModel.pasteWheelActive else { return }
        chatViewModel.pasteWheelItems = clipboardHistory.items
        chatViewModel.pasteWheelFlashSlot = nil
        chatViewModel.pasteWheelActive = true
        // Panel must stay non-key: the paste target keeps focus the whole time.
        showStatusPanel()
    }

    func dismissPasteWheel() {
        pasteTap.reset()
        if let m = wheelMouseMonitor { NSEvent.removeMonitor(m); wheelMouseMonitor = nil }
        guard chatViewModel.pasteWheelActive else { return }
        chatViewModel.pasteWheelActive = false
        chatViewModel.pasteWheelFlashSlot = nil
    }

    /// Mouse entered the wheel — keyboard release no longer commits; click-away
    /// dismisses (Escape is handled by the event tap).
    private func pinPasteWheel() {
        guard chatViewModel.pasteWheelActive else { return }
        pasteTap.pin()
        guard wheelMouseMonitor == nil else { return }
        wheelMouseMonitor = NSEvent.addGlobalMonitorForEvents(
            matching: [.leftMouseDown, .rightMouseDown]
        ) { [weak self] _ in
            let loc = NSEvent.mouseLocation
            Task { @MainActor [weak self] in
                guard let self else { return }
                if !self.statusPanel.frame.contains(loc) { self.dismissPasteWheel() }
            }
        }
    }

    /// Paste the item in `slot`: promote + write to pasteboard, then a
    /// synthetic ⌘V to the frontmost app right away.
    private func commitPasteWheel(slot: Int) {
        guard chatViewModel.pasteWheelActive else { return }
        guard chatViewModel.pasteWheelItems.indices.contains(slot) else {
            dismissPasteWheel()
            return
        }
        let item = chatViewModel.pasteWheelItems[slot]
        chatViewModel.pasteWheelFlashSlot = slot
        clipboardHistory.promote(item)
        clipboardHistory.writeToPasteboard(item)
        PasteEventTap.postSyntheticPaste()

        // Chip flash reads for ~0.12s, then the wheel folds.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.12) { [weak self] in
            self?.dismissPasteWheel()
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
            onTap: { [weak self] in
                guard let self else { return }
                // Tapping retires the transient cmux banner (dot state persists).
                self.cmuxBannerDismissWork?.cancel()
                self.chatViewModel.cmuxBanner = nil
                self.isNotchInteractionShowing ? self.collapse() : self.presentInputOnly()
            },
            onHoverChanged: { [weak self] hovering in self?.hoverChanged(isHovered: hovering) }
        )
        panel.contentView = NSHostingView(rootView: content)
        self.statusPanel = panel
        reconcileStatusPanelVisibility()
    }

    private func hoverChanged(isHovered _: Bool) {
        // The panel already owns the maximum fixed frame. Even assigning that
        // same frame here forces NSHostingView layout and can re-enter SwiftUI
        // while a hover/update transaction is still being rendered.
    }

    private func resetNotchHoverState() {
        chatViewModel.pillHovered = false
        chatViewModel.notchHoverResetID = UUID()
        hoverChanged(isHovered: false)
    }

    /// Open the notch input surface (⌥Space when collapsed).
    func presentInputOnly() {
        guard chatViewModel.notchInteractionMode == .collapsed else { return }
        if chatViewModel.isThinking {
            presentOutputChat()
            return
        }
        previousApp = NSWorkspace.shared.frontmostApplication

        chatViewModel.applyNotchIntent(.openInput)
        hoverChanged(isHovered: true)

        statusPanel.styleMask = [.borderless]
        showStatusPanel(makeKey: true)
        NSApp.activate(ignoringOtherApps: true)
        installPanelMonitors()
    }

    /// Reveal streamed output in the notch (first assistant token).
    func presentOutputChat() {
        guard chatViewModel.notchInteractionMode != .output else { return }
        if previousApp == nil {
            previousApp = NSWorkspace.shared.frontmostApplication
        }

        chatViewModel.applyNotchIntent(.openOutput)
        hoverChanged(isHovered: true)

        statusPanel.styleMask = [.borderless]
        showStatusPanel()
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
                if !self.statusPanel.frame.contains(loc) {
                    self.collapse()
                }
            }
        }

        // Option+click anywhere in the notch panel copies the debug trace.
        localMouseMonitor = NSEvent.addLocalMonitorForEvents(matching: [.leftMouseDown]) { [weak self] event in
            guard let self,
                  event.modifierFlags.contains(.option),
                  event.window === self.statusPanel
            else { return event }
            self.chatViewModel.copyDebugTrace()
            return nil
        }

        localKeyMonitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) { [weak self] event in
            guard let self else { return event }
            if event.type == .flagsChanged {
                let forced = event.modifierFlags.intersection(.deviceIndependentFlagsMask).contains(.control)
                if self.chatViewModel.notchInteractionMode == .input {
                    self.chatViewModel.isSourcePickerForced = forced
                }
                return event
            }
            if event.keyCode == 53 {
                // Esc dismisses slash suggestions first, then steps back in
                // the automations surface, then exits response-history
                // browsing, then collapses the notch.
                if !self.chatViewModel.slashSuggestions.isEmpty {
                    self.chatViewModel.dismissSlashSuggestions()
                    return nil
                }
                if self.chatViewModel.notchInteractionMode == .automations,
                   self.chatViewModel.automationsGoBack() {
                    return nil
                }
                if self.chatViewModel.historyBrowseIndex != nil {
                    self.chatViewModel.exitHistoryBrowse()
                    return nil
                }
                self.collapse()
                return nil
            }
            // Automations: ↑/↓ select a list row, Return opens the detail or
            // advances the editor one step (keyboard equivalent of "Ďalej").
            if self.chatViewModel.notchInteractionMode == .automations {
                switch event.keyCode {
                case 123:
                    if self.chatViewModel.automationsGoBack() { return nil }
                case 124:
                    if self.chatViewModel.openSelectedAutomationDetail() { return nil }
                case 126: if self.chatViewModel.moveAutomationsSelection(by: -1) { return nil }
                case 125: if self.chatViewModel.moveAutomationsSelection(by: 1) { return nil }
                case 36:
                    if self.chatViewModel.openSelectedAutomationDetail() { return nil }
                    switch self.chatViewModel.automationsSurface {
                    case .editorTask, .editorSchedule, .editorRecurrence, .editorReview:
                        self.chatViewModel.automationEditorNext()
                        return nil
                    default:
                        break
                    }
                default: break
                }
            }
            // Slash-command suggestions consume ↑/↓ (selection), Tab and
            // Return (accept) while visible.
            if !self.chatViewModel.slashSuggestions.isEmpty {
                switch event.keyCode {
                case 126: if self.chatViewModel.moveSlashSelection(by: -1) { return nil }
                case 125: if self.chatViewModel.moveSlashSelection(by: 1) { return nil }
                case 48, 36:
                    if self.chatViewModel.acceptSlashSuggestion() { return nil }
                default: break
                }
            }
            // ←/→ switch settings pages while the /settings surface is open.
            if self.chatViewModel.notchInteractionMode == .settings {
                if event.keyCode == 123 {
                    self.chatViewModel.notchSettingsPage = self.chatViewModel.notchSettingsPage.previous
                    return nil
                }
                if event.keyCode == 124 {
                    self.chatViewModel.notchSettingsPage = self.chatViewModel.notchSettingsPage.next
                    return nil
                }
            }
            // ↑/↓ on empty notch input browse past assistant responses.
            // Arrow keys always carry .function + .numericPad — ignore those.
            if event.modifierFlags.intersection(.deviceIndependentFlagsMask)
                .subtracting([.function, .numericPad]).isEmpty {
                if event.keyCode == 126, self.chatViewModel.browseOlderResponse() { return nil }
                if event.keyCode == 125, self.chatViewModel.browseNewerResponse() { return nil }
            }
            // Any printable key while browsing returns to the input and types it.
            if self.chatViewModel.historyBrowseIndex != nil,
               !event.modifierFlags.contains(.command),
               let chars = event.characters,
               chars.rangeOfCharacter(from: .alphanumerics.union(.punctuationCharacters).union(.symbols).union(.whitespaces)) != nil {
                self.chatViewModel.exitHistoryBrowse()
                self.chatViewModel.inputText += chars
                return nil
            }
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
        guard chatViewModel.notchInteractionMode == .input else { return false }
        let modes = Array(chatViewModel.topSourceModes.prefix(4))
        guard modes.indices.contains(index) else { return false }
        let mode = modes[index]
        chatViewModel.selectedSourceMode = chatViewModel.selectedSourceMode == mode ? nil : mode
        chatViewModel.hoveredSourceMode = nil
        return true
    }

    private func collapseInputForThinking() {
        guard chatViewModel.notchInteractionMode == .input else { return }
        chatViewModel.applyNotchIntent(.collapse)
        reconcileStatusPanelVisibility()
    }

    func collapse() {
        guard isNotchInteractionShowing else { return }
        chatViewModel.applyNotchIntent(.collapse)
        chatViewModel.isSourcePickerForced = false
        resetNotchHoverState()

        if let m = localKeyMonitor    { NSEvent.removeMonitor(m); localKeyMonitor    = nil }
        if let m = localMouseMonitor  { NSEvent.removeMonitor(m); localMouseMonitor  = nil }
        if let m = globalMouseMonitor { NSEvent.removeMonitor(m); globalMouseMonitor = nil }
        statusPanel.styleMask = [.borderless, .nonactivatingPanel]
        reconcileStatusPanelVisibility()

        // Restore focus to the app that was active before bagent opened, once the
        // notch spring-out has settled.
        let appToRestore = previousApp
        previousApp = nil
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.22) {
            appToRestore?.activate(options: [])
        }
    }

    // MARK: - Fullscreen detection (hide notch over fullscreen video)

    private var fullscreenPollTimer: Timer?
    /// Tracks last known hide state to avoid redundant show/hide calls.
    private var notchHiddenForFullscreen = false

    private var statusPanelAllowedOverFullscreen: Bool {
        switch chatViewModel.notchInteractionMode {
        case .input, .output, .settings, .automations:
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
        checkCmuxSeen()
        let shouldHide = isExternalFullscreenActive()
        guard shouldHide != notchHiddenForFullscreen else {
            reconcileStatusPanelVisibility()
            return
        }
        notchHiddenForFullscreen = shouldHide

        if shouldHide {
            if chatViewModel.pasteWheelActive { dismissPasteWheel() }
            if chatViewModel.notchInteractionMode != .output, isNotchInteractionShowing {
                collapse()
            }
        }
        reconcileStatusPanelVisibility()
    }

    /// Auto-clear cmux cues the user has actually looked at: if cmux is frontmost
    /// and the notified workspace is the one selected on screen, treat it as seen
    /// and fly it off. Same predicate as delivery-time suppression
    /// (`ChatViewModel.handleCmuxEvent`). Cheap — only runs while cues are pending.
    private func checkCmuxSeen() {
        guard !chatViewModel.cmuxPending.isEmpty,
              CmuxEventMonitor.isCmuxFrontmost() else { return }
        Task { @MainActor [weak self] in
            guard let self else { return }
            let visible = await CmuxEventMonitor.visibleWorkspaceIds()
            let seen = Set(self.chatViewModel.cmuxPending.map(\.workspaceId)).intersection(visible)
            for workspaceId in seen {
                self.chatViewModel.markCmuxSeen(workspaceId: workspaceId)
            }
        }
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
    }
}
