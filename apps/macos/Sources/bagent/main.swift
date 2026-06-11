import AppKit

// Retain delegate for the lifetime of the process.
let _delegate = AppDelegate()
NSApplication.shared.delegate = _delegate
NSApp.run()
