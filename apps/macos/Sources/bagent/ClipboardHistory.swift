import AppKit
import SwiftUI

// MARK: - Clipboard item

/// One captured pasteboard state — raw representations kept so a repaste is
/// lossless (target app picks its preferred type, same as a real paste).
struct ClipboardItem: Identifiable, Equatable {
    enum Kind: Equatable {
        case text(preview: String)
        case image(thumbnail: NSImage)
        case files(urls: [URL])
    }

    let id = UUID()
    let kind: Kind
    /// Raw pasteboard representations per item: [(type, data)].
    let representations: [[(NSPasteboard.PasteboardType, Data)]]
    let capturedAt = Date()

    static func == (lhs: ClipboardItem, rhs: ClipboardItem) -> Bool {
        lhs.id == rhs.id
    }

    /// Cheap content fingerprint for consecutive-duplicate detection.
    var fingerprint: Int {
        var hasher = Hasher()
        for item in representations {
            for (type, data) in item {
                hasher.combine(type.rawValue)
                hasher.combine(data.count)
                hasher.combine(data.prefix(256))
            }
        }
        return hasher.finalize()
    }

    /// Short human label for accessibility / debug.
    var label: String {
        switch kind {
        case .text(let preview):    return preview
        case .image:                return "Obrázok"
        case .files(let urls):
            return urls.first.map { $0.lastPathComponent } ?? "Súbory"
        }
    }
}

// MARK: - History store

/// In-memory ring buffer of the 5 most recent pasteboard states.
/// Never persisted to disk — clipboard content is the most sensitive data on
/// the machine.
@MainActor
final class ClipboardHistory: ObservableObject {
    static let capacity = 5
    /// Per-representation size cap (~10MB). Oversized reps are dropped; if an
    /// item ends up with no representations it is skipped entirely.
    private static let maxRepresentationBytes = 10 * 1024 * 1024
    /// Password managers mark transient/secret content with this type.
    private static let concealedType = NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType")
    private static let transientType = NSPasteboard.PasteboardType("org.nspasteboard.TransientType")

    @Published private(set) var items: [ClipboardItem] = []

    private var timer: Timer?
    private var lastChangeCount = NSPasteboard.general.changeCount
    /// changeCount produced by our own writeToPasteboard — the poller skips it
    /// (the item was promoted in-place, re-capturing would duplicate it).
    private var selfWriteChangeCount: Int?

    func start() {
        guard timer == nil else { return }
        // Capture whatever is on the pasteboard right now as slot 1.
        lastChangeCount = -1
        poll()
        let t = Timer.scheduledTimer(withTimeInterval: 0.3, repeats: true) { [weak self] _ in
            Task { @MainActor [weak self] in self?.poll() }
        }
        t.tolerance = 0.1
        timer = t
    }

    func stop() {
        timer?.invalidate()
        timer = nil
    }

    private func poll() {
        let pb = NSPasteboard.general
        let count = pb.changeCount
        guard count != lastChangeCount else { return }
        lastChangeCount = count
        if count == selfWriteChangeCount {
            selfWriteChangeCount = nil
            return
        }
        guard let item = Self.capture(from: pb) else { return }
        if let head = items.first, head.fingerprint == item.fingerprint { return }
        items.insert(item, at: 0)
        if items.count > Self.capacity { items.removeLast(items.count - Self.capacity) }
    }

    /// Snapshot the pasteboard into a ClipboardItem. Returns nil for concealed
    /// content, empty pasteboards, or items whose every representation is oversized.
    private static func capture(from pb: NSPasteboard) -> ClipboardItem? {
        guard let pbItems = pb.pasteboardItems, !pbItems.isEmpty else { return nil }

        var reps: [[(NSPasteboard.PasteboardType, Data)]] = []
        for pbItem in pbItems {
            if pbItem.types.contains(concealedType) || pbItem.types.contains(transientType) {
                return nil
            }
            var itemReps: [(NSPasteboard.PasteboardType, Data)] = []
            for type in pbItem.types {
                guard let data = pbItem.data(forType: type),
                      data.count <= maxRepresentationBytes else { continue }
                itemReps.append((type, data))
            }
            if !itemReps.isEmpty { reps.append(itemReps) }
        }
        guard !reps.isEmpty else { return nil }
        guard let kind = classify(pb: pb) else { return nil }
        return ClipboardItem(kind: kind, representations: reps)
    }

    private static func classify(pb: NSPasteboard) -> ClipboardItem.Kind? {
        if let urls = pb.readObjects(forClasses: [NSURL.self],
                                     options: [.urlReadingFileURLsOnly: true]) as? [URL],
           !urls.isEmpty {
            return .files(urls: urls)
        }
        if let image = NSImage(pasteboard: pb) {
            return .image(thumbnail: downscale(image, maxDimension: 96))
        }
        if let string = pb.string(forType: .string) {
            let collapsed = string
                .trimmingCharacters(in: .whitespacesAndNewlines)
                .replacingOccurrences(of: "\n", with: " ⏎ ")
            let preview = collapsed.count > 60
                ? String(collapsed.prefix(60)) + "…"
                : collapsed
            return .text(preview: preview.isEmpty ? "(prázdny text)" : preview)
        }
        return nil
    }

    /// Small preview thumbnail rendered once at capture time so the wheel never
    /// holds a full-size decode.
    private static func downscale(_ image: NSImage, maxDimension: CGFloat) -> NSImage {
        let size = image.size
        guard size.width > 0, size.height > 0 else { return image }
        let scale = min(1, maxDimension / max(size.width, size.height))
        guard scale < 1 else { return image }
        let target = NSSize(width: size.width * scale, height: size.height * scale)
        let thumb = NSImage(size: target)
        thumb.lockFocus()
        image.draw(in: NSRect(origin: .zero, size: target),
                   from: .zero, operation: .copy, fraction: 1)
        thumb.unlockFocus()
        return thumb
    }

    // MARK: - Paste support

    /// Move an item to slot 1 ("whatever I last pasted is what ⌘V gives me next").
    func promote(_ item: ClipboardItem) {
        guard let index = items.firstIndex(of: item), index != 0 else { return }
        items.remove(at: index)
        items.insert(item, at: 0)
    }

    /// Write the item's raw representations back to the general pasteboard.
    /// The poller ignores this write.
    func writeToPasteboard(_ item: ClipboardItem) {
        let pb = NSPasteboard.general
        pb.clearContents()
        var pbItems: [NSPasteboardItem] = []
        for reps in item.representations {
            let pbItem = NSPasteboardItem()
            for (type, data) in reps {
                pbItem.setData(data, forType: type)
            }
            pbItems.append(pbItem)
        }
        pb.writeObjects(pbItems)
        selfWriteChangeCount = pb.changeCount
        lastChangeCount = pb.changeCount
    }
}
