import Foundation
import WebKit

@MainActor
final class BrowserProfile {
    static let identifier = UUID(uuidString: "7D3C9E70-0AE5-4C3B-B9D4-4F3235C426CF")!

    let dataStore: WKWebsiteDataStore

    init(identifier: UUID = BrowserProfile.identifier) {
        dataStore = WKWebsiteDataStore(forIdentifier: identifier)
    }

    var isPersistent: Bool { dataStore.isPersistent }

    func clear() async {
        await withCheckedContinuation { continuation in
            dataStore.removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast) {
                continuation.resume()
            }
        }
    }
}
