import AppKit
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

    /// Always-visible pill that shows the label/status — never moves.
    private var statusPanel: BagentPanel!
    /// The expandable chat sheet — appears below the pill, hidden when collapsed.
    private var chatPanel: BagentPanel!
    private let chatViewModel: ChatViewModel
    private(set) var isExpanded = false
    private var hasNotch = false
    private var localKeyMonitor: Any?
    private var globalMouseMonitor: Any?

    private var pillFrame: NSRect = .zero
    private var chatFrame: NSRect = .zero
    private var notchWidth: CGFloat = 0
    private var notchHeight: CGFloat = 0

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

        // Chat panel starts immediately below the pill — no overlap with the status bar.
        let chatW: CGFloat = 400
        let chatH: CGFloat = 520
        chatFrame = NSRect(
            x: notchCenterX - chatW / 2,
            y: pillFrame.minY - chatH,
            width: chatW,
            height: chatH
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
            isOnNotch: hasNotch,
            notchWidth: notchWidth,
            notchHeight: notchHeight,
            onTap: { [weak self] in self?.toggle() },
            onHoverChanged: { [weak self] hovering in self?.hoverChanged(isHovered: hovering) }
        )
        panel.contentView = NSHostingView(rootView: content)
        panel.orderFront(nil)
        self.statusPanel = panel
    }

    private func hoverChanged(isHovered: Bool) {
        guard hasNotch else { return }
        // Expand the status panel frame to show the bridge on hover, shrink on idle.
        let screen = NSScreen.main
        let newBridge = isHovered ? NotchWrapMetrics.hoverBridgeHeight : NotchWrapMetrics.idleBridgeHeight
        let totalW = 2 * NotchWrapMetrics.hoverWingWidth + notchWidth
        let totalH = notchHeight + newBridge
        let newFrame = NSRect(
            x: pillFrame.midX - totalW / 2,
            y: (screen?.frame.maxY ?? pillFrame.maxY + notchHeight) - notchHeight - newBridge,
            width: totalW,
            height: totalH
        )
        NSAnimationContext.runAnimationGroup { ctx in
            ctx.duration = 0.28
            ctx.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            statusPanel.animator().setFrame(newFrame, display: true)
        }
    }

    private func buildChatPanel() {
        let panel = makeBasePanel(frame: chatFrame, styleMask: [.borderless, .nonactivatingPanel])
        let content = ChatPanelContent(
            viewModel: chatViewModel,
            onCollapse: { [weak self] in self?.collapse() }
        )
        panel.contentView = NSHostingView(rootView: content)
        // Stays hidden until expand() is called.
        self.chatPanel = panel
    }

    // MARK: - Toggle

    func toggle() {
        isExpanded ? collapse() : expand()
    }

    func expand() {
        guard !isExpanded else { return }
        isExpanded = true

        chatPanel.styleMask = [.borderless]
        chatPanel.hasShadow = true
        // Frame is already correct; show panel with deferred redraw so SwiftUI
        // has the full chatFrame to work with on the first render pass.
        chatPanel.setFrame(chatFrame, display: false)
        chatPanel.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)

        // Triggers the spring pop animation in ChatPanelContent.
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

        // Hide the chat panel after the spring settles (~0.35 s).
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { [weak self] in
            guard let self else { return }
            self.chatPanel.hasShadow = false
            self.chatPanel.orderOut(nil)
            self.chatPanel.resignKey()
        }
    }

    // MARK: - Screen changes

    @objc private func screensChanged() {
        computeGeometry()
        statusPanel.setFrame(pillFrame, display: true)
        if isExpanded {
            chatPanel.setFrame(chatFrame, display: true)
        }
    }
}
