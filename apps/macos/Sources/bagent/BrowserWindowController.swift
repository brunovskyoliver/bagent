import AppKit
import WebKit

private final class BrowserPanel: NSPanel {
    var onManualInput: (() -> Void)?
    var onUserResize: (() -> Void)?
    var suppressResizeCallback = false
    /// CoreAnimation keeps delivering window frames after an animation group's
    /// completion handler runs, and an animated `setFrame` reports
    /// `inLiveResize`, so neither a flag flip nor the live-resize signal is
    /// enough on its own: suppression is held for the animation's whole span.
    var suppressResizeUntil: Date?

    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }

    override func sendEvent(_ event: NSEvent) {
        switch event.type {
        case .keyDown, .keyUp, .leftMouseDown, .rightMouseDown, .otherMouseDown, .scrollWheel:
            // The panel's own chrome (Collapse / Move to top right) isn't page
            // input — clicking it must not revoke the agent's Control Lease.
            let hit = event.type == .leftMouseDown || event.type == .rightMouseDown
                ? contentView?.hitTest(event.locationInWindow)
                : nil
            if !BrowserPanel.isPanelChrome(hit) { onManualInput?() }
        default:
            break
        }
        super.sendEvent(event)
    }

    /// True for the panel's own window controls, including the capsule padding
    /// around them.
    static func isPanelChrome(_ view: NSView?) -> Bool {
        var node = view
        while let current = node {
            if current is BrowserPanelControlButton || current is BrowserPanelControlCluster { return true }
            node = current.superview
        }
        return false
    }

    override func setFrame(_ frameRect: NSRect, display flag: Bool) {
        let sizeChanged = frame.size != frameRect.size
        super.setFrame(frameRect, display: flag)
        let suppressed = suppressResizeCallback
            || (suppressResizeUntil.map { Date() < $0 } ?? false)
        guard sizeChanged, !suppressed else { return }
        onUserResize?()
    }

    /// A resize only preempts the agent when the user actually performed it.
    /// `setFrame` cannot tell its callers apart, and the panel resizes itself
    /// for reveal/collapse animations and viewport changes — CoreAnimation even
    /// delivers frames after the animation group's completion handler has run.
    /// Dragging a window's edge always goes through live resize, so that is the
    /// signal; `NSApp.currentEvent` is not (it can hold a stale mouse event).
    private var isUserDrivenResize: Bool { inLiveResize }
}

/// Marker types so `BrowserPanel.sendEvent` can tell the panel's own chrome
/// apart from a click landing on the page (WKWebView). The cluster counts too:
/// a click on the capsule's padding hit-tests to the container, and that must
/// not revoke the agent's Control Lease either.
final class BrowserPanelControlButton: NSButton {
    private var hovering = false { didSet { updateBackground() } }
    private var pressed = false { didSet { updateBackground() } }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(rect: bounds,
                                       options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
                                       owner: self))
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }

    override func mouseDown(with event: NSEvent) {
        pressed = true
        // NSButton tracks the whole press internally and returns on mouse-up.
        super.mouseDown(with: event)
        pressed = false
        hovering = bounds.contains(convert(event.locationInWindow, from: nil))
    }

    func updateBackground() {
        wantsLayer = true
        layer?.cornerRadius = bounds.height / 2
        let alpha: CGFloat = pressed ? 0.34 : (hovering ? 0.20 : 0)
        layer?.backgroundColor = NSColor.white.withAlphaComponent(alpha).cgColor
    }
}

/// Dark translucent capsule behind the panel's two window controls so white SF
/// Symbols stay legible over white, black, image-heavy, and transparent pages.
final class BrowserPanelControlCluster: NSView {
    static let buttonSize: CGFloat = 24
    static let inset: CGFloat = 3
    /// Gap between the capsule and the panel's top/right edges.
    static let margin: CGFloat = 10
    static let spacing: CGFloat = 2

    let buttons: [BrowserPanelControlButton]

