import AppKit
import SwiftUI

/// Opt-in event trace for the cold-first-mouse cue path. Enabled only when
/// `BAGENT_BROWSER_CUE_TRACE=1` is set in the app's environment, so the signed
/// release bundle can be traced during a live run and stays silent otherwise.
/// Writes to `/tmp/bagent-cue-trace.log`; it distinguishes "the panel never saw
/// the event", "the panel saw it but hit-testing missed the cue", "the cue saw
/// mouse-down but no drag", and "the coordinator got the phases".
enum BrowserCueTrace {
    static let isEnabled = ProcessInfo.processInfo.environment["BAGENT_BROWSER_CUE_TRACE"] == "1"
    private static let path = "/tmp/bagent-cue-trace.log"

    static func log(_ message: @autoclosure () -> String) {
        guard isEnabled else { return }
        let line = "\(Date().formatted(date: .omitted, time: .standard)) \(message())\n"
        guard let data = line.data(using: .utf8) else { return }
        if let handle = FileHandle(forWritingAtPath: path) {
            handle.seekToEndOfFile()
            try? handle.write(contentsOf: data)
            try? handle.close()
        } else {
            try? data.write(to: URL(fileURLWithPath: path))
        }
    }
}

/// The Browser Cue's real hit target.
///
/// SwiftUI's `DragGesture` never saw the very first click while bagent was not
/// the active app. The hosting view's `acceptsFirstMouse` override did get the
/// event delivered (the hit test over the cue resolves to the hosting view),
/// but SwiftUI then dropped it before the cue's gesture. A plain `NSView`
/// subview ends the AppKit hit test here instead: it opts into first mouse
/// itself and owns the whole down/drag/up sequence through an explicit
/// tracking loop, rather than relying on AppKit routing drags to a view in a
/// non-key, nonactivating panel.
final class BrowserCueHitView: NSView {
    var onClick: (() -> Void)?
    /// ⌥-click: destroy this Browser Session (or shake, if its agent is busy).
    var onOptionClick: (() -> Void)?
    var onDragPhase: ((BrowserCueDragPhase, CGPoint) -> Void)?
    /// (hovering, cursor in screen coordinates). Reported from AppKit because
    /// this view sits on top of the cue, so SwiftUI's `.onHover` on the icon row
    /// no longer sees the pointer.
    var onHoverChanged: ((Bool, CGPoint) -> Void)?
    /// The cue's rect in screen coordinates — the Browser Panel grows out of it
    /// and collapses back into it.
    var onGeometryChanged: ((CGRect) -> Void)?

    override func layout() {
        super.layout()
        reportGeometry()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        reportGeometry()
    }

    private func reportGeometry() {
        guard let window else { return }
        onGeometryChanged?(window.convertToScreen(convert(bounds, to: nil)))
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        trackingAreas.forEach(removeTrackingArea)
        addTrackingArea(NSTrackingArea(
            rect: bounds,
            options: [.mouseEnteredAndExited, .mouseMoved, .activeAlways, .inVisibleRect],
            owner: self
        ))
    }

    override func mouseEntered(with event: NSEvent) { onHoverChanged?(true, NSEvent.mouseLocation) }
    override func mouseMoved(with event: NSEvent) { onHoverChanged?(true, NSEvent.mouseLocation) }
    override func mouseExited(with event: NSEvent) { onHoverChanged?(false, NSEvent.mouseLocation) }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// This view only owns the left-button sequence. The context menu (close /
    /// approve submission / reclaim) still belongs to the SwiftUI cue behind
    /// it, and forwarding `menu(for:)` up the superview chain does not reach
    /// it — so make the view invisible to hit-testing for the events that open
    /// that menu and let SwiftUI hit-test the cue as before.
    override func hitTest(_ point: NSPoint) -> NSView? {
        let current = NSApp.currentEvent
        // Only a live event for this window counts — NSApp.currentEvent can
        // hold a stale event from another window entirely.
        let isOurs = current?.window == nil || current?.window === window
        return isOurs && Self.opensContextMenu(current) ? nil : super.hitTest(point)
    }

    static func opensContextMenu(_ event: NSEvent?) -> Bool {
        guard let event else { return false }
        switch event.type {
        case .rightMouseDown, .rightMouseUp, .rightMouseDragged:
            return true
        case .leftMouseDown, .leftMouseUp:
            return event.modifierFlags.contains(.control)
        default:
            return false
        }
    }

