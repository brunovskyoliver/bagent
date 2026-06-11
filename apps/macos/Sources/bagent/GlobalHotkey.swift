import Carbon.HIToolbox

// Module-level storage — accessed from a C callback, must be nonisolated(unsafe).
private nonisolated(unsafe) var _hotkeyHandler: (@Sendable () -> Void)?
private nonisolated(unsafe) var _hotkeyRef: EventHotKeyRef?
private nonisolated(unsafe) var _eventHandlerRef: EventHandlerRef?

// C-compatible file-scope function (no captured state — safe for use as EventHandlerUPP).
private func carbonCallback(
    _ nextHandler: EventHandlerCallRef?,
    _ event: EventRef?,
    _ userData: UnsafeMutableRawPointer?
) -> OSStatus {
    _hotkeyHandler?()
    return noErr
}

enum GlobalHotkey {
    /// Register ⌥Space as a system-wide hotkey. Requires no special permissions.
    static func register(handler: @escaping @Sendable () -> Void) {
        _hotkeyHandler = handler

        var eventSpec = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )
        InstallEventHandler(
            GetApplicationEventTarget(),
            carbonCallback,
            1, &eventSpec,
            nil, &_eventHandlerRef
        )

        let hotkeyID = EventHotKeyID(signature: 0x62676E74, id: 1)  // 'bgnt'
        // keyCode 49 = Space, optionKey = ⌥
        RegisterEventHotKey(
            49, UInt32(optionKey), hotkeyID,
            GetApplicationEventTarget(), 0, &_hotkeyRef
        )
    }

    static func unregister() {
        if let ref = _hotkeyRef { UnregisterEventHotKey(ref); _hotkeyRef = nil }
        if let ref = _eventHandlerRef { RemoveEventHandler(ref); _eventHandlerRef = nil }
        _hotkeyHandler = nil
    }
}