    init(buttons: [BrowserPanelControlButton]) {
        self.buttons = buttons
        super.init(frame: .zero)
        wantsLayer = true
        // Explicit dark fill rather than a material/tint: contrast must not
        // depend on the page behind the panel or on the system appearance.
        layer?.backgroundColor = NSColor.black.withAlphaComponent(0.62).cgColor
        layer?.borderColor = NSColor.white.withAlphaComponent(0.18).cgColor
        layer?.borderWidth = 0.5
        buttons.forEach(addSubview)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    static func size(buttonCount: Int) -> CGSize {
        let n = CGFloat(buttonCount)
        return CGSize(
            width: 2 * inset + n * buttonSize + max(0, n - 1) * spacing,
            height: 2 * inset + buttonSize
        )
    }

    override func layout() {
        super.layout()
        layer?.cornerRadius = bounds.height / 2
        var x = Self.inset
        for button in buttons {
            button.frame = CGRect(x: x, y: Self.inset, width: Self.buttonSize, height: Self.buttonSize)
            button.updateBackground()
            x += Self.buttonSize + Self.spacing
        }
    }
}

@MainActor
private func makeControlButton(symbolName: String, tooltip: String, accessibilityLabel: String) -> BrowserPanelControlButton {
    let configuration = NSImage.SymbolConfiguration(pointSize: 11, weight: .semibold)
    let image = NSImage(systemSymbolName: symbolName, accessibilityDescription: accessibilityLabel)?
        .withSymbolConfiguration(configuration)
    let button = BrowserPanelControlButton(image: image ?? NSImage(), target: nil, action: nil)
    button.bezelStyle = .texturedRounded
    button.isBordered = false
    button.imageScaling = .scaleProportionallyDown
    button.contentTintColor = .white
    button.toolTip = tooltip
    button.setAccessibilityLabel(accessibilityLabel)
    button.frame = CGRect(x: 0, y: 0,
                          width: BrowserPanelControlCluster.buttonSize,
                          height: BrowserPanelControlCluster.buttonSize)
    return button
}

private final class BrowserPanelContentView: NSView {
    let webView: WKWebView
    private let dragStrip = BrowserDragStrip()
    private let originOverlay = BrowserOriginOverlay()
    private let collapseButton = makeControlButton(
        symbolName: "eye.slash",
        tooltip: String(localized: "browser.panel.hide", defaultValue: "Hide browser"),
        accessibilityLabel: String(localized: "browser.panel.hide", defaultValue: "Hide browser")
    )
    private let moveToTopRightButton = makeControlButton(
        symbolName: "arrow.up.right",
        tooltip: String(localized: "browser.panel.move_top_right", defaultValue: "Move to top right"),
        accessibilityLabel: String(localized: "browser.panel.move_top_right", defaultValue: "Move to top right")
    )

    private lazy var controlCluster = BrowserPanelControlCluster(
        buttons: [collapseButton, moveToTopRightButton]
    )

    var onCollapse: (() -> Void)?
    var onMoveToTopRight: (() -> Void)?
    var onDragPhase: ((BrowserCueDragPhase, CGPoint) -> Void)?
    var dragStripShouldFollowCursor: () -> Bool = { true } {
        didSet { dragStrip.shouldFollowCursor = dragStripShouldFollowCursor }
    }

