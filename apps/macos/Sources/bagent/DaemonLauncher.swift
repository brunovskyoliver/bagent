import Foundation
import Darwin

/// Builds the LaunchAgent plist for the daemon. Pure — unit-testable.
enum DaemonLaunchAgent {
    static let label = "com.bagent.daemon"

    static var plistURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents/\(label).plist")
    }

    static var logURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/bagent/daemon.log")
    }

    /// launchd property list. KeepAlive on crash only: a clean exit (explicit
    /// shutdown, `launchctl bootout`) stays down; a crash is restarted by
    /// launchd with its default throttle.
    static func plistContent(binaryPath: String, environment: [String: String]) -> String {
        let envEntries = environment
            .sorted { $0.key < $1.key }
            .map { "        <key>\($0.key)</key>\n        <string>\($0.value)</string>" }
            .joined(separator: "\n")
        return """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>Label</key>
            <string>\(label)</string>
            <key>ProgramArguments</key>
            <array>
                <string>\(binaryPath)</string>
            </array>
            <key>EnvironmentVariables</key>
            <dict>
        \(envEntries)
            </dict>
            <key>RunAtLoad</key>
            <true/>
            <key>KeepAlive</key>
            <dict>
                <key>SuccessfulExit</key>
                <false/>
            </dict>
            <key>StandardOutPath</key>
            <string>\(logURL.path)</string>
            <key>StandardErrorPath</key>
            <string>\(logURL.path)</string>
        </dict>
        </plist>
        """
    }
}

/// Dedicated BaseRT runtime for bagent. Port 8080 remains reserved for the
/// user's independent BaseRT/Claude-local service.
enum BaseRTLaunchAgent {
    static let label = "com.bagent.basert"
    static let model = "basecompute/Qwen3-4B-Instruct-2507"
    static let synthesisModel = "basecompute/Qwen3.6-35B-A3B"
    static let apiKey = "basert-local"
    static let port = 8082
    static let idleTimeoutSeconds = 20 * 60

    static var plistURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/LaunchAgents/\(label).plist")
    }

    static var logURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/bagent/basert.log")
    }

    static var modelRegistryURL: URL {
        FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first!
            .appendingPathComponent("bagent/basert-models", isDirectory: true)
    }

    static func cachedModelURL(_ modelID: String) -> URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Caches/baseRT/models")
            .appendingPathComponent(modelID)
            .appendingPathComponent("default-q4/model.base")
    }

    static func plistContent(
        binaryPath: String,
        modelDirectory: String = modelRegistryURL.path
    ) -> String {
        let arguments = [
            binaryPath,
            "serve",
            "--model-dir", modelDirectory,
            "--host", "127.0.0.1",
            "--port", String(port),
            "--api-key", apiKey,
            "--idle-timeout", String(idleTimeoutSeconds),
            "--max-context", "4096",
            "--kv-bits", "4",
            "--max-tokens", "2048",
            "--max-batch-size", "1",
            "--request-timeout", "300000",
            "--verbose",
        ]
        let argumentXML = arguments
            .map { "            <string>\($0)</string>" }
            .joined(separator: "\n")
        return """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>Label</key>
            <string>\(label)</string>
            <key>ProgramArguments</key>
            <array>
        \(argumentXML)
            </array>
            <key>RunAtLoad</key>
            <true/>
            <key>KeepAlive</key>
            <dict>
                <key>SuccessfulExit</key>
                <false/>
            </dict>
            <key>StandardOutPath</key>
            <string>\(logURL.path)</string>
            <key>StandardErrorPath</key>
            <string>\(logURL.path)</string>
        </dict>
        </plist>
        """
    }
}

