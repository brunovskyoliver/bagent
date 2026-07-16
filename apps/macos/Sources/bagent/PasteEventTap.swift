import AppKit
@preconcurrency import CoreGraphics

/// CGEventTap that watches the RIGHT ⌘ key to drive the clipboard wheel.
///
/// Hold right ⌘ ≥0.3s → the wheel opens; the tap then consumes digits 1–5 and
/// Escape until ⌘ is released (commit) or the wheel is pinned by the mouse.
/// A quick tap or any key pressed during the hold cancels silently — right ⌘
/// alone does nothing in apps, so nothing is ever swallowed outside wheel mode.
///
/// Requires Accessibility permission. Without it (or when disabled) the tap is
/// never installed and the keyboard behaves natively everywhere.
@MainActor
final class PasteEventTap {

    enum WheelState {
        case idle
        /// Right ⌘ is down, hold timer running.
        case holdPending
        /// Wheel is showing; `pinned` set once the mouse enters the wheel.
        case wheelActive
    }

    /// Marks events we synthesize so this tap passes them through.
    private static let syntheticTag: Int64 = 0x62_67_6E_74  // 'bgnt'
    private static let holdDelay: TimeInterval = 0.3

    private static let keyV: Int64 = 9
    private static let keyEscape: Int64 = 53
    private static let keyRightCommand: Int64 = 54
    /// Device-specific right-⌘ bit inside CGEventFlags (NX_DEVICERCMDKEYMASK).
    private static let rightCommandBit: UInt64 = 0x0010
    /// keycodes for 1…5 → slot index 0…4
    private static let digitSlots: [Int64: Int] = [18: 0, 19: 1, 20: 2, 21: 3, 23: 4]

    // Callbacks — all invoked on the main actor.
    var canOpenWheel: () -> Bool = { false }
    var onWheelOpen: () -> Void = {}
    var onDigit: (Int) -> Void = { _ in }
    /// ⌘ released with no digit → paste newest.
    var onCommit: () -> Void = {}
    var onCancel: () -> Void = {}

    private(set) var state: WheelState = .idle
    private(set) var isPinned = false

    private var tap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private var holdWork: DispatchWorkItem?
    private var retryTimer: Timer?

    // MARK: - Lifecycle

    /// Install the tap; retries every 5s until Accessibility is granted.
    func start() {
        installTap()
        guard tap == nil else { return }
        retryTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.installTap()
                if self.tap != nil {
                    self.retryTimer?.invalidate()
                    self.retryTimer = nil
                }
            }
        }
    }

    func stop() {
        retryTimer?.invalidate()
        retryTimer = nil
        holdWork?.cancel()
        holdWork = nil
        if let source = runLoopSource {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes)
            runLoopSource = nil
        }
        if let tap {
            CGEvent.tapEnable(tap: tap, enable: false)
            self.tap = nil
        }
        state = .idle
        isPinned = false
    }

    private func installTap() {
        guard tap == nil else { return }
        let mask: CGEventMask =
            (1 << CGEventType.keyDown.rawValue) |
            (1 << CGEventType.keyUp.rawValue) |
            (1 << CGEventType.flagsChanged.rawValue)
        let refcon = Unmanaged.passUnretained(self).toOpaque()
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: .defaultTap,
            eventsOfInterest: mask,
            callback: { _, type, event, refcon in
                guard let refcon else { return Unmanaged.passUnretained(event) }
                let tap = Unmanaged<PasteEventTap>.fromOpaque(refcon).takeUnretainedValue()
                // The tap's run-loop source lives on the main run loop, so the
                // callback always runs on the main thread.
                return MainActor.assumeIsolated { tap.handle(type: type, event: event) }
            },
            userInfo: refcon
        ) else {
            // Fails when Accessibility isn't granted for THIS build's signature —
            // note that a changed codesign invalidates an older TCC grant.
            NSLog("bagent: paste-wheel event tap unavailable (Accessibility not granted; trusted=%d)",
                  AXIsProcessTrusted() ? 1 : 0)
            return
        }
        NSLog("bagent: paste-wheel event tap installed")

        self.tap = tap
        let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        runLoopSource = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
    }

    // MARK: - Wheel coordination (called by the window controller)

    /// Mouse entered the wheel — keyboard release no longer commits/dismisses.
    func pin() {
        guard state == .wheelActive else { return }
        isPinned = true
    }

    /// Wheel closed (commit, cancel, click-away, drag) — back to idle.
    func reset() {
        holdWork?.cancel()
        holdWork = nil
        state = .idle
        isPinned = false
    }

    // MARK: - Event handling

    private func handle(type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        // OS disables a tap that stalls — re-enable and let events flow.
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let tap { CGEvent.tapEnable(tap: tap, enable: true) }
            reset()
            return Unmanaged.passUnretained(event)
        }
        // Our own synthetic paste — never re-intercept.
        if event.getIntegerValueField(.eventSourceUserData) == Self.syntheticTag {
            return Unmanaged.passUnretained(event)
        }

        let keyCode = event.getIntegerValueField(.keyboardEventKeycode)
        let rightCmdDown = event.flags.rawValue & Self.rightCommandBit != 0

        switch state {
        case .idle:
            if type == .flagsChanged, keyCode == Self.keyRightCommand,
               rightCmdDown, canOpenWheel() {
                state = .holdPending
                scheduleHoldTimer()
            }
            return Unmanaged.passUnretained(event)

        case .holdPending:
            // Any key or another modifier during the hold = a real shortcut,
            // not a wheel gesture. Quick release cancels too — nothing was
            // swallowed, so nothing is owed.
            if type == .keyDown
                || (type == .flagsChanged && !(keyCode == Self.keyRightCommand && rightCmdDown)) {
                cancelHold()
            }
            return Unmanaged.passUnretained(event)

        case .wheelActive:
            if type == .keyDown, keyCode == Self.keyEscape {
                state = .idle
                isPinned = false
                onCancel()
                return nil
            }
            if type == .keyDown, let slot = Self.digitSlots[keyCode] {
                state = .idle
                isPinned = false
                onDigit(slot)
                return nil
            }
            if type == .flagsChanged, keyCode == Self.keyRightCommand, !rightCmdDown, !isPinned {
                state = .idle
                onCommit()
            }
            return Unmanaged.passUnretained(event)
        }
    }

    private func scheduleHoldTimer() {
        holdWork?.cancel()
        let work = DispatchWorkItem { [weak self] in
            guard let self, self.state == .holdPending else { return }
            self.holdWork = nil
            guard self.canOpenWheel() else {
                self.state = .idle
                return
            }
            self.state = .wheelActive
            self.isPinned = false
            self.onWheelOpen()
        }
        holdWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.holdDelay, execute: work)
    }

    private func cancelHold() {
        holdWork?.cancel()
        holdWork = nil
        state = .idle
    }

    /// Post a tagged ⌘V to the frontmost app.
    static func postSyntheticPaste() {
        guard let source = CGEventSource(stateID: .combinedSessionState) else { return }
        source.userData = syntheticTag
        let down = CGEvent(keyboardEventSource: source, virtualKey: CGKeyCode(keyV), keyDown: true)
        let up = CGEvent(keyboardEventSource: source, virtualKey: CGKeyCode(keyV), keyDown: false)
        down?.flags = .maskCommand
        up?.flags = .maskCommand
        down?.post(tap: .cghidEventTap)
        up?.post(tap: .cghidEventTap)
    }
}
