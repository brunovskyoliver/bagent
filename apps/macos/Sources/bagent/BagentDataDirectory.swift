import Foundation

/// Resolves persistent bagent state without allowing development worktrees to
/// share migration databases, daemon credentials, or runtime markers.
enum BagentDataDirectory {
    static let environmentKey = "BAGENT_DATA_DIR"

    static var url: URL {
        if let override = ProcessInfo.processInfo.environment[environmentKey], !override.isEmpty {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        return url(forBundleURL: Bundle.main.bundleURL)
    }

    static func url(forBundleURL bundleURL: URL) -> URL {
        let applicationSupport = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first!
        let shared = applicationSupport.appendingPathComponent("bagent", isDirectory: true)

        guard let identifier = worktreeIdentifier(forBundleURL: bundleURL) else {
            return shared
        }
        return shared
            .appendingPathComponent("worktrees", isDirectory: true)
            .appendingPathComponent(identifier, isDirectory: true)
    }

    static func worktreeIdentifier(forBundleURL bundleURL: URL) -> String? {
        let bundleURL = bundleURL.standardizedFileURL
        guard bundleURL.pathExtension == "app" else { return nil }

        let macosDirectory = bundleURL.deletingLastPathComponent()
        let appsDirectory = macosDirectory.deletingLastPathComponent()
        guard macosDirectory.lastPathComponent == "macos",
              appsDirectory.lastPathComponent == "apps" else {
            return nil
        }

        let worktreeRoot = appsDirectory.deletingLastPathComponent()
        return stableIdentifier(for: worktreeRoot.path)
    }

    private static func stableIdentifier(for value: String) -> String {
        // FNV-1a is sufficient here: this is a directory namespace, not a
        // security boundary. The full path remains the source of truth.
        var hash: UInt64 = 14_695_981_039_346_656_037
        for byte in value.utf8 {
            hash ^= UInt64(byte)
            hash &*= 1_099_511_628_211
        }
        return String(format: "%016llx", hash)
    }
}