/// Manages daemon residency via a per-user launchd agent.
///
/// Lifecycle contract:
/// - App launch reinstalls the LaunchAgent (current binary path + model env)
///   and restarts the daemon — deterministic upgrade/packaging behavior.
/// - App exit leaves the daemon running so scheduled automations continue.
/// - Crashes are restarted by launchd (`KeepAlive.SuccessfulExit=false`).
/// - Explicit shutdown is `shutdownDaemon()` (launchctl bootout).
/// - Port/token discovery is unchanged: the daemon itself writes
///   `daemon.port` / `daemon.token` / `daemon.pid` in Application Support.
@MainActor
final class DaemonLauncher {
    func launch() {
        guard let url = findBinary() else {
            print("[bagentd] binary not found — run `cargo build` first")
            return
        }
        terminateLegacyChildDaemon()
        Task {
            async let baseRT: Void = ensureBaseRTRunning()
            await installAndStart(binary: url)
            await baseRT
        }
    }

    /// Intentionally does not stop the daemon: scheduled automations keep
    /// running after the notch app exits.
    func stop() {}

    /// Explicit shutdown — unloads the agent; launchd will not restart it.
    func shutdownDaemon() {
        _ = runLaunchctl(["bootout", "gui/\(getuid())/\(DaemonLaunchAgent.label)"])
        _ = runLaunchctl(["bootout", "gui/\(getuid())/\(BaseRTLaunchAgent.label)"])
    }

    // MARK: - launchd

