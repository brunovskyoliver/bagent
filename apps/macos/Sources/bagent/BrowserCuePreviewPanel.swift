import AppKit

/// Placement for the Browser Cue's hover preview: directly under the cursor,
/// clamped so it never spills off the display the cursor is on.
enum BrowserCuePreviewGeometry {
    static let size = CGSize(width: 232, height: 172)
    static let cursorGap: CGFloat = 16

    static func frame(under cursor: CGPoint, size: CGSize = size, visibleFrame: CGRect,
                      gap: CGFloat = cursorGap) -> CGRect {
        let maxX = max(visibleFrame.minX, visibleFrame.maxX - size.width)
        let maxY = max(visibleFrame.minY, visibleFrame.maxY - size.height)
        // Below the cursor when there is room, above it otherwise.
        let below = cursor.y - gap - size.height
        let y = below >= visibleFrame.minY ? below : min(maxY, cursor.y + gap)
        return CGRect(
            origin: CGPoint(
                x: min(maxX, max(visibleFrame.minX, cursor.x - size.width / 2)),
                y: min(maxY, max(visibleFrame.minY, y))
            ),
            size: size
        )
    }
}

/// Host-side hooks `BrowserCoordinator` drives to show the cue hover preview,
/// without owning the AppKit view that renders it.
@MainActor
protocol BrowserCuePreviewHosting: AnyObject {
    func showCuePreview(title: String, origin: String?, at cursor: CGPoint)
    func setCuePreviewThumbnail(_ image: NSImage?)
    func moveCuePreview(to cursor: CGPoint)
    func hideCuePreview()
}

/// Small card that tells the user which tab a Browser Cue belongs to before
/// they drag it. Lives as a plain subview inside the notch's own panel
/// content view (positioned under the cursor in that panel's local
/// coordinates) rather than a floating `NSPanel` of its own — ADR-0005 grants
/// exactly one floating panel per live Browser Session, and this hover card
/// isn't one. Never takes mouse events — it only follows the cursor.
@MainActor
final class BrowserCuePreviewView: NSView, BrowserCuePreviewHosting {
    private let thumbnail = NSImageView()
    private let titleLabel = NSTextField(labelWithString: "")
    private let originLabel = NSTextField(labelWithString: "")
    private unowned let hostWindow: NSWindow

    init(hostWindow: NSWindow) {
        self.hostWindow = hostWindow
        super.init(frame: CGRect(origin: .zero, size: BrowserCuePreviewGeometry.size))

        isHidden = true
        wantsLayer = true
        layer?.backgroundColor = NSColor.black.withAlphaComponent(0.82).cgColor
        layer?.cornerRadius = 10
        layer?.borderColor = NSColor.white.withAlphaComponent(0.16).cgColor
        layer?.borderWidth = 0.5
        layer?.masksToBounds = true

        thumbnail.imageScaling = .scaleProportionallyUpOrDown
        thumbnail.wantsLayer = true
        thumbnail.layer?.backgroundColor = NSColor.white.withAlphaComponent(0.06).cgColor
        titleLabel.font = .systemFont(ofSize: 11, weight: .semibold)
        titleLabel.textColor = .white
        titleLabel.lineBreakMode = .byTruncatingTail
        originLabel.font = .systemFont(ofSize: 10, weight: .regular)
        originLabel.textColor = NSColor.white.withAlphaComponent(0.6)
        originLabel.lineBreakMode = .byTruncatingMiddle

        addSubview(thumbnail)
        addSubview(titleLabel)
        addSubview(originLabel)
        let width = BrowserCuePreviewGeometry.size.width
        let height = BrowserCuePreviewGeometry.size.height
        thumbnail.frame = CGRect(x: 8, y: 40, width: width - 16, height: height - 48)
        titleLabel.frame = CGRect(x: 10, y: 21, width: width - 20, height: 14)
        originLabel.frame = CGRect(x: 10, y: 7, width: width - 20, height: 12)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("unavailable") }

    /// Never claims a click — the card is purely informational and sits over
    /// interactive notch content.
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func showCuePreview(title: String, origin: String?, at cursor: CGPoint) {
        titleLabel.stringValue = title.isEmpty
            ? String(localized: "browser.cue.title", defaultValue: "bagent Browser")
            : title
        originLabel.stringValue = origin ?? ""
        moveCuePreview(to: cursor)
        isHidden = false
    }

    func setCuePreviewThumbnail(_ image: NSImage?) {
        thumbnail.image = image
    }

    func moveCuePreview(to cursor: CGPoint) {
        guard let screen = NSScreen.screens.first(where: { $0.frame.contains(cursor) }) ?? NSScreen.main,
              let superview else { return }
        let onScreen = BrowserCuePreviewGeometry.frame(under: cursor, visibleFrame: screen.visibleFrame)
        // The notch panel already reserves its max (voice-mode) size, so this
        // only needs a coordinate-space translation, never a window resize.
        let windowOrigin = hostWindow.convertPoint(fromScreen: onScreen.origin)
        frame = CGRect(origin: superview.convert(windowOrigin, from: nil), size: onScreen.size)
    }

    func hideCuePreview() {
        isHidden = true
        thumbnail.image = nil
    }
}