    init(webView: WKWebView) {
        self.webView = webView
        super.init(frame: .zero)
        addSubview(webView)
        dragStrip.onDragPhase = { [weak self] phase, pointer in
            self?.onDragPhase?(phase, pointer)
        }
        dragStrip.shouldFollowCursor = { [weak self] in self?.dragStripShouldFollowCursor() ?? true }
        addSubview(dragStrip)
        addSubview(originOverlay)
        collapseButton.target = self
        collapseButton.action = #selector(collapseTapped)
        moveToTopRightButton.target = self
        moveToTopRightButton.action = #selector(moveToTopRightTapped)
        // Added after the drag strip and the web view, so reverse-order
        // hit-testing puts the controls in front of both.
        addSubview(controlCluster)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layout() {
        super.layout()
        webView.frame = bounds
        dragStrip.frame = CGRect(x: bounds.minX, y: bounds.maxY - 26, width: bounds.width, height: 26)
        originOverlay.frame = CGRect(x: bounds.minX + 12, y: bounds.maxY - 54, width: min(420, bounds.width - 24), height: 24)
        // Inset from the panel's own corner rather than centred on the drag
        // strip: the strip's mid-line put the capsule's top edge past the top
        // of the panel, so it was clipped and sat off-centre in the corner.
        let clusterSize = BrowserPanelControlCluster.size(buttonCount: 2)
        controlCluster.frame = CGRect(
            x: bounds.maxX - BrowserPanelControlCluster.margin - clusterSize.width,
            y: bounds.maxY - BrowserPanelControlCluster.margin - clusterSize.height,
            width: clusterSize.width,
            height: clusterSize.height
        )
        controlCluster.layout()
    }

    func showOrigin(_ origin: String) {
        originOverlay.show(origin)
    }

    @objc private func collapseTapped() { onCollapse?() }
    @objc private func moveToTopRightTapped() { onMoveToTopRight?() }
}

private final class BrowserOriginOverlay: NSView {
    private let label = NSTextField(labelWithString: "")
    private var hideTask: Task<Void, Never>?

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.withAlphaComponent(0.78).cgColor
        layer?.cornerRadius = 6
        label.font = .systemFont(ofSize: 11, weight: .medium)
        label.textColor = .white
        label.lineBreakMode = .byTruncatingTail
        addSubview(label)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func layout() {
        super.layout()
        label.frame = bounds.insetBy(dx: 8, dy: 2)
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func show(_ origin: String) {
        hideTask?.cancel()
        label.stringValue = origin
        isHidden = false
        alphaValue = 1
        hideTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            self?.animator().alphaValue = 0
        }
    }
}

/// The panel's top drag area. It tracks the drag itself instead of handing the
/// window to `performDrag(with:)`, so the drop point is known — dropping the
/// panel back onto the notch collapses it into its Browser Cue.
final class BrowserDragStrip: NSView {
    var onDragPhase: ((BrowserCueDragPhase, CGPoint) -> Void)?
    /// False while the panel is previewing its collapse into the notch, or
    /// animating in or out of that preview — the window must not be yanked back
    /// under the cursor mid-animation.
    var shouldFollowCursor: () -> Bool = { true }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        // Where inside the window the pointer grabbed it.
        let grab = event.locationInWindow
        var moved = false
        onDragPhase?(.began, window.convertPoint(toScreen: event.locationInWindow))

        window.trackEvents(
            matching: [.leftMouseDragged, .leftMouseUp],
            timeout: NSEvent.foreverDuration,
            mode: .eventTracking
        ) { tracked, stop in
            MainActor.assumeIsolated {
                guard let tracked else {
                    stop.pointee = true
                    return
                }
                let pointer = NSEvent.mouseLocation
                switch tracked.type {
                case .leftMouseDragged:
                    moved = true
                    if self.shouldFollowCursor() {
                        window.setFrameOrigin(CGPoint(x: pointer.x - grab.x, y: pointer.y - grab.y))
                    }
                    self.onDragPhase?(.changed, pointer)
                case .leftMouseUp:
                    _ = moved
                    self.onDragPhase?(.ended, pointer)
                    stop.pointee = true
                default:
                    break
                }
            }
        }
    }
}

@MainActor
final class BrowserWindowController: NSObject {
    private(set) var panel: NSPanel
    private(set) var webView: WKWebView
    private var contentView: BrowserPanelContentView
    private(set) var lastDisplayID: CGDirectDisplayID?

    var onManualInput: (() -> Void)? {
        didSet { (panel as? BrowserPanel)?.onManualInput = onManualInput }
    }
    var onManualResize: (() -> Void)? {
        didSet { (panel as? BrowserPanel)?.onUserResize = onManualResize }
    }
    /// Hides the panel without closing the session — routed to
    /// `BrowserSession.hide()` so `pageInfo.visibility` and the Browser Cue
    /// stay in sync and clicking the cue reopens the same panel.
    var onCollapseRequested: (() -> Void)? {
        didSet { contentView.onCollapse = onCollapseRequested }
    }
    /// Dragging the panel by its top strip, in screen coordinates. Dropping it
    /// on the notch collapses it back into its Browser Cue.
    var onDragPhase: ((BrowserCueDragPhase, CGPoint) -> Void)? {
        didSet { contentView.onDragPhase = onDragPhase }
    }
    /// Top-right margin used by "Move to top right", matching the drag
    /// panel's other fixed insets.
    private static let topRightMargin: CGFloat = 12

