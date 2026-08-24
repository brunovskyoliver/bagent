import Foundation

@MainActor
final class BrowserAuditLog {
    private(set) var entries: [BrowserAuditEntry] = []
    private let maximumEntries = 256

    func record(connectionLabel: String, tool: String, origin: String?, resultClass: String) {
        entries.append(BrowserAuditEntry(
            timestamp: Date(),
            connectionLabel: String(connectionLabel.prefix(80)),
            tool: String(tool.prefix(80)),
            origin: origin.map { BrowserAuditLog.redactedOrigin($0) },
            resultClass: String(resultClass.prefix(80))
        ))
        if entries.count > maximumEntries {
            entries.removeFirst(entries.count - maximumEntries)
        }
    }

    private static func redactedOrigin(_ value: String) -> String {
        guard var components = URLComponents(string: value) else { return "[invalid-origin]" }
        components.query = nil
        components.fragment = nil
        return components.string ?? "[invalid-origin]"
    }
}