    private func installAndStart(binary: URL) async {
        let env = [
            "BAGENT_BASERT_BASE_URL": "http://127.0.0.1:8082/v1",
            "BAGENT_BASERT_API_KEY": BaseRTLaunchAgent.apiKey,
            "BAGENT_BASERT_LOG_PATH": BaseRTLaunchAgent.logURL.path,
            "BAGENT_DEFAULT_MODEL": BaseRTLaunchAgent.model,
            "BAGENT_CLASSIFIER_MODEL": BaseRTLaunchAgent.model,
            "BAGENT_SYNTHESIS_MODEL_PATH":
                BaseRTLaunchAgent.cachedModelURL(BaseRTLaunchAgent.synthesisModel).path,
            "BAGENT_SYNTHESIS_FALLBACK_MODEL_PATH":
                BaseRTLaunchAgent.cachedModelURL(BaseRTLaunchAgent.model).path,
        ]
        let plist = DaemonLaunchAgent.plistContent(binaryPath: binary.path, environment: env)
        let plistURL = DaemonLaunchAgent.plistURL
        do {
            try FileManager.default.createDirectory(
                at: plistURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try FileManager.default.createDirectory(
                at: DaemonLaunchAgent.logURL.deletingLastPathComponent(), withIntermediateDirectories: true)
            try plist.write(to: plistURL, atomically: true, encoding: .utf8)
        } catch {
            print("[bagentd] failed to write LaunchAgent: \(error)")
            return
        }
        // Deterministic restart into the current binary + env on every app
        // launch (same semantics the app had when it owned the process).
        let previousPID = launchdManagedPid()
        _ = runLaunchctl(["bootout", "gui/\(getuid())/\(DaemonLaunchAgent.label)"])
        if let previousPID {
            for _ in 0..<60 {
                if kill(previousPID, 0) != 0 { break }
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
        let status = await bootstrapLaunchAgent(plistURL: plistURL)
        print(status == 0
            ? "[bagentd] launchd agent bootstrapped"
            : "[bagentd] launchctl bootstrap failed (\(status))")
    }

    @discardableResult
    private func runLaunchctl(_ args: [String]) -> Int32 {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = args
        p.standardOutput = FileHandle.nullDevice
        p.standardError = FileHandle.nullDevice
        do {
            try p.run()
            p.waitUntilExit()
            return p.terminationStatus
        } catch {
            print("[bagentd] launchctl \(args.first ?? "") failed: \(error)")
            return -1
        }
    }

    private func bootstrapLaunchAgent(plistURL: URL) async -> Int32 {
        var status: Int32 = -1
        for attempt in 1...3 {
            status = runLaunchctl(["bootstrap", "gui/\(getuid())", plistURL.path])
            if status == 0 { return status }
            if attempt < 3 {
                try? await Task.sleep(for: .milliseconds(750))
            }
        }
        return status
    }

    /// Pre-launchd app versions spawned bagentd as a child and recorded its
    /// pid. Kill such a daemon once so launchd becomes the single owner.
    private func terminateLegacyChildDaemon() {
        let pidURL = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask)
            .first!
            .appendingPathComponent("bagent")
            .appendingPathComponent("daemon.pid")
        guard let raw = try? String(contentsOf: pidURL, encoding: .utf8),
              let pid = Int32(raw.trimmingCharacters(in: .whitespacesAndNewlines)),
              pid > 0 else { return }
        // Only kill daemons that are NOT launchd-managed (launchd-managed pids
        // are listed under our label).
        if launchdManagedPid() == pid { return }
        if kill(pid, 0) == 0 {
            print("[bagentd] terminating legacy child daemon pid \(pid)")
            kill(pid, SIGTERM)
            Thread.sleep(forTimeInterval: 0.25)
        }
        try? FileManager.default.removeItem(at: pidURL)
    }

    /// Current daemon pid according to launchd, or nil when not running.
    private func launchdManagedPid() -> Int32? {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = ["print", "gui/\(getuid())/\(DaemonLaunchAgent.label)"]
        let pipe = Pipe()
        p.standardOutput = pipe
        p.standardError = FileHandle.nullDevice
        guard (try? p.run()) != nil else { return nil }
        let data = pipe.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        guard p.terminationStatus == 0,
              let out = String(data: data, encoding: .utf8) else { return nil }
        for line in out.split(separator: "\n") {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("pid = "), let pid = Int32(trimmed.dropFirst(6)) {
                return pid
            }
        }
        return nil
    }

    // MARK: - BaseRT autostart

    private func ensureBaseRTRunning() async {
        do {
            try prepareBaseRTModelRegistry()
        } catch {
            print("[BaseRT] failed to prepare model registry: \(error)")
            return
        }
        if await isBaseRTReady() {
            print("[BaseRT] bagent runtime already ready on port \(BaseRTLaunchAgent.port)")
            return
        }
        guard let binary = findBaseRTBinary() else {
            print("[BaseRT] binary not found — install from https://basecompute.co")
            return
        }

        let plist = BaseRTLaunchAgent.plistContent(binaryPath: binary.path)
        do {
            try FileManager.default.createDirectory(
                at: BaseRTLaunchAgent.plistURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try FileManager.default.createDirectory(
                at: BaseRTLaunchAgent.logURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try plist.write(
                to: BaseRTLaunchAgent.plistURL,
                atomically: true,
                encoding: .utf8
            )
        } catch {
            print("[BaseRT] failed to write LaunchAgent: \(error)")
            return
        }

        _ = runLaunchctl(["bootout", "gui/\(getuid())/\(BaseRTLaunchAgent.label)"])
        // BaseRT may spend several seconds draining/unmapping model weights after
        // launchd removes the old job. Wait for the dedicated port to close so
        // the replacement cannot lose a bind race and disappear.
        if let healthURL = URL(string: "http://127.0.0.1:8082/health") {
            for _ in 0..<60 {
                if await authenticatedGet(healthURL) == nil { break }
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
        let status = await bootstrapLaunchAgent(plistURL: BaseRTLaunchAgent.plistURL)
        guard status == 0 else {
            print("[BaseRT] launchctl bootstrap failed (\(status))")
            return
        }

        // Service readiness is independent of optional model availability.
        // The daemon starts in parallel; its runtime manager owns bounded
        // preferred-model readiness and fallback.
        for attempt in 1...20 {
            try? await Task.sleep(for: .milliseconds(500))
            if await isBaseRTReady() {
                print("[BaseRT] ready after \(attempt) poll(s)")
                return
            }
        }
        print("[BaseRT] did not become ready within 10 seconds; see \(BaseRTLaunchAgent.logURL.path)")
    }

    private func prepareBaseRTModelRegistry() throws {
        let registry = BaseRTLaunchAgent.modelRegistryURL
        try FileManager.default.createDirectory(
            at: registry,
            withIntermediateDirectories: true
        )
        for legacyName in ["fallback-4b.base", "synthesis-35b.base"] {
            let legacyLink = registry.appendingPathComponent(legacyName)
            if FileManager.default.fileExists(atPath: legacyLink.path)
                || (try? FileManager.default.attributesOfItem(atPath: legacyLink.path)) != nil {
                try FileManager.default.removeItem(at: legacyLink)
            }
        }
        for modelID in [
            BaseRTLaunchAgent.model,
            BaseRTLaunchAgent.synthesisModel,
        ] {
            let sourceDirectory = BaseRTLaunchAgent.cachedModelURL(modelID)
                .deletingLastPathComponent()
            let modelSource = sourceDirectory.appendingPathComponent("model.base")
            let metadataSource = sourceDirectory.appendingPathComponent("hub.json")
            guard FileManager.default.fileExists(atPath: modelSource.path),
                  FileManager.default.fileExists(atPath: metadataSource.path) else {
                print("[BaseRT] cached model unavailable: \(modelID)")
                continue
            }
            let destinationDirectory = registry
                .appendingPathComponent(modelID)
                .appendingPathComponent("default-q4")
            try FileManager.default.createDirectory(
                at: destinationDirectory,
                withIntermediateDirectories: true
            )
            for (source, filename) in [
                (modelSource, "model.base"),
                (metadataSource, "hub.json"),
            ] {
                let link = destinationDirectory.appendingPathComponent(filename)
                if let destination = try? FileManager.default.destinationOfSymbolicLink(
                    atPath: link.path
                ), destination == source.path {
                    continue
                }
                if FileManager.default.fileExists(atPath: link.path)
                    || (try? FileManager.default.attributesOfItem(atPath: link.path)) != nil {
                    try FileManager.default.removeItem(at: link)
                }
                try FileManager.default.createSymbolicLink(
                    at: link,
                    withDestinationURL: source
                )
            }
        }
    }

    private func isBaseRTReady() async -> Bool {
        guard let healthURL = URL(string: "http://127.0.0.1:8082/health"),
              let modelsURL = URL(string: "http://127.0.0.1:8082/v1/models") else {
            return false
        }
        guard await authenticatedGet(healthURL) != nil,
              let modelData = await authenticatedGet(modelsURL),
              let value = try? JSONSerialization.jsonObject(with: modelData) as? [String: Any],
              let models = value["data"] as? [[String: Any]] else {
            return false
        }
        return models.allSatisfy { $0["id"] is String }
    }

    private func authenticatedGet(_ url: URL) async -> Data? {
        var req = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 1)
        req.httpMethod = "GET"
        req.setValue("Bearer \(BaseRTLaunchAgent.apiKey)", forHTTPHeaderField: "Authorization")
        guard let (data, response) = try? await URLSession.shared.data(for: req),
              (response as? HTTPURLResponse)?.statusCode == 200 else {
            return nil
        }
        return data
    }

    private func findBaseRTBinary() -> URL? {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let candidates = [
            "\(home)/.basert/basert",
            "\(home)/.local/bin/basert",
            "/opt/homebrew/bin/basert",
            "/usr/local/bin/basert",
        ]
        for path in candidates {
            let url = URL(fileURLWithPath: path)
            if FileManager.default.isExecutableFile(atPath: url.path) { return url }
        }
        return nil
    }

    // MARK: - Binary discovery

    private func findBinary() -> URL? {
        if let execURL = Bundle.main.executableURL {
            let bundled = execURL.deletingLastPathComponent().appendingPathComponent("bagentd")
            if FileManager.default.fileExists(atPath: bundled.path) { return bundled }
        }
        let dev = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("target/debug/bagentd")
        if FileManager.default.fileExists(atPath: dev.path) { return dev }
        return nil
    }
}