    init(webView: WKWebView, frame: NSRect = NSRect(x: 180, y: 180, width: 960, height: 640)) {
        self.webView = webView
        self.contentView = BrowserPanelContentView(webView: webView)
        let panel = BrowserPanel(
            contentRect: frame,
            styleMask: [.titled, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        self.panel = panel
        super.init()

        panel.title = String(localized: "browser.cue.title", defaultValue: "bagent Browser")
        panel.titleVisibility = .hidden
        panel.titlebarAppearsTransparent = true
        panel.isMovableByWindowBackground = false
        panel.standardWindowButton(.closeButton)?.isHidden = true
        panel.standardWindowButton(.miniaturizeButton)?.isHidden = true
        panel.standardWindowButton(.zoomButton)?.isHidden = true
        panel.level = .floating
        // NSPanel hides itself on app deactivation unless it is a nonactivating
        // panel — and this one isn't. Left at the default, the Browser Panel is
        // invisible in its normal state (bagent not frontmost): a cue drag
        // ordered it front and nothing appeared, and a panel shown while bagent
        // was active vanished the moment the user clicked another app.
        panel.hidesOnDeactivate = false
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle]
        panel.isOpaque = true
        panel.backgroundColor = .black
        panel.hasShadow = true
        panel.contentView = contentView
        panel.delegate = self
        panel.orderOut(nil)
        panel.layoutIfNeeded()
        contentView.onMoveToTopRight = { [weak self] in self?.moveToTopRight() }
        contentView.dragStripShouldFollowCursor = { [weak self] in !(self?.suspendsDragFollowing ?? false) }
        rememberDisplay()
    }

    func replaceWebView(_ webView: WKWebView) {
        contentView.removeFromSuperview()
        self.webView = webView
        contentView = BrowserPanelContentView(webView: webView)
        contentView.onCollapse = onCollapseRequested
        contentView.onMoveToTopRight = { [weak self] in self?.moveToTopRight() }
        contentView.onDragPhase = onDragPhase
        contentView.dragStripShouldFollowCursor = { [weak self] in !(self?.suspendsDragFollowing ?? false) }
        panel.contentView = contentView
        panel.layoutIfNeeded()
    }

    /// Moves the panel to the top-right corner of the display containing most
    /// of its current frame (falling back to the last remembered display),
    /// preserving size and respecting the menu bar/Dock via `visibleFrame`.
    /// Snaps the panel to the display's top-right corner at the compact
    /// picture-in-picture size browsers use for floating video.
    func moveToTopRight() {
        guard let screen = screenContainingMostOfPanel() else { return }
        let size = BrowserPanelGeometry.pictureInPictureSize(
            for: panel.frame.size,
            visibleFrame: screen.visibleFrame
        )
        let origin = BrowserPanelGeometry.topRightOrigin(
            panelSize: size,
            visibleFrame: screen.visibleFrame,
            margin: Self.topRightMargin
        )
        let target = CGRect(origin: origin, size: size)
        panel.orderFront(nil)
        if reduceMotion {
            panel.setFrame(target, display: true)
        } else {
            animating { _ in self.panel.animator().setFrame(target, display: true) }
        }
        lastDisplayID = screen.displayID
    }

    private func screenContainingMostOfPanel() -> NSScreen? {
        let panelFrame = panel.frame
        let best = NSScreen.screens
            .map { ($0, $0.frame.intersection(panelFrame)) }
            .max { $0.1.width * $0.1.height < $1.1.width * $1.1.height }
        if let best, best.1.width > 0, best.1.height > 0 { return best.0 }
        return display(for: lastDisplayID) ?? NSScreen.main
    }

    /// How the panel enters the screen.
    enum Reveal {
        /// Grow out of the Browser Cue in the notch.
        case fromCue
        /// Fade up in place — used while a cue drag is already moving the panel.
        case inPlace
        case none
    }

    static let animationDuration: TimeInterval = 0.26
    /// Screen rect of this session's Browser Cue, so the panel can grow out of
    /// it and collapse back into it.
    var cueScreenRect: CGRect?
    /// Reveal style for the next `show(focus:)`. Consumed on use, so the agent
    /// API and the UI share one entry point.
    var nextReveal: Reveal = .fromCue

    private var reduceMotion: Bool {
        NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
    }

    /// Frame the panel collapses into: a small rect centred on the cue.
    private func collapsedFrame(target: NSRect) -> NSRect {
        let anchor = cueScreenRect.map { CGPoint(x: $0.midX, y: $0.midY) }
            ?? CGPoint(x: target.midX, y: target.maxY)
        let size = CGSize(width: target.width * 0.14, height: target.height * 0.14)
        return NSRect(x: anchor.x - size.width / 2, y: anchor.y - size.height / 2,
                      width: size.width, height: size.height)
    }

    /// Panel-owned frame changes (animation staging and restore) must never be
    /// mistaken for a user resize — that would revoke the agent's Control Lease.
    private func setFrameWithoutUserResize(_ frame: NSRect, display: Bool) {
        let browserPanel = panel as? BrowserPanel
        let previous = browserPanel?.suppressResizeCallback ?? false
        browserPanel?.suppressResizeCallback = true
        panel.setFrame(frame, display: display)
        browserPanel?.suppressResizeCallback = previous
    }

    /// Window animations resize the panel, which would otherwise look like a
    /// user resize and revoke the agent's Control Lease.
    private func animating(_ duration: TimeInterval = animationDuration,
                           completion: (() -> Void)? = nil,
                           _ body: (NSAnimationContext) -> Void) {
        (panel as? BrowserPanel)?.suppressResizeCallback = true
        (panel as? BrowserPanel)?.suppressResizeUntil = Date().addingTimeInterval(duration + 0.3)
        NSAnimationContext.runAnimationGroup { context in
            context.duration = duration
            context.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            context.allowsImplicitAnimation = true
            body(context)
        } completionHandler: { [weak self] in
            // The completion restores the pre-animation frame, so it has to run
            // while resize callbacks are still suppressed — otherwise collapsing
            // the panel looks like a user resize and revokes the Control Lease.
            completion?()
            (self?.panel as? BrowserPanel)?.suppressResizeCallback = false
        }
    }

    // MARK: - Drop preview (dragging the panel over the notch)

    /// True while the panel is shrunk against the notch, previewing the
    /// collapse that releasing the mouse will confirm.
    private(set) var isDropPreviewActive = false
    /// Frame to return to — restored when the pointer leaves the notch, and the
    /// size the panel reopens at after a confirmed collapse.
    private var preDropFrame: NSRect?
    private var dragGrab: CGPoint = .zero
    private var isPreviewTransitioning = false

    /// The drag strip must stop following the cursor while the panel is docked
    /// against the notch or animating between the two states.
    var suspendsDragFollowing: Bool { isDropPreviewActive || isPreviewTransitioning }

    /// Where the panel would be if it were still following the cursor — used
    /// for drop detection so a collapsed preview (which sits inside the notch
    /// by definition) can't latch the drop state on.
    func projectedDragFrame(at pointer: CGPoint) -> CGRect {
        let size = (preDropFrame ?? panel.frame).size
        return CGRect(origin: CGPoint(x: pointer.x - dragGrab.x, y: pointer.y - dragGrab.y), size: size)
    }

    func beginPanelDrag(at pointer: CGPoint) {
        dragGrab = CGPoint(x: pointer.x - panel.frame.minX, y: pointer.y - panel.frame.minY)
    }

    /// Entering the notch shrinks the panel toward it; leaving restores it under
    /// the cursor and hands it back to the drag.
    func setDropPreview(_ active: Bool, notchRect _: CGRect, pointer: CGPoint) {
        guard active != isDropPreviewActive else { return }
        isDropPreviewActive = active

        if active {
            let full = preDropFrame ?? panel.frame
            preDropFrame = full
            // Collapse the whole way into the cue, exactly like a confirmed
            // hide. A half-scaled panel parked under the notch just loses the
            // cursor; this clears the screen and leaves the drag free.
            animateDropPreview(to: collapsedFrame(target: full), alpha: 0)
        } else {
            let size = (preDropFrame ?? panel.frame).size
            preDropFrame = nil
            let origin = CGPoint(x: pointer.x - dragGrab.x, y: pointer.y - dragGrab.y)
            animateDropPreview(to: NSRect(origin: origin, size: size), alpha: 1)
        }
    }

    /// Terminates any window animation still in flight. Without this its
    /// remaining frames land *after* the next state is applied and win — the
    /// panel would stay collapsed after springing back, or reopen at the
    /// collapsed size after a confirmed hide.
    private func stopWindowAnimations() {
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            panel.animator().setFrame(panel.frame, display: false)
            panel.animator().alphaValue = panel.alphaValue
        }
    }