    override func mouseDown(with event: NSEvent) {
        guard let window else { return }
        if event.modifierFlags.contains(.option) {
            BrowserCueTrace.log("option click")
            onHoverChanged?(false, NSEvent.mouseLocation)
            onOptionClick?()
            return
        }
        onHoverChanged?(false, NSEvent.mouseLocation)
        let start = convert(event.locationInWindow, from: nil)
        var state = beginTracking(at: start)

        BrowserCueTrace.log(
            "mouseDown appActive=\(NSApp.isActive) keyWindow=\(window.isKeyWindow) local=\(start)"
        )

        // Own the sequence rather than relying on AppKit routing drags to a
        // view in a non-key, nonactivating panel.
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
                let finished = self.track(
                    tracked.type,
                    local: self.convert(tracked.locationInWindow, from: nil),
                    screen: window.convertPoint(toScreen: tracked.locationInWindow),
                    from: start,
                    state: &state
                )
                if finished { stop.pointee = true }
            }
        }
    }

    /// The state machine classifies in local coordinates (its cue bounds are
    /// this view's bounds); the coordinator is driven in screen coordinates.
    func beginTracking(at start: CGPoint) -> BrowserCueDragState {
        var state = BrowserCueDragState(cueBounds: bounds)
        _ = state.update(startLocation: start, location: start)
        return state
    }

    /// One step of the tracked sequence. Split out of `mouseDown` so the
    /// classification and the emitted phases are testable without the AppKit
    /// event pump. Returns `true` when the sequence is over.
    @discardableResult
    func track(
        _ type: NSEvent.EventType,
        local: CGPoint,
        screen: CGPoint,
        from start: CGPoint,
        state: inout BrowserCueDragState
    ) -> Bool {
        switch type {
        case .leftMouseDragged:
            switch state.update(startLocation: start, location: local) {
            case .began: emit(.began, screen)
            case .changed: emit(.changed, screen)
            case .waiting, .clicked, .ended, .beganAndEnded: break
            }
            return false
        case .leftMouseUp:
            switch state.end(startLocation: start, location: local) {
            case .clicked:
                BrowserCueTrace.log("click")
                onClick?()
            case .ended:
                emit(.ended, screen)
            case .beganAndEnded:
                emit(.began, screen)
                emit(.ended, screen)
            case .waiting, .began, .changed: break
            }
            return true
        default:
            return false
        }
    }

    private func emit(_ phase: BrowserCueDragPhase, _ screenLocation: CGPoint) {
        BrowserCueTrace.log("drag \(phase) screen=\(screenLocation)")
        onDragPhase?(phase, screenLocation)
    }
}

private struct BrowserCueHitTarget: NSViewRepresentable {
    let accessibilityLabel: String
    let onClick: () -> Void
    let onOptionClick: () -> Void
    let onDragPhase: (BrowserCueDragPhase, CGPoint) -> Void
    let onHoverChanged: (Bool, CGPoint) -> Void
    let onGeometryChanged: (CGRect) -> Void

    func makeNSView(context: Context) -> BrowserCueHitView {
        BrowserCueHitView()
    }

    func updateNSView(_ view: BrowserCueHitView, context: Context) {
        view.setAccessibilityRole(.button)
        view.setAccessibilityLabel(accessibilityLabel)
        view.onClick = onClick
        view.onOptionClick = onOptionClick
        view.onDragPhase = onDragPhase
        view.onHoverChanged = onHoverChanged
        view.onGeometryChanged = onGeometryChanged
    }
}

struct BrowserCueInteractionView: View {
    let cue: BrowserCue
    let reduceMotion: Bool
    let onClick: () -> Void
    let onOptionClick: () -> Void
    let onDragPhase: (BrowserCueDragPhase, CGPoint) -> Void
    let onHoverChanged: (Bool, CGPoint) -> Void
    let onGeometryChanged: (CGRect) -> Void
    /// Increments to play the busy shake.
    let shakes: Int

    var body: some View {
        BrowserCueIconView(cue: cue, reduceMotion: reduceMotion)
            .overlay(
                BrowserCueHitTarget(
                    accessibilityLabel: accessibilityLabel,
                    onClick: onClick,
                    onOptionClick: onOptionClick,
                    onDragPhase: onDragPhase,
                    onHoverChanged: onHoverChanged,
                    onGeometryChanged: onGeometryChanged
                )
            )
            .modifier(BrowserCueShakeEffect(animatableData: CGFloat(shakes)))
            .animation(.easeInOut(duration: 0.42), value: shakes)
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { onClick() }
    }

    private var accessibilityLabel: String {
        String(localized: "browser.cue.accessibility", defaultValue: "bagent Browser \(cue.label)")
    }
}


/// Refuses-to-close shake: three quick lateral passes per increment.
struct BrowserCueShakeEffect: GeometryEffect {
    var animatableData: CGFloat

    func effectValue(size: CGSize) -> ProjectionTransform {
        let travel = sin(animatableData * .pi * 6) * 3
        return ProjectionTransform(CGAffineTransform(translationX: travel, y: 0))
    }
}
