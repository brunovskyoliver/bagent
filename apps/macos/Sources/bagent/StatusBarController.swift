import AppKit

@MainActor
final class StatusBarController {
    private var statusItem: NSStatusItem?
    private let onToggle: @MainActor () -> Void

    init(onToggle: @escaping @MainActor () -> Void) {
        self.onToggle = onToggle
        setup()
    }

    private func setup() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        guard let button = statusItem?.button else { return }
        button.image = NSImage(systemSymbolName: "sparkles", accessibilityDescription: "bagent")
        button.image?.isTemplate = true
        button.action = #selector(handleClick)
        button.target = self
        button.toolTip = "bagent — ⌥Space"
    }

    /// Update the badge count shown on the status item (0 = no badge).
    func setBadge(_ count: Int) {
        guard let button = statusItem?.button else { return }
        if count > 0 {
            button.image = NSImage(systemSymbolName: "sparkles.rectangle.stack", accessibilityDescription: "bagent — \(count) pending")
            button.image?.isTemplate = true
        } else {
            button.image = NSImage(systemSymbolName: "sparkles", accessibilityDescription: "bagent")
            button.image?.isTemplate = true
        }
    }

    @objc private func handleClick() {
        onToggle()
    }
}