    private func animateDropPreview(to frame: NSRect, alpha: CGFloat) {
        stopWindowAnimations()
        guard !reduceMotion else {
            setFrameWithoutUserResize(frame, display: true)
            panel.alphaValue = alpha
            return
        }
        isPreviewTransitioning = true
        animating(0.18, completion: { [weak self] in self?.isPreviewTransitioning = false }) { _ in
            self.panel.animator().setFrame(frame, display: true)
            self.panel.animator().alphaValue = alpha
        }
    }

    /// Collapse into the Browser Cue. The session, page and cue stay alive.
    private func hideAnimated() {
        // Dropping on the notch already played the collapse during the preview.
        if isDropPreviewActive {
            stopWindowAnimations()
            panel.orderOut(nil)
            if let restore = preDropFrame { setFrameWithoutUserResize(restore, display: false) }
            preDropFrame = nil
            isDropPreviewActive = false
            panel.alphaValue = 1
            return
        }
        guard panel.isVisible, !reduceMotion else {
            panel.orderOut(nil)
            if let restore = preDropFrame {
                setFrameWithoutUserResize(restore, display: false)
                preDropFrame = nil
            }
            isDropPreviewActive = false
            return
        }
        let restore = preDropFrame ?? panel.frame
        preDropFrame = nil
        isDropPreviewActive = false
        let shrunk = collapsedFrame(target: panel.frame)
        animating(completion: { [weak self] in
            guard let self else { return }
            self.panel.orderOut(nil)
            self.setFrameWithoutUserResize(restore, display: false)
            self.panel.alphaValue = 1
        }) { _ in
            self.panel.animator().alphaValue = 0
            self.panel.animator().setFrame(shrunk, display: true)
        }
    }

    func setFrame(_ frame: NSRect) {
        panel.setFrame(frame, display: false)
        rememberDisplay()
    }

    func setFrameForCueDrag(at pointer: CGPoint) {
        guard let screen = screen(containing: pointer) ?? display(for: lastDisplayID) else { return }
        let geometry = BrowserPanelGeometry(
            panelSize: panel.frame.size,
            visibleFrame: screen.visibleFrame
        )
        panel.setFrame(geometry.frame(anchoredTo: pointer), display: true)
        lastDisplayID = screen.displayID
    }

    func setContentSize(_ size: NSSize) {
        (panel as? BrowserPanel)?.suppressResizeCallback = true
        panel.setContentSize(size)
        (panel as? BrowserPanel)?.suppressResizeCallback = false
    }

    func show(focus: Bool) {
        let reveal = nextReveal
        nextReveal = .fromCue
        rememberDisplay()
        guard !panel.isVisible, !reduceMotion, reveal != .none else {
            orderPanelFront(focus: focus)
            return
        }
        let target = panel.frame
        panel.alphaValue = 0
        if reveal == .fromCue {
            setFrameWithoutUserResize(collapsedFrame(target: target), display: false)
        }
        orderPanelFront(focus: focus)
        animating(reveal == .fromCue ? Self.animationDuration : 0.16) { _ in
            self.panel.animator().alphaValue = 1
            if reveal == .fromCue { self.panel.animator().setFrame(target, display: true) }
        }
    }

    private func orderPanelFront(focus: Bool) {
        if focus {
            panel.makeKeyAndOrderFront(nil)
        } else {
            panel.orderFront(nil)
        }
    }

    func showOrigin(_ origin: String) {
        contentView.showOrigin(origin)
    }

    func hide() {
        hideAnimated()
    }

    func display(for displayID: CGDirectDisplayID?) -> NSScreen? {
        if let displayID {
            return NSScreen.screens.first { $0.deviceDescription[ NSDeviceDescriptionKey("NSScreenNumber") ] as? CGDirectDisplayID == displayID }
        }
        if let lastDisplayID {
            return NSScreen.screens.first { $0.deviceDescription[ NSDeviceDescriptionKey("NSScreenNumber") ] as? CGDirectDisplayID == lastDisplayID }
        }
        return NSScreen.main
    }

    private func screen(containing pointer: CGPoint) -> NSScreen? {
        NSScreen.screens.first { $0.frame.contains(pointer) }
    }

    private func rememberDisplay() {
        guard let screen = NSScreen.screens.first(where: { $0.frame.intersects(panel.frame) }) else { return }
        lastDisplayID = screen.displayID
    }
}

private extension NSScreen {
    var displayID: CGDirectDisplayID? {
        deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID
    }
}

extension BrowserWindowController: NSWindowDelegate {
    nonisolated func windowDidMove(_ notification: Notification) {
        Task { @MainActor [weak self] in self?.rememberDisplay() }
    }

}
